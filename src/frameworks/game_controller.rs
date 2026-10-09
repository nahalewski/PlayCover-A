/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! GameController framework.
//!
//! Only the notification names are provided. Games weak-link `GCController`
//! (`NSClassFromString`), so leaving the class out makes them take their "no
//! controller support" path, but they still read these constants directly when
//! registering observers.

use crate::dyld::{ConstantExports, HostConstant};

pub const GCControllerDidConnectNotification: &str = "GCControllerDidConnectNotification";
pub const GCControllerDidDisconnectNotification: &str = "GCControllerDidDisconnectNotification";

/// `NSNotificationName` values.
pub const CONSTANTS: ConstantExports = &[
    (
        "_GCControllerDidConnectNotification",
        HostConstant::NSString(GCControllerDidConnectNotification),
    ),
    (
        "_GCControllerDidDisconnectNotification",
        HostConstant::NSString(GCControllerDidDisconnectNotification),
    ),
];

pub const DYLIB: crate::dyld::HostDylib = crate::dyld::HostDylib {
    path: "/System/Library/Frameworks/GameController.framework/GameController",
    aliases: &[],
    class_exports: &[],
    constant_exports: &[CONSTANTS],
    function_exports: &[],
};
