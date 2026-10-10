/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Synthetic, emulator-owned UIKit Mach-O image with genuine LP64 objc2
//! metadata (class_t, class_ro_t, method lists, __objc_classlist,
//! __objc_imageinfo, __objc_selrefs).
//!
//! The image is laid out exactly like a compiled MH_DYLIB so that it can be
//! handed either to the emulator's own Objective-C registry (desktop tests and
//! the owned-Foundation path) or, later, to the genuine cached libobjc through
//! the ordinary dyld ObjC "mapped" notification (see ARM64_UIKIT_PLAN.md).
//! Nothing here is an Apple binary: every method IMP is either a registered
//! bridge service trampoline or a small owned thunk in this image's __text.
//!
//! Classes carry no ivar list. Instances are `isa` plus inherited storage;
//! all UIKit state lives host-side keyed by object address. A guest subclass
//! compiled against the real SDK keeps its compiled ivar offsets, because its
//! instance_start is never smaller than our (tiny) instance_size.
use super::super::A64Cpu;
use std::collections::BTreeMap;

const PAGE: u64 = 0x4000;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
const MH_DYLIB: u32 = 6;
const LC_SEGMENT_64: u32 = 0x19;
const LC_ID_DYLIB: u32 = 0xd;
const LC_UUID: u32 = 0x1b;
/// libobjc's `ISA_MASK` on arm64 (non-ptrauth) keeps bits 3..35. Raw-isa
/// static objects and class pointers must therefore stay below 64 GiB.
pub(super) const ISA_ADDRESS_LIMIT: u64 = 0x10_0000_0000;
/// Limits are explicit, not open-ended (raise deliberately when needed).
pub(super) const MAX_CLASSES: usize = 256;
pub(super) const MAX_METHODS: usize = 8192;
pub(super) const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;

/// Addresses the image refers to but does not own.
#[derive(Clone, Copy, Debug)]
pub(super) struct External {
    /// Root class (NSObject) and its metaclass; genuine cached or owned.
    pub root_class: u64,
    pub root_metaclass: u64,
    /// `_objc_empty_cache`, or zero where the registry ignores it.
    pub empty_cache: u64,
}

pub(super) struct MethodDef {
    pub selector: &'static str,
    pub types: &'static str,
}

pub(super) struct ClassDef {
    pub name: &'static str,
    /// `None` attaches the class directly to the external root (NSObject).
    pub parent: Option<&'static str>,
    pub instance_size: u32,
    /// Every method of this class traps to this one dispatcher service; the
    /// handler switches on the selector name.
    pub dispatcher: u64,
    pub instance_methods: Vec<MethodDef>,
    pub class_methods: Vec<MethodDef>,
    /// When set, `-dealloc` is an owned thunk: call this host service with
    /// (self, _cmd), then `[super dealloc]` through objc_msgSendSuper2.
    pub dealloc_cleanup: Option<u64>,
}

/// A static, immortal instance (e.g. the shared UIApplication) whose only
/// storage is its isa word plus zero padding.
pub(super) struct StaticObject {
    pub class: &'static str,
    pub size: u32,
}

#[derive(Debug, Clone)]
pub(super) struct Layout {
    pub header: u64,
    pub text: (u64, u64),
    pub data: (u64, u64),
    pub install_name: String,
    /// name -> (class, metaclass)
    pub classes: BTreeMap<String, (u64, u64)>,
    pub class_list: Vec<u64>,
    /// selector name -> __objc_selrefs slot (only selectors the thunks use)
    pub selector_refs: BTreeMap<String, u64>,
    /// selector name -> address of its name string in __objc_methname
    pub selector_names: BTreeMap<String, u64>,
    pub static_objects: Vec<(String, u64)>,
    pub thunks: Vec<(String, u64)>,
    /// Bound pointer slot for `objc_msgSendSuper2` (written at link time,
    /// like a GOT entry), read by the -dealloc thunks.
    pub msg_send_super2_slot: u64,
}

pub(super) struct BuiltImage {
    pub layout: Layout,
    text: Vec<u8>,
    data: Vec<u8>,
}

impl Layout {
    pub(super) fn class(&self, name: &str) -> Option<u64> {
        self.classes.get(name).map(|&(c, _)| c)
    }
    pub(super) fn metaclass(&self, name: &str) -> Option<u64> {
        self.classes.get(name).map(|&(_, m)| m)
    }
    pub(super) fn static_object(&self, class: &str) -> Option<u64> {
        self.static_objects
            .iter()
            .find(|(c, _)| c == class)
            .map(|&(_, a)| a)
    }
    /// `_OBJC_CLASS_$_X` / `_OBJC_METACLASS_$_X` substitutes for app binds.
    pub(super) fn exports(&self) -> BTreeMap<String, u64> {
        let mut result = BTreeMap::new();
        for (name, &(class, meta)) in &self.classes {
            result.insert(format!("_OBJC_CLASS_$_{name}"), class);
            result.insert(format!("_OBJC_METACLASS_$_{name}"), meta);
        }
        result
    }
    pub(super) fn contains_code(&self, address: u64, length: u64) -> bool {
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
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
}

fn valid_identifier(name: &str, allow_colon: bool) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || (allow_colon && b == b':'))
}

/// Owned `-dealloc` thunk, equivalent to compiled
/// `{ cleanup(self, _cmd); [super dealloc]; }`. The selector is loaded from
/// this image's own __objc_selrefs slot so the registering runtime's fixup
/// (genuine libobjc map_images or the owned Registry plan) is honoured.
fn dealloc_thunk(cleanup: u64, class: u64, selref: u64, super2_slot: u64) -> Vec<u8> {
    // Instructions occupy bytes 0..64; the literal pool is at 64..96.
    let literal = |index: u64, register: u32, instruction: u64| -> u32 {
        let offset = 64 + index * 8 - instruction * 4;
        0x5800_0000 | (((offset / 4) as u32 & 0x7ffff) << 5) | register
    };
    let words: [u32; 16] = [
        0xa9be_7bfd,        // stp x29, x30, [sp, #-32]!
        0x9100_03fd,        // mov x29, sp
        0xf900_0be0,        // str x0, [sp, #16]
        literal(0, 16, 3),  // ldr x16, =cleanup service (x0=self, x1=_cmd)
        0xd63f_0200,        // blr x16
        0xf940_0be0,        // ldr x0, [sp, #16]
        literal(1, 9, 6),   // ldr x9, =this class
        0xa901_27e0,        // stp x0, x9, [sp, #16]   ; objc_super2
        0x9100_43e0,        // add x0, sp, #16
        literal(2, 1, 9),   // ldr x1, =&selref
        0xf940_0021,        // ldr x1, [x1]
        literal(3, 16, 11), // ldr x16, =&objc_msgSendSuper2 slot
        0xf940_0210,        // ldr x16, [x16]
        0xd63f_0200,        // blr x16
        0xa8c2_7bfd,        // ldp x29, x30, [sp], #32
        0xd65f_03c0,        // ret
    ];
    let mut bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    for value in [cleanup, class, selref, super2_slot] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}
pub(super) const DEALLOC_THUNK_BYTES: usize = 96;

pub(super) fn build(
    base: u64,
    install_name: &str,
    classes: &[ClassDef],
    statics: &[StaticObject],
    send_selectors: &[&'static str],
    external: External,
) -> Result<BuiltImage, String> {
    if base == 0 || base % PAGE != 0 || base.checked_add(MAX_IMAGE_BYTES * 2).is_none_or(|end| end > ISA_ADDRESS_LIMIT) {
        return Err("UIKit image base must be 16 KiB aligned and below the 64 GiB isa limit".into());
    }
    if classes.is_empty() || classes.len() > MAX_CLASSES {
        return Err("UIKit image class count invalid".into());
    }
    if install_name.is_empty() || !install_name.starts_with('/') || install_name.len() > 512 || install_name.contains('\0') {
        return Err("UIKit image install name invalid".into());
    }
    for address in [external.root_class, external.root_metaclass] {
        if address == 0 || address & 7 != 0 || address >= ISA_ADDRESS_LIMIT {
            return Err("UIKit image external root class identity invalid".into());
        }
    }
    // Validate names, parents and selector sets before any layout work.
    let mut seen = BTreeMap::new();
    let mut method_total = 0usize;
    for (index, class) in classes.iter().enumerate() {
        if !valid_identifier(class.name, false) || seen.insert(class.name, index).is_some() {
            return Err(format!("invalid or duplicate UIKit class name {}", class.name));
        }
        if let Some(parent) = class.parent {
            let Some(&parent_index) = seen.get(parent) else {
                return Err(format!("UIKit class {} parent {parent} must precede it", class.name));
            };
            if parent_index == index {
                return Err("UIKit class cannot be its own parent".into());
            }
            if class.instance_size < classes[parent_index].instance_size {
                return Err(format!("UIKit class {} smaller than its parent", class.name));
            }
        }
        if class.instance_size < 8 || class.instance_size > 4096 || class.instance_size % 8 != 0 {
            return Err(format!("UIKit class {} instance size invalid", class.name));
        }
        if class.dispatcher == 0 || class.dispatcher & 3 != 0 {
            return Err(format!("UIKit class {} dispatcher invalid", class.name));
        }
        for list in [&class.instance_methods, &class.class_methods] {
            let mut names = std::collections::BTreeSet::new();
            for method in list.iter() {
                if !valid_identifier(method.selector, true)
                    || method.types.is_empty()
                    || method.types.len() > 255
                    || method.types.contains('\0')
                    || !names.insert(method.selector)
                {
                    return Err(format!("UIKit class {} has invalid/duplicate selector {}", class.name, method.selector));
                }
            }
            method_total += list.len();
        }
        if class.dealloc_cleanup.is_some() && class.instance_methods.iter().any(|m| m.selector == "dealloc") {
            return Err("UIKit class declares both host dealloc and a cleanup thunk".into());
        }
    }
    if method_total > MAX_METHODS {
        return Err("UIKit image method budget exceeded".into());
    }
    let mut referenced: Vec<&str> = send_selectors.to_vec();
    if classes.iter().any(|c| c.dealloc_cleanup.is_some()) {
        referenced.push("dealloc");
    }
    referenced.sort_unstable();
    referenced.dedup();
    if referenced.len() > 1024 || referenced.iter().any(|s| !valid_identifier(s, true)) {
        return Err("UIKit image sent-selector list invalid".into());
    }
    for object in statics {
        if !seen.contains_key(object.class) || object.size < 8 || object.size % 8 != 0 || object.size > 4096 {
            return Err(format!("invalid static UIKit object of class {}", object.class));
        }
    }

    // ---- __TEXT: header, load commands, __text (thunks), cstrings ----
    let data_sections = [
        ("__objc_classlist", 3u32, 0x1000_0000u32),
        ("__objc_imageinfo", 2, 0),
        ("__objc_selrefs", 3, 0x1000_0005),
        ("__got", 3, 6),
        ("__objc_const", 3, 0),
        ("__objc_data", 3, 0),
        ("__data", 3, 0),
    ];
    let text_sections = [("__text", 2u32, 0x8000_0400u32), ("__objc_methname", 0, 2), ("__objc_classname", 0, 2), ("__objc_methtype", 0, 2)];
    let id_len = (24 + install_name.len() + 1 + 7) & !7;
    let commands_len = (72 + 80 * text_sections.len()) + (72 + 80 * data_sections.len()) + id_len + 24;
    let mut text = Buffer { base, bytes: Vec::new() };
    text.reserve(32 + commands_len, 8)?;
    // Thunk code first (fixed-size per class) so later literal values are known.
    let thunk_count = classes.iter().filter(|c| c.dealloc_cleanup.is_some()).count();
    let code_start = text.reserve(DEALLOC_THUNK_BYTES * thunk_count.max(1), 16)?;
    let code_len = (DEALLOC_THUNK_BYTES * thunk_count) as u64;
    let mut methname = BTreeMap::new();
    let strings_of = |text: &mut Buffer, table: &mut BTreeMap<String, u64>, s: &str| -> Result<u64, String> {
        if let Some(&a) = table.get(s) {
            return Ok(a);
        }
        let a = text.reserve(s.len() + 1, 1)?;
        text.put(a, s.as_bytes());
        table.insert(s.to_string(), a);
        Ok(a)
    };
    let methname_start = text.len() + base;
    let mut all_selectors: Vec<&str> = classes
        .iter()
        .flat_map(|c| c.instance_methods.iter().chain(c.class_methods.iter()).map(|m| m.selector))
        .collect();
    all_selectors.extend(referenced.iter().copied());
    for s in &all_selectors {
        strings_of(&mut text, &mut methname, s)?;
    }
    let methname_end = text.len() + base;
    let mut classname = BTreeMap::new();
    for class in classes {
        strings_of(&mut text, &mut classname, class.name)?;
    }
    let classname_end = text.len() + base;
    let mut methtype = BTreeMap::new();
    for class in classes {
        for m in class.instance_methods.iter().chain(class.class_methods.iter()) {
            strings_of(&mut text, &mut methtype, m.types)?;
        }
    }
    if thunk_count > 0 {
        strings_of(&mut text, &mut methtype, "v16@0:8")?;
    }
    let methtype_end = text.len() + base;
    let text_size = (text.len() + PAGE - 1) & !(PAGE - 1);

    // ---- __DATA ----
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
    let selrefs_end = data.len() + data_base;
    let super2_slot = data.reserve(8, 8)?;
    let got_end = data.len() + data_base;
    // __objc_const: class_ro_t pairs and method lists; addresses are assigned
    // after __objc_data is sized, so reserve __objc_data first in a scratch
    // pass: each class_t is 40 bytes, metaclass 40 bytes.
    let const_start = data.len() + data_base;
    debug_assert_eq!(const_start, got_end);
    let mut ro_addresses = Vec::new();
    for _ in classes {
        let ro = data.reserve(72, 8)?;
        let meta_ro = data.reserve(72, 8)?;
        ro_addresses.push((ro, meta_ro));
    }
    let mut method_lists = Vec::new();
    for class in classes {
        let mut lists = [0u64; 2];
        let instance_count = class.instance_methods.len() + usize::from(class.dealloc_cleanup.is_some());
        for (slot, count) in [(0, instance_count), (1, class.class_methods.len())] {
            if count > 0 {
                lists[slot] = data.reserve(8 + 24 * count, 8)?;
            }
        }
        method_lists.push(lists);
    }
    let const_end = data.len() + data_base;
    let objc_data_start = data.len() + data_base;
    let mut class_addresses = Vec::new();
    for _ in classes {
        let class = data.reserve(40, 8)?;
        let meta = data.reserve(40, 8)?;
        class_addresses.push((class, meta));
    }
    let objc_data_end = data.len() + data_base;
    let mut static_objects = Vec::new();
    for object in statics {
        let address = data.reserve(object.size as usize, 16)?;
        static_objects.push((object.class.to_string(), address));
    }
    let data_data_end = data.len() + data_base;
    let data_size = (data.len().max(8) + PAGE - 1) & !(PAGE - 1);
    if text_size + data_size > MAX_IMAGE_BYTES {
        return Err("UIKit image exceeds byte budget".into());
    }

    // ---- populate classes ----
    let mut names = BTreeMap::new();
    let mut thunks = Vec::new();
    let mut thunk_cursor = code_start;
    for (index, class) in classes.iter().enumerate() {
        let (normal, meta) = class_addresses[index];
        let (ro, meta_ro) = ro_addresses[index];
        let (parent, parent_meta, parent_size) = match class.parent {
            Some(parent) => {
                let parent_index = seen[parent];
                (class_addresses[parent_index].0, class_addresses[parent_index].1, classes[parent_index].instance_size)
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
        let mut instance: Vec<(u64, u64, u64)> = class
            .instance_methods
            .iter()
            .map(|m| (methname[m.selector], methtype[m.types], class.dispatcher))
            .collect();
        if let Some(cleanup) = class.dealloc_cleanup {
            if cleanup == 0 || cleanup & 3 != 0 {
                return Err("UIKit dealloc cleanup service invalid".into());
            }
            let code = dealloc_thunk(cleanup, normal, selector_refs["dealloc"], super2_slot);
            debug_assert_eq!(code.len(), DEALLOC_THUNK_BYTES);
            text.put(thunk_cursor, &code);
            instance.push((methname["dealloc"], methtype["v16@0:8"], thunk_cursor));
            thunks.push((class.name.to_string(), thunk_cursor));
            thunk_cursor += DEALLOC_THUNK_BYTES as u64;
        }
        let class_methods: Vec<(u64, u64, u64)> = class
            .class_methods
            .iter()
            .map(|m| (methname[m.selector], methtype[m.types], class.dispatcher))
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
        names.insert(class.name.to_string(), (normal, meta));
    }
    for (class, address) in &static_objects {
        data.u64(*address, names[class].0);
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
    let text_section_values = [
        (text_sections[0].0, code_start, code_len, text_sections[0].1, text_sections[0].2),
        (text_sections[1].0, methname_start, methname_end - methname_start, 0, 2),
        (text_sections[2].0, methname_end, classname_end - methname_end, 0, 2),
        (text_sections[3].0, classname_end, methtype_end - classname_end, 0, 2),
    ];
    write_segment(&mut text, &mut cursor, "__TEXT", base, text_size, 5, &text_section_values);
    let data_section_values = [
        (data_sections[0].0, classlist, 8 * classes.len() as u64, 3, data_sections[0].2),
        (data_sections[1].0, imageinfo, 8, 2, 0),
        (data_sections[2].0, selrefs_start, selrefs_end - selrefs_start, 3, data_sections[2].2),
        (data_sections[3].0, super2_slot, 8, 3, data_sections[3].2),
        (data_sections[4].0, const_start, const_end - const_start, 3, 0),
        (data_sections[5].0, objc_data_start, objc_data_end - objc_data_start, 3, 0),
        (data_sections[6].0, objc_data_end, data_data_end - objc_data_end, 4, 0),
    ];
    write_segment(&mut text, &mut cursor, "__DATA", data_base, data_size, 3, &data_section_values);
    text.u32(cursor, LC_ID_DYLIB);
    text.u32(cursor + 4, id_len as u32);
    text.u32(cursor + 8, 24);
    text.u32(cursor + 16, 0x0001_0000);
    text.u32(cursor + 20, 0x0001_0000);
    text.put(cursor + 24, install_name.as_bytes());
    cursor += id_len as u64;
    text.u32(cursor, LC_UUID);
    text.u32(cursor + 4, 24);
    // Deterministic, recognisably synthetic UUID ("touchHLE UIKit  ").
    text.put(cursor + 8, b"touchHLE UIKit\x00\x01");
    cursor += 24;
    debug_assert_eq!(cursor, base + 32 + commands_len as u64);
    for (offset, value) in [
        (0u64, MH_MAGIC_64),
        (4, CPU_TYPE_ARM64),
        (8, 0),
        (12, MH_DYLIB),
        (16, 4),
        (20, commands_len as u32),
        // MH_NOUNDEFS | MH_DYLDLINK | MH_TWOLEVEL
        (24, 0x1 | 0x4 | 0x80),
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
            classes: names,
            selector_refs,
            selector_names: methname,
            static_objects,
            thunks,
            msg_send_super2_slot: super2_slot,
        },
        text: text.bytes,
        data: data.bytes,
    })
}

impl BuiltImage {
    /// Map __TEXT read/execute and __DATA read/write. Placement must be free.
    pub(super) fn map(&self, cpu: &mut A64Cpu) -> Result<(), String> {
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
