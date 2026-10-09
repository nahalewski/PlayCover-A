/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Dynamic linker.
//!
//! iPhone OS's dynamic linker, `dyld`, is the namesake of this module.
//!
//! This is where the magic of "high-level emulation" can begin to happen.
//! The guest app will reference various functions, constants, classes etc from
//! iPhone OS's system frameworks and other dynamically-linked libraries, but
//! instead of actually loading and linking the original framework binaries,
//! this "dynamic linker" will generate appropriate stubs for calling into
//! touchHLE's own implementations of the frameworks, which are "host code"
//! (i.e. not themselves running under emulation).
//!
//! This also does normal dynamic linking for libgcc, libstdc++, etc.
//!
//! See [crate::mach_o] for resources.

mod dylib_list;

use crate::abi::{CallFromGuest, GuestFunction};
use crate::cpu::Cpu;
use crate::frameworks::foundation::ns_string;
use crate::mach_o::{MachO, SectionType};
use crate::mem::{ConstVoidPtr, GuestUSize, Mem, MutPtr, MutVoidPtr, Ptr};
use crate::objc::{nil, ClassExports, ObjC};
use crate::Environment;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

pub use dylib_list::DYLIB_LIST;

/// Struct used to expose a host implementation of a dynamic library (usually a
/// framework) to the linker.
///
/// Each module that wants to expose a library to guest code should export a
/// constant using this type, which collects all the relevant [ClassExports],
/// [ConstantExports] and [FunctionExports] for the library. For example:
///
/// ```ignore
/// pub const DYLIB: HostDylib = HostDylib {
///     path: "/System/Library/Frameworks/FooBarKit.framework/FooBarKit",
///     aliases: &[],
///     class_exports: &[baz::CLASSES],
///     constant_exports: &[qux::CONSTANTS],
///     function_exports: &[qux::FUNCTIONS, baz::FUNCTIONS],
/// };
/// ```
///
/// The `path` should be the canonical notional filesystem path that the library
/// is referenced by on the real OS, for example `"/usr/lib/libobjc.A.dylib"`
/// or `"/System/Library/Frameworks/Foundation.framework/Foundation"`. For
/// libraries that have several symlinked paths, non-canonical alternate
/// paths can be listed under `aliases`, for example `"/usr/lib/libobjc.dylib"`.
pub struct HostDylib {
    pub path: &'static str,
    pub aliases: &'static [&'static str],
    pub class_exports: &'static [ClassExports],
    pub constant_exports: &'static [ConstantExports],
    pub function_exports: &'static [FunctionExports],
}

pub type HostFunction = &'static dyn CallFromGuest;

/// Type for lists of functions exported by host implementations of dynamic
/// libraries (usually frameworks).
///
/// Each module that wants to expose functions to guest code should export a
/// constant using this type, e.g.:
///
/// ```ignore
/// pub const FUNCTIONS: FunctionExports = &[
///    ("_NSFoo", &/* ... */),
///    ("_NSBar", &/* ... */),
///    /* ... */
/// ];
/// ```
///
/// All the constants like this can then be collected into a [HostDylib].
///
/// The strings are the mangled symbol names. For C functions, this is just the
/// name prefixed with an underscore.
///
/// For convenience, use [export_c_func]:
///
/// ```ignore
/// pub const FUNCTIONS: FunctionExports = &[
///     export_c_func!(NSFoo(_, _)),
///     export_c_func!(NSBar()),
/// ];
/// ```
///
/// See also [ConstantExports] and [ClassExports].
pub type FunctionExports = &'static [(&'static str, HostFunction)];

/// Macro for exporting a function with C-style name mangling. See
/// [FunctionExports].
///
/// ```ignore
/// export_c_func!(NSFoo(_, _))
/// ```
///
/// will desugar to:
///
/// ```ignore
/// ("_NSFoo", &(NSFoo as (&mut Environment, _, _) -> _))
/// ```
///
/// The function needs to be explicitly casted because a bare function reference
/// defaults to a different type than a pure fn pointer, which is the type that
/// [CallFromGuest] is implemented on. This macro will do the casting for you,
/// but you will need to supply an underscore for each parameter.
#[macro_export]
macro_rules! export_c_func {
    ($name:ident ($($_:ty),*)) => {
        (
            concat!("_", stringify!($name)),
            &($name as fn(&mut $crate::Environment, $($_),*) -> _)
        )
    };
}
pub use crate::export_c_func; // #[macro_export] is weird...

/// Other variant of [export_c_func] macro, allowing to define an alias
/// for the exporting function. This is useful then alias may contain
/// characters not normally allowed for Rust function's names. (e.g. `$`)
#[macro_export]
macro_rules! export_c_func_aliased {
    ($alias:literal, $name:ident ($($_:ty),*)) => {
        (
            concat!("_", $alias),
            &($name as fn(&mut $crate::Environment, $($_),*) -> _)
        )
    };
}
pub use crate::export_c_func_aliased; // #[macro_export] is weird...

/// Type for describing a constant (C `extern const` symbol) that will be
/// created by the linker if the guest app references it. See [ConstantExports].
pub enum HostConstant {
    NSString(&'static str),
    NullPtr,
    Custom(fn(&mut Environment) -> ConstVoidPtr),
}

/// Type for lists of constants exported by host implementations of  dynamic
/// libraries (usually frameworks).
///
/// Each module that wants to expose functions to guest code should export a
/// constant using this type, e.g.:
///
/// ```ignore
/// pub const CONSTANT: ConstantExports = &[
///    ("_kNSFooBar", HostConstant::NSString("NSFooBar")),
///    /* ... */
/// ];
/// ```
///
/// All the constants like this can then be collected into a [HostDylib].
///
/// The strings are the mangled symbol names. For C constants, this is just the
/// name prefixed with an underscore.
///
/// See also [FunctionExports], [ClassExports].
pub type ConstantExports = &'static [(&'static str, HostConstant)];

/// Search the list of [HostDylib]s for a class/constant/function by its symbol.
///
/// Example usage: `search_host_dylibs(|dylib| dylib.function_exports, "_foo")`
pub fn search_host_dylibs<T, F>(get_exports: F, symbol: &str) -> Option<&'static (&'static str, T)>
where
    F: Fn(&HostDylib) -> &'static [&'static [(&'static str, T)]],
{
    // TODO: In general, we should rarely if ever need to search the full set
    //       of dylibs for a symbol. Now that we know which symbols belong to
    //       which libraries, we should at least only search libraries that are
    //       referenced by the app and currently "loaded". We probably should
    //       also implement the Mach-O two-level symbol namespacing eventually.
    DYLIB_LIST
        .iter()
        .copied()
        .map(get_exports)
        .find_map(|lists| search_lists(lists, symbol))
}

/// Helper for working with [ClassExports]/[ConstantExports]/[FunctionExports].
fn search_lists<T>(
    lists: &'static [&'static [(&'static str, T)]],
    symbol: &str,
) -> Option<&'static (&'static str, T)> {
    lists
        .iter()
        .flat_map(|&n| n)
        .find(|&(sym, _)| *sym == symbol)
}

fn encode_a32_svc(imm: u32) -> u32 {
    assert!(imm & 0xff000000 == 0);
    imm | 0xef000000
}
fn encode_a32_ret() -> u32 {
    0xe12fff1e
}
fn encode_a32_trap() -> u32 {
    0xe7ffdefe
}

fn write_return_to_host_routine(mem: &mut Mem, svc: u32) -> GuestFunction {
    let routine = [
        encode_a32_svc(svc),
        // When a return-to-host occurs, it's the host's responsibility
        // to reset the PC to somewhere else. So something has gone
        // wrong if this is executed.
        encode_a32_trap(),
    ];
    let ptr: MutPtr<u32> = mem.alloc(4 * 2).cast();
    mem.write(ptr + 0, routine[0]);
    mem.write(ptr + 1, routine[1]);
    let ptr = GuestFunction::from_addr_with_thumb_bit(ptr.to_bits());
    assert!(!ptr.is_thumb());
    ptr
}
/// Debugging aid, set from `--ignore-unknown-selectors`: when true, calling a C
/// function touchHLE doesn't implement logs a warning and returns 0 instead of
/// crashing. Like that option, this hides real bugs.
pub static IGNORE_UNIMPLEMENTED_FUNCTIONS: AtomicBool = AtomicBool::new(false);

/// Set by `--trace-messages`: also log every call to an already-linked host
/// (C) function, not just ObjC messages.
pub static TRACE_HOST_CALLS: AtomicBool = AtomicBool::new(false);

/// Stand-in for unimplemented functions while [IGNORE_UNIMPLEMENTED_FUNCTIONS]
/// is set: does nothing and returns 0.
fn ignored_unimplemented_function(_env: &mut Environment) -> i32 {
    0
}

pub struct Dyld {
    /// List of host functions that have been "linked" and had SVCs assigned.
    ///
    /// The `&'static str` part here is purely for debugging and could be
    /// removed in release builds if it's ever necessary.
    linked_host_functions: Vec<(&'static str, HostFunction)>,
    return_to_host_routine: Option<GuestFunction>,
    thread_exit_routine: Option<GuestFunction>,
    constants_to_link_later: Vec<(MutPtr<ConstVoidPtr>, &'static HostConstant)>,
    /// Unknown data symbols that look like NSString constants (notification
    /// names, dictionary keys...): linked to a string holding the symbol name.
    guessed_string_constants: Vec<(MutPtr<ConstVoidPtr>, String)>,
    non_lazy_host_functions: HashMap<&'static str, GuestFunction>,
    /// Unimplemented functions already warned about (see
    /// [IGNORE_UNIMPLEMENTED_FUNCTIONS]), so each is only logged once.
    warned_unimplemented: std::collections::HashSet<String>,
    /// One block of lazy-link trampolines per binary, see
    /// [Self::setup_lazy_linking].
    lazy_trampolines: Vec<LazyTrampolines>,
}

/// Trampolines for the lazy symbol stubs of one binary.
///
/// The stubs in `__symbol_stub*` are left exactly as the compiler emitted them
/// (some apps checksum their own `__TEXT`, e.g. the Airplay/Marmalade loader),
/// and instead each `__la_symbol_ptr` is pointed at an 8-byte trampoline
/// (`svc; bx lr`) in separate guest memory. Trampoline `i` belongs to stub `i`.
struct LazyTrampolines {
    bin_idx: usize,
    start: u32,
    /// Address of the `__la_symbol_ptr` entry each stub loads from.
    la_ptrs: Vec<u32>,
}
const TRAMPOLINE_SIZE: u32 = 8;

impl Dyld {
    /// We reserve this SVC ID for invoking the lazy linker.
    pub const SVC_LAZY_LINK: u32 = 0;
    /// We reserve this SVC ID for the exit routine for spawned threads.
    pub const SVC_THREAD_EXIT: u32 = 1;
    /// We reserve this SVC ID for the special return-to-host routine.
    pub const SVC_RETURN_TO_HOST: u32 = 2;
    /// The range of SVC IDs `SVC_LINKED_FUNCTIONS_BASE..` is used to reference
    /// [Self::linked_host_functions] entries.
    pub const SVC_LINKED_FUNCTIONS_BASE: u32 = Self::SVC_RETURN_TO_HOST + 1;
    /// We reserve this SVC ID for lazy linking and returning right after.
    /// It is also a mask for the linked functions to indicate that an
    /// additional return instruction needs to be manually executed after
    /// handling the SVC.
    pub const SVC_LAZY_LINK_RET_FLAG: u32 = 0x800000;

    const SYMBOL_STUB1_INSTRUCTIONS: [u32; 1] = [0xe59ff000]; // mask this with lowest 12 bits to restore instructions
    const SYMBOL_STUB_INSTRUCTIONS: [u32; 2] = [0xe59fc000, 0xe59cf000];
    const PIC_SYMBOL_STUB_INSTRUCTIONS: [u32; 3] = [0xe59fc004, 0xe08fc00c, 0xe59cf000];

    pub fn new() -> Dyld {
        Dyld {
            linked_host_functions: Vec::new(),
            return_to_host_routine: None,
            thread_exit_routine: None,
            warned_unimplemented: Default::default(),
            constants_to_link_later: Vec::new(),
            guessed_string_constants: Vec::new(),
            non_lazy_host_functions: HashMap::new(),
            lazy_trampolines: Vec::new(),
        }
    }

    pub fn return_to_host_routine(&self) -> GuestFunction {
        self.return_to_host_routine.unwrap()
    }

    pub fn thread_exit_routine(&self) -> GuestFunction {
        self.thread_exit_routine.unwrap()
    }

    /// Do linking-related tasks that need doing right after loading the
    /// binaries.
    pub fn do_initial_linking(&mut self, bins: &[MachO], mem: &mut Mem, objc: &mut ObjC) {
        assert!(self.return_to_host_routine.is_none());
        assert!(self.thread_exit_routine.is_none());
        self.return_to_host_routine =
            Some(write_return_to_host_routine(mem, Self::SVC_RETURN_TO_HOST));
        self.thread_exit_routine = Some(write_return_to_host_routine(mem, Self::SVC_THREAD_EXIT));

        // Currently assuming only the app binary contains Objective-C things.

        objc.register_bin_selectors(&bins[0], mem);
        objc.register_host_selectors(mem);

        for (bin_idx, bin) in bins.iter().enumerate() {
            self.setup_lazy_linking(bin_idx, bin, mem);
            // Must happen before `register_bin_classes`, else superclass
            // pointers will be wrong.
            self.do_non_lazy_linking(bin, bins, mem, objc);
        }

        objc.register_bin_classes(&bins[0], mem);
        objc.register_bin_categories(&bins[0], mem);

        ns_string::register_constant_strings(&bins[0], mem, objc);
    }

    /// Dumps all lazy symbols (functions) referenced by the binary
    /// as JSON to stdout.
    ///
    /// The JSON has the following form:
    /// ```json
    /// {
    ///     "object": "lazy_symbols",
    ///     "symbols": [
    ///         {
    ///             "symbol": ((name of symbol)),
    ///             "linked_to": "host" | "dylib" | null,
    ///             "dylib": ((name of dylib)) | null,
    ///         },
    ///         ...
    ///     ]
    /// }
    /// ```
    pub fn dump_lazy_symbols(
        &mut self,
        bins: &[MachO],
        file: &mut std::fs::File,
    ) -> Result<(), std::io::Error> {
        use std::io::Write;
        // Guest binary is always bin 0.
        let stubs = bins[0].get_section(SectionType::SymbolStubs).unwrap();
        let info = stubs.dyld_indirect_symbol_info.as_ref().unwrap();
        writeln!(
            file,
            "{{\n    \"object\":\"lazy_symbols\",\n    \"symbols\": ["
        )?;

        'sym: for (i, symbol) in info.indirect_undef_symbols.iter().enumerate() {
            // Why doesn't json allow trailing commas...
            let comma = if i == info.indirect_undef_symbols.len() - 1 {
                ""
            } else {
                ","
            };
            let symbol = symbol.as_ref().unwrap();
            if let Some(&(_, _)) = search_host_dylibs(|dylib| dylib.function_exports, symbol) {
                writeln!(
                    file,
                    "        {{ \"symbol\": \"{symbol}\", \"linked_to\": \"host\"}}{comma}"
                )?;
                continue;
            }
            for dylib in bins.iter() {
                if dylib.exported_symbols.contains_key(symbol) {
                    writeln!(
                        file,
                        "        {{ \"symbol\": \"{}\", \"linked_to\": \"dylib\", \"dylib\": \"{}\"}}{}",
                        symbol, dylib.name, comma
                    )?;
                    continue 'sym;
                }
            }
            writeln!(file, "        {{ \"symbol\": \"{symbol}\" }}{comma}")?;
        }
        writeln!(file, "    ]\n}}")
    }

    /// Dumps all non-objc symbols provided by touchHLE.
    ///
    /// The dump format is Objective-C code (with meaningless types) that can be
    /// compiled to generate stub libraries that can be linked against, with
    /// comments providing the paths each library would be installed to.
    /// This is used for building the integration tests.
    pub fn dump_host_symbols(file: &mut std::fs::File) -> Result<(), std::io::Error> {
        use std::io::Write;
        for dylib in DYLIB_LIST {
            writeln!(file, "// {}", dylib.path)?;
            for alias in dylib.aliases {
                writeln!(file, "// {alias}")?;
            }
            for (class_name, _) in dylib.class_exports.iter().copied().flatten() {
                writeln!(file, "@interface {class_name}")?;
                writeln!(file, "@end")?;
                writeln!(file, "@implementation {class_name}")?;
                writeln!(file, "@end")?;
            }
            for (constant_symbol, _) in dylib.constant_exports.iter().copied().flatten() {
                writeln!(file, "int {};", constant_symbol.strip_prefix("_").unwrap())?;
            }
            for (function_symbol, _) in dylib.function_exports.iter().copied().flatten() {
                writeln!(
                    file,
                    "void {}() {{}}",
                    function_symbol.strip_prefix("_").unwrap()
                )?;
            }
        }
        Ok(())
    }

    /// [Self::do_initial_linking] but for when this is the app picker's special
    /// environment with no binary (see [crate::Environment::new_without_app]).
    pub fn do_initial_linking_with_no_bins(&mut self, mem: &mut Mem, objc: &mut ObjC) {
        assert!(self.return_to_host_routine.is_none());
        assert!(self.thread_exit_routine.is_none());
        self.return_to_host_routine =
            Some(write_return_to_host_routine(mem, Self::SVC_RETURN_TO_HOST));
        self.thread_exit_routine = Some(write_return_to_host_routine(mem, Self::SVC_THREAD_EXIT));

        objc.register_host_selectors(mem);
    }

    /// Set up lazy-linking stubs for a loaded binary.
    ///
    /// Dynamic linking of functions on iPhone OS usually happens "lazily",
    /// which means that the linking is delayed until the function is first
    /// called. This is achieved by using stub functions. Instead of calling the
    /// external function directly, the app code will call a stub function, and
    /// that stub will either jump to the dynamic linker (which will link in the
    /// external function and then jump to it), or on subsequent calls, jump
    /// straight to the external function.
    ///
    /// The stubs already exist in the binary and are not modified (an app may
    /// hash its own code, and real dyld doesn't touch them either). Instead,
    /// the `__la_symbol_ptr` entry each stub jumps through is made to point at
    /// a trampoline of ours that invokes the dynamic linker.
    fn setup_lazy_linking(&mut self, bin_idx: usize, bin: &MachO, mem: &mut Mem) {
        let Some(stubs) = bin.get_section(SectionType::SymbolStubs) else {
            return;
        };

        let entry_size = stubs.dyld_indirect_symbol_info.as_ref().unwrap().entry_size;
        assert!(matches!(entry_size, 4 | 12 | 16));
        assert!(stubs.size % entry_size == 0);
        let stub_count = stubs.size / entry_size;

        let start = mem.alloc(stub_count * TRAMPOLINE_SIZE).to_bits();
        let mut la_ptrs = Vec::with_capacity(stub_count as usize);
        for i in 0..stub_count {
            let stub_addr = stubs.addr + i * entry_size;
            let stub: MutPtr<u32> = Ptr::from_bits(stub_addr);

            // two or three A32 instructions (PIC stub needs one more)
            // followed by the address or offset of the corresponding
            // __la_symbol_ptr; or just one `ldr pc, [pc, #imm]`.
            let la_ptr = match entry_size {
                4 => {
                    let instr = mem.read(stub);
                    assert!(
                        instr & 0xff7ff000 == 0xe51ff000, // ldr pc, [pc, #+/-imm12]
                        "Unexpected symbol stub {instr:#x} at {stub_addr:#x}"
                    );
                    let offset = instr & 0xfff;
                    if instr & (1 << 23) != 0 {
                        stub_addr + 8 + offset
                    } else {
                        stub_addr + 8 - offset
                    }
                }
                12 => {
                    for (j, &instr) in Self::SYMBOL_STUB_INSTRUCTIONS.iter().enumerate() {
                        assert!(mem.read(stub + j.try_into().unwrap()) == instr);
                    }
                    mem.read(stub + 2)
                }
                16 => {
                    for (j, &instr) in Self::PIC_SYMBOL_STUB_INSTRUCTIONS.iter().enumerate() {
                        assert!(mem.read(stub + j.try_into().unwrap()) == instr);
                    }
                    stub_addr + mem.read(stub + 3) + 12
                }
                _ => unreachable!(),
            };

            let trampoline_addr = start + i * TRAMPOLINE_SIZE;
            let trampoline: MutPtr<u32> = Ptr::from_bits(trampoline_addr);
            mem.write(trampoline + 0, encode_a32_svc(Self::SVC_LAZY_LINK));
            mem.write(trampoline + 1, encode_a32_ret());
            // Initially this pointed into __stub_helper.
            mem.write(MutPtr::<u32>::from_bits(la_ptr), trampoline_addr);
            la_ptrs.push(la_ptr);
        }
        self.lazy_trampolines.push(LazyTrampolines {
            bin_idx,
            start,
            la_ptrs,
        });
    }

    /// Link non-lazy symbols for a loaded binary.
    ///
    /// These are usually constants, Objective-C classes, or vtable pointers.
    /// Since the linking must be done upfront, we can't in general delay errors
    /// about missing implementations until the point of use. For that reason,
    /// this will spit out a warning to stderr for everything missing, so that
    /// there's at least some indication about why the emulator might crash.
    ///
    /// `bin` is the binary to link non-lazy symbols for, `bins` is the set of
    /// binaries symbols may be looked up in.
    fn do_non_lazy_linking(&mut self, bin: &MachO, bins: &[MachO], mem: &mut Mem, objc: &mut ObjC) {
        let mut unhandled_relocations: HashMap<&str, Vec<u32>> = HashMap::new();
        // Lazy symbol pointers belong to the lazy-link trampolines (see
        // [Self::setup_lazy_linking]) and are resolved when first called, even
        // if the binary also has a bind-at-launch entry for them (as weak
        // symbols such as C++ `operator new` do). Applying such a relocation
        // would add the trampoline address to the target as if it were an
        // addend.
        let lazy_pointers: std::collections::HashSet<u32> = self
            .lazy_trampolines
            .iter()
            .flat_map(|block| block.la_ptrs.iter().copied())
            .collect();
        for &(ptr_ptr, ref name) in &bin.external_relocations {
            if lazy_pointers.contains(&ptr_ptr) {
                continue;
            }
            let ptr_ptr: MutPtr<ConstVoidPtr> = Ptr::from_bits(ptr_ptr);
            // There will be an existing value at the address, which is an
            // offset that should be applied to the external symbol's address.
            // It is often 0, but not always.
            let offset: u32 = mem.read(ptr_ptr).to_bits();
            let target: ConstVoidPtr = if let Some(name) = name.strip_prefix("_OBJC_CLASS_$_") {
                objc.link_class(name, /* is_metaclass: */ false, mem)
                    .cast()
                    .cast_const()
            } else if let Some(name) = name.strip_prefix("_OBJC_METACLASS_$_") {
                objc.link_class(name, /* is_metaclass: */ true, mem)
                    .cast()
                    .cast_const()
            } else if name == "___CFConstantStringClassReference" {
                // See ns_string::register_constant_strings
                nil.cast().cast_const()
            } else if let Some(&external_addr) = bins
                .iter()
                .flat_map(|other_bin| other_bin.exported_symbols.get(name))
                .next()
            {
                // Often used for C++ RTTI
                Ptr::from_bits(external_addr)
            } else if let Some((symbol, _)) =
                search_host_dylibs(|dylib| dylib.function_exports, name)
            {
                // We want the same symbol name to always point to the same
                // function.
                let trampoline_ptr = self
                    .create_proc_address_no_inval(mem, symbol)
                    .unwrap()
                    .to_ptr();
                log_dbg!(
                    "Linked external relocation to host function {} at {:?}",
                    symbol,
                    trampoline_ptr
                );
                trampoline_ptr
            } else if let Some((_, template)) =
                search_host_dylibs(|dylib| dylib.constant_exports, name)
            {
                // Binds of host constants. Those in __nl_symbol_ptr are also
                // handled when reading that section (writing the same value
                // twice is harmless), but others, e.g. the `isa` of a block
                // literal pointing at `_NSConcreteGlobalBlock`, live in other
                // sections and are only reachable here. Linking needs a
                // `&mut Environment`, so it happens later.
                self.constants_to_link_later.push((ptr_ptr, template));
                continue;
            } else {
                unhandled_relocations
                    .entry(name)
                    .or_default()
                    .push(ptr_ptr.to_bits());
                continue;
            };
            // wrapping_add() is used in case the offset is negative. I haven't
            // seen it happen, but it would make sense if that is allowed.
            mem.write(
                ptr_ptr,
                Ptr::from_bits(target.to_bits().wrapping_add(offset)),
            )
        }
        // Collecting unhandled relocations for the same symbol onto one line
        // makes the log output much less spammy.
        for (name, addrs) in unhandled_relocations {
            log!(
                "Warning: unhandled external relocation {:?} in {:?} at {}",
                name,
                bin.name,
                addrs
                    .into_iter()
                    .map(|addr| format!("{addr:#x}"))
                    .collect::<Vec<String>>()
                    .join(", "),
            );
        }

        let Some(ptrs) = bin.get_section(SectionType::NonLazySymbolPointers) else {
            return;
        };
        let info = ptrs.dyld_indirect_symbol_info.as_ref().unwrap();

        let entry_size = info.entry_size;
        assert!(entry_size == 4);
        assert!(ptrs.size % entry_size == 0);
        let ptr_count = ptrs.size / entry_size;
        'ptr_loop: for i in 0..ptr_count {
            let Some(symbol) = info.indirect_undef_symbols[i as usize].as_deref() else {
                continue;
            };

            let ptr_ptr: MutPtr<ConstVoidPtr> = Ptr::from_bits(ptrs.addr + i * entry_size);

            for other_bin in bins {
                if let Some(&addr) = other_bin.exported_symbols.get(symbol) {
                    mem.write(ptr_ptr, Ptr::from_bits(addr));
                    continue 'ptr_loop;
                }
            }

            if let Some((symbol, _)) = search_host_dylibs(|dylib| dylib.function_exports, symbol) {
                // We want the same symbol name to always point to the same
                // function. It could point to a specific stub entry, but it's
                // easier to just create a new function and point all the stub
                // entries to it.
                let trampoline_ptr = self
                    .create_proc_address_no_inval(mem, symbol)
                    .unwrap()
                    .to_ptr();
                mem.write(ptr_ptr, trampoline_ptr);
                log_dbg!(
                    "Linked non-lazy host function {} at {:?}",
                    symbol,
                    trampoline_ptr
                );
                log_dbg!("{:?}", self.non_lazy_host_functions);
                continue;
            }
            if let Some((_, template)) = search_host_dylibs(|dylib| dylib.constant_exports, symbol)
            {
                // Delay linking of constant until we have a `&mut Environment`,
                // that makes it much easier to build NSString objects etc.
                self.constants_to_link_later.push((ptr_ptr, template));
                continue;
            }

            if looks_like_string_constant(symbol) {
                log!(
                    "Warning: non-lazy symbol {:?} at {:?} in \"{}\" is not implemented, guessing it is an NSString constant",
                    symbol,
                    ptr_ptr,
                    bin.name
                );
                self.guessed_string_constants
                    .push((ptr_ptr, symbol.trim_start_matches('_').to_string()));
                continue;
            }
            log!(
                "Warning: unhandled non-lazy symbol {:?} at {:?} in \"{}\"",
                symbol,
                ptr_ptr,
                bin.name
            );
        }

        // FIXME: check for internal relocations?
    }

    /// Do linking that can only be done once there is a full [Environment].
    /// Not to be confused with lazy linking.
    pub fn do_late_linking(env: &mut Environment) {
        // TODO: do symbols ever appear in __nl_symbol_ptr multiple times?

        let to_link = std::mem::take(&mut env.dyld.constants_to_link_later);
        for (symbol_ptr_ptr, template) in to_link {
            let symbol_ptr: ConstVoidPtr = match template {
                HostConstant::NSString(static_str) => {
                    let string_ptr = ns_string::get_static_str(env, static_str);
                    let string_ptr_ptr = env.mem.alloc_and_write(string_ptr);
                    string_ptr_ptr.cast().cast_const()
                }
                HostConstant::NullPtr => {
                    let null_ptr: ConstVoidPtr = Ptr::null();
                    let null_ptr_ptr = env.mem.alloc_and_write(null_ptr);
                    null_ptr_ptr.cast().cast_const()
                }
                HostConstant::Custom(f) => f(env),
            };
            env.mem.write(symbol_ptr_ptr, symbol_ptr.cast());
        }
        let guessed = std::mem::take(&mut env.dyld.guessed_string_constants);
        for (symbol_ptr_ptr, name) in guessed {
            // Leaked on purpose: a handful of short strings for the app's lifetime.
            let string_ptr = ns_string::get_static_str(env, Box::leak(name.into_boxed_str()));
            let string_ptr_ptr = env.mem.alloc_and_write(string_ptr);
            env.mem.write(symbol_ptr_ptr, string_ptr_ptr.cast().cast_const());
        }
    }

    /// Return a host function that can be called to handle an SVC instruction
    /// encountered during CPU emulation. If `None` is returned, the execution
    /// needs to resume at `svc_pc`.
    pub fn get_svc_handler(
        &mut self,
        bins: &[MachO],
        mem: &mut Mem,
        cpu: &mut Cpu,
        svc_pc: u32,
        svc: u32,
    ) -> Option<HostFunction> {
        match svc {
            Self::SVC_LAZY_LINK | Self::SVC_LAZY_LINK_RET_FLAG => {
                self.do_lazy_link(bins, mem, cpu, svc_pc)
            }
            Self::SVC_THREAD_EXIT | Self::SVC_RETURN_TO_HOST => unreachable!(), // don't handle here
            Self::SVC_LINKED_FUNCTIONS_BASE.. => {
                let f = self.linked_host_functions.get(
                    ((svc & !Self::SVC_LAZY_LINK_RET_FLAG) - Self::SVC_LINKED_FUNCTIONS_BASE)
                        as usize,
                );
                let Some(&(symbol, f)) = f else {
                    panic!("Unexpected SVC #{svc} at {svc_pc:#x}");
                };
                if TRACE_HOST_CALLS.load(Ordering::Relaxed) {
                    log!("[C] {} (lr={:#x})", symbol, cpu.regs()[Cpu::LR]);
                }
                log_dbg!("Call to host function, already linked: {}", symbol);
                Some(f)
            }
        }
    }

    fn do_lazy_link(
        &mut self,
        bins: &[MachO],
        mem: &mut Mem,
        cpu: &mut Cpu,
        svc_pc: u32,
    ) -> Option<HostFunction> {
        // Make the trampoline at `svc_pc` and the __la_symbol_ptr jump straight
        // to `target` (A32 or Thumb, indicated by the lowest bit).
        fn link_to_address(mem: &mut Mem, cpu: &mut Cpu, svc_pc: u32, la_ptr: u32, target: u32) {
            let trampoline: MutPtr<u32> = Ptr::from_bits(svc_pc);
            // ldr pc, [pc, #-4]
            mem.write(trampoline + 0, 0xe51ff004);
            mem.write(trampoline + 1, target);
            cpu.invalidate_cache_range(svc_pc, TRAMPOLINE_SIZE);
            mem.write(MutPtr::<u32>::from_bits(la_ptr), target);
        }

        let (block, idx) = self
            .lazy_trampolines
            .iter()
            .find_map(|block| {
                let end = block.start + block.la_ptrs.len() as u32 * TRAMPOLINE_SIZE;
                if (block.start..end).contains(&svc_pc) {
                    Some((block, ((svc_pc - block.start) / TRAMPOLINE_SIZE) as usize))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("Lazy link SVC at unexpected address {svc_pc:#x}"));
        let la_ptr = block.la_ptrs[idx];
        let stubs = bins[block.bin_idx]
            .get_section(SectionType::SymbolStubs)
            .unwrap();
        let info = stubs.dyld_indirect_symbol_info.as_ref().unwrap();

        let symbol = info.indirect_undef_symbols[idx].as_deref().unwrap();

        if let Some(&addr) = self.non_lazy_host_functions.get(symbol) {
            // The host function was already linked non-lazily, point the
            // __la_symbol_ptr to the function. It calls the host function.
            link_to_address(mem, cpu, svc_pc, la_ptr, addr.addr_with_thumb_bit());
            log_dbg!(
                "Linked host function {} at {:#x}/{:#x} to existing stub ({:?}).",
                symbol,
                svc_pc,
                la_ptr,
                addr,
            );
            return None;
        }

        if let Some(&(symbol, f)) = search_host_dylibs(|dylib| dylib.function_exports, symbol) {
            // Allocate an SVC ID for this host function
            let idx: u32 = self.linked_host_functions.len().try_into().unwrap();
            let svc = idx + Self::SVC_LINKED_FUNCTIONS_BASE;
            assert!(svc < Self::SVC_LAZY_LINK_RET_FLAG);
            self.linked_host_functions.push((symbol, f));

            // Rewrite the trampoline to call this host function (the `bx lr`
            // after it is kept).
            let trampoline: MutPtr<u32> = Ptr::from_bits(svc_pc);
            mem.write(trampoline, encode_a32_svc(svc));
            assert!(mem.read(trampoline + 1) == encode_a32_ret());
            cpu.invalidate_cache_range(svc_pc, 4);

            if TRACE_HOST_CALLS.load(Ordering::Relaxed) {
                log!("[C] {} (first call, lr={:#x})", symbol, cpu.regs()[Cpu::LR]);
            }
            log_dbg!("Linked {} at {:#x} to host implementation", symbol, svc_pc);

            // Return the host function so that we can call it now that we're
            // done.
            return Some(f);
        }

        for dylib in bins.iter() {
            if let Some(&addr) = dylib.exported_symbols.get(symbol) {
                link_to_address(mem, cpu, svc_pc, la_ptr, addr);
                log_dbg!(
                    "Linked {} at {:#x}/{:#x} to {:#x} from {}",
                    symbol,
                    svc_pc,
                    la_ptr,
                    addr,
                    dylib.name
                );
                // Tell the caller it needs to restart execution at svc_pc.
                return None;
            }
        }

        if IGNORE_UNIMPLEMENTED_FUNCTIONS.load(Ordering::Relaxed) {
            if self.warned_unimplemented.insert(symbol.to_string()) {
                log!(
                    "Ignoring call to unimplemented function {} (returning 0)",
                    symbol
                );
            }
            return Some(&(ignored_unimplemented_function as fn(&mut Environment) -> i32));
        }

        panic!("Call to unimplemented function {symbol}");
    }

    /// Creates a guest function that will call a host function with the name
    /// `symbol`. This can be used to implement "get proc address" functions.
    /// Note that no attempt is made to deduplicate or deallocate these, so
    /// excessive use would create a memory leak.
    ///
    /// The name must be the mangled symbol name. Returns [Err] if there's no
    /// such function.
    pub fn create_proc_address(
        &mut self,
        mem: &mut Mem,
        cpu: &mut Cpu,
        symbol: &str,
    ) -> Result<GuestFunction, ()> {
        let function_ptr = self.create_proc_address_no_inval(mem, symbol)?;

        // Just in case
        cpu.invalidate_cache_range(function_ptr.addr_without_thumb_bit(), 8);
        Ok(function_ptr)
    }

    /// Internal [Self::create_proc_address] that doesn't invalidate the cache.
    /// For use before a [Cpu] is available.
    fn create_proc_address_no_inval(
        &mut self,
        mem: &mut Mem,
        symbol: &str,
    ) -> Result<GuestFunction, ()> {
        let &(symbol, f) = search_host_dylibs(|dylib| dylib.function_exports, symbol).ok_or(())?;
        if let Some(&cached_fn) = self.non_lazy_host_functions.get(symbol) {
            return Ok(cached_fn);
        }
        let function_ptr = self.create_guest_function(mem, symbol, f);
        self.non_lazy_host_functions.insert(symbol, function_ptr);
        Ok(function_ptr)
    }

    pub fn create_guest_function(
        &mut self,
        mem: &mut Mem,
        symbol: &'static str,
        f: HostFunction,
    ) -> GuestFunction {
        // Allocate an SVC ID for this host function
        let idx: u32 = self.linked_host_functions.len().try_into().unwrap();
        let svc = idx + Self::SVC_LINKED_FUNCTIONS_BASE;
        self.linked_host_functions.push((symbol, f));

        // Create guest function to call this host function
        let function_ptr = mem.alloc(8);
        let function_ptr: MutPtr<u32> = function_ptr.cast();
        mem.write(function_ptr + 0, encode_a32_svc(svc));
        mem.write(function_ptr + 1, encode_a32_ret());

        GuestFunction::from_addr_with_thumb_bit(function_ptr.to_bits())
    }

    /// Like [Self::create_proc_address], but takes an unmangled C name and
    /// returns a raw pointer to guest memory.
    pub fn create_function_address(
        &mut self,
        mem: &mut Mem,
        cpu: &mut Cpu,
        name: &str,
    ) -> Result<MutVoidPtr, ()> {
        let symbol = format!("_{name}");
        let address = self.create_proc_address(mem, cpu, &symbol)?;
        Ok(Ptr::from_bits(address.addr_with_thumb_bit()))
    }
}

/// Heuristic for data symbols that hold an `NSString *` in real frameworks
/// (`UIApplicationDidChangeStatusBarFrameNotification`, `NSURLIsExcludedFromBackupKey`...).
/// An app that uses one we don't implement would otherwise read address 0.
fn looks_like_string_constant(symbol: &str) -> bool {
    let name = symbol.trim_start_matches('_');
    const PREFIXES: &[&str] = &[
        "NS", "UI", "CF", "CL", "AV", "MP", "SK", "GK", "AD", "EA", "CA", "CG", "kCF", "kCA", "kSec", "kUTType", "kAudio", "kCV",
    ];
    const SUFFIXES: &[&str] = &[
        "Notification", "Key", "Keys", "Name", "Names", "Mode", "Domain", "Identifier", "Option", "Options",
        "Type", "Status", "Attribute", "Attributes", "SoundName", "Method", "Host", "User", "Scheme", "Gravity", "Proxy", "Application",
    ];
    // Keychain constants (kSecAttrAccessible, kSecAttrLabel...) are all CFStrings.
    if name.starts_with("kSec") {
        return true;
    }
    // Keychain constants (kSecAttrAccessible, kSecAttrLabel...) are all CFStrings.
    if name.starts_with("kSec") {
        return true;
    }
    PREFIXES.iter().any(|p| name.starts_with(p)) && SUFFIXES.iter().any(|s| name.ends_with(s))
}
