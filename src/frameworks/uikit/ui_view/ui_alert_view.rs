/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIAlertView`.
//!
//! The alert is drawn by the Android host (`AlertOverlay.java`) as a card laid
//! over the game surface. touchHLE writes the alert to `alert_cmd.txt`; when a
//! button is tapped Java reports it in `alert_evt.txt`, which `poll_events`
//! turns into the delegate callbacks, like on iOS.

use crate::frameworks::foundation::ns_string::{from_rust_string, to_rust_string};
use crate::frameworks::foundation::NSInteger;
use crate::objc::{
    id, msg, msg_send_no_type_checking, msg_super, nil, objc_classes, release, retain,
    ClassExports,
};
use crate::Environment;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

struct AlertState {
    title: String,
    message: String,
    buttons: Vec<String>,
    cancel_index: NSInteger,
    delegate: u32,
}

/// Alert state, keyed by the guest object's address.
static ALERTS: Mutex<Option<HashMap<u32, AlertState>>> = Mutex::new(None);
/// The alert currently shown by the overlay (guest object address) and the
/// sequence number of the command that showed it.
static ACTIVE_ALERT: AtomicU32 = AtomicU32::new(0);
static ACTIVE_SEQ: AtomicU64 = AtomicU64::new(0);
static SEQ: AtomicU64 = AtomicU64::new(1);

fn with_state<R>(this: id, f: impl FnOnce(&mut AlertState) -> R) -> R {
    let mut guard = ALERTS.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    let state = map.entry(this.to_bits()).or_insert_with(|| AlertState {
        title: String::new(),
        message: String::new(),
        buttons: Vec::new(),
        cancel_index: -1,
        delegate: 0,
    });
    f(state)
}

fn escape(s: &str) -> String {
    s.replace('%', "%25").replace('\n', "%0A").replace('\r', "%0D")
}

fn write_command(command: &str, state: Option<&AlertState>) -> u64 {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut text = format!("{}\n{}\n", seq, command);
    if let Some(s) = state {
        text.push_str(&format!(
            "{}\n{}\n{}\n",
            escape(&s.title),
            escape(&s.message),
            s.buttons.len()
        ));
        for b in &s.buttons {
            text.push_str(&escape(b));
            text.push('\n');
        }
    }
    let dir = crate::paths::user_data_base_path();
    let tmp = dir.join("alert_cmd.tmp");
    let dest = dir.join("alert_cmd.txt");
    if std::fs::write(&tmp, text).is_err() || std::fs::rename(&tmp, &dest).is_err() {
        log!("UIAlertView: couldn't write the alert overlay command file");
    }
    seq
}

fn send_delegate(env: &mut Environment, delegate: id, name: &str, alert: id, index: NSInteger) {
    if delegate == nil {
        return;
    }
    let sel = env
        .objc
        .register_host_selector(name.to_string(), &mut env.mem);
    let responds: bool = msg![env; delegate respondsToSelector:sel];
    if responds {
        let _: () = msg_send_no_type_checking(env, (delegate, sel, alert, index));
    }
}

/// Finish the active alert with the given button: tell the delegate and
/// release the alert (retained while it was on screen).
fn finish_alert(env: &mut Environment, alert: id, index: NSInteger) {
    ACTIVE_ALERT.store(0, Ordering::Relaxed);
    let delegate_bits = with_state(alert, |s| s.delegate);
    let delegate: id = crate::mem::Ptr::from_bits(delegate_bits);
    send_delegate(env, delegate, "alertView:clickedButtonAtIndex:", alert, index);
    send_delegate(env, delegate, "alertView:willDismissWithButtonIndex:", alert, index);
    send_delegate(env, delegate, "alertView:didDismissWithButtonIndex:", alert, index);
    release(env, alert);
}

/// Called from the UIKit event loop: handle a button tap reported by Java.
pub fn poll_events(env: &mut Environment) {
    let alert_bits = ACTIVE_ALERT.load(Ordering::Relaxed);
    if alert_bits == 0 {
        return;
    }
    let path = crate::paths::user_data_base_path().join("alert_evt.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let mut lines = text.splitn(2, '\n');
    let Some(seq) = lines.next().and_then(|l| l.trim().parse::<u64>().ok()) else {
        return;
    };
    let Some(index) = lines.next().and_then(|l| l.trim().parse::<NSInteger>().ok()) else {
        return;
    };
    if seq != ACTIVE_SEQ.load(Ordering::Relaxed) {
        return; // stale report for an earlier alert
    }
    let alert: id = crate::mem::Ptr::from_bits(alert_bits);
    log!("UIAlertView: button {} tapped", index);
    finish_alert(env, alert, index);
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIAlertView: UIView

- (id)initWithTitle:(id)title
                      message:(id)message
                     delegate:(id)delegate
            cancelButtonTitle:(id)cancelButtonTitle
            otherButtonTitles:(id)otherButtonTitles, ...args {
    let this: id = msg_super![env; this init];

    let title_s = if title == nil { String::new() } else { to_rust_string(env, title).to_string() };
    let message_s = if message == nil { String::new() } else { to_rust_string(env, message).to_string() };
    let mut buttons = Vec::new();
    let mut cancel_index: NSInteger = -1;
    if cancelButtonTitle != nil {
        cancel_index = 0;
        buttons.push(to_rust_string(env, cancelButtonTitle).to_string());
    }
    if otherButtonTitles != nil {
        buttons.push(to_rust_string(env, otherButtonTitles).to_string());
        let mut varargs = args.start();
        loop {
            let next: id = varargs.next(env);
            if next == nil {
                break;
            }
            buttons.push(to_rust_string(env, next).to_string());
        }
    }
    log!("UIAlertView: title: {:?}, message: {:?}, buttons: {:?}", title_s, message_s, buttons);
    with_state(this, |s| {
        s.title = title_s;
        s.message = message_s;
        s.buttons = buttons;
        s.cancel_index = cancel_index;
        s.delegate = delegate.to_bits();
    });
    this
}

- (id)delegate {
    let bits = with_state(this, |s| s.delegate);
    crate::mem::Ptr::from_bits(bits)
}
- (())setDelegate:(id)delegate {
    with_state(this, |s| s.delegate = delegate.to_bits());
}

- (id)title {
    let t = with_state(this, |s| s.title.clone());
    from_rust_string(env, t)
}
- (())setTitle:(id)title {
    let t = if title == nil { String::new() } else { to_rust_string(env, title).to_string() };
    with_state(this, |s| s.title = t);
}
- (id)message {
    let m = with_state(this, |s| s.message.clone());
    from_rust_string(env, m)
}
- (())setMessage:(id)message {
    let m = if message == nil { String::new() } else { to_rust_string(env, message).to_string() };
    with_state(this, |s| s.message = m);
}

- (NSInteger)addButtonWithTitle:(id)title {
    let t = if title == nil { String::new() } else { to_rust_string(env, title).to_string() };
    with_state(this, |s| {
        s.buttons.push(t);
        (s.buttons.len() - 1) as NSInteger
    })
}
- (NSInteger)numberOfButtons {
    with_state(this, |s| s.buttons.len() as NSInteger)
}
- (id)buttonTitleAtIndex:(NSInteger)index {
    let t = with_state(this, |s| s.buttons.get(index as usize).cloned());
    match t {
        Some(t) => from_rust_string(env, t),
        None => nil,
    }
}
- (NSInteger)cancelButtonIndex {
    with_state(this, |s| s.cancel_index)
}
- (())setCancelButtonIndex:(NSInteger)index {
    with_state(this, |s| s.cancel_index = index);
}
- (NSInteger)firstOtherButtonIndex {
    with_state(this, |s| {
        if s.buttons.len() > (if s.cancel_index == 0 { 1 } else { 0 }) {
            if s.cancel_index == 0 { 1 } else { 0 }
        } else {
            -1
        }
    })
}
- (NSInteger)alertViewStyle { 0 }
- (())setAlertViewStyle:(NSInteger)_style {}
- (bool)isVisible {
    ACTIVE_ALERT.load(Ordering::Relaxed) == this.to_bits()
}

- (())show {
    log!("UIAlertView: showing alert {:?}", this);
    // A second alert replaces the first one on screen.
    let previous = ACTIVE_ALERT.load(Ordering::Relaxed);
    if previous != 0 && previous != this.to_bits() {
        let prev: id = crate::mem::Ptr::from_bits(previous);
        finish_alert(env, prev, -1);
    }
    if previous == this.to_bits() {
        return;
    }
    retain(env, this); // kept alive while it is on screen
    ACTIVE_ALERT.store(this.to_bits(), Ordering::Relaxed);
    let seq = {
        let mut guard = ALERTS.lock().unwrap();
        let map = guard.get_or_insert_with(HashMap::new);
        write_command("show", map.get(&this.to_bits()))
    };
    ACTIVE_SEQ.store(seq, Ordering::Relaxed);
}

- (())dismissWithClickedButtonIndex:(NSInteger)index animated:(bool)_animated {
    if ACTIVE_ALERT.load(Ordering::Relaxed) == this.to_bits() {
        write_command("hide", None);
        finish_alert(env, this, index);
    }
}

- (())dealloc {
    let mut guard = ALERTS.lock().unwrap();
    if let Some(map) = guard.as_mut() {
        map.remove(&this.to_bits());
    }
    drop(guard);
    msg_super![env; this dealloc]
}

@end

};
