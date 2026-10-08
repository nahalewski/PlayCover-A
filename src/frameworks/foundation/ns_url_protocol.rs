/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSURLProtocol`.
//!
//! Stub: apps (usually network libraries) may register custom protocol
//! classes with it, but touchHLE's URL loading never consults them.

use crate::objc::{id, objc_classes, ClassExports};

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSURLProtocol: NSObject

+ (bool)registerClass:(id)_protocol_class { // Class
    log!("TODO: +[NSURLProtocol registerClass:] (custom protocols are never used)");
    true
}

+ (())unregisterClass:(id)_protocol_class {} // Class

@end

};
