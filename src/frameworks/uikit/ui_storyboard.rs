/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIStoryboard` and the Auto Layout classes that compiled storyboards
//! reference by name.
//!
//! A compiled storyboard is a `<Name>.storyboardc` directory containing an
//! `Info.plist` (entry point and identifier -> nib name table), one nib per
//! scene's view controller and one nib per view controller's view. Layout
//! constraints are decoded but not applied: the views keep the frames the nib
//! stored, which is what the app was laid out against.

use crate::frameworks::core_graphics::CGFloat;
use crate::frameworks::foundation::ns_string::{from_rust_string, get_static_str, to_rust_string};
use crate::frameworks::uikit::ui_nib::instantiate_nib_at_path;
use crate::frameworks::uikit::ui_view_controller::set_storyboard_dir;
use crate::objc::{
    autorelease, id, msg, msg_class, nil, objc_classes, release,
    retain, ClassExports, HostObject, NSZonePtr,
};
use crate::Environment;

#[derive(Default)]
struct UIStoryboardHostObject {
    /// Absolute path of the `.storyboardc` directory (Rust string).
    dir: String,
}
impl HostObject for UIStoryboardHostObject {}

fn read_info_plist(env: &mut Environment, dir: &str) -> id {
    let path = from_rust_string(env, format!("{}/Info.plist", dir));
    let dict: id = msg_class![env; NSDictionary dictionaryWithContentsOfFile:path];
    release(env, path);
    dict
}

/// Loads the scene whose view controller nib is `nib_name` and returns the
/// view controller (retained for the caller), or nil.
fn instantiate_scene(env: &mut Environment, storyboard: id, nib_name: &str) -> id {
    let dir = env.objc.borrow::<UIStoryboardHostObject>(storyboard).dir.clone();
    let path = format!("{}/{}.nib", dir, nib_name);
    let objects = instantiate_nib_at_path(env, path, storyboard);
    if objects == nil {
        log!("UIStoryboard: couldn't load scene nib {:?}", nib_name);
        return nil;
    }
    let vc_class = env.objc.get_known_class("UIViewController", &mut env.mem);
    let count: crate::frameworks::foundation::NSUInteger = msg![env; objects count];
    for i in 0..count {
        let obj: id = msg![env; objects objectAtIndex:i];
        let class = msg![env; obj class];
        if env.objc.class_is_subclass_of(class, vc_class) {
            retain(env, obj);
            set_storyboard_dir(env, obj, &dir);
            return obj;
        }
    }
    log!("UIStoryboard: scene nib {:?} contained no view controller", nib_name);
    nil
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIStoryboard: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::<UIStoryboardHostObject>::default(), &mut env.mem)
}

+ (id)storyboardWithName:(id)name // NSString*
                  bundle:(id)bundle { // NSBundle*
    let bundle: id = if bundle == nil { msg_class![env; NSBundle mainBundle] } else { bundle };
    let bundle_path: id = msg![env; bundle bundlePath];
    let dir = format!(
        "{}/{}.storyboardc",
        to_rust_string(env, bundle_path),
        to_rust_string(env, name)
    );
    let new: id = msg![env; this alloc];
    env.objc.borrow_mut::<UIStoryboardHostObject>(new).dir = dir;
    autorelease(env, new)
}

- (id)instantiateInitialViewController {
    let dir = env.objc.borrow::<UIStoryboardHostObject>(this).dir.clone();
    let plist = read_info_plist(env, &dir);
    if plist == nil {
        log!("UIStoryboard: couldn't read {}/Info.plist", dir);
        return nil;
    }
    let key = get_static_str(env, "UIStoryboardDesignatedEntryPointIdentifier");
    let ident: id = msg![env; plist objectForKey:key];
    if ident == nil {
        return nil;
    }
    msg![env; this instantiateViewControllerWithIdentifier:ident]
}

- (id)instantiateViewControllerWithIdentifier:(id)identifier { // NSString*
    let dir = env.objc.borrow::<UIStoryboardHostObject>(this).dir.clone();
    let plist = read_info_plist(env, &dir);
    if plist == nil {
        return nil;
    }
    let key = get_static_str(env, "UIViewControllerIdentifiersToNibNames");
    let table: id = msg![env; plist objectForKey:key];
    let nib_name: id = if table == nil { nil } else { msg![env; table objectForKey:identifier] };
    if nib_name == nil {
        log!("UIStoryboard: no scene with identifier {:?}", to_rust_string(env, identifier));
        return nil;
    }
    let nib_name = to_rust_string(env, nib_name).to_string();
    let vc = instantiate_scene(env, this, &nib_name);
    if vc != nil {
        autorelease(env, vc);
    }
    vc
}

// Nibs connect each scene's view controller back to its storyboard through
// these outlets; nothing needs to be stored.
- (())setSceneViewController:(id)_vc {}
- (())setStoryboard:(id)_storyboard {}

@end

// `NSLayoutConstraint` and friends are decoded from storyboard nibs only so
// that the nib loads; their constraints are not solved (see the module docs).
@implementation NSLayoutConstraint: NSObject
- (id)initWithCoder:(id)_coder { this }
@end

@implementation _UILayoutSupportConstraint: NSLayoutConstraint
@end

// The top/bottom layout guides of a view controller.
@implementation _UILayoutGuide: UIView
- (CGFloat)length { 0.0 }
@end

};
