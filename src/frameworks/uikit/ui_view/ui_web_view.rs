/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIWebView`.

use crate::frameworks::core_graphics::CGRect;
use crate::frameworks::foundation::ns_string::get_static_str;
use crate::frameworks::foundation::ns_string::to_rust_string;
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_super, nil, objc_classes, ClassExports, NSZonePtr,
};
use crate::Environment;
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

/// With network access enabled, the page is shown by an Android `WebView`
/// laid over the game's surface. The two sides talk through a small command
/// file (`webview_cmd.txt`, see `WebOverlay.java`): sequence number, command
/// (`show`/`html`/`hide`), physical pixel rectangle, base URL, payload.
/// The UIWebView currently shown by the overlay (guest object address).
static ACTIVE_WEBVIEW: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static LAST_EVENT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn send_web_command(env: &mut Environment, view: id, command: &str, base_url: &str, payload: &str) {
    use std::sync::atomic::{AtomicU64, Ordering};
    if command == "show" {
        let authority = payload.split_once("://").map_or("", |(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""));
        if crate::ad_blocklist::is_blocked_host(authority.rsplit('@').next().unwrap_or(authority)) {
            log!("UIWebView: blocked advertising page {:?}", payload.chars().take(80).collect::<String>());
            return;
        }
    }
    ACTIVE_WEBVIEW.store(if command == "hide" { 0 } else { view.to_bits() }, Ordering::Relaxed);
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let rect = if command == "hide" {
        (0.0, 0.0, 0.0, 0.0)
    } else {
        let bounds: CGRect = msg![env; view bounds];
        let in_window: CGRect = msg![env; view convertRect:bounds toView:nil];
        let (x0, y0) = (in_window.origin.x, in_window.origin.y);
        let (x1, y1) = (x0 + in_window.size.width, y0 + in_window.size.height);
        let window = env.window();
        let a = window.unrotated_to_physical((x0, y0));
        let b = window.unrotated_to_physical((x1, y1));
        (a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
    };
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let text = format!(
        "{}
{}
{} {} {} {}
{}
{}",
        seq, command, rect.0 as i32, rect.1 as i32, rect.2 as i32, rect.3 as i32, base_url, payload
    );
    let dir = crate::paths::user_data_base_path();
    let tmp = dir.join("webview_cmd.tmp");
    let dest = dir.join("webview_cmd.txt");
    if std::fs::write(&tmp, text).is_err() || std::fs::rename(&tmp, &dest).is_err() {
        log!("UIWebView: couldn't write the web overlay command file");
    }
    log!("UIWebView: {} {:?} -> {:?}", command, payload.chars().take(80).collect::<String>(), rect);
}

/// Pages in the overlay navigate to links the app wants to see (custom URL
/// schemes such as a page's "close" button). Java reports them in
/// `webview_evt.txt`; they are handed to the delegate like on iOS.
pub fn poll_events(env: &mut Environment) {
    use std::sync::atomic::Ordering;
    let view_bits = ACTIVE_WEBVIEW.load(Ordering::Relaxed);
    if view_bits == 0 {
        return;
    }
    let path = crate::paths::user_data_base_path().join("webview_evt.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let mut lines = text.splitn(2, '\n');
    let Some(seq) = lines.next().and_then(|l| l.trim().parse::<u64>().ok()) else {
        return;
    };
    let Some(url) = lines.next().map(|u| u.trim().to_string()) else {
        return;
    };
    if seq == LAST_EVENT_SEQ.load(Ordering::Relaxed) {
        return;
    }
    LAST_EVENT_SEQ.store(seq, Ordering::Relaxed);
    let view: id = crate::mem::Ptr::from_bits(view_bits);
    log!("UIWebView: page navigated to {:?}", url);
    let delegate = env.objc.borrow::<UIWebViewHostObject>(view).delegate;
    if delegate == nil {
        return;
    }
    let url_string = crate::frameworks::foundation::ns_string::from_rust_string(env, url);
    let ns_url: id = crate::objc::msg_class![env; NSURL URLWithString:url_string];
    let request: id = crate::objc::msg_class![env; NSURLRequest requestWithURL:ns_url];
    let sel = env
        .objc
        .register_host_selector("webView:shouldStartLoadWithRequest:navigationType:".to_string(), &mut env.mem);
    let responds: bool = msg![env; delegate respondsToSelector:sel];
    if responds {
        let _: bool = crate::objc::msg_send_no_type_checking(env, (delegate, sel, view, request, 0i32));
    }
}

/// Tell the delegate the page finished loading (the real load happens in the
/// Android overlay, which the app can't observe).
fn notify_loaded(env: &mut Environment, view: id) {
    let delegate = env.objc.borrow::<UIWebViewHostObject>(view).delegate;
    if delegate == nil {
        return;
    }
    for name in ["webViewDidStartLoad:", "webViewDidFinishLoad:"] {
        let sel = env.objc.register_host_selector(name.to_string(), &mut env.mem);
        let responds: bool = msg![env; delegate respondsToSelector:sel];
        if responds {
            let _: () = crate::objc::msg_send_no_type_checking(env, (delegate, sel, view));
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
    if env.options.network_access && !url_string.is_empty() {
        send_web_command(env, this, "show", "", &url_string);
        notify_loaded(env, this);
        return;
    }
    log!("TODO: [(UIWebView*) {:?} loadRequest:{:?} ({})]", this, request, url_string);
}

- (())removeFromSuperview {
    if env.options.network_access {
        send_web_command(env, this, "hide", "", "");
    }
    msg_super![env; this removeFromSuperview]
}

- (())setHidden:(bool)hidden {
    if env.options.network_access && hidden {
        send_web_command(env, this, "hide", "", "");
    }
    msg_super![env; this setHidden:hidden]
}

- (())dealloc {
    if env.options.network_access {
        send_web_command(env, this, "hide", "", "");
    }
    msg_super![env; this dealloc]
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
- (())loadHTMLString:(id)string baseURL:(id)base_url { // NSString *, NSURL *
    if env.options.network_access && string != nil {
        let html = to_rust_string(env, string).to_string();
        let base = if base_url != nil {
            let d: id = msg![env; base_url description];
            to_rust_string(env, d).to_string()
        } else {
            String::new()
        };
        send_web_command(env, this, "html", &base, &html);
        notify_loaded(env, this);
        return;
    }
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
