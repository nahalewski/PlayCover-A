/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Objective-C runtime.
//!
//! Apple's [Programming with Objective-C](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/ProgrammingWithObjectiveC/Introduction/Introduction.html)
//! is a useful introduction to the language from a user's perspective.
//! There are further resources in the child modules of this module, but they
//! are more implementation-specific.
//!
//! The strategy for this emulator will be to provide our own implementations of
//! an Objective-C runtime and libraries for it (Foundation etc). These
//! implementations will be "host code": Rust code forming part of the emulator,
//! not emulated code. The runtime will need to be able to handle classes that
//! originate from the guest app, classes defined by the host, and sometimes
//! classes that are both (considering Objective-C's support for inheritance,
//! categories and dynamic class editing).

use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant, HostDylib};
use crate::objc::messages::ThreadInitializer;
use crate::MutexId;
use std::collections::HashMap;

mod arc;
pub(crate) mod blocks;
mod classes;
mod messages;
mod methods;
mod objects;
mod properties;
mod selectors;
mod synchronization;

pub use classes::{install_skipped_method, objc_classes, Class, ClassExports, ClassTemplate};
pub use messages::{
    autorelease, msg, msg_class, msg_send, msg_send_no_initialize, msg_send_no_type_checking,
    msg_send_super2, msg_super, objc_super, release, retain,
};
pub use methods::{call_cxx_construct, HostIMP, IMP};
pub use objects::{
    id, impl_HostObject_with_superclass, nil, AnyHostObject, HostObject, TrivialHostObject,
};
pub use properties::todo_objc_setter;
pub use selectors::{selector, SEL};

use crate::mem::ConstVoidPtr;
use crate::Environment;
use classes::{
    class_addMethod, class_conformsToProtocol, class_getClassMethod, class_getInstanceMethod, class_getInstanceSize,
    class_getMethodImplementation, class_getName, class_getProperty, class_getSuperclass,
    class_replaceMethod, method_exchangeImplementations, method_getImplementation,
    method_setImplementation, objc_copyClassList, objc_enumerationMutation, objc_getClass, objc_getClassList, objc_lookUpClass,
    ClassHostObject, FakeClass, UnimplementedClass,
};
pub(crate) use messages::objc_msgSend;
use messages::{
    objc_msgSendSuper2, objc_msgSendSuper2_stret, objc_msgSend_stret, MsgSendSignature,
    MsgSendSuperSignature,
};
use methods::method_list_t;
use objects::{objc_object, object_getClass, HostObjectEntry};
use properties::{
    ivar_list_t, objc_copyStruct, objc_getProperty, objc_setProperty, objc_setProperty_atomic,
    objc_setProperty_atomic_copy, objc_setProperty_nonatomic, objc_setProperty_nonatomic_copy,
};
use selectors::{sel_getUid, sel_registerName};
use synchronization::{objc_sync_enter, objc_sync_exit};

/// Typedef for `NSZone *`. This is a [fossil type] found in the signature of
/// `allocWithZone:` and similar methods. Its value is always ignored.
///
/// [fossil type]: https://en.wiktionary.org/wiki/fossil_word
pub type NSZonePtr = crate::mem::MutVoidPtr;

/// Main type holding Objective-C runtime state.
pub struct ObjC {
    /// Known selectors (interned method name strings).
    selectors: HashMap<String, SEL>,

    /// Mapping of known (guest) object pointers to their host objects.
    ///
    /// If an object isn't in this map, we will consider it not to exist.
    objects: HashMap<id, HostObjectEntry>,

    /// Known classes.
    ///
    /// Look at the `isa` to get the metaclass for a class.
    classes: HashMap<String, Class>,

    /// Mutexes used in @synchronized blocks (objc_sync_enter/exit).
    sync_mutexes: HashMap<id, MutexId>,

    /// Mutexes for running the +initialize function.
    initializer_threads: HashMap<id, ThreadInitializer>,

    /// Temporary storage for optional type information when sending a message.
    /// Type information isn't part of the `objc_msgSend` ABI, so an alternative
    /// channel is needed.
    message_type_info: Option<(std::any::TypeId, &'static str)>,

    /// Guest C strings returned by `class_getName`, cached so each class name
    /// is only allocated once (callers expect the pointer to stay valid).
    class_name_cstrs: HashMap<Class, crate::mem::ConstPtr<u8>>,

    /// ARC `__weak` variables: slot address -> object it points at
    /// (see `arc.rs`). Slots are zeroed when the object is deallocated.
    weak_slots: HashMap<u32, id>,
    /// The reverse of `weak_slots`: object -> slots that point at it weakly.
    weak_targets: HashMap<id, Vec<u32>>,
}

impl ObjC {
    pub fn new() -> ObjC {
        ObjC {
            selectors: HashMap::new(),
            objects: HashMap::new(),
            classes: HashMap::new(),
            sync_mutexes: HashMap::new(),
            initializer_threads: HashMap::new(),
            message_type_info: None,
            class_name_cstrs: HashMap::new(),
            weak_slots: HashMap::new(),
            weak_targets: HashMap::new(),
        }
    }
}

pub const DYLIB: HostDylib = HostDylib {
    path: "/usr/lib/libobjc.A.dylib",
    aliases: &["/usr/lib/libobjc.dylib"],
    class_exports: &[blocks::CLASSES],
    constant_exports: &[CONSTANTS, blocks::CONSTANTS],
    function_exports: &[FUNCTIONS, arc::FUNCTIONS, blocks::FUNCTIONS],
};

const CONSTANTS: ConstantExports = &[
    // We don't use these in our Objective-C runtime, but exporting useless
    // symbols for these silences the warning about the unhandled relocation,
    // and avoids a linker error for the integration tests.
    ("__objc_empty_vtable", HostConstant::NullPtr),
    ("__objc_empty_cache", HostConstant::NullPtr),
];

const FUNCTIONS: FunctionExports = &[
    export_c_func!(class_getInstanceSize(_)),
    export_c_func!(class_getSuperclass(_)),
    export_c_func!(class_getProperty(_, _)),
    export_c_func!(class_getMethodImplementation(_, _)),
    export_c_func!(class_getInstanceMethod(_, _)),
    export_c_func!(class_getClassMethod(_, _)),
    export_c_func!(method_getImplementation(_)),
    export_c_func!(method_setImplementation(_, _)),
    export_c_func!(method_exchangeImplementations(_, _)),
    export_c_func!(class_replaceMethod(_, _, _, _)),
    export_c_func!(class_addMethod(_, _, _, _)),
    export_c_func!(objc_msgSend(_, _)),
    export_c_func!(objc_msgSend_stret(_, _, _)),
    export_c_func!(objc_msgSendSuper2(_, _)),
    export_c_func!(objc_msgSendSuper2_stret(_, _, _)),
    export_c_func!(objc_getClass(_)),
    export_c_func!(class_getName(_)),
    export_c_func!(objc_enumerationMutation(_)),
    export_c_func!(objc_getClassList(_, _)),
    export_c_func!(objc_lookUpClass(_)),
    export_c_func!(objc_copyClassList(_)),
    export_c_func!(class_conformsToProtocol(_, _)),
    export_c_func!(objc_getProperty(_, _, _, _)),
    export_c_func!(objc_setProperty(_, _, _, _, _, _)),
    export_c_func!(objc_setProperty_atomic(_, _, _, _)),
    export_c_func!(objc_setProperty_nonatomic(_, _, _, _)),
    export_c_func!(objc_setProperty_atomic_copy(_, _, _, _)),
    export_c_func!(objc_setProperty_nonatomic_copy(_, _, _, _)),
    export_c_func!(objc_copyStruct(_, _, _, _, _)),
    export_c_func!(objc_sync_enter(_)),
    export_c_func!(objc_sync_exit(_)),
    export_c_func!(object_getClass(_)),
    export_c_func!(sel_registerName(_)),
    export_c_func!(sel_getUid(_)),
];
