/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! AVAudioSession

use crate::dyld::{ConstantExports, HostConstant};
use crate::frameworks::foundation::ns_string;
use crate::mem::MutPtr;
use crate::objc::{id, objc_classes, ClassExports, TrivialHostObject};
use crate::todo_objc_setter;

type AVAudioSessionCategory = id; // NSString *

#[derive(Default)]
pub struct State {
    /// [AVAudioSession sharedInstance]
    shared_instance: Option<id>,
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

// This is a singleton.
@implementation AVAudioSession: NSObject

+ (id)sharedInstance {
    if let Some(audio_session) =
        env.framework_state.avfoundation.av_audio_session.shared_instance {
        audio_session
    } else {
        let new = env.objc.alloc_static_object(
            this,
            Box::new(TrivialHostObject),
            &mut env.mem
        );
        env.framework_state.avfoundation.av_audio_session.shared_instance = Some(new);
        new
    }
}

- (id)retain { this }
- (())release {}
- (id)autorelease { this }

- (())setDelegate:(id)delegate {
    todo_objc_setter!(this, delegate);
}

- (bool)setCategory:(AVAudioSessionCategory)category
              error:(MutPtr<id>)error { // NSError **
    log!(
        "TODO: [(AVAudioSession *){:?} setCategory:'{}' error:{:?}] -> true",
        this,
        ns_string::to_rust_string(env, category),
        error
    );
    true
}

- (bool)setActive:(bool)active
            error:(MutPtr<id>)error { // NSError **
    log!(
        "TODO: [(AVAudioSession *){:?} setActive:{} error:{:?}] -> true",
        this,
        active,
        error
    );
    true
}

@end

};

// Values might not be correct, but as these are linked symbol constants, it
// shouldn't matter.
const AVAudioSessionCategoryAmbient: &str = "AVAudioSessionCategoryAmbient";
const AVAudioSessionCategoryMultiRoute: &str = "AVAudioSessionCategoryMultiRoute";
const AVAudioSessionCategoryPlayAndRecord: &str = "AVAudioSessionCategoryPlayAndRecord";
const AVAudioSessionCategoryPlayback: &str = "AVAudioSessionCategoryPlayback";
const AVAudioSessionCategoryRecord: &str = "AVAudioSessionCategoryRecord";
const AVAudioSessionCategorySoloAmbient: &str = "AVAudioSessionCategorySoloAmbient";
const AVAudioSessionCategoryAudioProcessing: &str = "AVAudioSessionCategoryAudioProcessing";

/// `AVAudioSessionCategory` constants
pub const CONSTANTS: ConstantExports = &[
    (
        "_AVAudioSessionCategoryAmbient",
        HostConstant::NSString(AVAudioSessionCategoryAmbient),
    ),
    (
        "_AVAudioSessionCategoryMultiRoute",
        HostConstant::NSString(AVAudioSessionCategoryMultiRoute),
    ),
    (
        "_AVAudioSessionCategoryPlayAndRecord",
        HostConstant::NSString(AVAudioSessionCategoryPlayAndRecord),
    ),
    (
        "_AVAudioSessionCategoryPlayback",
        HostConstant::NSString(AVAudioSessionCategoryPlayback),
    ),
    (
        "_AVAudioSessionCategoryRecord",
        HostConstant::NSString(AVAudioSessionCategoryRecord),
    ),
    (
        "_AVAudioSessionCategorySoloAmbient",
        HostConstant::NSString(AVAudioSessionCategorySoloAmbient),
    ),
    (
        "_AVAudioSessionCategoryAudioProcessing",
        HostConstant::NSString(AVAudioSessionCategoryAudioProcessing),
    ),
    // Notification keys / port names that apps (e.g. Flappy Bird's audio
    // helper) dereference unconditionally. Their values never matter, but a
    // NULL symbol makes the guest read address 0.
    (
        "_AVLayerVideoGravityResizeAspect",
        HostConstant::NSString("AVLayerVideoGravityResizeAspect"),
    ),
    (
        "_AVLayerVideoGravityResizeAspectFill",
        HostConstant::NSString("AVLayerVideoGravityResizeAspectFill"),
    ),
    (
        "_AVLayerVideoGravityResize",
        HostConstant::NSString("AVLayerVideoGravityResize"),
    ),
    (
        "_kCFStreamPropertyAppendToFile",
        HostConstant::NSString("kCFStreamPropertyAppendToFile"),
    ),
    // CMTime { value: i64, timescale: i32, flags: u32 (valid = 1), epoch: i64 }
    (
        "_kCMTimeZero",
        HostConstant::Custom(|env| {
            let time = env.mem.alloc(24);
            env.mem.bytes_at_mut(time.cast::<u8>(), 24).fill(0);
            env.mem.write(time.cast::<i32>() + 2, 1i32);
            env.mem.write(time.cast::<u32>() + 3, 1u32);
            time.cast().cast_const()
        }),
    ),
    (
        "_AVAudioSessionInterruptionNotification",
        HostConstant::NSString("AVAudioSessionInterruptionNotification"),
    ),
    (
        "_AVAudioSessionInterruptionOptionKey",
        HostConstant::NSString("AVAudioSessionInterruptionOptionKey"),
    ),
    (
        "_AVAudioSessionInterruptionTypeKey",
        HostConstant::NSString("AVAudioSessionInterruptionTypeKey"),
    ),
    (
        "_AVAudioSessionPortBuiltInSpeaker",
        HostConstant::NSString("AVAudioSessionPortBuiltInSpeaker"),
    ),
    (
        "_AVAudioSessionPortHeadphones",
        HostConstant::NSString("AVAudioSessionPortHeadphones"),
    ),
    (
        "_UIApplicationSignificantTimeChangeNotification",
        HostConstant::NSString("UIApplicationSignificantTimeChangeNotification"),
    ),
    (
        "_NSGregorianCalendar",
        HostConstant::NSString("gregorian"),
    ),
    (
        "_NSUnderlyingErrorKey",
        HostConstant::NSString("NSUnderlyingError"),
    ),
    (
        "_SKStoreProductParameterITunesItemIdentifier",
        HostConstant::NSString("SKStoreProductParameterITunesItemIdentifier"),
    ),
    (
        "_ADBannerContentSizeIdentifierPortrait",
        HostConstant::NSString("ADBannerContentSize320x50"),
    ),
    (
        "_ADBannerContentSizeIdentifierLandscape",
        HostConstant::NSString("ADBannerContentSize480x32"),
    ),
];
