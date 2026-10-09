/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSUUID`.

use super::ns_string;
use crate::mem::MutPtr;
use crate::objc::{autorelease, id, msg, nil, objc_classes, ClassExports, HostObject, NSZonePtr};
use uuid::Uuid;

struct UuidHostObject {
    uuid: Uuid,
}
impl HostObject for UuidHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSUUID: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(UuidHostObject { uuid: Uuid::new_v4() });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

+ (id)UUID {
    let uuid: id = msg![env; this new];
    autorelease(env, uuid)
}

- (id)initWithUUIDString:(id)string { // NSString*
    let string = ns_string::to_rust_string(env, string);
    match Uuid::parse_str(&string) {
        Ok(uuid) => {
            env.objc.borrow_mut::<UuidHostObject>(this).uuid = uuid;
            this
        }
        Err(_) => nil,
    }
}

- (id)UUIDString {
    let string = env.objc.borrow::<UuidHostObject>(this).uuid.hyphenated().to_string().to_uppercase();
    let string = ns_string::from_rust_string(env, string);
    autorelease(env, string)
}

- (())getUUIDBytes:(MutPtr<u8>)bytes {
    let uuid = env.objc.borrow::<UuidHostObject>(this).uuid;
    env.mem.bytes_at_mut(bytes, 16).copy_from_slice(uuid.as_bytes());
}

- (bool)isEqual:(id)other {
    if this == other {
        return true;
    }
    if other == nil {
        return false;
    }
    let class: id = msg![env; this class];
    let other_class: id = msg![env; other class];
    if class != other_class {
        return false;
    }
    env.objc.borrow::<UuidHostObject>(this).uuid == env.objc.borrow::<UuidHostObject>(other).uuid
}

- (id)description {
    msg![env; this UUIDString]
}

@end

};
