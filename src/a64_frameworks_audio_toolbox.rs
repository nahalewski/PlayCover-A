/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Emulator-owned AudioToolbox subset for ARM64 guests: the 14 functions
//! Coromon (Corona's audio session manager and ALmixer's CoreAudio decoder)
//! imports.
//!
//! - `AudioSession*` keeps the session state the app sets. There is no
//!   mediaserverd, so there are no interruptions or route changes. Hardware
//!   properties report the host mixer's nominal values.
//! - `AudioFileOpenWithCallbacks` is a driven call: the guest size and read
//!   procs run from guest code (see `Drive`). The whole file is read into the
//!   host, then decoded to linear PCM by the shared host decoder
//!   (`crate::audio::AudioFile`: WAV via hound; MP3/AAC/ALAC/CAF via
//!   symphonia).
//! - `ExtAudioFile*` serves that PCM in the client format. Only signed 16-bit
//!   native-endian interleaved output at the file's rate and channel count is
//!   implemented, which is exactly what ALmixer requests. Anything else
//!   returns Core Audio's "format not supported" status, never a silent
//!   conversion.
use super::{arg_u32, ret_void, Arena, Drive, Family};
use crate::a64::bridge::{ReturnValues, ServiceFrame};
use crate::audio::{AudioFile, AudioFormat};
use std::collections::BTreeMap;

pub(super) const PROVIDER: &str =
    "/System/Library/Frameworks/AudioToolbox.framework/AudioToolbox";

const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}
// OSStatus values (MacErrors.h / AudioToolbox headers).
const NO_ERR: u32 = 0;
const SESSION_NOT_INITIALIZED: u32 = fourcc(b"!ini");
const SESSION_ALREADY_INITIALIZED: u32 = fourcc(b"init");
const SESSION_UNSUPPORTED_PROPERTY: u32 = fourcc(b"pty?");
const SESSION_BAD_PROPERTY_SIZE: u32 = fourcc(b"!siz");
const FILE_UNSUPPORTED_TYPE: u32 = fourcc(b"typ?");
const FILE_INVALID: u32 = fourcc(b"dta?");
const FILE_UNSUPPORTED_PROPERTY: u32 = fourcc(b"pty?");
const FILE_BAD_PROPERTY_SIZE: u32 = fourcc(b"!siz");
const FILE_NOT_OPEN: u32 = (-38i32) as u32; // kAudioFileNotOpenError
const FORMAT_NOT_SUPPORTED: u32 = fourcc(b"fmt?");
const EXT_INVALID_PROPERTY: u32 = (-66561i32) as u32; // kExtAudioFileError_InvalidProperty
const EXT_NON_PCM_CLIENT: u32 = (-66563i32) as u32; // kExtAudioFileError_NonPCMClientFormat
const PARAM_ERR: u32 = (-50i32) as u32;

const LPCM: u32 = fourcc(b"lpcm");
const FLAG_FLOAT: u32 = 1;
const FLAG_BIG_ENDIAN: u32 = 2;
const FLAG_SIGNED: u32 = 4;
const FLAG_PACKED: u32 = 8;
const ASBD_BYTES: usize = 40;

/// Upper bound for one encoded file read through guest callbacks. Coromon's
/// largest track (credits.mp3) is 8.2 MB.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Bytes requested per read proc call; the last 16 bytes of the I/O buffer
/// hold the read proc's `actualCount` output.
const READ_CHUNK: u64 = super::IO_BYTES - 16;
/// Nominal host mixer values (OpenAL Soft's default output).
const HOST_SAMPLE_RATE: f64 = 44100.0;
const HOST_IO_BUFFER: f32 = 1024.0 / 44100.0;

macro_rules! symbols {
    ($($name:ident),* $(,)?) => {
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Func { $($name),* }
        const FNS: &[Func] = &[$(Func::$name),*];
        const SYMBOLS: &[&str] = &[$(concat!("_", stringify!($name))),*];
    };
}
symbols!(
    AudioSessionInitialize, AudioSessionSetActive, AudioSessionSetActiveWithFlags,
    AudioSessionGetProperty, AudioSessionSetProperty,
    AudioFileOpenWithCallbacks, AudioFileGetProperty, AudioFileClose,
    ExtAudioFileWrapAudioFileID, ExtAudioFileSetProperty, ExtAudioFileRead,
    ExtAudioFileSeek, ExtAudioFileDispose,
    AudioServicesPlaySystemSound,
);

/// An AudioStreamBasicDescription (LP64 layout, 40 bytes).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Asbd {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
}
impl Asbd {
    fn to_bytes(self) -> [u8; ASBD_BYTES] {
        let mut out = [0u8; ASBD_BYTES];
        out[..8].copy_from_slice(&self.sample_rate.to_le_bytes());
        for (i, v) in [
            self.format_id,
            self.format_flags,
            self.bytes_per_packet,
            self.frames_per_packet,
            self.bytes_per_frame,
            self.channels_per_frame,
            self.bits_per_channel,
            0,
        ]
        .iter()
        .enumerate()
        {
            out[8 + 4 * i..12 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
    fn from_bytes(bytes: &[u8]) -> Self {
        let word = |i: usize| u32::from_le_bytes(bytes[8 + 4 * i..12 + 4 * i].try_into().unwrap());
        Self {
            sample_rate: f64::from_le_bytes(bytes[..8].try_into().unwrap()),
            format_id: word(0),
            format_flags: word(1),
            bytes_per_packet: word(2),
            frames_per_packet: word(3),
            bytes_per_frame: word(4),
            channels_per_frame: word(5),
            bits_per_channel: word(6),
        }
    }
}

/// A decoded file. The host decoder exposes 8-bit unsigned or 16-bit signed
/// little-endian interleaved PCM.
struct DecodedFile {
    audio: AudioFile,
    rate: f64,
    channels: u32,
    bits: u32,
    frames: u64,
    duration: f64,
    wrappers: u32,
}
impl DecodedFile {
    fn decode(bytes: Vec<u8>) -> Result<Self, u32> {
        // hound accepts WAV variants the shared decoder would panic on
        // (24-bit, float); reject those first.
        if let Ok(reader) = hound::WavReader::new(std::io::Cursor::new(&bytes)) {
            let spec = reader.spec();
            if !matches!(spec.bits_per_sample, 8 | 16)
                || spec.sample_format != hound::SampleFormat::Int
            {
                return Err(FORMAT_NOT_SUPPORTED);
            }
        }
        let audio = AudioFile::read_from_vec(bytes).map_err(|_| FILE_UNSUPPORTED_TYPE)?;
        let description = audio.audio_description();
        let AudioFormat::LinearPcm {
            is_float: false,
            is_little_endian: true,
        } = description.format
        else {
            return Err(FORMAT_NOT_SUPPORTED);
        };
        let channels = description.channels_per_frame;
        let bits = description.bits_per_channel;
        if channels == 0 || !matches!(bits, 8 | 16) {
            return Err(FORMAT_NOT_SUPPORTED);
        }
        let frame_bytes = u64::from(channels * bits / 8);
        Ok(Self {
            frames: audio.byte_count() / frame_bytes,
            duration: audio.estimated_duration(),
            rate: description.sample_rate,
            channels,
            bits,
            audio,
            wrappers: 0,
        })
    }
    fn file_format(&self) -> Asbd {
        let frame = self.channels * self.bits / 8;
        Asbd {
            sample_rate: self.rate,
            format_id: LPCM,
            format_flags: FLAG_PACKED | if self.bits == 16 { FLAG_SIGNED } else { 0 },
            bytes_per_packet: frame,
            frames_per_packet: 1,
            bytes_per_frame: frame,
            channels_per_frame: self.channels,
            bits_per_channel: self.bits,
        }
    }
    /// Read up to `frames` frames from `position` as signed 16-bit LE.
    fn read_s16(&mut self, position: u64, frames: u64) -> Result<Vec<u8>, String> {
        let frames = frames.min(self.frames.saturating_sub(position));
        let source_frame = u64::from(self.channels * self.bits / 8);
        let mut raw = vec![0u8; (frames * source_frame) as usize];
        let read = self
            .audio
            .read_bytes(position * source_frame, &mut raw)
            .map_err(|_| "host audio decoder read failed".to_string())?;
        raw.truncate(read - read % source_frame as usize);
        Ok(if self.bits == 16 {
            raw
        } else {
            raw.iter()
                .flat_map(|&b| (((b as i16) - 128) << 8).to_le_bytes())
                .collect()
        })
    }
}

struct ExtFile {
    file: u64,
    client: Option<Asbd>,
    position: u64,
}

enum Phase {
    Size,
    Read,
}
/// One in-flight AudioFileOpenWithCallbacks.
struct OpenOperation {
    client_data: u64,
    read_proc: u64,
    out_file: u64,
    io: (u64, u64),
    size: u64,
    bytes: Vec<u8>,
    phase: Phase,
}

#[derive(Default)]
pub(in crate::a64) struct AudioToolbox {
    session_initialized: bool,
    session_active: bool,
    session_properties: BTreeMap<u32, Vec<u8>>,
    files: BTreeMap<u64, DecodedFile>,
    ext_files: BTreeMap<u64, ExtFile>,
    opening: Vec<OpenOperation>,
    warned_system_sound: bool,
}

fn status(value: u32) -> ReturnValues {
    ReturnValues::integer(u64::from(value))
}

impl AudioToolbox {
    fn session_property(&self, id: u32) -> Option<Vec<u8>> {
        if let Some(value) = self.session_properties.get(&id) {
            return Some(value.clone());
        }
        let f32b = |v: f32| v.to_le_bytes().to_vec();
        let u32b = |v: u32| v.to_le_bytes().to_vec();
        Some(match &id.to_be_bytes() {
            b"acat" => u32b(fourcc(b"solo")), // SoloAmbientSound default
            b"chsr" | b"hwsr" => HOST_SAMPLE_RATE.to_le_bytes().to_vec(),
            b"chbd" | b"iobd" => f32b(HOST_IO_BUFFER),
            b"colt" => f32b(HOST_IO_BUFFER),
            b"cilt" => f32b(0.0),
            b"chov" => f32b(1.0),
            b"othr" | b"duck" | b"cmix" => u32b(0),
            b"choc" => u32b(2),
            _ => return None,
        })
    }
    fn settable_session_property(id: u32) -> Option<usize> {
        match &id.to_be_bytes() {
            b"acat" | b"cmix" | b"duck" => Some(4),
            b"iobd" => Some(4),
            b"hwsr" => Some(8),
            _ => None,
        }
    }

    fn session(&mut self, function: Func, frame: &mut ServiceFrame<'_>) -> Result<u32, String> {
        use Func::*;
        if function == AudioSessionInitialize {
            if self.session_initialized {
                return Ok(SESSION_ALREADY_INITIALIZED);
            }
            self.session_initialized = true;
            return Ok(NO_ERR);
        }
        if !self.session_initialized {
            return Ok(SESSION_NOT_INITIALIZED);
        }
        Ok(match function {
            AudioSessionSetActive | AudioSessionSetActiveWithFlags => {
                self.session_active = arg_u32(frame, 0)? & 0xff != 0;
                NO_ERR
            }
            AudioSessionGetProperty => {
                let id = arg_u32(frame, 0)?;
                let size_ptr = frame.integer(1)?;
                let out = frame.integer(2)?;
                let Some(value) = self.session_property(id) else {
                    return Ok(SESSION_UNSUPPORTED_PROPERTY);
                };
                if size_ptr == 0 || out == 0 {
                    return Ok(PARAM_ERR);
                }
                let size = u32::from_le_bytes(frame.read(size_ptr, 4)?.try_into().unwrap());
                if (size as usize) < value.len() {
                    return Ok(SESSION_BAD_PROPERTY_SIZE);
                }
                frame.write(out, &value)?;
                frame.write(size_ptr, &(value.len() as u32).to_le_bytes())?;
                NO_ERR
            }
            AudioSessionSetProperty => {
                let id = arg_u32(frame, 0)?;
                let size = arg_u32(frame, 1)? as usize;
                let data = frame.integer(2)?;
                let Some(expected) = Self::settable_session_property(id) else {
                    return Ok(SESSION_UNSUPPORTED_PROPERTY);
                };
                if size != expected || data == 0 {
                    return Ok(SESSION_BAD_PROPERTY_SIZE);
                }
                // 'hwsr' is a preference: the current rate ('chsr') stays
                // the host mixer's.
                let value = frame.read(data, size)?;
                self.session_properties.insert(id, value);
                NO_ERR
            }
            _ => unreachable!(),
        })
    }

    fn open_step(&mut self, frame: &mut ServiceFrame<'_>, arena: &mut Arena) -> Result<Drive, String> {
        let op = self.opening.last_mut().ok_or("no AudioFile open in flight")?;
        if op.bytes.len() as u64 >= op.size {
            return self.finish_open(frame, arena);
        }
        let position = op.bytes.len() as u64;
        let request = (op.size - position).min(READ_CHUNK);
        let actual = op.io.0 + op.io.1 - 16;
        frame.write(actual, &0u32.to_le_bytes())?;
        op.phase = Phase::Read;
        Ok(Drive::Call {
            function: op.read_proc,
            args: [op.client_data, position, request, op.io.0, actual, 0],
        })
    }
    fn fail_open(&mut self, arena: &mut Arena, code: u32) -> Result<Drive, String> {
        self.opening.pop();
        arena.release_io();
        Ok(Drive::Done(u64::from(code)))
    }
    fn finish_open(&mut self, frame: &mut ServiceFrame<'_>, arena: &mut Arena) -> Result<Drive, String> {
        let op = self.opening.pop().ok_or("no AudioFile open in flight")?;
        arena.release_io();
        let decoded = match DecodedFile::decode(op.bytes) {
            Ok(decoded) => decoded,
            Err(code) => return Ok(Drive::Done(u64::from(code))),
        };
        let handle = arena.allocate_handle()?;
        frame.write(op.out_file, &handle.to_le_bytes())?;
        self.files.insert(handle, decoded);
        Ok(Drive::Done(u64::from(NO_ERR)))
    }

    fn file_call(&mut self, function: Func, frame: &mut ServiceFrame<'_>, arena: &mut Arena) -> Result<u32, String> {
        use Func::*;
        Ok(match function {
            AudioFileGetProperty => {
                let Some(file) = self.files.get(&frame.integer(0)?) else {
                    return Ok(FILE_NOT_OPEN);
                };
                let id = arg_u32(frame, 1)?;
                let size_ptr = frame.integer(2)?;
                let out = frame.integer(3)?;
                let value = match &id.to_be_bytes() {
                    b"dfmt" => file.file_format().to_bytes().to_vec(),
                    b"edur" => file.duration.to_le_bytes().to_vec(),
                    b"pcnt" => file.frames.to_le_bytes().to_vec(),
                    _ => return Ok(FILE_UNSUPPORTED_PROPERTY),
                };
                if size_ptr == 0 || out == 0 {
                    return Ok(PARAM_ERR);
                }
                let size = u32::from_le_bytes(frame.read(size_ptr, 4)?.try_into().unwrap());
                if (size as usize) < value.len() {
                    return Ok(FILE_BAD_PROPERTY_SIZE);
                }
                frame.write(out, &value)?;
                frame.write(size_ptr, &(value.len() as u32).to_le_bytes())?;
                NO_ERR
            }
            AudioFileClose => {
                let handle = frame.integer(0)?;
                match self.files.get(&handle) {
                    // ALmixer closes after a failed open with an
                    // uninitialized id; that is an error, not a crash.
                    None => FILE_NOT_OPEN,
                    // The ExtAudioFile wrapper keeps the data alive until it
                    // is disposed, matching the real ownership rules closely
                    // enough for close-after-dispose and dispose-after-close.
                    Some(file) if file.wrappers > 0 => {
                        self.files.get_mut(&handle).unwrap().wrappers |= 0x8000_0000;
                        NO_ERR
                    }
                    Some(_) => {
                        self.files.remove(&handle);
                        arena.release_handle(handle)?;
                        NO_ERR
                    }
                }
            }
            ExtAudioFileWrapAudioFileID => {
                let file = frame.integer(0)?;
                let for_writing = arg_u32(frame, 1)? & 0xff != 0;
                let out = frame.integer(2)?;
                let Some(decoded) = self.files.get_mut(&file) else {
                    return Ok(FILE_NOT_OPEN);
                };
                if for_writing {
                    return Ok(FORMAT_NOT_SUPPORTED);
                }
                if out == 0 {
                    return Ok(PARAM_ERR);
                }
                decoded.wrappers += 1;
                let handle = arena.allocate_handle()?;
                frame.write(out, &handle.to_le_bytes())?;
                self.ext_files.insert(
                    handle,
                    ExtFile {
                        file,
                        client: None,
                        position: 0,
                    },
                );
                NO_ERR
            }
            ExtAudioFileSetProperty => {
                let handle = frame.integer(0)?;
                let id = arg_u32(frame, 1)?;
                let size = arg_u32(frame, 2)? as usize;
                let data = frame.integer(3)?;
                let Some(ext) = self.ext_files.get(&handle) else {
                    return Ok(PARAM_ERR);
                };
                if &id.to_be_bytes() != b"cfmt" {
                    return Ok(EXT_INVALID_PROPERTY);
                }
                if size < ASBD_BYTES || data == 0 {
                    return Ok(FILE_BAD_PROPERTY_SIZE);
                }
                let client = Asbd::from_bytes(&frame.read(data, ASBD_BYTES)?);
                let file = &self.files[&ext.file];
                if client.format_id != LPCM {
                    return Ok(EXT_NON_PCM_CLIENT);
                }
                let supported = client.bits_per_channel == 16
                    && client.format_flags & (FLAG_SIGNED | FLAG_FLOAT | FLAG_BIG_ENDIAN)
                        == FLAG_SIGNED
                    && client.channels_per_frame == file.channels
                    && (client.sample_rate - file.rate).abs() < 0.5
                    && client.frames_per_packet == 1
                    && client.bytes_per_frame == 2 * file.channels;
                if !supported {
                    log!("[a64] ExtAudioFile client format {client:?} needs an unimplemented conversion");
                    return Ok(FORMAT_NOT_SUPPORTED);
                }
                self.ext_files.get_mut(&handle).unwrap().client = Some(client);
                NO_ERR
            }
            ExtAudioFileRead => {
                let handle = frame.integer(0)?;
                let frames_ptr = frame.integer(1)?;
                let list = frame.integer(2)?;
                let Some(ext) = self.ext_files.get(&handle) else {
                    return Ok(PARAM_ERR);
                };
                if frames_ptr == 0 || list == 0 {
                    return Ok(PARAM_ERR);
                }
                let decoded = self.files.get_mut(&ext.file).ok_or("wrapped file vanished")?;
                // Without a client format the file format is used, which
                // this layer only serves when it already is 16-bit.
                if ext.client.is_none() && decoded.bits != 16 {
                    return Ok(FORMAT_NOT_SUPPORTED);
                }
                let requested =
                    u32::from_le_bytes(frame.read(frames_ptr, 4)?.try_into().unwrap());
                let count = u32::from_le_bytes(frame.read(list, 4)?.try_into().unwrap());
                if count != 1 {
                    // Deinterleaved client layouts are not implemented.
                    return Ok(FORMAT_NOT_SUPPORTED);
                }
                let buffer = frame.read(list + 8, 16)?;
                let capacity = u32::from_le_bytes(buffer[4..8].try_into().unwrap());
                let data = u64::from_le_bytes(buffer[8..16].try_into().unwrap());
                let frame_bytes = 2 * decoded.channels;
                let frames = u64::from(requested.min(capacity / frame_bytes));
                if frames > 0 && data == 0 {
                    return Ok(PARAM_ERR);
                }
                let position = ext.position;
                let pcm = decoded.read_s16(position, frames)?;
                let read_frames = pcm.len() as u64 / u64::from(frame_bytes);
                frame.write_bulk(data, &pcm)?;
                frame.write(list + 12, &(pcm.len() as u32).to_le_bytes())?;
                frame.write(frames_ptr, &(read_frames as u32).to_le_bytes())?;
                self.ext_files.get_mut(&handle).unwrap().position = position + read_frames;
                NO_ERR
            }
            ExtAudioFileSeek => {
                let handle = frame.integer(0)?;
                let offset = frame.integer(1)? as i64;
                let Some(ext) = self.ext_files.get_mut(&handle) else {
                    return Ok(PARAM_ERR);
                };
                let frames = self.files[&ext.file].frames;
                if offset < 0 || offset as u64 > frames {
                    return Ok(PARAM_ERR);
                }
                ext.position = offset as u64;
                NO_ERR
            }
            ExtAudioFileDispose => {
                let handle = frame.integer(0)?;
                let Some(ext) = self.ext_files.remove(&handle) else {
                    return Ok(PARAM_ERR);
                };
                arena.release_handle(handle)?;
                let file = self.files.get_mut(&ext.file).ok_or("wrapped file vanished")?;
                let closed = file.wrappers & 0x8000_0000 != 0;
                file.wrappers = (file.wrappers & 0x7fff_ffff) - 1;
                if closed {
                    if file.wrappers == 0 {
                        self.files.remove(&ext.file);
                        arena.release_handle(ext.file)?;
                    } else {
                        file.wrappers |= 0x8000_0000;
                    }
                }
                NO_ERR
            }
            _ => unreachable!(),
        })
    }
}

impl Family for AudioToolbox {
    fn name(&self) -> &'static str {
        "audio_toolbox"
    }
    fn provider(&self) -> &'static str {
        PROVIDER
    }
    fn symbols(&self) -> &'static [&'static str] {
        SYMBOLS
    }
    fn driven(&self, index: usize) -> bool {
        FNS.get(index) == Some(&Func::AudioFileOpenWithCallbacks)
    }
    fn call(
        &mut self,
        index: usize,
        frame: &mut ServiceFrame<'_>,
        arena: &mut Arena,
    ) -> Result<ReturnValues, String> {
        use Func::*;
        let function = *FNS.get(index).ok_or("AudioToolbox dispatch index invalid")?;
        match function {
            AudioServicesPlaySystemSound => {
                // void; the only sound Coromon can pass is the vibrate ID
                // (no AudioServicesCreateSystemSoundID import). The host has
                // no vibration motor to drive.
                if !self.warned_system_sound {
                    self.warned_system_sound = true;
                    log!("[a64] AudioServicesPlaySystemSound({:#x}): no host haptics; ignored", frame.integer(0)?);
                }
                Ok(ret_void())
            }
            AudioSessionInitialize
            | AudioSessionSetActive
            | AudioSessionSetActiveWithFlags
            | AudioSessionGetProperty
            | AudioSessionSetProperty => Ok(status(self.session(function, frame)?)),
            AudioFileOpenWithCallbacks => Err("AudioFileOpenWithCallbacks must be driven".into()),
            _ => Ok(status(self.file_call(function, frame, arena)?)),
        }
    }
    fn begin(
        &mut self,
        _index: usize,
        frame: &mut ServiceFrame<'_>,
        arena: &mut Arena,
    ) -> Result<Drive, String> {
        let client_data = frame.integer(0)?;
        let read_proc = frame.integer(1)?;
        let get_size_proc = frame.integer(3)?;
        let out_file = frame.integer(6)?;
        if read_proc == 0 || get_size_proc == 0 || out_file == 0 {
            return Ok(Drive::Done(u64::from(PARAM_ERR)));
        }
        let io = arena.claim_io("AudioFileOpenWithCallbacks")?;
        self.opening.push(OpenOperation {
            client_data,
            read_proc,
            out_file,
            io,
            size: 0,
            bytes: Vec::new(),
            phase: Phase::Size,
        });
        Ok(Drive::Call {
            function: get_size_proc,
            args: [client_data, 0, 0, 0, 0, 0],
        })
    }
    fn resume(
        &mut self,
        _index: usize,
        result: u64,
        frame: &mut ServiceFrame<'_>,
        arena: &mut Arena,
    ) -> Result<Drive, String> {
        let op = self.opening.last_mut().ok_or("AudioFile resume without open")?;
        match op.phase {
            Phase::Size => {
                let size = result as i64;
                if size <= 0 {
                    return self.fail_open(arena, FILE_INVALID);
                }
                if size as u64 > MAX_FILE_BYTES {
                    log!("[a64] AudioFileOpenWithCallbacks: {size}-byte file exceeds the {MAX_FILE_BYTES}-byte host read limit");
                    return self.fail_open(arena, FILE_INVALID);
                }
                op.size = size as u64;
                op.bytes.reserve(op.size as usize);
            }
            Phase::Read => {
                let code = result as u32;
                if code != NO_ERR {
                    return self.fail_open(arena, code);
                }
                let actual = op.io.0 + op.io.1 - 16;
                let count = u32::from_le_bytes(frame.read(actual, 4)?.try_into().unwrap());
                let requested = (op.size - op.bytes.len() as u64).min(READ_CHUNK);
                if u64::from(count) > requested {
                    return Err("AudioFile read proc reported more bytes than requested".into());
                }
                if count == 0 {
                    // Early end of data: decode what the source provided.
                    op.size = op.bytes.len() as u64;
                } else {
                    let chunk = frame.read_bulk(op.io.0, count as usize)?;
                    op.bytes.extend_from_slice(&chunk);
                }
            }
        }
        self.open_step(frame, arena)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{install, Frameworks};
    use super::*;
    use crate::a64::bridge::{GuestBridge, GuestCall};
    use crate::a64::A64Cpu;

    const DATA: u64 = 0x80000;
    const SOURCE: u64 = 0x100000;
    const GUEST_CODE: u64 = 0x300000;
    const CLIENT: u64 = 0x90000;

    struct Guest {
        cpu: A64Cpu,
        bridge: GuestBridge,
        frameworks: Frameworks,
    }
    impl Guest {
        fn new() -> Self {
            let mut cpu = A64Cpu::new_sparse();
            cpu.map_zeroed(DATA, 0x20000, 3).unwrap();
            cpu.map_zeroed(SOURCE, 0x100000, 3).unwrap();
            let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
            let frameworks = install(
                &mut cpu,
                &mut bridge,
                0x200000,
                vec![Box::new(AudioToolbox::default())],
            )
            .unwrap();
            Self {
                cpu,
                bridge,
                frameworks,
            }
        }
        fn entry(&self, name: &str) -> u64 {
            self.frameworks
                .bindings
                .iter()
                .find(|b| b.symbol == format!("_{name}"))
                .unwrap()
                .address
        }
        fn try_call(&mut self, name: &str, integers: &[u64]) -> Result<ReturnValues, String> {
            let entry = self.entry(name);
            self.bridge.call(
                &mut self.cpu,
                &GuestCall {
                    entry,
                    integers: integers.to_vec(),
                    ..Default::default()
                },
                1_000_000,
            )
        }
        fn call(&mut self, name: &str, integers: &[u64]) -> u32 {
            self.try_call(name, integers)
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .integers[0] as u32
        }
        fn u32_at(&self, a: u64) -> u32 {
            u32::from_le_bytes(self.cpu.read_bytes(a, 4).unwrap().try_into().unwrap())
        }
        fn u64_at(&self, a: u64) -> u64 {
            u64::from_le_bytes(self.cpu.read_bytes(a, 8).unwrap().try_into().unwrap())
        }
        /// Install guest ARM64 size/read procs over an in-memory "RWops"
        /// whose client data is {data pointer, length, read count}.
        fn install_procs(&mut self, bytes: &[u8]) -> (u64, u64, u64) {
            self.cpu.write_bytes(SOURCE, bytes);
            let client = CLIENT;
            self.cpu.write_bytes(client, &SOURCE.to_le_bytes());
            self.cpu.write_bytes(client + 8, &(bytes.len() as u64).to_le_bytes());
            self.cpu.write_bytes(client + 16, &0u64.to_le_bytes());
            let size_proc: [u32; 2] = [
                0xF9400400, // ldr x0, [x0, #8]
                0xD65F03C0, // ret
            ];
            // read(client x0, position x1, count w2, buffer x3, actual x4):
            // memcpy from data+position, clamped to length; count calls.
            let read_proc: [u32; 24] = [
                0xF9400009, // ldr x9, [x0]        data
                0xF940040A, // ldr x10, [x0, #8]   length
                0xF9400810, // ldr x16, [x0, #16]  call count
                0x91000610, // add x16, x16, #1
                0xF9000810, // str x16, [x0, #16]
                0xCB01014A, // sub x10, x10, x1    remaining
                0x2A0203EB, // mov w11, w2
                0xEB0A017F, // cmp x11, x10
                0x9A8A916B, // csel x11, x11, x10, ls
                0xB900008B, // str w11, [x4]
                0x8B010129, // add x9, x9, x1
                0xF100217F, // words: cmp x11, #8
                0x540000A3, // b.lo bytes
                0xF840852C, // ldr x12, [x9], #8
                0xF800846C, // str x12, [x3], #8
                0xD100216B, // sub x11, x11, #8
                0x17FFFFFB, // b words
                0xB40000AB, // bytes: cbz x11, done
                0x3840152C, // ldrb w12, [x9], #1
                0x3800146C, // strb w12, [x3], #1
                0xD100056B, // sub x11, x11, #1
                0x17FFFFFC, // b bytes
                0x52800000, // done: mov w0, #0
                0xD65F03C0, // ret
            ];
            self.cpu.map_zeroed(GUEST_CODE, 4096, 5).unwrap_or(());
            let code: Vec<u8> = size_proc
                .iter()
                .chain(read_proc.iter())
                .flat_map(|w| w.to_le_bytes())
                .collect();
            self.cpu.write_bytes(GUEST_CODE, &code);
            (client, GUEST_CODE + 8, GUEST_CODE)
        }
    }

    fn wav(channels: u16, rate: u32, bits: u16, frames: u32) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: bits,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
            for i in 0..frames * u32::from(channels) {
                if bits == 16 {
                    writer.write_sample((i as i16).wrapping_mul(37)).unwrap();
                } else {
                    writer.write_sample(((i % 200) as i8).wrapping_sub(100)).unwrap();
                }
            }
            writer.finalize().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn routes_exactly_coromons_audio_toolbox_imports() {
        assert_eq!(SYMBOLS.len(), 14);
        let unique: std::collections::BTreeSet<_> = SYMBOLS.iter().collect();
        assert_eq!(unique.len(), 14);
    }

    #[test]
    fn audio_session_lifecycle_and_properties() {
        let mut guest = Guest::new();
        let size = DATA;
        let out = DATA + 0x10;
        guest.cpu.write_bytes(size, &8u32.to_le_bytes());
        // Not initialized yet: honest error.
        assert_eq!(guest.call("AudioSessionSetActive", &[1]), SESSION_NOT_INITIALIZED);
        assert_eq!(guest.call("AudioSessionInitialize", &[0, 0, 0, 0]), 0);
        assert_eq!(
            guest.call("AudioSessionInitialize", &[0, 0, 0, 0]),
            SESSION_ALREADY_INITIALIZED
        );
        assert_eq!(guest.call("AudioSessionSetActive", &[1]), 0);
        assert_eq!(guest.call("AudioSessionSetActiveWithFlags", &[0, 0]), 0);
        // Category round trip.
        guest.cpu.write_bytes(DATA + 0x20, &fourcc(b"ambi").to_le_bytes());
        assert_eq!(
            guest.call("AudioSessionSetProperty", &[fourcc(b"acat") as u64, 4, DATA + 0x20]),
            0
        );
        guest.cpu.write_bytes(size, &4u32.to_le_bytes());
        assert_eq!(
            guest.call("AudioSessionGetProperty", &[fourcc(b"acat") as u64, size, out]),
            0
        );
        assert_eq!(guest.u32_at(out), fourcc(b"ambi"));
        // Hardware sample rate is a double; a 4-byte buffer is too small.
        assert_eq!(
            guest.call("AudioSessionGetProperty", &[fourcc(b"chsr") as u64, size, out]),
            SESSION_BAD_PROPERTY_SIZE
        );
        guest.cpu.write_bytes(size, &8u32.to_le_bytes());
        assert_eq!(
            guest.call("AudioSessionGetProperty", &[fourcc(b"chsr") as u64, size, out]),
            0
        );
        assert_eq!(f64::from_bits(guest.u64_at(out)), HOST_SAMPLE_RATE);
        assert_eq!(
            guest.call("AudioSessionGetProperty", &[fourcc(b"zzzz") as u64, size, out]),
            SESSION_UNSUPPORTED_PROPERTY
        );
        // Wrong size for a settable property is rejected, not truncated.
        assert_eq!(
            guest.call("AudioSessionSetProperty", &[fourcc(b"hwsr") as u64, 4, DATA + 0x20]),
            SESSION_BAD_PROPERTY_SIZE
        );
    }

    /// Replays ALmixer's CoreAudio decoder: open through guest callbacks,
    /// query format and duration, wrap, set the S16 client format, read in
    /// pieces to EOF, rewind, dispose, close.
    #[test]
    fn almixer_coreaudio_decoder_sequence_through_guest_callbacks() {
        let mut guest = Guest::new();
        // 280 KB of PCM: more than one 256 KiB read-proc chunk.
        let frames = 70_000u32;
        let file = wav(2, 22050, 16, frames);
        let (client, read_proc, size_proc) = guest.install_procs(&file);
        let out = DATA;
        let status = guest.call(
            "AudioFileOpenWithCallbacks",
            &[client, read_proc, 0, size_proc, 0, fourcc(b"WAVE") as u64, out],
        );
        assert_eq!(status, 0);
        assert!(guest.u64_at(CLIENT + 16) >= 2, "read proc ran in chunks");
        let audio_file = guest.u64_at(out);
        assert_ne!(audio_file, 0);

        let size = DATA + 0x10;
        let asbd = DATA + 0x40;
        guest.cpu.write_bytes(size, &(ASBD_BYTES as u32).to_le_bytes());
        assert_eq!(
            guest.call("AudioFileGetProperty", &[audio_file, fourcc(b"dfmt") as u64, size, asbd]),
            0
        );
        let format = Asbd::from_bytes(guest.cpu.read_bytes(asbd, ASBD_BYTES).unwrap());
        assert_eq!(format.sample_rate, 22050.0);
        assert_eq!(format.channels_per_frame, 2);
        assert_eq!(format.bits_per_channel, 16);
        guest.cpu.write_bytes(size, &8u32.to_le_bytes());
        assert_eq!(
            guest.call("AudioFileGetProperty", &[audio_file, fourcc(b"edur") as u64, size, DATA + 0x80]),
            0
        );
        let duration = f64::from_bits(guest.u64_at(DATA + 0x80));
        assert!((duration - frames as f64 / 22050.0).abs() < 1e-6);

        let ext_out = DATA + 0x90;
        assert_eq!(guest.call("ExtAudioFileWrapAudioFileID", &[audio_file, 0, ext_out]), 0);
        let ext = guest.u64_at(ext_out);
        // Unsupported client: float output is refused, not converted.
        let float_client = Asbd {
            format_flags: FLAG_FLOAT | FLAG_PACKED,
            bits_per_channel: 32,
            bytes_per_frame: 8,
            bytes_per_packet: 8,
            ..format
        };
        guest.cpu.write_bytes(asbd, &float_client.to_bytes());
        assert_eq!(
            guest.call("ExtAudioFileSetProperty", &[ext, fourcc(b"cfmt") as u64, 40, asbd]),
            FORMAT_NOT_SUPPORTED
        );
        let client_format = Asbd {
            format_flags: FLAG_SIGNED | FLAG_PACKED,
            ..format
        };
        guest.cpu.write_bytes(asbd, &client_format.to_bytes());
        assert_eq!(
            guest.call("ExtAudioFileSetProperty", &[ext, fourcc(b"cfmt") as u64, 40, asbd]),
            0
        );

        // Read in 16 KiB pieces until EOF (0 frames).
        let pcm = 0x400000;
        guest.cpu.map_zeroed(pcm, 0x40000, 3).unwrap();
        let list = DATA + 0x100;
        let frames_ptr = DATA + 0x140;
        let mut collected = Vec::new();
        loop {
            let piece = 16 * 1024u32;
            guest.cpu.write_bytes(list, &1u32.to_le_bytes());
            guest.cpu.write_bytes(list + 8, &2u32.to_le_bytes());
            guest.cpu.write_bytes(list + 12, &piece.to_le_bytes());
            guest.cpu.write_bytes(list + 16, &pcm.to_le_bytes());
            guest.cpu.write_bytes(frames_ptr, &(piece / 4).to_le_bytes());
            assert_eq!(guest.call("ExtAudioFileRead", &[ext, frames_ptr, list]), 0);
            let got = guest.u32_at(frames_ptr);
            assert_eq!(guest.u32_at(list + 12), got * 4);
            if got == 0 {
                break;
            }
            collected.extend_from_slice(guest.cpu.read_bytes(pcm, got as usize * 4).unwrap());
        }
        assert_eq!(collected.len(), frames as usize * 4);
        let expected: Vec<u8> = (0..frames * 2)
            .flat_map(|i| (i as i16).wrapping_mul(37).to_le_bytes())
            .collect();
        assert_eq!(collected, expected);

        // Rewind and read again from the start.
        assert_eq!(guest.call("ExtAudioFileSeek", &[ext, 0]), 0);
        guest.cpu.write_bytes(frames_ptr, &4u32.to_le_bytes());
        guest.cpu.write_bytes(list + 12, &16u32.to_le_bytes());
        assert_eq!(guest.call("ExtAudioFileRead", &[ext, frames_ptr, list]), 0);
        assert_eq!(guest.cpu.read_bytes(pcm, 16).unwrap(), &expected[..16]);
        assert_eq!(guest.call("ExtAudioFileSeek", &[ext, u64::MAX]), PARAM_ERR);

        assert_eq!(guest.call("ExtAudioFileDispose", &[ext]), 0);
        assert_eq!(guest.call("AudioFileClose", &[audio_file]), 0);
        assert_eq!(guest.call("AudioFileClose", &[audio_file]), FILE_NOT_OPEN);
        // The I/O buffer was released: a second open works.
        let status = guest.call(
            "AudioFileOpenWithCallbacks",
            &[client, read_proc, 0, size_proc, 0, 0, out],
        );
        assert_eq!(status, 0);
    }

    #[test]
    fn eight_bit_files_convert_and_bad_data_fails_honestly() {
        let mut guest = Guest::new();
        let file = wav(1, 8000, 8, 1000);
        let (client, read_proc, size_proc) = guest.install_procs(&file);
        let out = DATA;
        assert_eq!(
            guest.call("AudioFileOpenWithCallbacks", &[client, read_proc, 0, size_proc, 0, 0, out]),
            0
        );
        let audio_file = guest.u64_at(out);
        let ext_out = DATA + 0x90;
        assert_eq!(guest.call("ExtAudioFileWrapAudioFileID", &[audio_file, 0, ext_out]), 0);
        let ext = guest.u64_at(ext_out);
        let client_format = Asbd {
            sample_rate: 8000.0,
            format_id: LPCM,
            format_flags: FLAG_SIGNED | FLAG_PACKED,
            bytes_per_packet: 2,
            frames_per_packet: 1,
            bytes_per_frame: 2,
            channels_per_frame: 1,
            bits_per_channel: 16,
        };
        guest.cpu.write_bytes(DATA + 0x40, &client_format.to_bytes());
        assert_eq!(
            guest.call("ExtAudioFileSetProperty", &[ext, fourcc(b"cfmt") as u64, 40, DATA + 0x40]),
            0
        );
        let list = DATA + 0x100;
        guest.cpu.write_bytes(list, &1u32.to_le_bytes());
        guest.cpu.write_bytes(list + 12, &8u32.to_le_bytes());
        guest.cpu.write_bytes(list + 16, &(DATA + 0x200).to_le_bytes());
        guest.cpu.write_bytes(DATA + 0x140, &4u32.to_le_bytes());
        assert_eq!(guest.call("ExtAudioFileRead", &[ext, DATA + 0x140, list]), 0);
        let samples: Vec<i16> = guest
            .cpu
            .read_bytes(DATA + 0x200, 8)
            .unwrap()
            .chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(samples, vec![-100 << 8, -99 << 8, -98 << 8, -97 << 8]);
        // Close before dispose keeps the data alive for the wrapper.
        assert_eq!(guest.call("AudioFileClose", &[audio_file]), 0);
        guest.cpu.write_bytes(DATA + 0x140, &4u32.to_le_bytes());
        assert_eq!(guest.call("ExtAudioFileRead", &[ext, DATA + 0x140, list]), 0);
        assert_eq!(guest.call("ExtAudioFileDispose", &[ext]), 0);
        assert_eq!(guest.call("AudioFileClose", &[audio_file]), FILE_NOT_OPEN);

        // Garbage data: unsupported type, nothing written to *out.
        let (client, read_proc, size_proc) = guest.install_procs(&[0x5a; 4096]);
        guest.cpu.write_bytes(out, &0u64.to_le_bytes());
        assert_eq!(
            guest.call("AudioFileOpenWithCallbacks", &[client, read_proc, 0, size_proc, 0, 0, out]),
            FILE_UNSUPPORTED_TYPE
        );
        assert_eq!(guest.u64_at(out), 0);
        // Missing procs are a parameter error.
        assert_eq!(
            guest.call("AudioFileOpenWithCallbacks", &[client, 0, 0, size_proc, 0, 0, out]),
            PARAM_ERR
        );
    }

    /// Optional: decode a real Coromon MP3 through the guest-callback path.
    /// COROMON_MP3=/path/to/sound.mp3 cargo test ... -- --ignored
    #[test]
    #[ignore]
    fn coromon_mp3_through_guest_callbacks() {
        let path = std::env::var("COROMON_MP3").expect("COROMON_MP3");
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.len() < 0x100000, "use a small sound effect");
        let mut guest = Guest::new();
        let (client, read_proc, size_proc) = guest.install_procs(&bytes);
        let out = DATA;
        assert_eq!(
            guest.call(
                "AudioFileOpenWithCallbacks",
                &[client, read_proc, 0, size_proc, 0, fourcc(b"MPG3") as u64, out]
            ),
            0
        );
        let size = DATA + 0x10;
        guest.cpu.write_bytes(size, &(ASBD_BYTES as u32).to_le_bytes());
        let audio_file = guest.u64_at(out);
        assert_eq!(
            guest.call("AudioFileGetProperty", &[audio_file, fourcc(b"dfmt") as u64, size, DATA + 0x40]),
            0
        );
        let format = Asbd::from_bytes(guest.cpu.read_bytes(DATA + 0x40, ASBD_BYTES).unwrap());
        println!("COROMON_MP3 decoded format: {format:?}");
        assert!(format.sample_rate > 0.0 && format.channels_per_frame > 0);
    }
}
