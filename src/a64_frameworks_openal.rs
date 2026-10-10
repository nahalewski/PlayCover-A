/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Emulator-owned OpenAL.framework for ARM64 guests, backed by OpenAL Soft.
//!
//! Apple's OpenAL needs the audio HAL/mediaserverd and cannot run from the
//! shared cache. Guest `ALCdevice*`/`ALCcontext*` values are opaque arena
//! handles mapped to host pointers; buffer/source names are passed through
//! unchanged (OpenAL names are plain integers). Exactly the 57 functions that
//! Coromon (Corona/ALmixer) imports are routed.
use super::{
    arg_f32, arg_i32, arg_u32, read_c_string, read_u32s, ret_f32, ret_void, write_u32s, Arena,
    Family,
};
use crate::a64::bridge::{ReturnValues, ServiceFrame};
use crate::audio::openal::OpenALManager;
use std::collections::BTreeMap;
use std::ffi::{c_uint, c_void, CStr};
use touchHLE_openal_soft_wrapper as al;

// Core AL 1.1 entry points that the shared wrapper crate does not declare.
// They resolve against the same OpenAL Soft library the wrapper links.
mod ext {
    use std::ffi::{c_char, c_float, c_int};
    extern "C" {
        pub fn alDisable(capability: c_int);
        pub fn alIsEnabled(capability: c_int) -> c_char;
        pub fn alGetBoolean(param: c_int) -> c_char;
        pub fn alGetFloat(param: c_int) -> c_float;
        pub fn alGetFloatv(param: c_int, values: *mut c_float);
        pub fn alGetInteger(param: c_int) -> c_int;
        pub fn alGetIntegerv(param: c_int, values: *mut c_int);
        pub fn alGetString(param: c_int) -> *const c_char;
    }
}

pub(super) const PROVIDER: &str = "/System/Library/Frameworks/OpenAL.framework/OpenAL";
const ALC_INVALID_DEVICE: u64 = 0xA001;
const AL_POSITION: i32 = 0x1004;
const AL_DIRECTION: i32 = 0x1005;
const AL_VELOCITY: i32 = 0x1006;
const AL_ORIENTATION: i32 = 0x100F;
/// Generous but bounded name-array length for gen/delete/queue calls.
const MAX_NAMES: i32 = 4096;
/// alBufferData copies through the bridge's bulk budget (64 MiB per call),
/// with headroom for the call's other reads.
const MAX_BUFFER_DATA: i32 = crate::a64::bridge::MAX_BULK_MEMORY as i32;

macro_rules! symbols {
    ($($name:ident),* $(,)?) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(usize)]
        enum Func { $($name),* }
        const FNS: &[Func] = &[$(Func::$name),*];
        const SYMBOLS: &[&str] = &[$(concat!("_", stringify!($name))),*];
    };
}
symbols!(
    alcOpenDevice, alcCloseDevice, alcCreateContext, alcDestroyContext,
    alcMakeContextCurrent, alcGetCurrentContext, alcGetContextsDevice,
    alcProcessContext, alcSuspendContext, alcGetError,
    alGetError, alGetString, alGetEnumValue, alGetProcAddress, alIsExtensionPresent,
    alEnable, alDisable, alIsEnabled, alGetBoolean, alGetFloat, alGetFloatv,
    alGetInteger, alGetIntegerv, alDistanceModel, alDopplerFactor,
    alDopplerVelocity, alSpeedOfSound,
    alListenerf, alListener3f, alListenerfv, alListeneri, alListener3i,
    alGetListenerf, alGetListenerfv, alGetListeneri, alGetListeneriv,
    alGenSources, alDeleteSources, alIsSource, alSourcef, alSource3f,
    alSourcei, alSource3i, alGetSourcef, alGetSourcefv, alGetSourcei,
    alGetSourceiv, alSourcePlay, alSourcePause, alSourceStop, alSourceRewind,
    alSourceQueueBuffers, alSourceUnqueueBuffers,
    alGenBuffers, alDeleteBuffers, alIsBuffer, alBufferData,
);

/// Values per vector query: positions/velocities/directions are 3-vectors,
/// listener orientation is "at" + "up", everything else is scalar.
fn param_count(param: i32) -> usize {
    match param {
        AL_POSITION | AL_VELOCITY | AL_DIRECTION => 3,
        AL_ORIENTATION => 6,
        _ => 1,
    }
}

#[derive(Default)]
pub(in crate::a64) struct OpenAl {
    manager: Option<OpenALManager>,
    devices: BTreeMap<u64, *mut c_void>,
    contexts: BTreeMap<u64, (*mut c_void, u64)>,
    current: u64,
    strings: BTreeMap<i32, u64>,
    entries: Vec<(&'static str, u64)>,
}
impl Drop for OpenAl {
    fn drop(&mut self) {
        // Host resources must not outlive the guest session that owns them.
        if self.manager.is_none() {
            return;
        }
        unsafe {
            al::alcMakeContextCurrent(std::ptr::null_mut());
            for &(context, _) in self.contexts.values() {
                al::alcDestroyContext(context);
            }
            for &device in self.devices.values() {
                al::alcCloseDevice(device);
            }
        }
    }
}

impl OpenAl {
    fn host_device(&self, handle: u64) -> Option<*mut c_void> {
        self.devices.get(&handle).copied()
    }
    /// Every al* call needs a current context in OpenAL Soft; without one
    /// the spec leaves behaviour undefined and Soft silently ignores it.
    fn require_context(&self, function: Func) -> Result<(), String> {
        if self.current == 0 {
            return Err(format!("OpenAL {function:?} called without a current context"));
        }
        Ok(())
    }
    fn names(frame: &ServiceFrame<'_>, function: Func) -> Result<usize, String> {
        let n = arg_i32(frame, 0)?;
        if !(0..=MAX_NAMES).contains(&n) {
            return Err(format!("OpenAL {function:?} name count {n} out of range"));
        }
        Ok(n as usize)
    }
    fn read_floats(
        frame: &mut ServiceFrame<'_>,
        address: u64,
        count: usize,
    ) -> Result<Vec<f32>, String> {
        Ok(read_u32s(frame, address, count)?
            .into_iter()
            .map(f32::from_bits)
            .collect())
    }
    fn write_floats(
        frame: &mut ServiceFrame<'_>,
        address: u64,
        values: &[f32],
    ) -> Result<(), String> {
        let bits: Vec<u32> = values.iter().map(|v| v.to_bits()).collect();
        write_u32s(frame, address, &bits)
    }
    fn write_ints(frame: &mut ServiceFrame<'_>, address: u64, values: &[i32]) -> Result<(), String> {
        let bits: Vec<u32> = values.iter().map(|&v| v as u32).collect();
        write_u32s(frame, address, &bits)
    }

    fn dispatch(
        &mut self,
        function: Func,
        frame: &mut ServiceFrame<'_>,
        arena: &mut Arena,
    ) -> Result<ReturnValues, String> {
        use Func::*;
        let i = |index| ReturnValues::integer(index);
        // alc* functions validate their own handles; al* need a context.
        if !matches!(
            function,
            alcOpenDevice
                | alcCloseDevice
                | alcCreateContext
                | alcDestroyContext
                | alcMakeContextCurrent
                | alcGetCurrentContext
                | alcGetContextsDevice
                | alcProcessContext
                | alcSuspendContext
                | alcGetError
                | alGetProcAddress
        ) {
            self.require_context(function)?;
        }
        Ok(match function {
            alcOpenDevice => {
                let name = frame.integer(0)?;
                let name = if name == 0 {
                    None
                } else {
                    let mut bytes = read_c_string(frame, name, 256)?;
                    bytes.push(0);
                    Some(bytes)
                };
                if self.manager.is_none() {
                    self.manager = Some(OpenALManager::new()?);
                }
                let device = unsafe {
                    al::alcOpenDevice(name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr().cast()))
                };
                if device.is_null() {
                    log!("[a64] OpenAL: host device could not be opened");
                    i(0)
                } else {
                    let handle = arena.allocate_handle()?;
                    self.devices.insert(handle, device);
                    i(handle)
                }
            }
            alcCloseDevice => {
                let handle = frame.integer(0)?;
                let Some(device) = self.host_device(handle) else {
                    return Ok(i(0));
                };
                if self.contexts.values().any(|&(_, d)| d == handle) {
                    // OpenAL 1.1: closing a device with live contexts fails.
                    return Ok(i(0));
                }
                let ok = unsafe { al::alcCloseDevice(device) } != 0;
                if ok {
                    self.devices.remove(&handle);
                    arena.release_handle(handle)?;
                }
                i(ok as u64)
            }
            alcCreateContext => {
                let handle = frame.integer(0)?;
                let Some(device) = self.host_device(handle) else {
                    return Ok(i(0));
                };
                let list = frame.integer(1)?;
                let mut attributes = Vec::new();
                if list != 0 {
                    // Zero-terminated key/value pairs.
                    loop {
                        if attributes.len() >= 64 {
                            return Err("OpenAL context attribute list unterminated".into());
                        }
                        let key = read_u32s(frame, list + attributes.len() as u64 * 4, 1)?[0];
                        attributes.push(key as i32);
                        if key == 0 {
                            break;
                        }
                        let value =
                            read_u32s(frame, list + attributes.len() as u64 * 4, 1)?[0];
                        attributes.push(value as i32);
                    }
                }
                let context = unsafe {
                    al::alcCreateContext(
                        device,
                        if attributes.is_empty() {
                            std::ptr::null()
                        } else {
                            attributes.as_ptr()
                        },
                    )
                };
                if context.is_null() {
                    i(0)
                } else {
                    let guest = arena.allocate_handle()?;
                    self.contexts.insert(guest, (context, handle));
                    i(guest)
                }
            }
            alcDestroyContext => {
                let guest = frame.integer(0)?;
                if let Some((context, _)) = self.contexts.remove(&guest) {
                    if self.current == guest {
                        unsafe { al::alcMakeContextCurrent(std::ptr::null_mut()) };
                        self.current = 0;
                    }
                    unsafe { al::alcDestroyContext(context) };
                    arena.release_handle(guest)?;
                }
                ret_void()
            }
            alcMakeContextCurrent => {
                let guest = frame.integer(0)?;
                let host = if guest == 0 {
                    std::ptr::null_mut()
                } else if let Some(&(context, _)) = self.contexts.get(&guest) {
                    context
                } else {
                    return Ok(i(0));
                };
                let ok = unsafe { al::alcMakeContextCurrent(host) } != 0;
                if ok {
                    self.current = guest;
                }
                i(ok as u64)
            }
            alcGetCurrentContext => i(self.current),
            alcGetContextsDevice => {
                i(self.contexts.get(&frame.integer(0)?).map_or(0, |&(_, d)| d))
            }
            alcProcessContext | alcSuspendContext => {
                if let Some(&(context, _)) = self.contexts.get(&frame.integer(0)?) {
                    unsafe {
                        if function == alcProcessContext {
                            al::alcProcessContext(context)
                        } else {
                            al::alcSuspendContext(context)
                        }
                    }
                }
                ret_void()
            }
            alcGetError => {
                let handle = frame.integer(0)?;
                if handle == 0 {
                    i(unsafe { al::alcGetError(std::ptr::null_mut()) } as u32 as u64)
                } else if let Some(device) = self.host_device(handle) {
                    i(unsafe { al::alcGetError(device) } as u32 as u64)
                } else {
                    i(ALC_INVALID_DEVICE)
                }
            }
            alGetError => i(unsafe { al::alGetError() } as u32 as u64),
            alGetString => {
                let param = arg_i32(frame, 0)?;
                if let Some(&address) = self.strings.get(&param) {
                    return Ok(i(address));
                }
                let host = unsafe { ext::alGetString(param) };
                if host.is_null() {
                    i(0)
                } else {
                    let text = unsafe { CStr::from_ptr(host) }.to_bytes().to_vec();
                    let address = arena.store_c_string(frame, &text)?;
                    self.strings.insert(param, address);
                    i(address)
                }
            }
            alGetEnumValue | alIsExtensionPresent => {
                let address = frame.integer(0)?;
                let mut name = read_c_string(frame, address, 256)?;
                name.push(0);
                let ptr = name.as_ptr().cast();
                i(unsafe {
                    if function == alGetEnumValue {
                        al::alGetEnumValue(ptr) as u32 as u64
                    } else {
                        al::alIsExtensionPresent(ptr) as u8 as u64
                    }
                })
            }
            alGetProcAddress => {
                // Only functions this layer really implements are returned;
                // Apple-only extensions (alcMacOSXMixerOutputRate, ...) are
                // absent, which ALmixer handles by skipping them.
                let address = frame.integer(0)?;
                let name = read_c_string(frame, address, 256)?;
                let wanted = format!("_{}", String::from_utf8_lossy(&name));
                i(self
                    .entries
                    .iter()
                    .find(|(symbol, _)| *symbol == wanted)
                    .map_or(0, |&(_, entry)| entry))
            }
            alEnable => {
                unsafe { al::alEnable(arg_i32(frame, 0)?) };
                ret_void()
            }
            alDisable => {
                unsafe { ext::alDisable(arg_i32(frame, 0)?) };
                ret_void()
            }
            alIsEnabled => i(unsafe { ext::alIsEnabled(arg_i32(frame, 0)?) } as u8 as u64),
            alGetBoolean => i(unsafe { ext::alGetBoolean(arg_i32(frame, 0)?) } as u8 as u64),
            alGetFloat => ret_f32(unsafe { ext::alGetFloat(arg_i32(frame, 0)?) }),
            alGetInteger => i(unsafe { ext::alGetInteger(arg_i32(frame, 0)?) } as u32 as u64),
            alGetFloatv | alGetIntegerv => {
                let param = arg_i32(frame, 0)?;
                let out = frame.integer(1)?;
                let mut values = [0u32; 6];
                let count = param_count(param);
                unsafe {
                    if function == alGetFloatv {
                        ext::alGetFloatv(param, values.as_mut_ptr().cast());
                    } else {
                        ext::alGetIntegerv(param, values.as_mut_ptr().cast());
                    }
                }
                write_u32s(frame, out, &values[..count])?;
                ret_void()
            }
            alDistanceModel => {
                unsafe { al::alDistanceModel(arg_i32(frame, 0)?) };
                ret_void()
            }
            alDopplerFactor | alDopplerVelocity | alSpeedOfSound => {
                let value = arg_f32(frame, 0)?;
                unsafe {
                    match function {
                        alDopplerFactor => al::alDopplerFactor(value),
                        alDopplerVelocity => al::alDopplerVelocity(value),
                        _ => al::alSpeedOfSound(value),
                    }
                }
                ret_void()
            }
            alListenerf => {
                unsafe { al::alListenerf(arg_i32(frame, 0)?, arg_f32(frame, 0)?) };
                ret_void()
            }
            alListener3f => {
                unsafe {
                    al::alListener3f(
                        arg_i32(frame, 0)?,
                        arg_f32(frame, 0)?,
                        arg_f32(frame, 1)?,
                        arg_f32(frame, 2)?,
                    )
                };
                ret_void()
            }
            alListenerfv => {
                let param = arg_i32(frame, 0)?;
                let values = Self::read_floats(frame, frame.integer(1)?, param_count(param))?;
                unsafe { al::alListenerfv(param, values.as_ptr()) };
                ret_void()
            }
            alListeneri => {
                unsafe { al::alListeneri(arg_i32(frame, 0)?, arg_i32(frame, 1)?) };
                ret_void()
            }
            alListener3i => {
                unsafe {
                    al::alListener3i(
                        arg_i32(frame, 0)?,
                        arg_i32(frame, 1)?,
                        arg_i32(frame, 2)?,
                        arg_i32(frame, 3)?,
                    )
                };
                ret_void()
            }
            alGetListenerf | alGetListenerfv => {
                let param = arg_i32(frame, 0)?;
                let out = frame.integer(1)?;
                let mut values = [0f32; 6];
                let count = if function == alGetListenerf { 1 } else { param_count(param) };
                unsafe { al::alGetListenerfv(param, values.as_mut_ptr()) };
                Self::write_floats(frame, out, &values[..count])?;
                ret_void()
            }
            alGetListeneri | alGetListeneriv => {
                let param = arg_i32(frame, 0)?;
                let out = frame.integer(1)?;
                let mut values = [0i32; 6];
                let count = if function == alGetListeneri { 1 } else { param_count(param) };
                unsafe { al::alGetListeneriv(param, values.as_mut_ptr()) };
                Self::write_ints(frame, out, &values[..count])?;
                ret_void()
            }
            alGenSources | alGenBuffers => {
                let n = Self::names(frame, function)?;
                let out = frame.integer(1)?;
                let mut names = vec![0 as c_uint; n];
                unsafe {
                    if function == alGenSources {
                        al::alGenSources(n as i32, names.as_mut_ptr())
                    } else {
                        al::alGenBuffers(n as i32, names.as_mut_ptr())
                    }
                }
                // On failure Soft leaves the names untouched (zero) and sets
                // an AL error, which the guest reads with alGetError.
                write_u32s(frame, out, &names)?;
                ret_void()
            }
            alDeleteSources | alDeleteBuffers => {
                let n = Self::names(frame, function)?;
                let names = read_u32s(frame, frame.integer(1)?, n)?;
                unsafe {
                    if function == alDeleteSources {
                        al::alDeleteSources(n as i32, names.as_ptr())
                    } else {
                        al::alDeleteBuffers(n as i32, names.as_ptr())
                    }
                }
                ret_void()
            }
            alIsSource => i(unsafe { al::alIsSource(arg_u32(frame, 0)?) } as u8 as u64),
            alIsBuffer => i(unsafe { al::alIsBuffer(arg_u32(frame, 0)?) } as u8 as u64),
            alSourcef => {
                unsafe { al::alSourcef(arg_u32(frame, 0)?, arg_i32(frame, 1)?, arg_f32(frame, 0)?) };
                ret_void()
            }
            alSource3f => {
                unsafe {
                    al::alSource3f(
                        arg_u32(frame, 0)?,
                        arg_i32(frame, 1)?,
                        arg_f32(frame, 0)?,
                        arg_f32(frame, 1)?,
                        arg_f32(frame, 2)?,
                    )
                };
                ret_void()
            }
            alSourcei => {
                unsafe { al::alSourcei(arg_u32(frame, 0)?, arg_i32(frame, 1)?, arg_i32(frame, 2)?) };
                ret_void()
            }
            alSource3i => {
                unsafe {
                    al::alSource3i(
                        arg_u32(frame, 0)?,
                        arg_i32(frame, 1)?,
                        arg_i32(frame, 2)?,
                        arg_i32(frame, 3)?,
                        arg_i32(frame, 4)?,
                    )
                };
                ret_void()
            }
            alGetSourcef | alGetSourcefv => {
                let source = arg_u32(frame, 0)?;
                let param = arg_i32(frame, 1)?;
                let out = frame.integer(2)?;
                let mut values = [0f32; 6];
                let count = if function == alGetSourcef { 1 } else { param_count(param) };
                unsafe { al::alGetSourcefv(source, param, values.as_mut_ptr()) };
                Self::write_floats(frame, out, &values[..count])?;
                ret_void()
            }
            alGetSourcei | alGetSourceiv => {
                let source = arg_u32(frame, 0)?;
                let param = arg_i32(frame, 1)?;
                let out = frame.integer(2)?;
                let mut values = [0i32; 6];
                let count = if function == alGetSourcei { 1 } else { param_count(param) };
                unsafe { al::alGetSourceiv(source, param, values.as_mut_ptr()) };
                Self::write_ints(frame, out, &values[..count])?;
                ret_void()
            }
            alSourcePlay | alSourcePause | alSourceStop | alSourceRewind => {
                let source = arg_u32(frame, 0)?;
                unsafe {
                    match function {
                        alSourcePlay => al::alSourcePlay(source),
                        alSourcePause => al::alSourcePause(source),
                        alSourceStop => al::alSourceStop(source),
                        _ => al::alSourceRewind(source),
                    }
                }
                ret_void()
            }
            alSourceQueueBuffers | alSourceUnqueueBuffers => {
                let source = arg_u32(frame, 0)?;
                let n = arg_i32(frame, 1)?;
                if !(0..=MAX_NAMES).contains(&n) {
                    return Err(format!("OpenAL {function:?} buffer count {n} out of range"));
                }
                let address = frame.integer(2)?;
                if function == alSourceQueueBuffers {
                    let names = read_u32s(frame, address, n as usize)?;
                    unsafe { al::alSourceQueueBuffers(source, n, names.as_ptr()) };
                } else {
                    // Only write back when Soft accepted the request; on error
                    // the guest array must stay untouched.
                    let mut names = vec![0 as c_uint; n as usize];
                    unsafe {
                        al::alGetError();
                        al::alSourceUnqueueBuffers(source, n, names.as_mut_ptr());
                    }
                    let error = unsafe { al::alGetError() };
                    if error == 0 {
                        write_u32s(frame, address, &names)?;
                    } else {
                        // Re-raise the same error for the guest's alGetError.
                        unsafe { al::alSourceUnqueueBuffers(source, n, names.as_mut_ptr()) };
                    }
                }
                ret_void()
            }
            alBufferData => {
                let buffer = arg_u32(frame, 0)?;
                let format = arg_i32(frame, 1)?;
                let data = frame.integer(2)?;
                let size = arg_i32(frame, 3)?;
                let frequency = arg_i32(frame, 4)?;
                if !(0..=MAX_BUFFER_DATA).contains(&size) {
                    return Err(format!(
                        "OpenAL alBufferData of {size} bytes exceeds the bridge's per-call bulk copy budget"
                    ));
                }
                let bytes = if size == 0 || data == 0 {
                    Vec::new()
                } else {
                    frame.read_bulk(data, size as usize)?
                };
                unsafe {
                    al::alBufferData(
                        buffer,
                        format,
                        if bytes.is_empty() { std::ptr::null() } else { bytes.as_ptr().cast() },
                        bytes.len() as i32,
                        frequency,
                    )
                };
                ret_void()
            }
        })
    }
}

impl Family for OpenAl {
    fn name(&self) -> &'static str {
        "openal"
    }
    fn provider(&self) -> &'static str {
        PROVIDER
    }
    fn symbols(&self) -> &'static [&'static str] {
        SYMBOLS
    }
    fn bind_entries(&mut self, entries: &[(&'static str, u64)]) {
        self.entries = entries.to_vec();
    }
    fn call(
        &mut self,
        index: usize,
        frame: &mut ServiceFrame<'_>,
        arena: &mut Arena,
    ) -> Result<ReturnValues, String> {
        let function = *FNS.get(index).ok_or("OpenAL dispatch index invalid")?;
        self.dispatch(function, frame, arena)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{install, Frameworks};
    use super::*;
    use crate::a64::bridge::{GuestBridge, GuestCall};
    use crate::a64::A64Cpu;
    use std::sync::Mutex;

    /// OpenALManager is a process-wide singleton; serialize device tests.
    static DEVICE: Mutex<()> = Mutex::new(());

    const DATA: u64 = 0x80000;
    /// A predecoded sound larger than the ordinary 1 MiB service budget.
    const BIG: u64 = 0x1000_0000;
    const BIG_LEN: u64 = 3 << 20;

    struct Guest {
        cpu: A64Cpu,
        bridge: GuestBridge,
        frameworks: Frameworks,
    }
    impl Guest {
        fn new() -> Self {
            // The null backend works without audio hardware (WSL/CI).
            std::env::set_var("ALSOFT_DRIVERS", "null");
            let mut cpu = A64Cpu::new_sparse();
            cpu.map_zeroed(DATA, 0x20000, 3).unwrap();
            cpu.map_zeroed(BIG, BIG_LEN as usize, 3).unwrap();
            let mut bridge = GuestBridge::map(&mut cpu, 0x20000).unwrap();
            let frameworks = install(
                &mut cpu,
                &mut bridge,
                0x200000,
                vec![Box::new(OpenAl::default())],
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
                .unwrap_or_else(|| panic!("{name} not routed"))
                .address
        }
        fn try_call(
            &mut self,
            name: &str,
            integers: &[u64],
            floats: &[f32],
        ) -> Result<ReturnValues, String> {
            let entry = self.entry(name);
            self.bridge.call(
                &mut self.cpu,
                &GuestCall {
                    entry,
                    integers: integers.to_vec(),
                    vectors: floats.iter().map(|f| [f.to_bits() as u64, 0]).collect(),
                    ..Default::default()
                },
                1000,
            )
        }
        fn call(&mut self, name: &str, integers: &[u64], floats: &[f32]) -> u64 {
            self.try_call(name, integers, floats)
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .integers[0]
        }
        fn u32_at(&self, address: u64) -> u32 {
            u32::from_le_bytes(self.cpu.read_bytes(address, 4).unwrap().try_into().unwrap())
        }
        fn f32_at(&self, address: u64) -> f32 {
            f32::from_bits(self.u32_at(address))
        }
    }

    #[test]
    fn routes_exactly_coromons_openal_imports() {
        assert_eq!(SYMBOLS.len(), 57);
        assert_eq!(FNS.len(), SYMBOLS.len());
        let unique: std::collections::BTreeSet<_> = SYMBOLS.iter().collect();
        assert_eq!(unique.len(), SYMBOLS.len());
        assert!(SYMBOLS.contains(&"_alcOpenDevice"));
        assert!(SYMBOLS.contains(&"_alBufferData"));
    }

    #[test]
    fn al_calls_without_context_fail_explicitly() {
        let _lock = DEVICE.lock().unwrap_or_else(|e| e.into_inner());
        let mut guest = Guest::new();
        assert!(guest
            .try_call("alGenSources", &[1, DATA], &[])
            .unwrap_err()
            .contains("without a current context"));
        assert_eq!(guest.call("alcGetCurrentContext", &[], &[]), 0);
        assert_eq!(guest.call("alcMakeContextCurrent", &[0x1234], &[]), 0);
        assert_eq!(guest.call("alcGetError", &[0x1234], &[]), ALC_INVALID_DEVICE);
    }

    /// Replays ALmixer_Init and a one-shot sound through real guest
    /// trampolines: open, context, make current, 32 sources, a buffer,
    /// source parameters, play, state query, teardown.
    #[test]
    fn almixer_init_and_buffer_playback_sequence() {
        let _lock = DEVICE.lock().unwrap_or_else(|e| e.into_inner());
        let mut guest = Guest::new();
        let device = guest.call("alcOpenDevice", &[0], &[]);
        assert_ne!(device, 0, "OpenAL Soft null backend must open");
        assert_eq!(guest.cpu.mapped_permissions(device), Some(3));
        // {ALC_FREQUENCY, 44100, 0}
        let attrs = DATA + 0x100;
        let list: Vec<u8> = [0x1007u32, 44100, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        guest.cpu.write_bytes(attrs, &list);
        let context = guest.call("alcCreateContext", &[device, attrs], &[]);
        assert_ne!(context, 0);
        assert_eq!(guest.call("alcMakeContextCurrent", &[context], &[]), 1);
        assert_eq!(guest.call("alcGetCurrentContext", &[], &[]), context);
        assert_eq!(guest.call("alcGetContextsDevice", &[context], &[]), device);
        assert_eq!(guest.call("alGetError", &[], &[]), 0);

        // Strings are copied once into the arena and stay stable.
        let vendor = guest.call("alGetString", &[0xb001], &[]);
        assert_ne!(vendor, 0);
        assert_eq!(guest.call("alGetString", &[0xb001], &[]), vendor);
        assert!(!guest.cpu.read_bytes(vendor, 1).unwrap().contains(&0));

        // Enum lookup and extension probes read guest C strings.
        guest.cpu.write_bytes(DATA + 0x200, b"AL_FORMAT_STEREO16\0");
        assert_eq!(guest.call("alGetEnumValue", &[DATA + 0x200], &[]), 0x1103);
        guest.cpu.write_bytes(DATA + 0x240, b"alcMacOSXMixerOutputRate\0");
        assert_eq!(guest.call("alGetProcAddress", &[DATA + 0x240], &[]), 0);
        guest.cpu.write_bytes(DATA + 0x280, b"alSourcePlay\0");
        assert_eq!(
            guest.call("alGetProcAddress", &[DATA + 0x280], &[]),
            guest.entry("alSourcePlay")
        );

        // ALmixer allocates its channel sources up front.
        let sources = DATA + 0x400;
        guest.call("alGenSources", &[32, sources], &[]);
        assert_eq!(guest.call("alGetError", &[], &[]), 0);
        let names: Vec<u32> = (0..32).map(|i| guest.u32_at(sources + i * 4)).collect();
        assert!(names.iter().all(|&n| n != 0));
        assert_eq!(guest.call("alIsSource", &[names[0] as u64], &[]), 1);

        // A short 16-bit mono buffer.
        let buffers = DATA + 0x600;
        guest.call("alGenBuffers", &[1, buffers], &[]);
        let buffer = guest.u32_at(buffers);
        assert_eq!(guest.call("alIsBuffer", &[buffer as u64], &[]), 1);
        let pcm = DATA + 0x1000;
        let samples: Vec<u8> = (0..2048u32)
            .flat_map(|i| (((i % 64) as i16 - 32) * 512).to_le_bytes())
            .collect();
        guest.cpu.write_bytes(pcm, &samples);
        guest.call(
            "alBufferData",
            &[buffer as u64, 0x1101, pcm, samples.len() as u64, 22050],
            &[],
        );
        assert_eq!(guest.call("alGetError", &[], &[]), 0);

        // Float arguments arrive in s-registers, separate from integers.
        let source = names[0] as u64;
        guest.call("alSourcef", &[source, 0x100A], &[0.25]); // AL_GAIN
        let out = DATA + 0x700;
        guest.call("alGetSourcef", &[source, 0x100A, out], &[]);
        assert_eq!(guest.f32_at(out), 0.25);
        guest.call("alSource3f", &[source, AL_POSITION as u64], &[1.0, 2.0, 3.0]);
        guest.call("alGetSourcefv", &[source, AL_POSITION as u64, out], &[]);
        assert_eq!(
            [guest.f32_at(out), guest.f32_at(out + 4), guest.f32_at(out + 8)],
            [1.0, 2.0, 3.0]
        );
        guest.call("alListenerf", &[0x100A], &[0.5]);
        assert_eq!(
            f32::from_bits(
                guest.try_call("alGetFloat", &[0xC000], &[]).unwrap().vectors[0][0] as u32
            ),
            1.0 // AL_DOPPLER_FACTOR default
        );
        guest.call("alGetListenerf", &[0x100A, out], &[]);
        assert_eq!(guest.f32_at(out), 0.5);

        guest.call("alSourcei", &[source, 0x1009, buffer as u64], &[]); // AL_BUFFER
        guest.call("alSourcePlay", &[source], &[]);
        guest.call("alGetSourcei", &[source, 0x1010, out], &[]); // AL_SOURCE_STATE
        let state = guest.u32_at(out);
        assert!(state == 0x1012 || state == 0x1014, "state {state:#x}");
        guest.call("alSourceStop", &[source], &[]);
        guest.call("alGetSourcei", &[source, 0x1010, out], &[]);
        assert_eq!(guest.u32_at(out), 0x1014);
        guest.call("alSourcei", &[source, 0x1009, 0], &[]);

        // Streaming: queue, then unqueue after stop marks it processed.
        guest.call("alSourceQueueBuffers", &[source, 1, buffers], &[]);
        guest.call("alGetSourcei", &[source, 0x1015, out], &[]); // BUFFERS_QUEUED
        assert_eq!(guest.u32_at(out), 1);
        guest.call("alSourceStop", &[source], &[]);
        let unqueued = DATA + 0x800;
        guest.call("alSourceUnqueueBuffers", &[source, 1, unqueued], &[]);
        assert_eq!(guest.call("alGetError", &[], &[]), 0);
        assert_eq!(guest.u32_at(unqueued), buffer);
        // Unqueuing more than processed fails and leaves memory untouched.
        guest.cpu.write_bytes(unqueued, &0xdeadbeefu32.to_le_bytes());
        guest.call("alSourceUnqueueBuffers", &[source, 1, unqueued], &[]);
        assert_ne!(guest.call("alGetError", &[], &[]), 0);
        assert_eq!(guest.u32_at(unqueued), 0xdeadbeef);

        // A 3 MiB predecoded sound uses the bulk budget and arrives intact.
        let ramp: Vec<u8> = (0..BIG_LEN).map(|i| (i % 251) as u8).collect();
        guest.cpu.write_bytes(BIG, &ramp);
        guest.call("alBufferData", &[buffer as u64, 0x1101, BIG, BIG_LEN, 44100], &[]);
        assert_eq!(guest.call("alGetError", &[], &[]), 0);
        // AL_SIZE (0x2004) via the host to confirm the full copy landed.
        assert_eq!(
            unsafe {
                let mut value = 0;
                al::alGetBufferi(buffer, 0x2004, &mut value);
                value
            },
            BIG_LEN as i32
        );
        // Beyond the bulk budget fails explicitly instead of truncating.
        assert!(guest
            .try_call("alBufferData", &[buffer as u64, 0x1101, BIG, 65 << 20, 22050], &[])
            .unwrap_err()
            .contains("budget"));
        // An unmapped tail fails without touching the AL buffer.
        assert!(guest
            .try_call("alBufferData", &[buffer as u64, 0x1101, BIG, BIG_LEN + 4096, 22050], &[])
            .is_err());

        guest.call("alDeleteSources", &[32, sources], &[]);
        guest.call("alDeleteBuffers", &[1, buffers], &[]);
        assert_eq!(guest.call("alGetError", &[], &[]), 0);
        assert_eq!(guest.call("alIsSource", &[source], &[]), 0);
        // Device with a live context cannot close.
        assert_eq!(guest.call("alcCloseDevice", &[device], &[]), 0);
        assert_eq!(guest.call("alcMakeContextCurrent", &[0], &[]), 1);
        guest.call("alcDestroyContext", &[context], &[]);
        assert_eq!(guest.call("alcGetContextsDevice", &[context], &[]), 0);
        assert_eq!(guest.call("alcCloseDevice", &[device], &[]), 1);
        assert_eq!(guest.call("alcCloseDevice", &[device], &[]), 0);
    }
}
