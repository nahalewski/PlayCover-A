/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `AudioConverter.h` (Audio Converter Services)
//!
//! Converts Apple IMA4 or 16-bit linear PCM (mono or stereo, any sample rate)
//! into 16-bit little-endian linear PCM (mono or stereo, any sample rate).
//! Sample rate conversion is linear interpolation. Bejeweled 2 uses this to
//! turn its 22050 Hz mono sounds into 44100 Hz stereo for its mixer unit.

use std::collections::HashMap;

use crate::abi::{CallFromHost, GuestFunction};
use crate::audio::decode_ima4;
use crate::dyld::FunctionExports;
use crate::environment::Environment;
use crate::export_c_func;
use crate::frameworks::carbon_core::{paramErr, OSStatus};
use crate::frameworks::core_audio_types::{
    debug_fourcc, fourcc, kAudioFormatAppleIMA4, kAudioFormatFlagIsBigEndian,
    kAudioFormatFlagIsFloat, kAudioFormatLinearPCM, AudioStreamBasicDescription,
};
use crate::mem::{ConstPtr, MutPtr, MutVoidPtr, Ptr};

/// Opaque guest handle (`AudioConverterRef`).
pub type AudioConverterRef = MutVoidPtr;

const kAudioConverterErr_FormatNotSupported: OSStatus = fourcc(b"fmt?") as OSStatus;

#[derive(Default)]
pub struct State {
    converters: HashMap<AudioConverterRef, Converter>,
}
impl State {
    fn get(framework_state: &mut crate::frameworks::State) -> &mut Self {
        &mut framework_state.audio_toolbox.audio_converter
    }
}

struct Converter {
    source: AudioStreamBasicDescription,
    dest: AudioStreamBasicDescription,
    /// Decoded source frames not yet fully consumed, interleaved in the source
    /// channel count.
    pending: Vec<i16>,
    /// Position of the next output frame, in source frames, relative to the
    /// start of `pending`.
    position: f64,
}

fn is_s16_lpcm(d: &AudioStreamBasicDescription) -> bool {
    d.format_id == kAudioFormatLinearPCM
        && d.format_flags & kAudioFormatFlagIsFloat == 0
        && d.bits_per_channel == 16
        && d.frames_per_packet == 1
        && d.bytes_per_frame == d.channels_per_frame * 2
}

fn is_supported(source: &AudioStreamBasicDescription, dest: &AudioStreamBasicDescription) -> bool {
    let source_ok = match source.format_id {
        kAudioFormatAppleIMA4 => true,
        _ => is_s16_lpcm(source),
    };
    source_ok
        && (1..=2).contains(&{ source.channels_per_frame })
        && is_s16_lpcm(dest)
        && dest.format_flags & kAudioFormatFlagIsBigEndian == 0
        && (1..=2).contains(&{ dest.channels_per_frame })
        && source.sample_rate > 0.0
        && dest.sample_rate > 0.0
}

fn AudioConverterNew(
    env: &mut Environment,
    in_source_format: ConstPtr<AudioStreamBasicDescription>,
    in_destination_format: ConstPtr<AudioStreamBasicDescription>,
    out_audio_converter: MutPtr<AudioConverterRef>,
) -> OSStatus {
    return_if_null!(in_source_format);
    return_if_null!(in_destination_format);
    return_if_null!(out_audio_converter);
    let source = env.mem.read(in_source_format);
    let dest = env.mem.read(in_destination_format);
    if !is_supported(&source, &dest) {
        log!(
            "TODO: AudioConverterNew({:?} -> {:?}) is not supported, returning kAudioConverterErr_FormatNotSupported",
            source,
            dest
        );
        return kAudioConverterErr_FormatNotSupported;
    }
    let handle = env.mem.alloc(4);
    State::get(&mut env.framework_state).converters.insert(
        handle,
        Converter {
            source,
            dest,
            pending: Vec::new(),
            position: 0.0,
        },
    );
    env.mem.write(out_audio_converter, handle);
    log_dbg!("AudioConverterNew({:?} -> {:?}) -> {:?}", source, dest, handle);
    0
}

fn AudioConverterDispose(env: &mut Environment, in_audio_converter: AudioConverterRef) -> OSStatus {
    if State::get(&mut env.framework_state)
        .converters
        .remove(&in_audio_converter)
        .is_none()
    {
        return paramErr;
    }
    env.mem.free(in_audio_converter);
    0
}

fn AudioConverterReset(env: &mut Environment, in_audio_converter: AudioConverterRef) -> OSStatus {
    match State::get(&mut env.framework_state)
        .converters
        .get_mut(&in_audio_converter)
    {
        Some(converter) => {
            converter.pending.clear();
            converter.position = 0.0;
            0
        }
        None => paramErr,
    }
}

/// Decode `bytes` of source-format packets to interleaved 16-bit samples.
fn decode_source(source: &AudioStreamBasicDescription, bytes: &[u8]) -> Vec<i16> {
    let channels = source.channels_per_frame as usize;
    if source.format_id == kAudioFormatAppleIMA4 {
        // Each packet holds one 34-byte block per channel, 64 frames each.
        let mut out = Vec::with_capacity(bytes.len() / 34 * 64);
        for packet in bytes.chunks_exact(34 * channels) {
            let blocks: Vec<[i16; 64]> = packet
                .chunks_exact(34)
                .map(|block| decode_ima4(block.try_into().unwrap()))
                .collect();
            for frame in 0..64 {
                for block in &blocks {
                    out.push(block[frame]);
                }
            }
        }
        out
    } else {
        let big_endian = source.format_flags & kAudioFormatFlagIsBigEndian != 0;
        bytes
            .chunks_exact(2)
            .map(|b| {
                if big_endian {
                    i16::from_be_bytes([b[0], b[1]])
                } else {
                    i16::from_le_bytes([b[0], b[1]])
                }
            })
            .collect()
    }
}

/// One source sample for output channel `ch` (mono is duplicated, stereo is
/// averaged down to mono).
fn source_sample(pending: &[i16], src_channels: usize, dst_channels: usize, frame: usize, ch: usize) -> f64 {
    let base = frame * src_channels;
    if src_channels == 2 && dst_channels == 1 {
        (f64::from(pending[base]) + f64::from(pending[base + 1])) / 2.0
    } else {
        f64::from(pending[base + ch.min(src_channels - 1)])
    }
}

/// `AudioConverterComplexInputDataProc`:
/// `OSStatus (*)(AudioConverterRef, UInt32 *ioNumberDataPackets,
///  AudioBufferList *ioData, AudioStreamPacketDescription **outDataPacketDescription,
///  void *inUserData)`
fn AudioConverterFillComplexBuffer(
    env: &mut Environment,
    in_audio_converter: AudioConverterRef,
    in_input_data_proc: GuestFunction,
    in_input_data_proc_user_data: MutVoidPtr,
    io_output_data_packet_size: MutPtr<u32>,
    out_output_data: MutVoidPtr,
    _out_packet_description: MutVoidPtr,
) -> OSStatus {
    return_if_null!(io_output_data_packet_size);
    return_if_null!(out_output_data);
    let Some((source, dest, start_position)) = State::get(&mut env.framework_state)
        .converters
        .get(&in_audio_converter)
        .map(|c| (c.source, c.dest, c.position))
    else {
        return paramErr;
    };
    let src_channels = source.channels_per_frame as usize;
    let dst_channels = dest.channels_per_frame as usize;
    let step = source.sample_rate / dest.sample_rate;
    let (frames_per_packet, source_bytes_per_packet) = if source.format_id == kAudioFormatAppleIMA4
    {
        (64, 34 * source.channels_per_frame)
    } else {
        (1, source.bytes_per_frame)
    };

    // Output AudioBufferList: { u32 count; { u32 channels; u32 size; void *data; }[] }.
    let list_base = out_output_data.to_bits();
    let out_buffers: u32 = env.mem.read(Ptr::<u32, true>::from_bits(list_base));
    let read_buffer = |env: &Environment, i: u32| -> (u32, MutVoidPtr) {
        let at = list_base + 4 + i * 12;
        let size: u32 = env.mem.read(Ptr::<u32, true>::from_bits(at + 4));
        let data: MutVoidPtr = env.mem.read(Ptr::<MutVoidPtr, true>::from_bits(at + 8));
        (size, data)
    };
    let deinterleaved = out_buffers as usize == dst_channels && dst_channels > 1;
    if out_buffers == 0 || (out_buffers != 1 && !deinterleaved) {
        return paramErr;
    }
    let (first_size, _) = read_buffer(env, 0);
    let bytes_per_out_frame = if deinterleaved {
        2
    } else {
        2 * dst_channels as u32
    };
    let wanted_frames = env
        .mem
        .read(io_output_data_packet_size)
        .min(first_size / bytes_per_out_frame) as usize;

    // Source frames needed for `wanted_frames` interpolated output frames.
    let needed_source_frames = if wanted_frames == 0 {
        0
    } else {
        (start_position + (wanted_frames - 1) as f64 * step).floor() as usize + 2
    };

    let mut status: OSStatus = 0;
    let mut end_of_input = false;
    loop {
        let have = State::get(&mut env.framework_state).converters[&in_audio_converter]
            .pending
            .len()
            / src_channels;
        if have >= needed_source_frames {
            break;
        }
        let packets_needed = (needed_source_frames - have).div_ceil(frames_per_packet) as u32;
        // Scratch: packet count, one-buffer AudioBufferList, packet description pointer.
        let scratch = env.mem.alloc(4 + 16 + 4);
        let count_ptr: MutPtr<u32> = scratch.cast();
        let list_ptr: MutVoidPtr = Ptr::from_bits(scratch.to_bits() + 4);
        let desc_ptr: MutPtr<MutVoidPtr> = Ptr::from_bits(scratch.to_bits() + 20);
        env.mem.write(count_ptr, packets_needed);
        env.mem.write(list_ptr.cast::<u32>(), 1u32);
        env.mem.write(
            Ptr::<u32, true>::from_bits(list_ptr.to_bits() + 4),
            source.channels_per_frame,
        );
        env.mem
            .write(Ptr::<u32, true>::from_bits(list_ptr.to_bits() + 8), 0u32);
        env.mem.write(
            Ptr::<MutVoidPtr, true>::from_bits(list_ptr.to_bits() + 12),
            Ptr::null(),
        );
        env.mem.write(desc_ptr, Ptr::null());
        let result: OSStatus = in_input_data_proc.call_from_host(
            env,
            (
                in_audio_converter,
                count_ptr,
                list_ptr,
                desc_ptr,
                in_input_data_proc_user_data,
            ),
        );
        let packets = env.mem.read(count_ptr);
        let size: u32 = env
            .mem
            .read(Ptr::<u32, true>::from_bits(list_ptr.to_bits() + 8));
        let data: MutVoidPtr = env
            .mem
            .read(Ptr::<MutVoidPtr, true>::from_bits(list_ptr.to_bits() + 12));
        env.mem.free(scratch);

        let byte_count = size.min(packets.saturating_mul(source_bytes_per_packet));
        if packets > 0 && !data.is_null() && byte_count > 0 {
            let bytes = env
                .mem
                .bytes_at(data.cast::<u8>().cast_const(), byte_count)
                .to_vec();
            let decoded = decode_source(&source, &bytes);
            State::get(&mut env.framework_state)
                .converters
                .get_mut(&in_audio_converter)
                .unwrap()
                .pending
                .extend_from_slice(&decoded);
        }
        if result != 0 || packets == 0 || byte_count == 0 {
            // End of input (or an error the caller uses to say "no data now").
            status = result;
            end_of_input = true;
            break;
        }
    }

    // Interpolate the output frames.
    let converter = State::get(&mut env.framework_state)
        .converters
        .get_mut(&in_audio_converter)
        .unwrap();
    let have = converter.pending.len() / src_channels;
    let mut out: Vec<i16> = Vec::with_capacity(wanted_frames * dst_channels);
    let mut position = converter.position;
    let mut frames = 0;
    while frames < wanted_frames {
        let index = position.floor() as usize;
        let fraction = position - index as f64;
        if index + 1 < have {
            for ch in 0..dst_channels {
                let a = source_sample(&converter.pending, src_channels, dst_channels, index, ch);
                let b = source_sample(&converter.pending, src_channels, dst_channels, index + 1, ch);
                out.push((a + (b - a) * fraction).round() as i16);
            }
        } else if end_of_input && index < have {
            for ch in 0..dst_channels {
                out.push(source_sample(&converter.pending, src_channels, dst_channels, index, ch) as i16);
            }
        } else {
            break;
        }
        frames += 1;
        position += step;
    }
    let consumed = (position.floor() as usize).min(have);
    converter.pending.drain(..consumed * src_channels);
    converter.position = position - consumed as f64;

    if out.iter().any(|&s| s != 0) {
        static FIRST_AUDIO: std::sync::Once = std::sync::Once::new();
        FIRST_AUDIO.call_once(|| {
            log!(
                "AudioConverterFillComplexBuffer: first non-silent output ({} frames, {} {} Hz -> {} Hz)",
                frames,
                debug_fourcc(source.format_id),
                { source.sample_rate },
                { dest.sample_rate }
            );
        });
    }

    if deinterleaved {
        for ch in 0..dst_channels {
            let (_, data) = read_buffer(env, ch as u32);
            if data.is_null() {
                continue;
            }
            let bytes = env.mem.bytes_at_mut(data.cast::<u8>(), (frames * 2) as u32);
            for frame in 0..frames {
                bytes[frame * 2..frame * 2 + 2]
                    .copy_from_slice(&out[frame * dst_channels + ch].to_le_bytes());
            }
            env.mem.write(
                Ptr::<u32, true>::from_bits(list_base + 4 + ch as u32 * 12 + 4),
                (frames * 2) as u32,
            );
        }
    } else {
        let (_, data) = read_buffer(env, 0);
        if !data.is_null() {
            let bytes = env
                .mem
                .bytes_at_mut(data.cast::<u8>(), (out.len() * 2) as u32);
            for (i, sample) in out.iter().enumerate() {
                bytes[i * 2..i * 2 + 2].copy_from_slice(&sample.to_le_bytes());
            }
        }
        env.mem.write(
            Ptr::<u32, true>::from_bits(list_base + 8),
            (out.len() * 2) as u32,
        );
    }
    env.mem.write(io_output_data_packet_size, frames as u32);

    // Like Apple's converter, an error returned by the input proc is passed
    // back to the caller, along with whatever output was produced before it.
    status
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(AudioConverterNew(_, _, _)),
    export_c_func!(AudioConverterDispose(_)),
    export_c_func!(AudioConverterReset(_)),
    export_c_func!(AudioConverterFillComplexBuffer(_, _, _, _, _, _)),
];
