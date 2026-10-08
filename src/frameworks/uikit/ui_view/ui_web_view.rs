/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIWebView`.

use crate::frameworks::foundation::ns_string::get_static_str;
use crate::frameworks::foundation::ns_string::to_rust_string;
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_super, nil, objc_classes, ClassExports, NSZonePtr,
};
use std::borrow::Cow;

struct UIWebViewHostObject {
    superclass: super::UIViewHostObject,
    delegate: id,
    scales_page_to_fit: bool,
    data_detector_types: u32,
    allows_inline_media_playback: bool,
    media_playback_requires_user_action: bool,
}
impl_HostObject_with_superclass!(UIWebViewHostObject);
impl Default for UIWebViewHostObject {
    fn default() -> Self {
        Self {
            superclass: Default::default(),
            delegate: nil,
            scales_page_to_fit: false,
            data_detector_types: 1, // UIDataDetectorTypePhoneNumber
            allows_inline_media_playback: false,
            media_playback_requires_user_action: true,
        }
    }
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIWebView: UIView

+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::<UIWebViewHostObject>::default(), &mut env.mem)
}

// NSCoding implementation
- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];
    let key = get_static_str(env, "UIDataDetectorTypes");
    let present: bool = msg![env; coder containsValueForKey:key];
    if present {
        let types: i32 = msg![env; coder decodeIntForKey:key];
        env.objc.borrow_mut::<UIWebViewHostObject>(this).data_detector_types = types as u32;
    }
    // UIView restores the geometry and view tree. The owner subsequently
    // reconnects delegates; decoding a weak owner reference here can recurse
    // through an object that NSKeyedUnarchiver is still initializing.
    this
}

- (bool)scalesPageToFit { env.objc.borrow::<UIWebViewHostObject>(this).scales_page_to_fit }
- (())setScalesPageToFit:(bool)scales {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).scales_page_to_fit = scales;
}
- (id)delegate { env.objc.borrow::<UIWebViewHostObject>(this).delegate }
- (())setDelegate:(id)delegate {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).delegate = delegate;
}
- (())loadRequest:(id)request { // NSURLRequest*
    let url_string = if request != nil {
        let url = msg![env; request URL];
        let url_desc = msg![env; url description];
        to_rust_string(env, url_desc)
    } else {
        Cow::default()
    };
    log!("TODO: [(UIWebView*) {:?} loadRequest:{:?} ({})]", this, request, url_string);
}

// There is no real web view: pages never load and no delegate callbacks fire.
// The methods below are ones that embedded content (ad SDKs, mostly) calls
// during setup, so they are accepted and ignored.
- (u32)dataDetectorTypes { env.objc.borrow::<UIWebViewHostObject>(this).data_detector_types }
- (())setDataDetectorTypes:(u32)types { env.objc.borrow_mut::<UIWebViewHostObject>(this).data_detector_types = types; }
- (bool)allowsInlineMediaPlayback { env.objc.borrow::<UIWebViewHostObject>(this).allows_inline_media_playback }
- (())setAllowsInlineMediaPlayback:(bool)allows { env.objc.borrow_mut::<UIWebViewHostObject>(this).allows_inline_media_playback = allows; }
- (bool)mediaPlaybackRequiresUserAction { env.objc.borrow::<UIWebViewHostObject>(this).media_playback_requires_user_action }
- (())setMediaPlaybackRequiresUserAction:(bool)requires { env.objc.borrow_mut::<UIWebViewHostObject>(this).media_playback_requires_user_action = requires; }
- (())loadHTMLString:(id)_string baseURL:(id)_base_url { // NSString *, NSURL *
    log!("TODO: [(UIWebView*) {:?} loadHTMLString:baseURL:] (ignored)", this);
}
- (id)stringByEvaluatingJavaScriptFromString:(id)_script { // NSString *
    log!("TODO: [(UIWebView*) {:?} stringByEvaluatingJavaScriptFromString:] (ignored, returning nil)", this);
    nil
}
- (())stopLoading {}
- (())reload {}
- (bool)isLoading { false }
- (bool)canGoBack { false }
- (bool)canGoForward { false }

@end

};
