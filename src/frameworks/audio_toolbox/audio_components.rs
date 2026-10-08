/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `AudioComponent.h` (Audio Component Services)

use std::collections::HashMap;
use std::time::Instant;

use crate::abi::GuestFunction;
use crate::audio::openal::al_types::ALuint;
use crate::dyld::FunctionExports;
use crate::environment::Environment;
use crate::export_c_func;
use crate::frameworks::carbon_core::{paramErr, OSStatus};
use crate::frameworks::core_audio_types::debug_fourcc;
use crate::frameworks::core_audio_types::{
    fourcc, kAudioFormatFlagIsAlignedHigh, kAudioFormatFlagIsFloat, kAudioFormatFlagIsPacked,
    kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM, AudioStreamBasicDescription,
};
use crate::mem::{ConstPtr, ConstVoidPtr, MutPtr, Ptr, SafeRead};

const kAudioUnitType_Output: u32 = fourcc(b"auou");
const kAudioUnitType_Mixer: u32 = fourcc(b"aumx");
const kAudioUnitSubType_MultiChannelMixer: u32 = fourcc(b"mcmx");
const kAudioUnitSubType_AU3DMixerEmbedded: u32 = fourcc(b"3dem");
const kAudioUnitSubType_RemoteIO: u32 = fourcc(b"rioc");
const kAudioUnitManufacturer_Apple: u32 = fourcc(b"appl");

#[derive(Default)]
pub struct State {
    pub audio_component: AudioComponent,
    /// The component handed out for the (3D/multichannel) mixer unit.
    pub mixer_component: AudioComponent,
    pub audio_component_instances:
        HashMap<AudioComponentInstance, AudioComponentInstanceHostObject>,
}
impl State {
    pub fn get(framework_state: &mut crate::frameworks::State) -> &mut Self {
        &mut framework_state.audio_toolbox.audio_components
    }
}

/// Which kind of audio unit an instance is.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum AudioUnitKind {
    /// The RemoteIO output unit: pulled by the host, plays through OpenAL.
    RemoteIO,
    /// A mixer unit (`aumx`: 3D mixer or multichannel mixer): pulled with
    /// `AudioUnitRender()` by whoever uses it.
    Mixer,
}

#[derive(Clone)]
pub struct AudioComponentInstanceHostObject {
    pub kind: AudioUnitKind,
    /// Number of elements (buses) per scope, from `kAudioUnitProperty_ElementCount`.
    pub element_counts: HashMap<u32, u32>,
    /// Stream formats set per (scope, element).
    pub bus_formats: HashMap<(u32, u32), AudioStreamBasicDescription>,
    /// Render callbacks set per input element (bus).
    pub bus_callbacks: HashMap<u32, AURenderCallbackStruct>,
    /// Input element -> (source unit, source output element), from
    /// `kAudioUnitProperty_MakeConnection`.
    pub connections: HashMap<u32, (AudioComponentInstance, u32)>,
    pub started: bool,
    pub maximum_frames_per_slice: u32,
    pub global_stream_format: AudioStreamBasicDescription,
    pub input_stream_format: Option<AudioStreamBasicDescription>,
    pub output_stream_format: Option<AudioStreamBasicDescription>,
    pub render_callback: Option<AURenderCallbackStruct>,
    pub last_render_time: Option<Instant>,
    pub al_source: Option<ALuint>,
    pub is_running_handler: bool,
    /// Callbacks registered with `AudioUnitAddRenderNotify`, called before
    /// and after each render.
    pub render_notifies: Vec<AURenderCallbackStruct>,
    /// Values stored by `AudioUnitSetParameter`, keyed by (id, scope, element).
    pub parameters: HashMap<(u32, u32, u32), f32>,
}
impl Default for AudioComponentInstanceHostObject {
    fn default() -> Self {
        // Default values obtained from an iPod Touch 4 running iOS 6.1.6
        // through a test app built targetting iOS 2.0
        AudioComponentInstanceHostObject {
            started: false,
            kind: AudioUnitKind::RemoteIO,
            element_counts: HashMap::new(),
            bus_formats: HashMap::new(),
            bus_callbacks: HashMap::new(),
            connections: HashMap::new(),
            // returning 1024 based on https://developer.apple.com/documentation/audiotoolbox/kaudiounitproperty_maximumframesperslice
            maximum_frames_per_slice: 1024,
            global_stream_format: AudioStreamBasicDescription {
                sample_rate: 44100.0,
                format_id: kAudioFormatLinearPCM,
                format_flags: kAudioFormatFlagIsFloat
                    | kAudioFormatFlagIsSignedInteger
                    | kAudioFormatFlagIsPacked
                    | kAudioFormatFlagIsAlignedHigh,
                bytes_per_packet: 4,
                frames_per_packet: 1,
                bytes_per_frame: 4,
                channels_per_frame: 2,
                bits_per_channel: 32,
                _reserved: 0,
            },
            input_stream_format: None,
            output_stream_format: None,
            render_callback: None,
            last_render_time: None,
            al_source: None,
            is_running_handler: false,
            render_notifies: Vec::new(),
            parameters: HashMap::new(),
        }
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
#[allow(dead_code)]
pub struct AURenderCallbackStruct {
    pub input_proc: AURenderCallback,
    pub input_proc_ref_con: ConstVoidPtr,
}
unsafe impl SafeRead for AURenderCallbackStruct {}

#[repr(C, packed)]
pub struct OpaqueAudioComponent {
    _pad: u8,
}
unsafe impl SafeRead for OpaqueAudioComponent {}

type AudioComponent = MutPtr<OpaqueAudioComponent>;

pub type AURenderCallback = GuestFunction;

#[repr(C, packed)]
pub struct OpaqueAudioComponentInstance {
    _pad: u8,
}
unsafe impl SafeRead for OpaqueAudioComponentInstance {}

pub type AudioComponentInstance = MutPtr<OpaqueAudioComponentInstance>;

#[repr(C, packed)]
struct AudioComponentDescription {
    component_type: u32,
    component_sub_type: u32,
    component_manufacturer: u32,
    component_flags: u32,
    component_flags_mask: u32,
}
unsafe impl SafeRead for AudioComponentDescription {}

fn AudioComponentFindNext(
    env: &mut Environment,
    in_component: AudioComponent,
    in_desc: ConstPtr<AudioComponentDescription>,
) -> AudioComponent {
    assert!(in_component.is_null());

    let audio_comp_descr = env.mem.read(in_desc);
    // Only the RemoteIO output unit and the mixers exist. For anything else
    // (effects, format converters, …) report "no such component", which apps
    // are meant to handle.
    let is_mixer = audio_comp_descr.component_type == kAudioUnitType_Mixer
        && (audio_comp_descr.component_sub_type == kAudioUnitSubType_AU3DMixerEmbedded
            || audio_comp_descr.component_sub_type == kAudioUnitSubType_MultiChannelMixer)
        && audio_comp_descr.component_manufacturer == kAudioUnitManufacturer_Apple;
    if is_mixer {
        let state = State::get(&mut env.framework_state);
        if state.mixer_component.is_null() {
            state.mixer_component = env.mem.alloc_and_write(OpaqueAudioComponent { _pad: 0 });
        }
        let component = State::get(&mut env.framework_state).mixer_component;
        log!("TODO: AudioComponentFindNext() for a mixer unit -> {:?} (mixing is not fully implemented)", component);
        return component;
    }
    if audio_comp_descr.component_type != kAudioUnitType_Output
        || audio_comp_descr.component_sub_type != kAudioUnitSubType_RemoteIO
        || audio_comp_descr.component_manufacturer != kAudioUnitManufacturer_Apple
    {
        let (t, s, m) = (
            audio_comp_descr.component_type,
            audio_comp_descr.component_sub_type,
            audio_comp_descr.component_manufacturer,
        );
        log!(
            "TODO: AudioComponentFindNext() for unsupported component type {} subtype {} manufacturer {}, returning NULL",
            debug_fourcc(t),
            debug_fourcc(s),
            debug_fourcc(m)
        );
        return Ptr::null();
    }

    let state = State::get(&mut env.framework_state);
    if state.audio_component.is_null() {
        state.audio_component = env.mem.alloc_and_write(OpaqueAudioComponent { _pad: 0 });
    }

    let out_component: AudioComponent = state.audio_component;

    log!(
        "TODO: AudioComponentFindNext({:?}, {:?}) -> {:?}",
        in_component,
        in_desc,
        out_component
    );
    out_component
}

fn AudioComponentInstanceNew(
    env: &mut Environment,
    in_component: AudioComponent,
    out_instance: MutPtr<AudioComponentInstance>,
) -> OSStatus {
    let mut host_object = AudioComponentInstanceHostObject::default();
    {
        let mixer_component = State::get(&mut env.framework_state).mixer_component;
        if !mixer_component.is_null() && in_component == mixer_component {
            host_object.kind = AudioUnitKind::Mixer;
        }
    }

    let guest_instance: AudioComponentInstance = env
        .mem
        .alloc_and_write(OpaqueAudioComponentInstance { _pad: 0 });
    State::get(&mut env.framework_state)
        .audio_component_instances
        .insert(guest_instance, host_object);

    env.mem.write(out_instance, guest_instance);

    let result = 0; // success
    log_dbg!(
        "AudioComponentInstanceNew({:?}, {:?}) -> {:?}",
        in_component,
        out_instance,
        result
    );
    result
}

fn AudioComponentInstanceDispose(
    env: &mut Environment,
    in_instance: AudioComponentInstance,
) -> OSStatus {
    let result = if in_instance.is_null() {
        paramErr
    } else {
        State::get(&mut env.framework_state)
            .audio_component_instances
            .remove(&in_instance);
        env.mem.free(in_instance.cast());
        0
    };
    log_dbg!(
        "AudioComponentInstanceDispose({:?}) -> {:?}",
        in_instance,
        result
    );
    result
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(AudioComponentFindNext(_, _)),
    export_c_func!(AudioComponentInstanceNew(_, _)),
    export_c_func!(AudioComponentInstanceDispose(_)),
];
