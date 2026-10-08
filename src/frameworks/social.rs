/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! The Social framework.
//!
//! Stub only: every social service reports itself as unavailable, so apps
//! that check before offering a "share" button just hide it.

use crate::dyld::{ConstantExports, HostConstant, HostDylib};
use crate::objc::{id, nil, objc_classes, ClassExports};

pub const SLServiceTypeTwitter: &str = "com.apple.social.twitter";
pub const SLServiceTypeFacebook: &str = "com.apple.social.facebook";
pub const SLServiceTypeSinaWeibo: &str = "com.apple.social.sinaweibo";

pub const CONSTANTS: ConstantExports = &[
    (
        "_SLServiceTypeTwitter",
        HostConstant::NSString(SLServiceTypeTwitter),
    ),
    (
        "_SLServiceTypeFacebook",
        HostConstant::NSString(SLServiceTypeFacebook),
    ),
    (
        "_SLServiceTypeSinaWeibo",
        HostConstant::NSString(SLServiceTypeSinaWeibo),
    ),
];

pub const DYLIB: HostDylib = HostDylib {
    path: "/System/Library/Frameworks/Social.framework/Social",
    aliases: &[],
    class_exports: &[CLASSES],
    constant_exports: &[CONSTANTS],
    function_exports: &[],
};

const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation SLComposeViewController: UIViewController

+ (bool)isAvailableForServiceType:(id)_service_type { // NSString *
    false
}

+ (id)composeViewControllerForServiceType:(id)_service_type { // NSString *
    nil
}

@end

};
