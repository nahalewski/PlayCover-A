/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Synthetic, emulator-owned Mach-O image (UIKit, the QuartzCore classes and
//! the MetalANGLE MGLKit stand-in) with genuine LP64 objc2 metadata:
//! class_t, class_ro_t, method lists, __objc_classlist, __objc_imageinfo,
//! __objc_selrefs.
//!
//! The image is laid out like a compiled MH_DYLIB so that it can be handed
//! either to the emulator's own Objective-C registry (desktop tests, owned
//! Foundation path) or to the genuine cached libobjc through the ordinary dyld
//! ObjC "mapped" notification (see dev-docs/ARM64_UIKIT_PLAN.md). Nothing here
//! is an Apple binary.
//!
//! Every method IMP is its own 8-byte trampoline `movz x17,#index; b hub`,
//! and the hub branches to ONE bridge service. The service dispatches on the
//! index, not on the selector, so swizzling (method_exchangeImplementations)
//! keeps working and the whole layer costs a single bridge slot.
//!
//! Classes carry no ivar list. Instances are `isa` plus inherited storage;
//! all state lives host-side keyed by object address. A guest subclass
//! compiled against the real SDK keeps its compiled ivar offsets because its
//! instance_start is never smaller than our (tiny) instance_size.
use super::super::A64Cpu;
use super::asm::Asm;
use std::collections::{BTreeMap, BTreeSet};

const PAGE: u64 = 0x4000;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
const MH_DYLIB: u32 = 6;
const LC_SEGMENT_64: u32 = 0x19;
const LC_ID_DYLIB: u32 = 0xd;
const LC_UUID: u32 = 0x1b;
/// libobjc's `ISA_MASK` on arm64 (non-ptrauth) keeps bits 3..35. Raw-isa
/// static objects and class pointers must therefore stay below 64 GiB.
pub(in crate::a64) const ISA_ADDRESS_LIMIT: u64 = 0x10_0000_0000;
/// Explicit limits (raise deliberately when needed).
pub(in crate::a64) const MAX_CLASSES: usize = 256;
pub(in crate::a64) const MAX_ENTRIES: usize = 8192;
pub(in crate::a64) const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;

/// Addresses the image refers to but does not own.
#[derive(Clone, Copy, Debug)]
pub(in crate::a64) struct External {
    /// Root class (NSObject) and its metaclass; genuine cached or owned.
    pub root_class: u64,
    pub root_metaclass: u64,
    /// `_objc_empty_cache`, or zero where the registry ignores it.
    pub empty_cache: u64,
}

pub(in crate::a64) struct MethodDef {
    pub selector: &'static str,
    pub types: &'static str,
    /// Dispatcher entry index (trampoline number).
    pub index: u16,
}

pub(in crate::a64) struct ClassDef {
    pub name: &'static str,
    /// Install name the class is exported from (UIKit, QuartzCore, MetalANGLE).
    pub provider: &'static str,
    /// `None` attaches the class directly to the external root (NSObject).
    pub parent: Option<&'static str>,
    pub instance_size: u32,
    pub instance_methods: Vec<MethodDef>,
    pub class_methods: Vec<MethodDef>,
    /// `-dealloc` becomes an owned thunk: call the cleanup entry with
    /// (self, _cmd), then `[super dealloc]` through objc_msgSendSuper2.
    pub dealloc_cleanup: bool,
}

/// A static, immortal instance whose only storage is its isa word.
pub(in crate::a64) struct StaticObject {
    pub class: &'static str,
    pub size: u32,
}

/// Addresses available to generated code.
pub(in crate::a64) struct Symbols<'a> {
    pub entries: u64,
    pub got: &'a BTreeMap<String, u64>,
    pub selector_refs: &'a BTreeMap<String, u64>,
    pub classes: &'a BTreeMap<String, (u64, u64)>,
    pub scratch: u64,
}
impl Symbols<'_> {
    pub(in crate::a64) fn entry(&self, index: u16) -> u64 {
        self.entries + 8 * index as u64
    }
    pub(in crate::a64) fn got(&self, name: &str) -> u64 {
        self.got.get(name).copied().unwrap_or(0)
    }
    pub(in crate::a64) fn selref(&self, name: &str) -> u64 {
        self.selector_refs.get(name).copied().unwrap_or(0)
    }
    pub(in crate::a64) fn class(&self, name: &str) -> u64 {
        self.classes.get(name).map_or(0, |&(c, _)| c)
    }
}

/// Owned code exported as a C function (e.g. `_UIApplicationMain`) or used
/// internally. The generator is run twice (sizing, then final addresses);
/// it must emit the same number of instructions and literals each time.
pub(in crate::a64) struct FunctionDef {
    pub symbol: &'static str,
    /// Some(install name) to export; None for internal code.
    pub provider: Option<&'static str>,
    pub generate: fn(&Symbols<'_>) -> Asm,
}

pub(in crate::a64) struct ImageSpec<'a> {
    pub base: u64,
    pub install_name: &'a str,
    pub classes: &'a [ClassDef],
    pub statics: &'a [StaticObject],
    pub functions: &'a [FunctionDef],
    /// Selectors the host or generated code sends (fixed-up selrefs).
    pub send_selectors: &'a [&'static str],
    /// Named pointer slots bound at link time (like GOT entries).
    pub got: &'a [&'static str],
    /// Number of dispatcher entries (trampolines) to emit.
    pub entry_count: u16,
    /// Entry used by the -dealloc thunks.
    pub cleanup_entry: Option<u16>,
    /// The single bridge service every trampoline reaches.
    pub dispatcher: u64,
    pub scratch_bytes: u64,
    pub external: External,
}

#[derive(Debug, Clone)]
pub(in crate::a64) struct Layout {
    pub header: u64,
    pub text: (u64, u64),
    pub data: (u64, u64),
    pub install_name: String,
    /// name -> (class, metaclass)
    pub classes: BTreeMap<String, (u64, u64)>,
    pub class_providers: BTreeMap<String, String>,
    pub class_list: Vec<u64>,
    pub selector_refs: BTreeMap<String, u64>,
    pub static_objects: Vec<(String, u64)>,
    pub thunks: Vec<(String, u64)>,
    pub functions: BTreeMap<String, (Option<String>, u64)>,
    pub got: BTreeMap<String, u64>,
    pub entries: u64,
    pub entry_count: u16,
    pub scratch: (u64, u64),
}

pub(in crate::a64) struct BuiltImage {
    pub layout: Layout,
    text: Vec<u8>,
    data: Vec<u8>,
}

impl Layout {
    pub(in crate::a64) fn class(&self, name: &str) -> Option<u64> {
        self.classes.get(name).map(|&(c, _)| c)
    }
    pub(in crate::a64) fn metaclass(&self, name: &str) -> Option<u64> {
        self.classes.get(name).map(|&(_, m)| m)
    }
    pub(in crate::a64) fn static_object(&self, class: &str) -> Option<u64> {
        self.static_objects
            .iter()
            .find(|(c, _)| c == class)
            .map(|&(_, a)| a)
    }
    pub(in crate::a64) fn function(&self, symbol: &str) -> Option<u64> {
        self.functions.get(symbol).map(|&(_, a)| a)
    }
    pub(in crate::a64) fn entry(&self, index: u16) -> u64 {
        self.entries + 8 * index as u64
    }
    /// (provider install name, symbol, address) substitutes for app binds:
    /// `_OBJC_CLASS_$_X`, `_OBJC_METACLASS_$_X` and exported functions.
    pub(in crate::a64) fn exports(&self) -> Vec<(String, String, u64)> {
        let mut result = Vec::new();
        for (name, &(class, meta)) in &self.classes {
            let provider = &self.class_providers[name];
            result.push((provider.clone(), format!("_OBJC_CLASS_$_{name}"), class));
            result.push((provider.clone(), format!("_OBJC_METACLASS_$_{name}"), meta));
        }
        for (symbol, (provider, address)) in &self.functions {
            if let Some(provider) = provider {
                result.push((provider.clone(), symbol.clone(), *address));
            }
        }
        result
    }
    pub(in crate::a64) fn contains_code(&self, address: u64, length: u64) -> bool {
        address >= self.text.0
            && address
                .checked_add(length)
                .is_some_and(|end| end <= self.text.0 + self.text.1)
    }
}

struct Buffer {
    base: u64,
    bytes: Vec<u8>,
}
impl Buffer {
    fn reserve(&mut self, size: usize, align: usize) -> Result<u64, String> {
        let start = self
            .bytes
            .len()
            .checked_add(align - 1)
            .ok_or("UIKit image alignment overflow")?
            & !(align - 1);
        let end = start.checked_add(size).ok_or("UIKit image size overflow")?;
        if end as u64 > MAX_IMAGE_BYTES {
            return Err("UIKit image byte budget exceeded".into());
        }
        self.bytes.resize(end, 0);
        Ok(self.base + start as u64)
    }
    fn put(&mut self, address: u64, bytes: &[u8]) {
        let at = (address - self.base) as usize;
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn u32(&mut self, address: u64, value: u32) {
        self.put(address, &value.to_le_bytes());
    }
    fn u64(&mut self, address: u64, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
    fn end(&self) -> u64 {
        self.base + self.bytes.len() as u64
    }
    fn string(&mut self, table: &mut BTreeMap<String, u64>, s: &str) -> Result<u64, String> {
        if let Some(&a) = table.get(s) {
            return Ok(a);
        }
        let a = self.reserve(s.len() + 1, 1)?;
        self.put(a, s.as_bytes());
        table.insert(s.to_string(), a);
        Ok(a)
    }
}

fn valid_identifier(name: &str, allow_colon: bool) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || (allow_colon && b == b':'))
}

/// Owned `-dealloc`: `{ cleanup(self, _cmd); [super dealloc]; }`. The SEL
/// comes from this image's own selref (fixed up by the registering runtime)
/// and objc_msgSendSuper2 from a bound GOT slot.
fn dealloc_thunk(cleanup: u64, class: u64, selref: u64, super2_slot: u64) -> Asm {
    let mut a = Asm::default();
    a.prologue(32)
        .str(0, 31, 16)
        .ldr_literal(16, cleanup)
        .blr(16)
        .ldr(0, 31, 16)
        .ldr_literal(9, class)
        .stp(0, 9, 31, 16) // objc_super2 { receiver, current class }
        .add_imm(0, 31, 16)
        .ldr_indirect(1, selref)
        .ldr_indirect(16, super2_slot)
        .blr(16)
        .epilogue(32);
    a
}

pub(in crate::a64) fn build(spec: &ImageSpec<'_>) -> Result<BuiltImage, String> {
    let base = spec.base;
    if base == 0
        || base % PAGE != 0
        || base
            .checked_add(MAX_IMAGE_BYTES * 2)
            .is_none_or(|end| end > ISA_ADDRESS_LIMIT)
    {
        return Err("UIKit image base must be 16 KiB aligned and below the 64 GiB isa limit".into());
    }
    let classes = spec.classes;
    if classes.is_empty() || classes.len() > MAX_CLASSES {
        return Err("UIKit image class count invalid".into());
    }
    let install_name = spec.install_name;
    if !install_name.starts_with('/') || install_name.len() > 512 || install_name.contains('\0') {
        return Err("UIKit image install name invalid".into());
    }
    let external = spec.external;
    for address in [external.root_class, external.root_metaclass] {
        if address == 0 || address & 7 != 0 || address >= ISA_ADDRESS_LIMIT {
            return Err("UIKit image external root class identity invalid".into());
        }
    }
    if spec.dispatcher == 0 || spec.dispatcher & 3 != 0 {
        return Err("UIKit image dispatcher invalid".into());
    }
    if spec.entry_count as usize > MAX_ENTRIES || spec.scratch_bytes > 256 * 1024 {
        return Err("UIKit image entry/scratch budget exceeded".into());
    }
    let any_cleanup = classes.iter().any(|c| c.dealloc_cleanup);
    let cleanup = match (any_cleanup, spec.cleanup_entry) {
        (false, _) => None,
        (true, Some(index)) if index < spec.entry_count => Some(index),
        _ => return Err("UIKit dealloc thunks need a valid cleanup entry".into()),
    };
    // ---- validation ----
    let mut seen = BTreeMap::new();
    for (index, class) in classes.iter().enumerate() {
        if !valid_identifier(class.name, false) || seen.insert(class.name, index).is_some() {
            return Err(format!("invalid or duplicate UIKit class name {}", class.name));
        }
        if !class.provider.starts_with('/') && !class.provider.starts_with('@') {
            return Err(format!("UIKit class {} provider invalid", class.name));
        }
        if let Some(parent) = class.parent {
            let Some(&parent_index) = seen.get(parent) else {
                return Err(format!("UIKit class {} parent {parent} must precede it", class.name));
            };
            if parent_index == index || class.instance_size < classes[parent_index].instance_size {
                return Err(format!("UIKit class {} parent/size invalid", class.name));
            }
        }
        if class.instance_size < 8 || class.instance_size > 4096 || class.instance_size % 8 != 0 {
            return Err(format!("UIKit class {} instance size invalid", class.name));
        }
        for list in [&class.instance_methods, &class.class_methods] {
            let mut names = BTreeSet::new();
            for method in list.iter() {
                if !valid_identifier(method.selector, true)
                    || method.types.is_empty()
                    || method.types.len() > 255
                    || method.types.contains('\0')
                    || method.index >= spec.entry_count
                    || !names.insert(method.selector)
                {
                    return Err(format!(
                        "UIKit class {} has invalid/duplicate method {}",
                        class.name, method.selector
                    ));
                }
            }
        }
        if class.dealloc_cleanup && class.instance_methods.iter().any(|m| m.selector == "dealloc") {
            return Err("UIKit class declares both a host dealloc and a cleanup thunk".into());
        }
    }
    let mut referenced: Vec<&str> = spec.send_selectors.to_vec();
    if any_cleanup {
        referenced.push("dealloc");
    }
    referenced.sort_unstable();
    referenced.dedup();
    if referenced.len() > 1024 || referenced.iter().any(|s| !valid_identifier(s, true)) {
        return Err("UIKit image sent-selector list invalid".into());
    }
    let mut got_names: Vec<&str> = spec.got.to_vec();
    if any_cleanup {
        got_names.push("objc_msgSendSuper2");
    }
    got_names.sort_unstable();
    got_names.dedup();
    for object in spec.statics {
        if !seen.contains_key(object.class) || object.size < 8 || object.size % 8 != 0 || object.size > 4096 {
            return Err(format!("invalid static UIKit object of class {}", object.class));
        }
    }
    let mut function_names = BTreeSet::new();
    for function in spec.functions {
        if !valid_identifier(function.symbol, false) || !function_names.insert(function.symbol) {
            return Err(format!("invalid or duplicate UIKit function {}", function.symbol));
        }
    }

    // ---- sizing pass for generated code ----
    let empty = BTreeMap::new();
    let empty_classes = BTreeMap::new();
    let dummy = Symbols {
        entries: 0,
        got: &empty,
        selector_refs: &empty,
        classes: &empty_classes,
        scratch: 0,
    };
    let function_sizes: Vec<usize> = spec.functions.iter().map(|f| (f.generate)(&dummy).finish().len()).collect();
    let thunk_size = dealloc_thunk(0, 0, 0, 0).finish().len();
    let thunk_count = classes.iter().filter(|c| c.dealloc_cleanup).count();

    // ---- __TEXT layout ----
    let text_sections = ["__text", "__objc_methname", "__objc_classname", "__objc_methtype"];
    let data_sections = [
        "__objc_classlist",
        "__objc_imageinfo",
        "__objc_selrefs",
        "__got",
        "__objc_const",
        "__objc_data",
        "__data",
    ];
    let id_len = (24 + install_name.len() + 1 + 7) & !7;
    let commands_len = (72 + 80 * text_sections.len()) + (72 + 80 * data_sections.len()) + id_len + 24;
    let mut text = Buffer { base, bytes: Vec::new() };
    text.reserve(32 + commands_len, 8)?;
    let code_start = text.reserve(16, 16)?; // hub: ldr x16,=svc; br x16; .quad svc
    let entries = text.reserve(8 * spec.entry_count as usize, 8)?;
    let mut function_addresses = Vec::new();
    for size in &function_sizes {
        function_addresses.push(text.reserve(*size, 16)?);
    }
    let mut thunk_addresses = Vec::new();
    for _ in 0..thunk_count {
        thunk_addresses.push(text.reserve(thunk_size, 16)?);
    }
    let code_end = text.end();
    let mut methname = BTreeMap::new();
    let methname_start = text.end();
    for s in classes
        .iter()
        .flat_map(|c| c.instance_methods.iter().chain(c.class_methods.iter()).map(|m| m.selector))
        .chain(referenced.iter().copied())
    {
        text.string(&mut methname, s)?;
    }
    let methname_end = text.end();
    let mut classname = BTreeMap::new();
    for class in classes {
        text.string(&mut classname, class.name)?;
    }
    let classname_end = text.end();
    let mut methtype = BTreeMap::new();
    for m in classes.iter().flat_map(|c| c.instance_methods.iter().chain(c.class_methods.iter())) {
        text.string(&mut methtype, m.types)?;
    }
    if any_cleanup {
        text.string(&mut methtype, "v16@0:8")?;
    }
    let methtype_end = text.end();
    let text_size = (text.end() - base + PAGE - 1) & !(PAGE - 1);

    // ---- __DATA layout ----
    let data_base = base + text_size;
    let mut data = Buffer { base: data_base, bytes: Vec::new() };
    let classlist = data.reserve(8 * classes.len(), 8)?;
    let imageinfo = data.reserve(8, 8)?;
    data.u32(imageinfo + 4, 0x40);
    let selrefs_start = data.reserve(8 * referenced.len(), 8)?;
    let mut selector_refs = BTreeMap::new();
    for (index, name) in referenced.iter().enumerate() {
        let slot = selrefs_start + 8 * index as u64;
        data.u64(slot, methname[*name]);
        selector_refs.insert(name.to_string(), slot);
    }
    let got_start = data.reserve(8 * got_names.len(), 8)?;
    let got: BTreeMap<String, u64> = got_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.to_string(), got_start + 8 * i as u64))
        .collect();
    let got_end = data.end();
    let const_start = data.end();
    let ro_addresses: Vec<(u64, u64)> = classes
        .iter()
        .map(|_| Ok((data.reserve(72, 8)?, data.reserve(72, 8)?)))
        .collect::<Result<_, String>>()?;
    let mut method_lists = Vec::new();
    for class in classes {
        let mut lists = [0u64; 2];
        let instance_count = class.instance_methods.len() + usize::from(class.dealloc_cleanup);
        for (slot, count) in [(0, instance_count), (1, class.class_methods.len())] {
            if count > 0 {
                lists[slot] = data.reserve(8 + 24 * count, 8)?;
            }
        }
        method_lists.push(lists);
    }
    let const_end = data.end();
    let class_addresses: Vec<(u64, u64)> = classes
        .iter()
        .map(|_| Ok((data.reserve(40, 8)?, data.reserve(40, 8)?)))
        .collect::<Result<_, String>>()?;
    let objc_data_end = data.end();
    let mut static_objects = Vec::new();
    for object in spec.statics {
        static_objects.push((object.class.to_string(), data.reserve(object.size as usize, 16)?));
    }
    let scratch = data.reserve(spec.scratch_bytes as usize, 16)?;
    let data_end = data.end();
    let data_size = ((data_end - data_base).max(8) + PAGE - 1) & !(PAGE - 1);
    if text_size + data_size > MAX_IMAGE_BYTES {
        return Err("UIKit image exceeds byte budget".into());
    }

    // ---- classes ----
    let mut names = BTreeMap::new();
    for (index, class) in classes.iter().enumerate() {
        names.insert(class.name.to_string(), class_addresses[index]);
    }
    let mut thunks = Vec::new();
    let mut thunk_cursor = thunk_addresses.iter();
    for (index, class) in classes.iter().enumerate() {
        let (normal, meta) = class_addresses[index];
        let (ro, meta_ro) = ro_addresses[index];
        let (parent, parent_meta, parent_size) = match class.parent {
            Some(parent) => {
                let p = seen[parent];
                (class_addresses[p].0, class_addresses[p].1, classes[p].instance_size)
            }
            None => (external.root_class, external.root_metaclass, 8),
        };
        data.u64(normal, meta);
        data.u64(normal + 8, parent);
        data.u64(normal + 16, external.empty_cache);
        data.u64(normal + 32, ro);
        data.u64(meta, external.root_metaclass);
        data.u64(meta + 8, parent_meta);
        data.u64(meta + 16, external.empty_cache);
        data.u64(meta + 32, meta_ro);
        let name = classname[class.name];
        let [instance_list, class_list] = method_lists[index];
        for (ro, flags, start, size, list) in [
            (ro, 0u32, parent_size, class.instance_size, instance_list),
            (meta_ro, 1, 40, 40, class_list),
        ] {
            data.u32(ro, flags);
            data.u32(ro + 4, start);
            data.u32(ro + 8, size);
            data.u64(ro + 24, name);
            data.u64(ro + 32, list);
        }
        let entry = |i: u16| entries + 8 * i as u64;
        let mut instance: Vec<(u64, u64, u64)> = class
            .instance_methods
            .iter()
            .map(|m| (methname[m.selector], methtype[m.types], entry(m.index)))
            .collect();
        if class.dealloc_cleanup {
            let at = *thunk_cursor.next().unwrap();
            let code = dealloc_thunk(entry(cleanup.unwrap()), normal, selector_refs["dealloc"], got["objc_msgSendSuper2"]).finish();
            debug_assert_eq!(code.len(), thunk_size);
            text.put(at, &code);
            instance.push((methname["dealloc"], methtype["v16@0:8"], at));
            thunks.push((class.name.to_string(), at));
        }
        let class_methods: Vec<(u64, u64, u64)> = class
            .class_methods
            .iter()
            .map(|m| (methname[m.selector], methtype[m.types], entry(m.index)))
            .collect();
        for (list, methods) in [(instance_list, instance), (class_list, class_methods)] {
            if list == 0 {
                continue;
            }
            data.u32(list, 24);
            data.u32(list + 4, methods.len() as u32);
            for (i, (sel, types, imp)) in methods.into_iter().enumerate() {
                let at = list + 8 + 24 * i as u64;
                data.u64(at, sel);
                data.u64(at + 8, types);
                data.u64(at + 16, imp);
            }
        }
        data.u64(classlist + 8 * index as u64, normal);
    }
    for (class, address) in &static_objects {
        data.u64(*address, names[class].0);
    }

    // ---- code: hub, trampolines, functions ----
    let mut hub = Asm::default();
    hub.ldr_literal(16, spec.dispatcher).br(16);
    text.put(code_start, &hub.finish());
    for index in 0..spec.entry_count as u64 {
        let at = entries + 8 * index;
        let branch = 0x1400_0000u32 | (((code_start as i64 - (at + 4) as i64) / 4) as u32 & 0x3ff_ffff);
        text.u32(at, 0xd280_0000 | (index as u32) << 5 | 17); // movz x17, #index
        text.u32(at + 4, branch); // b hub
    }
    let symbols = Symbols {
        entries,
        got: &got,
        selector_refs: &selector_refs,
        classes: &names,
        scratch,
    };
    let mut functions = BTreeMap::new();
    for (function, &at) in spec.functions.iter().zip(&function_addresses) {
        let code = (function.generate)(&symbols).finish();
        if code.len() != function_sizes[functions.len()] {
            return Err(format!("UIKit function {} changed size between passes", function.symbol));
        }
        text.put(at, &code);
        functions.insert(function.symbol.to_string(), (function.provider.map(str::to_string), at));
    }

    // ---- header and load commands ----
    let mut cursor = base + 32;
    let write_segment = |text: &mut Buffer, cursor: &mut u64, name: &str, vmaddr: u64, vmsize: u64, prot: u32, sections: &[(&str, u64, u64, u32, u32)]| {
        let len = 72 + 80 * sections.len() as u64;
        text.u32(*cursor, LC_SEGMENT_64);
        text.u32(*cursor + 4, len as u32);
        text.put(*cursor + 8, name.as_bytes());
        text.u64(*cursor + 24, vmaddr);
        text.u64(*cursor + 32, vmsize);
        text.u64(*cursor + 40, vmaddr - base);
        text.u64(*cursor + 48, vmsize);
        text.u32(*cursor + 56, prot);
        text.u32(*cursor + 60, prot);
        text.u32(*cursor + 64, sections.len() as u32);
        for (i, &(sect, addr, size, align, flags)) in sections.iter().enumerate() {
            let at = *cursor + 72 + 80 * i as u64;
            text.put(at, sect.as_bytes());
            text.put(at + 16, name.as_bytes());
            text.u64(at + 32, addr);
            text.u64(at + 40, size);
            text.u32(at + 48, (addr - base) as u32);
            text.u32(at + 52, align);
            text.u32(at + 64, flags);
        }
        *cursor += len;
    };
    write_segment(
        &mut text,
        &mut cursor,
        "__TEXT",
        base,
        text_size,
        5,
        &[
            (text_sections[0], code_start, code_end - code_start, 4, 0x8000_0400),
            (text_sections[1], methname_start, methname_end - methname_start, 0, 2),
            (text_sections[2], methname_end, classname_end - methname_end, 0, 2),
            (text_sections[3], classname_end, methtype_end - classname_end, 0, 2),
        ],
    );
    write_segment(
        &mut text,
        &mut cursor,
        "__DATA",
        data_base,
        data_size,
        3,
        &[
            (data_sections[0], classlist, 8 * classes.len() as u64, 3, 0x1000_0000),
            (data_sections[1], imageinfo, 8, 2, 0),
            (data_sections[2], selrefs_start, 8 * referenced.len() as u64, 3, 0x1000_0005),
            (data_sections[3], got_start, got_end - got_start, 3, 6),
            (data_sections[4], const_start, const_end - const_start, 3, 0),
            (data_sections[5], const_end, objc_data_end - const_end, 3, 0),
            (data_sections[6], objc_data_end, data_end - objc_data_end, 4, 0),
        ],
    );
    text.u32(cursor, LC_ID_DYLIB);
    text.u32(cursor + 4, id_len as u32);
    text.u32(cursor + 8, 24);
    text.u32(cursor + 16, 0x0001_0000);
    text.u32(cursor + 20, 0x0001_0000);
    text.put(cursor + 24, install_name.as_bytes());
    cursor += id_len as u64;
    text.u32(cursor, LC_UUID);
    text.u32(cursor + 4, 24);
    text.put(cursor + 8, b"touchHLE UIKit\x00\x02");
    cursor += 24;
    debug_assert_eq!(cursor, base + 32 + commands_len as u64);
    for (offset, value) in [
        (0u64, MH_MAGIC_64),
        (4, CPU_TYPE_ARM64),
        (12, MH_DYLIB),
        (16, 4),
        (20, commands_len as u32),
        (24, 0x1 | 0x4 | 0x80), // MH_NOUNDEFS | MH_DYLDLINK | MH_TWOLEVEL
    ] {
        text.u32(base + offset, value);
    }
    text.bytes.resize(text_size as usize, 0);
    data.bytes.resize(data_size as usize, 0);
    Ok(BuiltImage {
        layout: Layout {
            header: base,
            text: (base, text_size),
            data: (data_base, data_size),
            install_name: install_name.to_string(),
            class_list: class_addresses.iter().map(|&(c, _)| c).collect(),
            class_providers: classes
                .iter()
                .map(|c| (c.name.to_string(), c.provider.to_string()))
                .collect(),
            classes: names,
            selector_refs,
            static_objects,
            thunks,
            functions,
            got,
            entries,
            entry_count: spec.entry_count,
            scratch: (scratch, spec.scratch_bytes),
        },
        text: text.bytes,
        data: data.bytes,
    })
}

impl BuiltImage {
    /// Map __TEXT read/execute and __DATA read/write. Placement must be free.
    pub(in crate::a64) fn map(&self, cpu: &mut A64Cpu) -> Result<(), String> {
        let (text, text_len) = self.layout.text;
        let (data, data_len) = self.layout.data;
        for address in (text..data + data_len).step_by(4096) {
            if cpu.mapped_permissions(address).is_some() {
                return Err("UIKit image overlaps an existing guest mapping".into());
            }
        }
        cpu.map_zeroed(text, text_len as usize, 5)?;
        cpu.try_write_bytes(text, &self.text)?;
        cpu.map_zeroed(data, data_len as usize, 3)?;
        cpu.try_write_bytes(data, &self.data)?;
        Ok(())
    }
}
