/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSIndexPath`.

use super::NSInteger;
use crate::objc::{autorelease, id, msg_class, objc_classes, ClassExports, HostObject, NSZonePtr};

#[derive(Default)]
struct IndexPathHostObject {
    row: NSInteger,
    section: NSInteger,
}
impl HostObject for IndexPathHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSIndexPath: NSObject
+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::<IndexPathHostObject>::default(), &mut env.mem)
}
+ (id)indexPathForRow:(NSInteger)row inSection:(NSInteger)section {
    let result: id = msg_class![env; NSIndexPath alloc];
    let state = env.objc.borrow_mut::<IndexPathHostObject>(result);
    state.row = row; state.section = section;
    autorelease(env, result)
}
- (NSInteger)row { env.objc.borrow::<IndexPathHostObject>(this).row }
- (NSInteger)section { env.objc.borrow::<IndexPathHostObject>(this).section }
@end

};
