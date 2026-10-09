/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `MPMoviePlayerController` etc.

use crate::dyld::{ConstantExports, HostConstant};
use crate::frameworks::core_graphics::CGRect;
use crate::frameworks::foundation::{ns_string, ns_url, NSInteger, NSTimeInterval};
use crate::frameworks::uikit::ui_device::UIDeviceOrientation;
use crate::objc::{
    id, msg, msg_class, nil, objc_classes, release, retain, todo_objc_setter, ClassExports,
    HostObject, NSZonePtr,
};
use crate::Environment;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct State {
    active_player: Option<id>,
    /// Various apps (e.g. Crash Bandicoot Nitro Kart 3D and Spore Origins)
    /// create or start a player and await some kind of notification, but can't
    /// handle it if that notification happens immediately. This queue lets us
    /// delay such notifications until the app next returns to the run loop,
    /// which seems to be late enough.
    pending_notifications: VecDeque<(&'static str, id, Instant)>,
    /// The player whose movie is currently shown by the Android video overlay
    /// (see `MovieOverlay.java`), and the sequence number of its command.
    overlay_player: Option<(id, u64)>,
    /// How the next `MPMoviePlayerPlaybackDidFinishNotification` of a player
    /// ends (`MPMovieFinishReason`); players without an entry failed.
    finish_reasons: std::collections::HashMap<id, NSInteger>,
}
impl State {
    fn get(env: &mut Environment) -> &mut Self {
        &mut env.framework_state.media_player.movie_player
    }
}

type MPMovieScalingMode = NSInteger;
type MPMovieControlStyle = NSInteger;

type MPMoviePlaybackState = NSInteger;
const MPMoviePlaybackStateStopped: MPMoviePlaybackState = 0;

// Values might not be correct, but as these are linked symbol constants, it
// shouldn't matter.
pub const MPMoviePlayerPlaybackDidFinishNotification: &str =
    "MPMoviePlayerPlaybackDidFinishNotification";
/// Apparently an undocumented, private API. Spore Origins uses it.
pub const MPMoviePlayerContentPreloadDidFinishNotification: &str =
    "MPMoviePlayerContentPreloadDidFinishNotification";
pub const MPMoviePlayerScalingModeDidChangeNotification: &str =
    "MPMoviePlayerScalingModeDidChangeNotification";
pub const MPMoviePlayerLoadStateDidChangeNotification: &str =
    "MPMoviePlayerLoadStateDidChangeNotification";
// TODO: More notifications?
const MPMoviePlayerPlaybackDidFinishReasonUserInfoKey: &str =
    "MPMoviePlayerPlaybackDidFinishReasonUserInfoKey";

/// `NSNotificationName` values and other constants.
pub const CONSTANTS: ConstantExports = &[
    (
        "_MPMoviePlayerPlaybackDidFinishNotification",
        HostConstant::NSString(MPMoviePlayerPlaybackDidFinishNotification),
    ),
    (
        "_MPMoviePlayerContentPreloadDidFinishNotification",
        HostConstant::NSString(MPMoviePlayerContentPreloadDidFinishNotification),
    ),
    (
        "_MPMoviePlayerScalingModeDidChangeNotification",
        HostConstant::NSString(MPMoviePlayerScalingModeDidChangeNotification),
    ),
    (
        "_MPMoviePlayerLoadStateDidChangeNotification",
        HostConstant::NSString(MPMoviePlayerLoadStateDidChangeNotification),
    ),
    (
        "_MPMoviePlayerPlaybackDidFinishReasonUserInfoKey",
        HostConstant::NSString(MPMoviePlayerPlaybackDidFinishReasonUserInfoKey),
    ),
];

struct MPMoviePlayerControllerHostObject {
    // NSURL *
    content_url: id,
    view: id,
    /// Lazily created black `-backgroundView`.
    background_view: id,
    window: id,
    previous_key_window: id,
    fullscreen: bool,
    should_autoplay: bool,
}
impl HostObject for MPMoviePlayerControllerHostObject {}

fn legacy_fullscreen(version: &str) -> bool {
    let mut parts = version.split('.');
    let major = parts
        .next()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(2);
    let minor = parts
        .next()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(0);
    (major, minor) < (3, 2)
}

fn player_view(env: &mut Environment, player: id) -> id {
    let view = env
        .objc
        .borrow::<MPMoviePlayerControllerHostObject>(player)
        .view;
    if view != nil {
        return view;
    }
    let screen: id = msg_class![env; UIScreen mainScreen];
    let bounds: CGRect = msg![env; screen bounds];
    let view: id = msg_class![env; UIView alloc];
    let view: id = msg![env; view initWithFrame:bounds];
    let black: id = msg_class![env; UIColor blackColor];
    () = msg![env; view setBackgroundColor:black];
    env.objc
        .borrow_mut::<MPMoviePlayerControllerHostObject>(player)
        .view = view;
    view
}

fn present_fullscreen(env: &mut Environment, player: id) {
    if env
        .objc
        .borrow::<MPMoviePlayerControllerHostObject>(player)
        .window
        != nil
    {
        return;
    }
    let previous = env
        .framework_state
        .uikit
        .ui_view
        .ui_window
        .key_window
        .unwrap_or(nil);
    retain(env, previous);
    let screen: id = msg_class![env; UIScreen mainScreen];
    let bounds: CGRect = msg![env; screen bounds];
    let window: id = msg_class![env; UIWindow alloc];
    let window: id = msg![env; window initWithFrame:bounds];
    let view = player_view(env, player);
    () = msg![env; window addSubview:view];
    {
        let host = env
            .objc
            .borrow_mut::<MPMoviePlayerControllerHostObject>(player);
        host.window = window;
        host.previous_key_window = previous;
    }
    () = msg![env; window makeKeyAndVisible];
}

fn dismiss_fullscreen(env: &mut Environment, player: id) {
    let (window, previous) = {
        let host = env
            .objc
            .borrow_mut::<MPMoviePlayerControllerHostObject>(player);
        (
            std::mem::replace(&mut host.window, nil),
            std::mem::replace(&mut host.previous_key_window, nil),
        )
    };
    if window == nil {
        return;
    }
    () = msg![env; window setHidden:true];
    let view = env
        .objc
        .borrow::<MPMoviePlayerControllerHostObject>(player)
        .view;
    () = msg![env; view removeFromSuperview];
    if env.framework_state.uikit.ui_view.ui_window.key_window == Some(window) && previous != nil {
        () = msg![env; previous makeKeyWindow];
    }
    release(env, window);
    release(env, previous);
}

/// Starts the movie in the Android video overlay (the Java side plays it with
/// the platform's decoder and reports the end in `movie_evt.txt`). The movie is
/// copied out of the app bundle first, as the platform player needs a real
/// file. Returns whether the overlay was started.
#[cfg(target_os = "android")]
fn start_overlay(env: &mut Environment, player: id) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let url = env
        .objc
        .borrow::<MPMoviePlayerControllerHostObject>(player)
        .content_url;
    if url == nil {
        return false;
    }
    let guest_path = ns_url::to_rust_path(env, url);
    let Some(name) = guest_path.file_name().map(str::to_owned) else {
        return false;
    };
    let Ok(bytes) = env.fs.read(&*guest_path) else {
        log!("Movie {:?} could not be read, not playing it", guest_path);
        return false;
    };
    let dir = crate::paths::user_data_base_path().join("movies");
    let file = dir.join(&name);
    let up_to_date = std::fs::metadata(&file).map_or(false, |m| m.len() == bytes.len() as u64);
    if !up_to_date
        && (std::fs::create_dir_all(&dir).is_err() || std::fs::write(&file, &bytes).is_err())
    {
        log!("Movie {:?} could not be copied for playback", name);
        return false;
    }
    let seq = SEQ.fetch_add(2, Ordering::Relaxed)
        + std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_millis() as u64 * 2);
    let (x, y, w, h) = env.window().viewport();
    let base = crate::paths::user_data_base_path();
    let _ = std::fs::remove_file(base.join("movie_evt.txt"));
    let text = format!("{}\nplay\n{} {} {} {}\n{}", seq, x, y, w, h, file.display());
    let tmp = base.join("movie_cmd.tmp");
    if std::fs::write(&tmp, text).is_err()
        || std::fs::rename(&tmp, base.join("movie_cmd.txt")).is_err()
    {
        return false;
    }
    log!("Playing movie {:?} in the video overlay", name);
    State::get(env).overlay_player = Some((player, seq));
    true
}
#[cfg(not(target_os = "android"))]
fn start_overlay(_env: &mut Environment, _player: id) -> bool {
    false
}

fn stop_overlay(env: &mut Environment, player: id) {
    let Some((overlay_player, seq)) = State::get(env).overlay_player else {
        return;
    };
    if overlay_player != player {
        return;
    }
    State::get(env).overlay_player = None;
    #[cfg(target_os = "android")]
    {
        let base = crate::paths::user_data_base_path();
        let tmp = base.join("movie_cmd.tmp");
        let text = format!("{}\nstop\n0 0 0 0\n", seq + 1);
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, base.join("movie_cmd.txt"));
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = seq;
}

/// The Java overlay reports the end of the movie (`done`, `user` for a tap
/// that skipped it, or `error`) in `movie_evt.txt`.
fn poll_overlay(env: &mut Environment) {
    let Some((player, seq)) = State::get(env).overlay_player else {
        return;
    };
    let path = crate::paths::user_data_base_path().join("movie_evt.txt");
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let mut lines = text.lines();
    if lines.next().and_then(|l| l.trim().parse::<u64>().ok()) != Some(seq) {
        return;
    }
    let reason = match lines.next().map(str::trim) {
        Some("done") => 0, // MPMovieFinishReasonPlaybackEnded
        Some("user") => 2, // MPMovieFinishReasonUserExited
        _ => 1,            // MPMovieFinishReasonPlaybackError
    };
    log!("Movie playback finished in the overlay (reason {})", reason);
    State::get(env).overlay_player = None;
    State::get(env).finish_reasons.insert(player, reason);
    retain(env, player); // Pending notifications own their sender.
    State::get(env).pending_notifications.push_back((
        MPMoviePlayerPlaybackDidFinishNotification,
        player,
        Instant::now(),
    ));
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation MPMoviePlayerController: NSObject

// TODO: actual playback

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(MPMoviePlayerControllerHostObject {
        content_url: nil,
        view: nil,
        background_view: nil,
        window: nil,
        previous_key_window: nil,
        fullscreen: false,
        should_autoplay: true,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithContentURL:(id)url { // NSURL*
    log!(
        "TODO: [(MPMoviePlayerController*){:?} initWithContentURL:{:?} ({:?})]",
        this,
        url,
        ns_url::to_rust_path(env, url),
    );

    retain(env, url);
    env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).content_url = url;

    // Defer the unsupported-decoder result until the application returns to
    // its run loop, allowing it to finish configuring the presentation.
    State::get(env).pending_notifications.push_back(
        (MPMoviePlayerContentPreloadDidFinishNotification, this, Instant::now())
    );
    retain(env, this); // Pending notifications own their sender.

    this
}

- (())dealloc {
    dismiss_fullscreen(env, this);
    let url = env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).content_url;
    release(env, url);
    let view = env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).view;
    release(env, view);
    let background_view = env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).background_view;
    release(env, background_view);

    env.objc.dealloc_object(this, &mut env.mem);
}

- (id)contentURL {
    env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).content_url
}

- (id)backgroundColor {
    msg_class![env; UIColor blackColor] // TODO
}
- (())setBackgroundColor:(id)color { // UIColor*
    todo_objc_setter!(this, color);
}

- (())setScalingMode:(MPMovieScalingMode)mode {
    todo_objc_setter!(this, mode);
}
- (())setUseApplicationAudioSession:(bool)use_session {
    todo_objc_setter!(this, use_session);
}
- (())setControlStyle:(MPMovieControlStyle)style {
    todo_objc_setter!(this, style);
}
- (())setFullscreen:(bool)fullsreen {
    env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).fullscreen = fullsreen;
    if fullsreen { present_fullscreen(env, this); } else { dismiss_fullscreen(env, this); }
}
- (())setInitialPlaybackTime:(NSTimeInterval)initial_time {
    todo_objc_setter!(this, initial_time);
}

- (id)view {
    player_view(env, this)
}

- (id)backgroundView {
    let existing = env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).background_view;
    if existing != nil {
        return existing;
    }
    let screen: id = msg_class![env; UIScreen mainScreen];
    let bounds: CGRect = msg![env; screen bounds];
    let view: id = msg_class![env; UIView alloc];
    let view: id = msg![env; view initWithFrame:bounds];
    let black: id = msg_class![env; UIColor blackColor];
    () = msg![env; view setBackgroundColor:black];
    env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).background_view = view;
    view
}

- (())setMovieSourceType:(NSInteger)_source_type {
    // Unused: there is no real decoder.
}

// Apps wait for a load-state notification before calling `play`. Report the
// content as ready (MPMovieLoadStatePlayable | MPMovieLoadStatePlaythroughOK);
// the later `play` ends with the "no decoder" completion notification.
- (())prepareToPlay {
    log!("TODO: [(MPMoviePlayerController*){:?} prepareToPlay]", this);
    State::get(env).pending_notifications.push_back(
        (MPMoviePlayerLoadStateDidChangeNotification, this, Instant::now())
    );
    retain(env, this); // Pending notifications own their sender.

    // Like on iOS, a player that should autoplay starts as soon as it is
    // prepared. Some apps (e.g. LEGO Harry Potter) never call `play` themselves
    // and only wait for the completion notification.
    if env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).should_autoplay {
        () = msg![env; this play];
        return;
    }
    // Without autoplay nothing starts the movie; report the (failed) playback
    // as finished shortly afterwards, otherwise apps that only wait for the
    // completion notification would wait forever on a black screen.
    let already_pending = State::get(env)
        .pending_notifications
        .iter()
        .any(|&(name, obj, _)| name == MPMoviePlayerPlaybackDidFinishNotification && obj == this);
    if !already_pending {
        State::get(env).pending_notifications.push_back((
            MPMoviePlayerPlaybackDidFinishNotification,
            this,
            Instant::now() + Duration::from_millis(1500),
        ));
        retain(env, this);
    }
}
- (bool)shouldAutoplay {
    env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).should_autoplay
}
- (())setShouldAutoplay:(bool)should_autoplay {
    env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).should_autoplay = should_autoplay;
}
- (bool)isPreparedToPlay {
    true
}
- (NSInteger)loadState {
    3
}

- (MPMoviePlaybackState)playbackState {
    MPMoviePlaybackStateStopped // TODO
}

// Apparently an undocumented, private API, but Spore Origins uses it.
- (())setMovieControlMode:(NSInteger)_mode {
    // As this is undocumented and we don't have real video playback yet, let's
    // ignore it.
}

// Another undocumented one! But some apps may still use it :/
// https://stackoverflow.com/a/1390079/2241008
- (())setOrientation:(UIDeviceOrientation)_orientation animated:(bool)_animated {

}

// MPMediaPlayback implementation
- (())play {
    log!("TODO: [(MPMoviePlayerController*){:?} play]", this);
    if let Some(old) = env.framework_state.media_player.movie_player.active_player {
        let _: () = msg![env; old stop];
    }
    assert!(env.framework_state.media_player.movie_player.active_player.is_none());
    // Movie player is retained by the runtime until it is stopped
    retain(env, this);
    env.framework_state.media_player.movie_player.active_player = Some(this);

    let version = env.options.reported_ios_version.clone()
        .or_else(|| env.bundle.minimum_os_version().map(str::to_owned))
        .unwrap_or_else(|| "2.0".into());
    if legacy_fullscreen(&version) || env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).fullscreen {
        present_fullscreen(env, this);
    }

    if State::get(env).overlay_player.map(|(player, _)| player) == Some(this) {
        return;
    }
    if State::get(env).overlay_player.is_none() && start_overlay(env, this) {
        // The overlay reports the end of the movie.
        return;
    }
    // Without a movie decoder, report a playback error through the documented
    // completion notification, rather than successful playback.
    let notif = (MPMoviePlayerPlaybackDidFinishNotification, this, Instant::now().checked_add(Duration::from_millis(1000)).unwrap());
    for (name, obj, _) in &mut State::get(env).pending_notifications {
        // De-duplicate similar notifications. This can happen if app is calling
        // `play` twice on the same player object (case of NOVA2).
        if *name == MPMoviePlayerPlaybackDidFinishNotification && *obj == this {
            return;
        }
    }
    State::get(env).pending_notifications.push_back(notif);
    retain(env, this);
}

- (())pause {
    log!("TODO: [(MPMoviePlayerController*){:?} pause]", this);
}

- (())stop {
    log!("[(MPMoviePlayerController*){:?} stop]", this);
    stop_overlay(env, this);
    dismiss_fullscreen(env, this);
    if env.framework_state.media_player.movie_player.active_player == Some(this) {
        // Some applications (like NOVA2) may send 2 `stop` messages for each
        // 1 `play` message for the player. In that case, we want to release
        // the active player only once.
        env.framework_state.media_player.movie_player.active_player = None;
        release(env, this);
    }
}

@end

@implementation MPMoviePlayerViewController: UIViewController

- (id)initWithContentURL:(id)url {
    log!(
        "TODO: [(MPMoviePlayerViewController*){:?} initWithContentURL:{:?} ({:?})] -> nil",
        this,
        url,
        ns_url::to_rust_path(env, url),
    );
    release(env, this);
    nil // TODO
}

@end

};

/// For use by `NSRunLoop` via [super::handle_players]: check movie players'
/// status, send notifications if necessary.
pub(super) fn handle_players(env: &mut Environment) {
    poll_overlay(env);
    let mut notifs_to_run = Vec::new();
    let pending_notifs = &mut State::get(env).pending_notifications;
    let mut i = 0;
    while i < pending_notifs.len() {
        let (name_str, object, time) = pending_notifs[i];
        if Instant::now() >= time {
            notifs_to_run.push((name_str, object));
            pending_notifs.swap_remove_back(i);
        } else {
            i += 1;
        }
    }
    for (name_str, object) in notifs_to_run {
        log_dbg!("Posting movie player notification {} for {:?}", name_str, object);
        let name = ns_string::get_static_str(env, name_str);
        let center: id = msg_class![env; NSNotificationCenter defaultCenter];
        if name_str == MPMoviePlayerPlaybackDidFinishNotification {
            let finished = State::get(env).finish_reasons.remove(&object);
            if let Some(finish_reason) = finished {
                // The movie really played (or the user skipped it).
                let reason: id = msg_class![env; NSNumber numberWithInteger:finish_reason];
                let key = ns_string::get_static_str(
                    env,
                    MPMoviePlayerPlaybackDidFinishReasonUserInfoKey,
                );
                let info: id = msg_class![env; NSMutableDictionary dictionary];
                () = msg![env; info setObject:reason forKey:key];
                () = msg![env; center postNotificationName:name object:object userInfo:info];
                if State::get(env).active_player == Some(object) {
                    () = msg![env; object stop];
                }
                release(env, object);
                continue;
            }
            let reason: id = msg_class![env; NSNumber numberWithInteger:2i32];
            let key =
                ns_string::get_static_str(env, MPMoviePlayerPlaybackDidFinishReasonUserInfoKey);
            let info: id = msg_class![env; NSMutableDictionary dictionary];
            () = msg![env; info setObject:reason forKey:key];
            let domain = ns_string::get_static_str(env, "touchHLEMediaPlaybackErrorDomain");
            let error_code: NSInteger = -1;
            let error: id =
                msg_class![env; NSError errorWithDomain:domain code:error_code userInfo:nil];
            let error_key = ns_string::get_static_str(env, "error");
            () = msg![env; info setObject:error forKey:error_key];
            log!("Movie playback failed: no movie decoder is available");
            () = msg![env; center postNotificationName:name object:object userInfo:info];
            // The callback can stop the player or start another one. Do not
            // dismiss a new presentation belonging to a different player.
            if State::get(env).active_player == Some(object) {
                () = msg![env; object stop];
            }
        } else {
            () = msg![env; center postNotificationName:name object:object];
        }
        release(env, object);
    }
}

#[cfg(test)]
mod tests {
    use super::legacy_fullscreen;

    #[test]
    fn legacy_presentation_changes_at_ios_3_2() {
        assert!(legacy_fullscreen("2.0"));
        assert!(legacy_fullscreen("3.0"));
        assert!(legacy_fullscreen("3.1.3"));
        assert!(!legacy_fullscreen("3.2"));
        assert!(!legacy_fullscreen("4.0"));
        assert!(!legacy_fullscreen("14.0"));
    }
}
