/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `AudioUnit.h` (Audio Unit Services)
//!
//! [Audio Unit Programming Guide](https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/AudioUnitProgrammingGuide/TheAudioUnit/TheAudioUnit.html)

use std::time::Instant;

// Explicit opt-in capture of emulated RE4 output, never microphone input.
// The marker is checked once per game process and output is bounded to 8 MiB.
fn capture_re4_output(
    bundle_id: &str,
    data: &[u8],
    format: AudioStreamBasicDescription,
    playback_rate: f64,
    frames: u32,
    elapsed_seconds: f64,
) {
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    struct Capture {
        pcm: std::fs::File,
        timing: std::fs::File,
        bytes: usize,
    }
    static CAPTURE: OnceLock<Mutex<Option<Capture>>> = OnceLock::new();
    if bundle_id != "jp.co.capcom.res4pad" {
        return;
    }
    let state = CAPTURE.get_or_init(|| {
        let base = crate::paths::user_data_base_path();
        let result = if base.join("touchHLE_audio_capture.enable").exists() {
            (|| -> std::io::Result<Capture> {
                let pcm = std::fs::File::create(base.join("re4-audio-output.pcm"))?;
                let mut timing = std::fs::File::create(base.join("re4-audio-timing.csv"))?;
                writeln!(
                    timing,
                    "# format={:?}; playback_rate={}; flags={:#x}",
                    format,
                    playback_rate,
                    { format.format_flags }
                )?;
                writeln!(timing, "frames,elapsed_seconds,bytes")?;
                Ok(Capture {
                    pcm,
                    timing,
                    bytes: 0,
                })
            })()
            .ok()
        } else {
            None
        };
        Mutex::new(result)
    });
    let Ok(mut state) = state.lock() else {
        return;
    };
    let Some(capture) = state.as_mut() else {
        return;
    };
    if capture.bytes >= 8 * 1024 * 1024 {
        return;
    }
    let length = data.len().min(8 * 1024 * 1024 - capture.bytes);
    if capture.pcm.write_all(&data[..length]).is_ok() {
        capture.bytes += length;
        let _ = writeln!(
            capture.timing,
            "{},{:.9},{}",
            frames, elapsed_seconds, length
        );
    }
}

use crate::audio::openal::al_types::{ALuint, ALvoid};
use crate::audio::openal::{AL_BUFFERS_PROCESSED, AL_PLAYING, AL_SOURCE_STATE};

use crate::abi::CallFromHost;
use crate::dyld::FunctionExports;
use crate::environment::Environment;
use crate::export_c_func;
use crate::frameworks::audio_toolbox::audio_components;
use crate::frameworks::audio_toolbox::audio_queue::{
    is_supported_audio_format, log_if_broken_audio_format,
};
use crate::frameworks::carbon_core::{paramErr, OSStatus};
use crate::frameworks::core_audio_types::{
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsPacked, kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM,
    AudioStreamBasicDescription,
};
use crate::frameworks::core_foundation::cf_run_loop::CFRunLoopGetMain;
use crate::frameworks::foundation::ns_run_loop;
use crate::mem::{guest_size_of, ConstVoidPtr, MutPtr, MutVoidPtr, Ptr, SafeRead};
use crate::objc::nil;

use super::audio_components::{
    AURenderCallback, AURenderCallbackStruct, AudioComponentInstance, AudioUnitKind,
};
use super::audio_queue::decode_buffer;
use super::audio_session;

/// Describe the interleaved PCM buffer allocated for a render callback. The
/// callback receives channels * sample bytes per frame even when the guest's
/// ASBD has contradictory stride fields. Keep this correction local to the
/// AudioUnit buffer rather than changing AudioQueue's legacy format handling.
fn callback_decode_format(
    mut format: AudioStreamBasicDescription,
    bundle_id: &str,
) -> Result<AudioStreamBasicDescription, &'static str> {
    const NON_INTERLEAVED: u32 = 1 << 5;
    // RE4 advertises non-interleaved mono-sized strides, but its callback at
    // 0x84534 writes four bytes per frame into each buffer and duplicates that
    // buffer. Its mixer at 0x84b64 uses two little-endian strh writes for the
    // same sample. Treat this specific contradictory format as packed stereo;
    // otherwise the legacy AudioQueue path drops a channel and swaps bytes.
    if bundle_id == "jp.co.capcom.res4pad"
        && format.format_id == kAudioFormatLinearPCM
        && format.format_flags
            & (kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked | NON_INTERLEAVED)
            == (kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked | NON_INTERLEAVED)
        && format.channels_per_frame == 2
        && format.bits_per_channel == 16
        && format.bytes_per_frame == 2
        && format.bytes_per_packet == 2
        && format.frames_per_packet == 1
    {
        format.format_flags &= !NON_INTERLEAVED;
    }
    if format.format_id == kAudioFormatLinearPCM
        && format.format_flags & kAudioFormatFlagIsPacked != 0
        && format.format_flags & NON_INTERLEAVED == 0
    {
        if format.bits_per_channel == 0 || format.bits_per_channel % 8 != 0 {
            return Err("Invalid packed PCM sample width");
        }
        let stride = format
            .channels_per_frame
            .checked_mul(format.bits_per_channel / 8)
            .filter(|&stride| stride != 0)
            .ok_or("Invalid packed PCM channel stride")?;
        format.bytes_per_frame = stride;
        format.bytes_per_packet = stride
            .checked_mul(format.frames_per_packet)
            .ok_or("Packed PCM packet stride overflow")?;
    }
    Ok(format)
}

#[cfg(test)]
mod callback_format_tests {
    use super::*;

    fn re4_format() -> AudioStreamBasicDescription {
        AudioStreamBasicDescription {
            sample_rate: 11025.0,
            format_id: kAudioFormatLinearPCM,
            format_flags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
            bytes_per_packet: 2,
            frames_per_packet: 1,
            bytes_per_frame: 2,
            channels_per_frame: 2,
            bits_per_channel: 16,
            _reserved: 0,
        }
    }

    #[test]
    fn re4_callback_uses_stereo_stride_without_changing_endianness() {
        let original = re4_format();
        let fixed = callback_decode_format(original, "").unwrap();
        let (frame, packet, flags, channels) = (
            fixed.bytes_per_frame,
            fixed.bytes_per_packet,
            fixed.format_flags,
            fixed.channels_per_frame,
        );
        assert_eq!((frame, packet, channels), (4, 4, 2));
        assert_eq!(
            flags,
            kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked
        );
        let original_stride = original.bytes_per_frame;
        assert_eq!(original_stride, 2);
        let mut mono = original;
        mono.channels_per_frame = 1;
        assert!(callback_decode_format(mono, "").unwrap() == mono);
    }

    #[test]
    fn callback_stride_checks_overflow_and_preserves_planar_formats() {
        let mut format = re4_format();
        format.format_flags |= 1 << 5;
        assert!(callback_decode_format(format, "").unwrap() == format);
        format = re4_format();
        format.channels_per_frame = u32::MAX;
        assert!(callback_decode_format(format, "").is_err());
        format = re4_format();
        format.frames_per_packet = u32::MAX;
        assert!(callback_decode_format(format, "").is_err());
    }

    #[test]
    fn re4_mislabeled_planar_callback_keeps_duplicated_little_endian_samples() {
        let mut original = re4_format();
        // The guest also sets alignment flags not shown by ASBD's Debug impl.
        original.format_flags |= (1 << 5) | (1 << 4);
        assert!(callback_decode_format(original, "another.app").unwrap() == original);
        let fixed = callback_decode_format(original, "jp.co.capcom.res4pad").unwrap();
        let (stride, packet, flags, channels) = (
            fixed.bytes_per_frame,
            fixed.bytes_per_packet,
            fixed.format_flags,
            fixed.channels_per_frame,
        );
        assert_eq!((stride, packet, channels), (4, 4, 2));
        assert_eq!(
            flags,
            kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked | (1 << 4)
        );
    }
}

pub type AudioUnit = AudioComponentInstance;
type AudioUnitPropertyID = u32;
type AudioUnitScope = u32;
type AudioUnitElement = u32;

#[repr(C, packed)]
pub struct AudioBufferList<const COUNT: usize> {
    pub number_buffers: u32,
    pub buffers: [AudioBuffer; COUNT],
}
unsafe impl SafeRead for AudioBufferList<1> {}
unsafe impl SafeRead for AudioBufferList<2> {}

#[repr(C, packed)]
pub struct AudioBuffer {
    pub number_channels: u32,
    pub data_byte_size: u32,
    pub data: MutVoidPtr,
}

// TODO: Other scopes
const kAudioUnitScope_Global: AudioUnitScope = 0;
const kAudioUnitScope_Input: AudioUnitScope = 1;
const kAudioUnitScope_Output: AudioUnitScope = 2;

const kAudioUnitProperty_SampleRate: AudioUnitPropertyID = 2;
const kAudioUnitProperty_ElementCount: AudioUnitPropertyID = 11;
const kAudioUnitProperty_MakeConnection: AudioUnitPropertyID = 1;

/// `kAudioUnitErr_InvalidProperty`
const kAudioUnitErr_InvalidProperty: OSStatus = -10879;
const kAudioUnitProperty_SetRenderCallback: AudioUnitPropertyID = 23;
const kAudioUnitProperty_MaximumFramesPerSlice: AudioUnitPropertyID = 14;
const kAudioUnitProperty_StreamFormat: AudioUnitPropertyID = 8;

const kAudioOutputUnitProperty_EnableIO: AudioUnitPropertyID = 2003;

fn unit_kind(env: &mut Environment, unit: AudioUnit) -> Option<AudioUnitKind> {
    audio_components::State::get(&mut env.framework_state)
        .audio_component_instances
        .get(&unit)
        .map(|host_object| host_object.kind)
}

fn AudioUnitInitialize(env: &mut Environment, in_unit: AudioUnit) -> OSStatus {
    if unit_kind(env, in_unit) == Some(AudioUnitKind::Mixer) {
        // Mixers are pulled with AudioUnitRender(), not run by the run loop.
        return 0;
    }
    let run_loop = CFRunLoopGetMain(env);
    ns_run_loop::add_audio_unit(env, run_loop, in_unit);
    0 // success
}

fn AudioUnitUninitialize(env: &mut Environment, in_unit: AudioUnit) -> OSStatus {
    if unit_kind(env, in_unit) == Some(AudioUnitKind::Mixer) {
        return 0;
    }
    let run_loop = CFRunLoopGetMain(env);
    match ns_run_loop::remove_audio_unit(env, run_loop, in_unit) {
        Ok(_) => 0,
        Err(_) => paramErr, // TODO: handle different errors
    }
}

fn AudioUnitSetProperty(
    env: &mut Environment,
    in_unit: AudioUnit,
    in_id: AudioUnitPropertyID,
    in_scope: AudioUnitScope,
    in_element: AudioUnitElement,
    in_data: ConstVoidPtr,
    in_data_size: u32,
) -> OSStatus {
    if unit_kind(env, in_unit) == Some(AudioUnitKind::Mixer) {
        return mixer_set_property(env, in_unit, in_id, in_scope, in_element, in_data);
    }
    assert!(in_element == 0);

    let host_object = audio_components::State::get(&mut env.framework_state)
        .audio_component_instances
        .get_mut(&in_unit)
        .unwrap();

    let result;
    match in_id {
        kAudioUnitProperty_SetRenderCallback => {
            assert_eq!(in_scope, kAudioUnitScope_Global);
            assert_eq!(in_data_size, guest_size_of::<AURenderCallbackStruct>());
            let render_callback = env.mem.read(in_data.cast::<AURenderCallbackStruct>());
            host_object.render_callback = Some(render_callback);
            result = 0;
            log_dbg!("AudioUnitSetProperty({:?}, kAudioUnitProperty_SetRenderCallback, {:?}, {:?}, {:?}, {:?}) -> {:?}", in_unit, in_scope, in_element, render_callback, in_data_size, result);
        }
        kAudioUnitProperty_StreamFormat => {
            assert_eq!(in_data_size, guest_size_of::<AudioStreamBasicDescription>());
            let stream_format = env.mem.read(in_data.cast::<AudioStreamBasicDescription>());
            log_if_broken_audio_format(&stream_format);
            match in_scope {
                kAudioUnitScope_Global => host_object.global_stream_format = stream_format,
                kAudioUnitScope_Output => host_object.output_stream_format = Some(stream_format),
                kAudioUnitScope_Input => host_object.input_stream_format = Some(stream_format),
                _ => unimplemented!("in_scope {}", in_scope),
            };
            result = 0;
            log_dbg!("AudioUnitSetProperty({:?}, kAudioUnitProperty_StreamFormat, {:?}, {:?}, {:?}, {:?}) -> {:?}", in_unit, in_scope, in_element, stream_format, in_data_size, result);
        }
        kAudioUnitProperty_MakeConnection => {
            let connection = env.mem.read(in_data.cast::<AudioUnitConnection>());
            host_object.connections.insert(
                connection.dest_input_number,
                (
                    connection.source_audio_unit,
                    connection.source_output_number,
                ),
            );
            result = 0;
        }
        kAudioOutputUnitProperty_EnableIO => {
            assert_eq!(in_scope, kAudioUnitScope_Output);
            assert_eq!(in_data_size, guest_size_of::<u32>());
            let enabled = env.mem.read(in_data.cast::<u32>());
            // Output is enabled by default.
            assert_eq!(enabled, 1);
            result = 0;
            log_dbg!("AudioUnitSetProperty({:?}, kAudioOutputUnitProperty_EnableIO, {:?}, {:?}, {:?}, {:?}) -> {:?}", in_unit, in_scope, in_element, enabled, in_data_size, result);
        }
        _ => {
            log!(
                "TODO: AudioUnitSetProperty({:?}, property {}, scope {}, element {}) is unsupported on this unit, returning an error",
                in_unit,
                in_id,
                in_scope,
                in_element
            );
            result = kAudioUnitErr_InvalidProperty;
        }
    };

    result
}

fn AudioUnitGetProperty(
    env: &mut Environment,
    in_unit: AudioUnit,
    in_id: AudioUnitPropertyID,
    in_scope: AudioUnitScope,
    in_element: AudioUnitElement,
    out_data: MutVoidPtr,
    io_data_size: MutPtr<u32>,
) -> OSStatus {
    if unit_kind(env, in_unit) == Some(AudioUnitKind::Mixer) {
        return mixer_get_property(
            env,
            in_unit,
            in_id,
            in_scope,
            in_element,
            out_data,
            io_data_size,
        );
    }
    assert!(in_element == 0);

    let host_object = audio_components::State::get(&mut env.framework_state)
        .audio_component_instances
        .get_mut(&in_unit)
        .unwrap();

    match in_id {
        kAudioUnitProperty_MaximumFramesPerSlice => {
            assert_eq!(env.mem.read(io_data_size), guest_size_of::<u32>());
            let max_frames: u32 = host_object.maximum_frames_per_slice;
            env.mem.write(out_data.cast(), max_frames);
            env.mem.write(io_data_size.cast(), guest_size_of::<u32>());
        }
        kAudioUnitProperty_StreamFormat => {
            assert_eq!(
                env.mem.read(io_data_size),
                guest_size_of::<AudioStreamBasicDescription>()
            );
            let stream_format = match in_scope {
                kAudioUnitScope_Global => host_object.global_stream_format,
                kAudioUnitScope_Output => host_object.output_stream_format.unwrap(),
                kAudioUnitScope_Input => host_object.input_stream_format.unwrap(),
                _ => unimplemented!(),
            };
            env.mem.write(out_data.cast(), stream_format);
            env.mem.write(
                io_data_size.cast(),
                guest_size_of::<AudioStreamBasicDescription>(),
            );
        }
        kAudioUnitProperty_SampleRate => {
            assert_eq!(env.mem.read(io_data_size), guest_size_of::<f64>());
            let sample_rate = match in_scope {
                kAudioUnitScope_Global => host_object.global_stream_format.sample_rate,
                kAudioUnitScope_Output => {
                    host_object
                        .output_stream_format
                        .unwrap_or(host_object.global_stream_format)
                        .sample_rate
                }
                kAudioUnitScope_Input => {
                    host_object
                        .input_stream_format
                        .unwrap_or(host_object.global_stream_format)
                        .sample_rate
                }
                _ => unimplemented!(),
            };
            env.mem.write(out_data.cast(), sample_rate);
            env.mem.write(io_data_size.cast(), guest_size_of::<f64>());
        }
        _ => unimplemented!("in_id {}", in_id),
    };
    0 // success
}

fn AudioUnitAddRenderNotify(
    env: &mut Environment,
    in_unit: AudioUnit,
    in_proc: AURenderCallback,
    in_proc_user_data: ConstVoidPtr,
) -> OSStatus {
    let Some(host_object) = audio_components::State::get(&mut env.framework_state)
        .audio_component_instances
        .get_mut(&in_unit)
    else {
        return paramErr;
    };
    host_object.render_notifies.push(AURenderCallbackStruct {
        input_proc: in_proc,
        input_proc_ref_con: in_proc_user_data,
    });
    log_dbg!(
        "AudioUnitAddRenderNotify({:?}, {:?}, {:?}) -> 0",
        in_unit,
        in_proc,
        in_proc_user_data
    );
    0 // success
}

type AudioUnitParameterID = u32;
type AudioUnitParameterValue = f32;

fn AudioUnitSetParameter(
    env: &mut Environment,
    in_unit: AudioUnit,
    in_id: AudioUnitParameterID,
    in_scope: AudioUnitScope,
    in_element: AudioUnitElement,
    in_value: AudioUnitParameterValue,
    _in_buffer_offset_in_frames: u32,
) -> OSStatus {
    let Some(host_object) = audio_components::State::get(&mut env.framework_state)
        .audio_component_instances
        .get_mut(&in_unit)
    else {
        return paramErr;
    };
    // TODO: parameters are only stored, not applied (RemoteIO has hardly any
    // parameters anyway).
    log_once!("TODO: AudioUnitSetParameter() values are stored but have no effect on output");
    host_object
        .parameters
        .insert((in_id, in_scope, in_element), in_value);
    0 // success
}

fn AudioUnitGetParameter(
    env: &mut Environment,
    in_unit: AudioUnit,
    in_id: AudioUnitParameterID,
    in_scope: AudioUnitScope,
    in_element: AudioUnitElement,
    out_value: MutPtr<AudioUnitParameterValue>,
) -> OSStatus {
    let Some(host_object) = audio_components::State::get(&mut env.framework_state)
        .audio_component_instances
        .get_mut(&in_unit)
    else {
        return paramErr;
    };
    let value = host_object
        .parameters
        .get(&(in_id, in_scope, in_element))
        .copied()
        .unwrap_or(0.0);
    env.mem.write(out_value, value);
    0 // success
}

fn AudioOutputUnitStart(env: &mut Environment, ci: AudioUnit) -> OSStatus {
    let context = env
        .framework_state
        .audio_toolbox
        .make_al_context_current(env.openal_manager.as_mut());

    let mut source: ALuint = 0;
    unsafe {
        context.GenSources(1, &mut source);
        context.SourcePlay(source);
        assert_eq!(context.GetError(), 0);
    }

    let audio_components_state = audio_components::State::get(&mut env.framework_state);
    let audio_unit_state = audio_components_state
        .audio_component_instances
        .get_mut(&ci)
        .unwrap();
    audio_unit_state.al_source = Some(source);
    audio_unit_state.last_render_time = Some(Instant::now());
    audio_unit_state.started = true;

    let result = 0; // Success
    log_dbg!("AudioOutputUnitStart({:?}) -> {:?}", ci, result);
    result
}

fn AudioOutputUnitStop(env: &mut Environment, ci: AudioUnit) -> OSStatus {
    let at_state = &mut env.framework_state.audio_toolbox;
    let context = at_state
        .al_context
        .make_al_context_current(env.openal_manager.as_mut());

    let audio_components_state = &mut at_state.audio_components;

    let result = if let Some(audio_unit_state) = audio_components_state
        .audio_component_instances
        .get_mut(&ci)
    {
        audio_unit_state.started = false;
        audio_unit_state.last_render_time = None;

        if let Some(al_source) = audio_unit_state.al_source {
            unsafe {
                context.DeleteSources(1, &al_source);
                assert_eq!(context.GetError(), 0);
            }
        }
        audio_unit_state.al_source = None;
        0 // success
    } else {
        -1
    };
    log_dbg!("AudioOutputUnitStop({:?}) -> {:?}", ci, result);
    result
}

pub fn render_audio_unit(env: &mut Environment, audio_unit: AudioUnit) {
    if env.bundle.bundle_identifier().starts_with("com.ea.simcity") {
        // If enabled, we have some random crashes inside AURenderCallback ;(
        log_dbg!("Applying game-specific hack for SimCity: skipping rendering of audio units");
        return;
    }

    let at_state = &mut env.framework_state.audio_toolbox;
    let context = at_state
        .al_context
        .make_al_context_current(env.openal_manager.as_mut());

    let audio_session::State {
        current_hardware_sample_rate,
        ..
    } = at_state.audio_session;

    let audio_components_state = &mut at_state.audio_components;
    let audio_unit_host_object = audio_components_state
        .audio_component_instances
        .get_mut(&audio_unit)
        .unwrap();

    if !audio_unit_host_object.started {
        return;
    }

    if audio_unit_host_object.is_running_handler {
        return;
    }

    audio_unit_host_object.is_running_handler = true;

    let input_stream_format = audio_unit_host_object.input_stream_format;
    let output_stream_format = audio_unit_host_object.output_stream_format;
    // An output unit with no render callback of its own can instead have another
    // unit (a mixer) connected to its input bus 0, which it pulls audio from.
    let connected_source = audio_unit_host_object.connections.get(&0).copied();
    let connected = audio_unit_host_object.render_callback.is_none() && connected_source.is_some();
    let stream_format = if input_stream_format.is_some()
        && output_stream_format.is_some()
        && input_stream_format != output_stream_format
    {
        unimplemented!("AudioUnit {:?} has non default and different input {:?} and output {:?} stream formats, conversion is needed", audio_unit, input_stream_format, output_stream_format);
    } else if connected && input_stream_format.is_none() && output_stream_format.is_none() {
        default_mixer_stream_format()
    } else {
        // For purposes, the only important part is that format is supported
        // and playable by OpenAL. Thus, it doesn't really matter if input or
        // output format is defined by the application.
        // (but not both at the same time, see the check above)
        input_stream_format
            .unwrap_or(output_stream_format.unwrap_or(audio_unit_host_object.global_stream_format))
    };
    let sample_rate = if let Some(input_stream_format) = input_stream_format {
        input_stream_format.sample_rate
    } else if connected && output_stream_format.is_none() {
        stream_format.sample_rate
    } else {
        assert!(output_stream_format.is_some());
        // TODO: confirm that this is the general behaviour
        // (and not only RE4 thing)
        current_hardware_sample_rate
    };

    assert!(is_supported_audio_format(&stream_format));

    let al_source = audio_unit_host_object.al_source.unwrap();
    let mut al_buffers = Vec::new();
    unsafe {
        let mut buffers_processed = 0;
        context.GetSourcei(al_source, AL_BUFFERS_PROCESSED, &mut buffers_processed);
        while buffers_processed > 0 {
            let mut al_buffer = 0;
            context.SourceUnqueueBuffers(al_source, 1, &mut al_buffer);
            al_buffers.push(al_buffer);
            context.GetSourcei(al_source, AL_BUFFERS_PROCESSED, &mut buffers_processed);
        }
        assert_eq!(context.GetError(), 0);
    }

    let now = Instant::now();

    // Calculate number of frames by checking how much time passed since
    // the last render. Limit to 100ms to prevent delay from adding up
    // if it's been too long since the last render.
    // Ace Combat Xi relies on it being 2048 frames (at 48000Hz, 42ms) or under
    // If it's higher, flawed game logic causes it to call memset in a loop for
    // every frame over 2048 until it reaches the provided frame number.
    // TODO: Verify if this behavior is right
    let elapsed_time = now.duration_since(audio_unit_host_object.last_render_time.unwrap());
    let number_frames = ((elapsed_time.as_secs_f64() * sample_rate) as u32).min(2048);

    let bytes_per_channel = stream_format.bits_per_channel / 8;
    let actual_bytes_per_frame = stream_format.channels_per_frame * bytes_per_channel;

    let buffer_size = number_frames * actual_bytes_per_frame;

    // Alloc callback arguments
    let action_flags = env.mem.alloc_and_write(0);

    let (audio_buffer_list, buffer1Data, buffer2Data): (
        MutVoidPtr,
        MutVoidPtr,
        Option<MutVoidPtr>,
    ) = if input_stream_format.is_some() || connected {
        let bufferData = env.mem.alloc(buffer_size);
        let audio_buffer_list: AudioBufferList<1> = AudioBufferList {
            number_buffers: 1,
            buffers: [AudioBuffer {
                number_channels: stream_format.channels_per_frame,
                data_byte_size: buffer_size,
                data: bufferData,
            }],
        };
        (
            env.mem.alloc_and_write(audio_buffer_list).cast(),
            bufferData,
            None,
        )
    } else {
        // Resident Evil 4 expects 2 buffers
        // though it copies the same data to both
        let buffer1Data = env.mem.alloc(buffer_size);
        let buffer2Data = env.mem.alloc(buffer_size);
        let audio_buffer_list: AudioBufferList<2> = AudioBufferList {
            number_buffers: 2,
            buffers: [
                AudioBuffer {
                    number_channels: stream_format.channels_per_frame,
                    data_byte_size: buffer_size,
                    data: buffer1Data,
                },
                AudioBuffer {
                    number_channels: stream_format.channels_per_frame,
                    data_byte_size: buffer_size,
                    data: buffer2Data,
                },
            ],
        };
        (
            env.mem.alloc_and_write(audio_buffer_list).cast(),
            buffer1Data,
            Some(buffer2Data),
        )
    };

    // Run render callback, surrounded by any render notify callbacks
    // (kAudioUnitRenderAction_PreRender / _PostRender).
    let render_callback = audio_unit_host_object.render_callback;
    let render_notifies = audio_unit_host_object.render_notifies.clone();
    let call_notifies = |env: &mut Environment, flags: u32| {
        for notify in &render_notifies {
            env.mem.write(action_flags, flags);
            let AURenderCallbackStruct {
                input_proc,
                input_proc_ref_con,
            } = *notify;
            let _: OSStatus = input_proc.call_from_host(
                env,
                (
                    input_proc_ref_con,
                    action_flags,
                    nil.cast_void().cast_const(),
                    0u32,
                    number_frames,
                    audio_buffer_list,
                ),
            );
        }
        env.mem.write(action_flags, 0);
    };
    const kAudioUnitRenderAction_PreRender: u32 = 1 << 2;
    const kAudioUnitRenderAction_PostRender: u32 = 1 << 3;
    call_notifies(env, kAudioUnitRenderAction_PreRender);

    if let Some(AURenderCallbackStruct {
        input_proc: inputProc,
        input_proc_ref_con: inputProcRefCon,
    }) = render_callback
    {
        let () = inputProc.call_from_host(
            env,
            (
                inputProcRefCon,
                action_flags,
                nil.cast_void().cast_const(),
                0u32,
                number_frames,
                audio_buffer_list,
            ),
        );
    } else if let Some((source_unit, _output)) = connected_source {
        // Connected mixer: mix its input buses.
        mix_mixer_buses(env, source_unit, action_flags, 0, number_frames, audio_buffer_list);
    } else {
        silence_audio_buffer_list(env, audio_buffer_list);
    }

    call_notifies(env, kAudioUnitRenderAction_PostRender);

    let at_state = &mut env.framework_state.audio_toolbox;
    let context = at_state
        .al_context
        .make_al_context_current(env.openal_manager.as_mut());

    // RE4's callback writes two signed little-endian samples per frame (4
    // bytes), but its ASBD reports 2. Decoding with that reported stride would
    // invoke AudioQueue's unrelated truncation/endian workaround and distort
    // those samples. Describe the actual callback allocation instead.
    let decode_format = callback_decode_format(stream_format, &env.bundle.bundle_identifier())
        .expect("Invalid AudioUnit callback format");
    let (al_format, _sample_rate, processed_data) =
        decode_buffer(&env.mem, &decode_format, buffer1Data.cast(), buffer_size);

    capture_re4_output(
        &env.bundle.bundle_identifier(),
        &processed_data,
        decode_format,
        sample_rate,
        number_frames,
        elapsed_time.as_secs_f64(),
    );

    unsafe {
        // Get an unqueued buffer or create a new one
        let al_buffer = al_buffers.pop().unwrap_or_else(|| {
            let mut al_buffer = 0;
            context.GenBuffers(1, &mut al_buffer);
            al_buffer
        });

        context.BufferData(
            al_buffer,
            al_format,
            processed_data.as_ptr() as *const ALvoid,
            processed_data.len().try_into().unwrap(),
            sample_rate as i32,
        );
        context.SourceQueueBuffers(al_source, 1, &al_buffer);

        let mut al_source_state = 0;
        context.GetSourcei(al_source, AL_SOURCE_STATE, &mut al_source_state);
        if al_source_state != AL_PLAYING {
            context.SourcePlay(al_source);
        }

        // TODO: Play buffer 2 (In RE4 its the same as buffer 1 though)

        // Clear unused buffers
        if !al_buffers.is_empty() {
            context.DeleteBuffers(al_buffers.len() as i32, al_buffers.as_ptr());
        }

        assert_eq!(context.GetError(), 0);
    }

    // TODO: Do something with the action flags?
    env.mem.free(action_flags.cast_void());

    env.mem.free(buffer1Data.cast_void());
    if let Some(buffer2Data) = buffer2Data {
        env.mem.free(buffer2Data.cast_void());
    }

    env.mem.free(audio_buffer_list.cast_void());

    let audio_unit_host_object = audio_components::State::get(&mut env.framework_state)
        .audio_component_instances
        .get_mut(&audio_unit)
        .unwrap();
    // Reborrow as mutable to update the last render time

    audio_unit_host_object.last_render_time = Some(now);
    audio_unit_host_object.is_running_handler = false;
}

// === Mixer units (`aumx`) ===
//
// Games that use the 3D mixer or multichannel mixer set up its input buses
// (element count, stream formats, render callbacks) and then pull mixed audio
// from it with `AudioUnitRender()`, usually from the RemoteIO render callback.

/// `struct AudioUnitConnection`, for `kAudioUnitProperty_MakeConnection`.
#[repr(C, packed)]
struct AudioUnitConnection {
    source_audio_unit: AudioUnit,
    source_output_number: u32,
    dest_input_number: u32,
}
unsafe impl SafeRead for AudioUnitConnection {}

/// What a mixer reports as its stream format until the app sets one: 16-bit
/// signed integer stereo at 44.1 kHz.
fn default_mixer_stream_format() -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        sample_rate: 44100.0,
        format_id: kAudioFormatLinearPCM,
        format_flags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
        bytes_per_packet: 4,
        frames_per_packet: 1,
        bytes_per_frame: 4,
        channels_per_frame: 2,
        bits_per_channel: 16,
        _reserved: 0,
    }
}

fn mixer_set_property(
    env: &mut Environment,
    unit: AudioUnit,
    id: AudioUnitPropertyID,
    scope: AudioUnitScope,
    element: AudioUnitElement,
    data: ConstVoidPtr,
) -> OSStatus {
    let host_object = audio_components::State::get(&mut env.framework_state)
        .audio_component_instances
        .get_mut(&unit)
        .unwrap();
    match id {
        kAudioUnitProperty_ElementCount => {
            let count = env.mem.read(data.cast::<u32>());
            host_object.element_counts.insert(scope, count);
        }
        kAudioUnitProperty_StreamFormat => {
            let format = env.mem.read(data.cast::<AudioStreamBasicDescription>());
            host_object.bus_formats.insert((scope, element), format);
        }
        kAudioUnitProperty_SetRenderCallback => {
            let callback = env.mem.read(data.cast::<AURenderCallbackStruct>());
            host_object.bus_callbacks.insert(element, callback);
        }
        kAudioUnitProperty_MaximumFramesPerSlice => {
            host_object.maximum_frames_per_slice = env.mem.read(data.cast::<u32>());
        }
        kAudioUnitProperty_MakeConnection => {
            let connection = env.mem.read(data.cast::<AudioUnitConnection>());
            host_object.connections.insert(
                connection.dest_input_number,
                (
                    connection.source_audio_unit,
                    connection.source_output_number,
                ),
            );
        }
        _ => {
            // 3D mixer positioning/attenuation settings and the like: accepted
            // but they have no effect on the output.
            log_once!("TODO: AudioUnitSetProperty() on a mixer unit: some properties are accepted but ignored");
        }
    }
    0 // success
}

fn mixer_get_property(
    env: &mut Environment,
    unit: AudioUnit,
    id: AudioUnitPropertyID,
    scope: AudioUnitScope,
    element: AudioUnitElement,
    out_data: MutVoidPtr,
    io_data_size: MutPtr<u32>,
) -> OSStatus {
    let host_object = audio_components::State::get(&mut env.framework_state)
        .audio_component_instances
        .get(&unit)
        .unwrap();
    let element_count = host_object.element_counts.get(&scope).copied();
    let stream_format = host_object
        .bus_formats
        .get(&(scope, element))
        .copied()
        .unwrap_or_else(default_mixer_stream_format);
    let max_frames = host_object.maximum_frames_per_slice;
    match id {
        kAudioUnitProperty_ElementCount => {
            let count = element_count.unwrap_or(1);
            env.mem.write(out_data.cast(), count);
            env.mem.write(io_data_size, guest_size_of::<u32>());
        }
        kAudioUnitProperty_StreamFormat => {
            env.mem.write(out_data.cast(), stream_format);
            env.mem
                .write(io_data_size, guest_size_of::<AudioStreamBasicDescription>());
        }
        kAudioUnitProperty_SampleRate => {
            env.mem.write(out_data.cast(), stream_format.sample_rate);
            env.mem.write(io_data_size, guest_size_of::<f64>());
        }
        kAudioUnitProperty_MaximumFramesPerSlice => {
            env.mem.write(out_data.cast(), max_frames);
            env.mem.write(io_data_size, guest_size_of::<u32>());
        }
        _ => {
            log_once!("TODO: AudioUnitGetProperty() on a mixer unit for an unsupported property");
            return kAudioUnitErr_InvalidProperty;
        }
    }
    0 // success
}

/// Fills the buffers of an `AudioBufferList` with silence.
fn silence_audio_buffer_list(env: &mut Environment, list: MutVoidPtr) {
    let number_buffers: u32 = env.mem.read(list.cast::<u32>());
    for i in 0..number_buffers {
        // Layout: u32 count, then { u32 channels, u32 byte size, void *data }.
        let entry = list.to_bits() + 4 + 12 * i;
        let byte_size: u32 = env.mem.read(Ptr::<u32, false>::from_bits(entry + 4));
        let data: MutVoidPtr = env.mem.read(Ptr::<MutVoidPtr, false>::from_bits(entry + 8));
        if !data.is_null() && byte_size > 0 {
            env.mem.bytes_at_mut(data.cast(), byte_size).fill(0);
        }
    }
}

/// `AudioUnitRender()`: ask `in_unit` to produce `in_number_frames` of audio
/// into `io_data`.
///
/// Mixer units mix their input buses (see `mix_mixer_buses`); every other
/// unit still renders silence.
fn AudioUnitRender(
    env: &mut Environment,
    in_unit: AudioUnit,
    io_action_flags: MutPtr<u32>,
    _in_time_stamp: ConstVoidPtr,
    in_output_bus_number: u32,
    in_number_frames: u32,
    io_data: MutVoidPtr,
) -> OSStatus {
    if in_unit.is_null() || io_data.is_null() {
        return paramErr;
    }
    if unit_kind(env, in_unit) == Some(AudioUnitKind::Mixer) {
        let flags = if io_action_flags.is_null() { env.mem.alloc_and_write(0u32) } else { io_action_flags };
        mix_mixer_buses(env, in_unit, flags, in_output_bus_number, in_number_frames, io_data);
        if io_action_flags.is_null() {
            env.mem.free(flags.cast_void());
        }
        return 0;
    }
    log_once!("TODO: AudioUnitRender() renders silence for units other than mixers");
    silence_audio_buffer_list(env, io_data);
    0 // success
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(AudioUnitInitialize(_)),
    export_c_func!(AudioUnitUninitialize(_)),
    export_c_func!(AudioUnitSetProperty(_, _, _, _, _, _)),
    export_c_func!(AudioUnitGetProperty(_, _, _, _, _, _)),
    export_c_func!(AudioUnitAddRenderNotify(_, _, _)),
    export_c_func!(AudioUnitSetParameter(_, _, _, _, _, _)),
    export_c_func!(AudioUnitGetParameter(_, _, _, _, _)),
    export_c_func!(AudioOutputUnitStart(_)),
    export_c_func!(AudioOutputUnitStop(_)),
    export_c_func!(AudioUnitRender(_, _, _, _, _, _)),
];

/// Mix every input bus of a mixer unit into the (16-bit signed interleaved)
/// `AudioBufferList` `io_data`: each bus's guest render callback is asked for
/// `frames` frames, converted to the output channel count (mono is duplicated
/// to stereo) and summed with saturation. Buses in other formats stay silent.
fn mix_mixer_buses(
    env: &mut Environment,
    mixer: AudioUnit,
    action_flags: MutPtr<u32>,
    _output_bus: u32,
    frames: u32,
    io_data: MutVoidPtr,
) {
    silence_audio_buffer_list(env, io_data);
    let (callbacks, formats) = {
        let host_object = audio_components::State::get(&mut env.framework_state)
            .audio_component_instances
            .get(&mixer);
        let Some(host_object) = host_object else { return };
        let mut callbacks: Vec<(u32, AURenderCallbackStruct)> =
            host_object.bus_callbacks.iter().map(|(bus, cb)| (*bus, *cb)).collect();
        callbacks.sort_by_key(|(bus, _)| *bus);
        (callbacks, host_object.bus_formats.clone())
    };
    if callbacks.is_empty() || frames == 0 {
        return;
    }

    // Output buffer: first buffer of the list ({ u32 channels, u32 size, void *data }).
    let out_channels: u32 = env.mem.read(Ptr::<u32, false>::from_bits(io_data.to_bits() + 4));
    let out_size: u32 = env.mem.read(Ptr::<u32, false>::from_bits(io_data.to_bits() + 8));
    let out_data: MutVoidPtr = env.mem.read(Ptr::<MutVoidPtr, false>::from_bits(io_data.to_bits() + 12));
    if out_data.is_null() || out_channels == 0 || out_channels > 2 {
        log_once!("TODO: mixer output format other than 16-bit interleaved mono/stereo is not mixed");
        return;
    }
    let out_samples = (out_size / 2).min(frames * out_channels) as usize;
    let mut mix = vec![0i32; out_samples];

    for (bus, callback) in callbacks {
        let format = formats
            .get(&(1, bus)) // kAudioUnitScope_Input
            .copied()
            .unwrap_or_else(default_mixer_stream_format);
        let channels = format.channels_per_frame;
        let is_s16 = format.format_id == kAudioFormatLinearPCM
            && format.format_flags & kAudioFormatFlagIsSignedInteger != 0
            && format.format_flags & kAudioFormatFlagIsFloat == 0
            && format.bits_per_channel == 16
            && format.bytes_per_frame == channels * 2;
        if !is_s16 || !(1..=2).contains(&channels) {
            log_once!("TODO: a mixer input bus has a format other than 16-bit interleaved mono/stereo, left silent");
            continue;
        }
        let bytes = frames * channels * 2;
        let scratch = env.mem.alloc(bytes);
        env.mem.bytes_at_mut(scratch.cast::<u8>(), bytes).fill(0);
        let list: AudioBufferList<1> = AudioBufferList {
            number_buffers: 1,
            buffers: [AudioBuffer {
                number_channels: channels,
                data_byte_size: bytes,
                data: scratch,
            }],
        };
        let list_ptr: MutVoidPtr = env.mem.alloc_and_write(list).cast();
        env.mem.write(action_flags, 0u32);
        let AURenderCallbackStruct {
            input_proc,
            input_proc_ref_con,
        } = callback;
        let _: OSStatus = input_proc.call_from_host(
            env,
            (
                input_proc_ref_con,
                action_flags,
                nil.cast_void().cast_const(),
                bus,
                frames,
                list_ptr,
            ),
        );
        let samples: Vec<i16> = env
            .mem
            .bytes_at(scratch.cast::<u8>().cast_const(), bytes)
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        for frame in 0..frames as usize {
            for ch in 0..out_channels as usize {
                let src_ch = if channels == 1 { 0 } else { ch.min(channels as usize - 1) };
                let at = frame * out_channels as usize + ch;
                if at < out_samples {
                    mix[at] += i32::from(samples[frame * channels as usize + src_ch]);
                }
            }
        }
        env.mem.free(list_ptr);
        env.mem.free(scratch);
    }

    let out = env.mem.bytes_at_mut(out_data.cast::<u8>(), (out_samples * 2) as u32);
    for (i, sample) in mix.iter().enumerate() {
        let clamped = (*sample).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        out[i * 2..i * 2 + 2].copy_from_slice(&clamped.to_le_bytes());
    }
}
