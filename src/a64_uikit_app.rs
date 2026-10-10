/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIApplicationMain` and the application launch sequence.
//!
//! `UIApplicationMain` is owned guest code ([pump_code]): a loop that asks
//! the host for the next piece of work (`PumpNext`), loads x0-x7/d0-d3 and
//! the entry from a record in the image's scratch data, calls it with `blr`,
//! stores x0/x1/d0 back and repeats. The run loop therefore lives on the
//! guest's own main stack and every callback is a genuine guest call; the
//! host only decides what happens next ([Launcher]). It never returns unless
//! the host stops it (tests, or a frame limit).
//!
//! The launch sequence follows UIKit with `UIApplicationMain(argc, argv, nil,
//! nil)`: the delegate and window come from the Info.plist main nib
//! ([parse_nib]; Coromon's MainWindow.nib). Then:
//! willFinishLaunching / didFinishLaunching (each only if the delegate
//! responds), make the nib's visible window key, applicationDidBecomeActive:,
//! viewWillAppear:/viewDidAppear: on the root view controller, then frames.
//! Notifications are NOT posted yet (needs the NSString constants, milestone
//! 2); that is recorded in the launch log.
use super::{
    asm::Asm, image::Symbols, ret, state::ApplicationState, Internal, UiKit,
};
use super::super::bridge::{ReturnValues, ServiceFrame};

pub(in crate::a64) const UI_APPLICATION_MAIN: &str = "_UIApplicationMain";
const RECORD_BYTES: u64 = 128;
const STRING_OFFSET: u64 = 256;
const STRING_BYTES: usize = 256;

/// Objects described by the main nib that UIKit instantiates at launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::a64) struct LaunchPlan {
    /// File's Owner `delegate` outlet target class (UIClassSwapper name).
    pub delegate_class: Option<String>,
    /// Class of the delegate's `window` outlet target, if any.
    pub window_class: Option<String>,
    pub window_visible: bool,
}

// ---------------------------------------------------------------- NIBArchive
#[derive(Clone, Debug, PartialEq)]
enum NibValue {
    Int(i64),
    Bool(bool),
    Float(f64),
    Data(Vec<u8>),
    Nil,
    Object(usize),
}
struct NibObject {
    class: String,
    values: Vec<(String, NibValue)>,
}
fn varint(bytes: &[u8], at: &mut usize) -> Result<usize, String> {
    // NIBArchive varints: 7-bit groups, little-endian, final byte has 0x80.
    let mut result = 0usize;
    for shift in (0..35).step_by(7) {
        let byte = *bytes.get(*at).ok_or("NIB varint truncated")?;
        *at += 1;
        result |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 != 0 {
            return Ok(result);
        }
    }
    Err("NIB varint too long".into())
}
fn read_n<const N: usize>(bytes: &[u8], at: &mut usize) -> Result<[u8; N], String> {
    let v = bytes.get(*at..*at + N).ok_or("NIB value truncated")?;
    *at += N;
    Ok(v.try_into().unwrap())
}
fn parse_objects(bytes: &[u8]) -> Result<Vec<NibObject>, String> {
    if bytes.len() < 50 || &bytes[..10] != b"NIBArchive" {
        return Err("not a NIBArchive".into());
    }
    let word = |i: usize| u32::from_le_bytes(bytes[18 + 4 * i..22 + 4 * i].try_into().unwrap()) as usize;
    let (nobj, oobj, nkey, okey, nval, oval, ncls, ocls) =
        (word(0), word(1), word(2), word(3), word(4), word(5), word(6), word(7));
    if nobj > 65536 || nkey > 65536 || nval > 1 << 20 || ncls > 65536 {
        return Err("NIB counts exceed limits".into());
    }
    let mut keys = Vec::with_capacity(nkey);
    let mut at = okey;
    for _ in 0..nkey {
        let len = varint(bytes, &mut at)?;
        let key = bytes.get(at..at + len).ok_or("NIB key truncated")?;
        keys.push(String::from_utf8_lossy(key).into_owned());
        at += len;
    }
    let mut classes = Vec::with_capacity(ncls);
    at = ocls;
    for _ in 0..ncls {
        let len = varint(bytes, &mut at)?;
        let extra = varint(bytes, &mut at)?;
        at += 4 * extra;
        let name = bytes.get(at..at + len).ok_or("NIB class truncated")?;
        classes.push(String::from_utf8_lossy(name).trim_end_matches('\0').to_string());
        at += len;
    }
    let mut values = Vec::with_capacity(nval);
    at = oval;
    for _ in 0..nval {
        let key = varint(bytes, &mut at)?;
        let kind = *bytes.get(at).ok_or("NIB value truncated")?;
        at += 1;
        let value = match kind {
            0 => NibValue::Int(i8::from_le_bytes(read_n(bytes, &mut at)?) as i64),
            1 => NibValue::Int(i16::from_le_bytes(read_n(bytes, &mut at)?) as i64),
            2 => NibValue::Int(i32::from_le_bytes(read_n(bytes, &mut at)?) as i64),
            3 => NibValue::Int(i64::from_le_bytes(read_n(bytes, &mut at)?)),
            4 => NibValue::Bool(false),
            5 => NibValue::Bool(true),
            6 => NibValue::Float(f32::from_le_bytes(read_n(bytes, &mut at)?) as f64),
            7 => NibValue::Float(f64::from_le_bytes(read_n(bytes, &mut at)?)),
            8 => {
                let len = varint(bytes, &mut at)?;
                let data = bytes.get(at..at + len).ok_or("NIB data truncated")?.to_vec();
                at += len;
                NibValue::Data(data)
            }
            9 => NibValue::Nil,
            10 => NibValue::Object(u32::from_le_bytes(read_n(bytes, &mut at)?) as usize),
            other => return Err(format!("NIB value type {other} unsupported")),
        };
        values.push((keys.get(key).ok_or("NIB key index invalid")?.clone(), value));
    }
    let mut objects = Vec::with_capacity(nobj);
    at = oobj;
    for _ in 0..nobj {
        let class = varint(bytes, &mut at)?;
        let start = varint(bytes, &mut at)?;
        let count = varint(bytes, &mut at)?;
        objects.push(NibObject {
            class: classes.get(class).ok_or("NIB class index invalid")?.clone(),
            values: values.get(start..start + count).ok_or("NIB value range invalid")?.to_vec(),
        });
    }
    Ok(objects)
}

/// Extract the launch objects from a compiled main nib (UINib format).
pub(in crate::a64) fn parse_nib(bytes: &[u8]) -> Result<LaunchPlan, String> {
    let objects = parse_objects(bytes)?;
    let get = |object: usize, key: &str| -> Option<&NibValue> {
        objects.get(object)?.values.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    };
    let object_ref = |object: usize, key: &str| match get(object, key) {
        Some(NibValue::Object(o)) => Some(*o),
        _ => None,
    };
    let string = |object: usize| -> Option<String> {
        match get(object, "NS.bytes") {
            Some(NibValue::Data(d)) => Some(String::from_utf8_lossy(d).into_owned()),
            _ => None,
        }
    };
    let class_of = |object: usize| -> Option<String> {
        let o = objects.get(object)?;
        if o.class == "UIClassSwapper" {
            string(object_ref(object, "UIClassName")?)
        } else {
            Some(o.class.clone())
        }
    };
    let array = |object: usize| -> Vec<usize> {
        objects.get(object).map_or(Vec::new(), |o| {
            o.values
                .iter()
                .filter_map(|(k, v)| match (k.as_str(), v) {
                    ("UINibEncoderEmptyKey", NibValue::Object(i)) => Some(*i),
                    _ => None,
                })
                .collect()
        })
    };
    let owner = objects
        .iter()
        .enumerate()
        .find(|(index, o)| {
            o.class == "UIProxyObject"
                && object_ref(*index, "UIProxiedObjectIdentifier").and_then(string).as_deref() == Some("IBFilesOwner")
        })
        .map(|(index, _)| index)
        .ok_or("NIB has no File's Owner proxy")?;
    let connections = object_ref(0, "UINibConnectionsKey").map(array).unwrap_or_default();
    let outlet = |source: usize, label: &str| -> Option<usize> {
        connections.iter().copied().find_map(|c| {
            let o = objects.get(c)?;
            (o.class == "UIRuntimeOutletConnection"
                && object_ref(c, "UISource") == Some(source)
                && object_ref(c, "UILabel").and_then(string).as_deref() == Some(label))
            .then(|| object_ref(c, "UIDestination"))
            .flatten()
        })
    };
    let delegate = outlet(owner, "delegate");
    let window = delegate.and_then(|d| outlet(d, "window"));
    let visible = object_ref(0, "UINibVisibleWindowsKey").map(array).unwrap_or_default();
    Ok(LaunchPlan {
        delegate_class: delegate.and_then(class_of),
        window_class: window.and_then(class_of),
        window_visible: window.is_some_and(|w| visible.contains(&w)),
    })
}

// -------------------------------------------------------------------- pump
/// `int UIApplicationMain(int, char **, NSString *, NSString *)`.
pub(in crate::a64) fn pump_code(sym: &Symbols<'_>) -> Asm {
    let mut a = Asm::default();
    let top = a.label();
    let done = a.label();
    a.prologue(32)
        .stp(19, 20, 31, 16)
        .ldr_literal(16, sym.entry(Internal::PumpStart as u16))
        .blr(16)
        .mov(19, 0);
    a.bind(top);
    a.mov(0, 19)
        .ldr_literal(16, sym.entry(Internal::PumpNext as u16))
        .blr(16)
        .cbz(0, done)
        .ldp(0, 1, 19, 0)
        .ldp(2, 3, 19, 16)
        .ldp(4, 5, 19, 32)
        .ldp(6, 7, 19, 48)
        .ldp_d(0, 1, 19, 64)
        .ldp_d(2, 3, 19, 80)
        .ldr(16, 19, 96)
        .blr(16)
        .stp(0, 1, 19, 104)
        .str_d(0, 19, 120)
        .b(top);
    a.bind(done);
    a.ldp(19, 20, 31, 16).epilogue(32);
    a
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    NotStarted,
    DelegateClass,
    AllocDelegate,
    WindowClass,
    AllocWindow,
    ConnectWindow,
    AskWill,
    CallWill,
    AskDid,
    CallDid,
    Activate,
    CallActive,
    WillAppear,
    DidAppear,
    Running,
    Stopped,
}

pub(in crate::a64) struct Launcher {
    plan: Option<LaunchPlan>,
    stage: Stage,
    record: u64,
    delegate: u64,
    window: u64,
    class: u64,
    responds_active: bool,
    /// Stop after this many frame callbacks (tests); `None` runs forever.
    pub frame_limit: Option<u64>,
    pub frame_callbacks: u64,
    /// Seconds per frame for the virtual media clock.
    pub frame_interval: f64,
    pub log: Vec<String>,
}

impl Launcher {
    pub(in crate::a64) fn new(plan: Option<LaunchPlan>) -> Self {
        Self {
            plan,
            stage: Stage::NotStarted,
            record: 0,
            delegate: 0,
            window: 0,
            class: 0,
            responds_active: false,
            frame_limit: None,
            frame_callbacks: 0,
            frame_interval: 1.0 / 60.0,
            log: Vec::new(),
        }
    }
    pub(in crate::a64) fn delegate(&self) -> u64 {
        self.delegate
    }
    pub(in crate::a64) fn window(&self) -> u64 {
        self.window
    }
    /// Test hook: a later `UIApplicationMain` call continues with frames
    /// (the launch sequence ran already).
    pub(in crate::a64) fn continue_frames(&mut self, limit: u64) {
        self.stage = Stage::Running;
        self.frame_limit = Some(self.frame_callbacks + limit);
    }
}

struct Work {
    entry: u64,
    integers: Vec<u64>,
}

fn read_constant_string(frame: &mut ServiceFrame<'_>, object: u64) -> Result<String, String> {
    // Only compile-time constant CFStrings ({isa, flags, ptr, len}) can be
    // read without Foundation; anything else is reported, not guessed.
    let header = frame.read(object, 32)?;
    let flags = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let pointer = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let length = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if flags != 0x7c8 || length == 0 || length > 255 {
        return Err("UIApplicationMain class-name argument is not a constant ASCII NSString".into());
    }
    String::from_utf8(frame.read(pointer, length as usize)?).map_err(|_| "class name is not UTF-8".into())
}

/// x0..x3 = argc, argv, principalClassName, delegateClassName.
pub(in crate::a64) fn pump_start(kit: &mut UiKit, frame: &mut ServiceFrame<'_>) -> Result<ReturnValues, String> {
    let (principal, delegate) = (frame.integer(2)?, frame.integer(3)?);
    let record = kit.layout()?.scratch.0;
    kit.launcher.record = record;
    if kit.launcher.stage == Stage::Running {
        return ret(record);
    }
    if kit.launcher.stage != Stage::NotStarted {
        return Err("UIApplicationMain called twice".into());
    }
    if principal != 0 {
        let name = read_constant_string(frame, principal)?;
        if name != "UIApplication" {
            return Err(format!("UIApplicationMain principal class {name} is not supported"));
        }
    }
    let delegate_name = if delegate != 0 {
        Some(read_constant_string(frame, delegate)?)
    } else {
        None
    };
    let plan = match (delegate_name, kit.launcher.plan.clone()) {
        (Some(name), plan) => LaunchPlan {
            delegate_class: Some(name),
            window_class: plan.as_ref().and_then(|p| p.window_class.clone()),
            window_visible: plan.is_some_and(|p| p.window_visible),
        },
        (None, Some(plan)) => plan,
        (None, None) => return Err("UIApplicationMain needs a main nib launch plan (NSMainNibFile)".into()),
    };
    kit.launcher.log.push(format!("launch {plan:?}"));
    kit.launcher.plan = Some(plan);
    kit.model.application.launched = true;
    kit.launcher.stage = Stage::DelegateClass;
    ret(record)
}

fn result_x0(frame: &mut ServiceFrame<'_>, record: u64) -> Result<u64, String> {
    Ok(u64::from_le_bytes(frame.read(record + 104, 8)?.try_into().unwrap()))
}

fn send(kit: &UiKit, frame: &mut ServiceFrame<'_>, receiver: u64, selector: &str, args: &[u64]) -> Result<Work, String> {
    let sel = super::sent_selector(kit, frame, selector)?;
    let mut integers = vec![receiver, sel];
    integers.extend_from_slice(args);
    Ok(Work { entry: kit.links()?.msg_send, integers })
}

/// Decide the next guest call. Returns 0 to make UIApplicationMain return.
pub(in crate::a64) fn pump_next(kit: &mut UiKit, frame: &mut ServiceFrame<'_>) -> Result<ReturnValues, String> {
    let record = frame.integer(0)?;
    if record != kit.launcher.record || record == 0 {
        return Err("UIApplicationMain pump record mismatch".into());
    }
    let previous = result_x0(frame, record)?;
    loop {
        let plan = kit.launcher.plan.clone().ok_or("UIApplicationMain not started")?;
        let links = kit.links()?;
        let app = kit.model.application.object;
        let stage = kit.launcher.stage;
        let work: Option<Work> = match stage {
            Stage::NotStarted | Stage::Stopped => return ret(0),
            Stage::DelegateClass => {
                kit.launcher.stage = Stage::AllocDelegate;
                match &plan.delegate_class {
                    Some(name) => {
                        let string = record + STRING_OFFSET;
                        if name.len() >= STRING_BYTES {
                            return Err("delegate class name too long".into());
                        }
                        let mut bytes = name.as_bytes().to_vec();
                        bytes.push(0);
                        frame.write(string, &bytes)?;
                        Some(Work { entry: links.get_class, integers: vec![string] })
                    }
                    None => {
                        kit.launcher.stage = Stage::WindowClass;
                        None
                    }
                }
            }
            Stage::AllocDelegate => {
                if previous == 0 {
                    return Err(format!("app delegate class {:?} not found", plan.delegate_class));
                }
                kit.launcher.stage = Stage::WindowClass;
                kit.launcher.class = previous;
                Some(Work { entry: links.alloc_init, integers: vec![previous] })
            }
            Stage::WindowClass => {
                if plan.delegate_class.is_some() && kit.launcher.delegate == 0 {
                    if previous == 0 {
                        return Err("app delegate allocation returned nil".into());
                    }
                    kit.launcher.delegate = previous;
                    kit.model.application.delegate = previous;
                    kit.launcher.log.push("delegate".into());
                }
                kit.launcher.stage = Stage::AllocWindow;
                match plan.window_class.as_deref() {
                    None => {
                        kit.launcher.stage = Stage::AskWill;
                        None
                    }
                    Some("UIWindow") => {
                        kit.launcher.class = kit.layout()?.class("UIWindow").ok_or("UIWindow absent")?;
                        None
                    }
                    Some(name) => {
                        let string = record + STRING_OFFSET;
                        let mut bytes = name.as_bytes().to_vec();
                        bytes.push(0);
                        if bytes.len() > STRING_BYTES {
                            return Err("window class name too long".into());
                        }
                        frame.write(string, &bytes)?;
                        kit.launcher.class = 0;
                        Some(Work { entry: links.get_class, integers: vec![string] })
                    }
                }
            }
            Stage::AllocWindow => {
                let class = if kit.launcher.class != 0 { kit.launcher.class } else { previous };
                if class == 0 {
                    return Err(format!("nib window class {:?} not found", plan.window_class));
                }
                kit.launcher.class = 0;
                kit.launcher.stage = Stage::ConnectWindow;
                // -init gives a window filling the screen, matching the nib's
                // UIResizesToFullScreen.
                Some(Work { entry: links.alloc_init, integers: vec![class] })
            }
            Stage::ConnectWindow => {
                if previous == 0 {
                    return Err("nib window allocation returned nil".into());
                }
                kit.launcher.window = previous;
                kit.launcher.stage = Stage::AskWill;
                kit.launcher.log.push("window".into());
                if kit.launcher.delegate != 0 {
                    Some(send(kit, frame, kit.launcher.delegate, "setWindow:", &[previous])?)
                } else {
                    None
                }
            }
            Stage::AskWill | Stage::AskDid => {
                let delegate = kit.launcher.delegate;
                if delegate == 0 {
                    kit.launcher.stage = Stage::Activate;
                    None
                } else {
                    let name = if stage == Stage::AskWill {
                        "application:willFinishLaunchingWithOptions:"
                    } else {
                        "application:didFinishLaunchingWithOptions:"
                    };
                    let probe = super::sent_selector(kit, frame, name)?;
                    kit.launcher.stage = if stage == Stage::AskWill { Stage::CallWill } else { Stage::CallDid };
                    Some(send(kit, frame, delegate, "respondsToSelector:", &[probe])?)
                }
            }
            Stage::CallWill | Stage::CallDid => {
                let (name, next) = if stage == Stage::CallWill {
                    ("application:willFinishLaunchingWithOptions:", Stage::AskDid)
                } else {
                    ("application:didFinishLaunchingWithOptions:", Stage::Activate)
                };
                kit.launcher.stage = next;
                if previous & 0xff != 0 {
                    kit.launcher.log.push(name.into());
                    Some(send(kit, frame, kit.launcher.delegate, name, &[app, 0])?)
                } else {
                    None
                }
            }
            Stage::Activate => {
                let window = kit.launcher.window;
                if window != 0 && plan.window_visible && kit.model.application.key_window == 0 {
                    kit.model.make_key_and_visible(window)?;
                }
                kit.model.application.state = ApplicationState::Active;
                kit.launcher.log.push("active (UIApplicationDidBecomeActiveNotification not posted: M2)".into());
                kit.launcher.stage = Stage::CallActive;
                let delegate = kit.launcher.delegate;
                if delegate != 0 {
                    let probe = super::sent_selector(kit, frame, "applicationDidBecomeActive:")?;
                    Some(send(kit, frame, delegate, "respondsToSelector:", &[probe])?)
                } else {
                    kit.launcher.responds_active = false;
                    None
                }
            }
            Stage::CallActive => {
                kit.launcher.stage = Stage::WillAppear;
                if kit.launcher.delegate != 0 && previous & 0xff != 0 {
                    kit.launcher.log.push("applicationDidBecomeActive:".into());
                    Some(send(kit, frame, kit.launcher.delegate, "applicationDidBecomeActive:", &[app])?)
                } else {
                    None
                }
            }
            Stage::WillAppear | Stage::DidAppear => {
                let key = kit.model.application.key_window;
                let root = if key != 0 { kit.model.view(key)?.root_view_controller } else { 0 };
                let (name, next) = if stage == Stage::WillAppear {
                    ("viewWillAppear:", Stage::DidAppear)
                } else {
                    ("viewDidAppear:", Stage::Running)
                };
                kit.launcher.stage = next;
                if root != 0 {
                    kit.launcher.log.push(name.into());
                    Some(send(kit, frame, root, name, &[1])?)
                } else {
                    None
                }
            }
            Stage::Running => {
                if kit.launcher.frame_limit.is_some_and(|limit| kit.launcher.frame_callbacks >= limit) {
                    kit.launcher.stage = Stage::Stopped;
                    return ret(0);
                }
                let targets = kit.mgl.frame_targets.clone();
                if targets.is_empty() {
                    // No display link: nothing to drive. With a frame limit
                    // (tests) this ends; otherwise it is an explicit stop
                    // until touch/timer sources exist.
                    kit.launcher.stage = Stage::Stopped;
                    kit.launcher.log.push("run loop idle: no frame source".into());
                    return ret(0);
                }
                let index = (kit.launcher.frame_callbacks % targets.len() as u64) as usize;
                if index == 0 {
                    kit.mgl.media_time += kit.launcher.frame_interval;
                }
                kit.launcher.frame_callbacks += 1;
                Some(send(kit, frame, targets[index], "frameStep", &[])?)
            }
        };
        if let Some(work) = work {
            write_record(frame, record, &work)?;
            return ret(1);
        }
    }
}

fn write_record(frame: &mut ServiceFrame<'_>, record: u64, work: &Work) -> Result<(), String> {
    if work.integers.len() > 8 {
        return Err("pump work exceeds x0-x7".into());
    }
    let mut bytes = vec![0u8; RECORD_BYTES as usize];
    for (i, value) in work.integers.iter().enumerate() {
        bytes[i * 8..i * 8 + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[96..104].copy_from_slice(&work.entry.to_le_bytes());
    frame.write(record, &bytes)
}


#[cfg(test)]
pub(in crate::a64) mod tests {
    use super::*;
    /// Minimal NIBArchive writer for tests (same layout parse_nib reads).
    pub(in super::super) fn write_nib(objects: &[(&str, Vec<(&str, NibTestValue)>)]) -> Vec<u8> {
        fn vint(out: &mut Vec<u8>, mut v: usize) {
            loop {
                let byte = (v & 0x7f) as u8;
                v >>= 7;
                if v == 0 {
                    out.push(byte | 0x80);
                    return;
                }
                out.push(byte);
            }
        }
        let mut keys: Vec<&str> = Vec::new();
        let mut classes: Vec<&str> = Vec::new();
        let mut obj_bytes = Vec::new();
        let mut val_bytes = Vec::new();
        let mut value_index = 0;
        for (class, values) in objects {
            let ci = classes.iter().position(|c| c == class).unwrap_or_else(|| {
                classes.push(class);
                classes.len() - 1
            });
            vint(&mut obj_bytes, ci);
            vint(&mut obj_bytes, value_index);
            vint(&mut obj_bytes, values.len());
            for (key, value) in values {
                let ki = keys.iter().position(|k| k == key).unwrap_or_else(|| {
                    keys.push(key);
                    keys.len() - 1
                });
                vint(&mut val_bytes, ki);
                match value {
                    NibTestValue::Object(o) => {
                        val_bytes.push(10);
                        val_bytes.extend_from_slice(&(*o as u32).to_le_bytes());
                    }
                    NibTestValue::Data(d) => {
                        val_bytes.push(8);
                        vint(&mut val_bytes, d.len());
                        val_bytes.extend_from_slice(d);
                    }
                    NibTestValue::True => val_bytes.push(5),
                }
                value_index += 1;
            }
        }
        let mut key_bytes = Vec::new();
        for k in &keys {
            vint(&mut key_bytes, k.len());
            key_bytes.extend_from_slice(k.as_bytes());
        }
        let mut class_bytes = Vec::new();
        for c in &classes {
            vint(&mut class_bytes, c.len() + 1);
            vint(&mut class_bytes, 0);
            class_bytes.extend_from_slice(c.as_bytes());
            class_bytes.push(0);
        }
        let mut out = b"NIBArchive".to_vec();
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&10u32.to_le_bytes());
        let header = 18 + 32;
        let oobj = header;
        let okey = oobj + obj_bytes.len();
        let oval = okey + key_bytes.len();
        let ocls = oval + val_bytes.len();
        for v in [objects.len(), oobj, keys.len(), okey, value_index, oval, classes.len(), ocls] {
            out.extend_from_slice(&(v as u32).to_le_bytes());
        }
        out.extend(obj_bytes);
        out.extend(key_bytes);
        out.extend(val_bytes);
        out.extend(class_bytes);
        out
    }
    pub(in super::super) enum NibTestValue {
        Object(usize),
        Data(Vec<u8>),
        True,
    }
    /// The object graph of Coromon's MainWindow.nib (from the nib dump).
    pub(in super::super) fn coromon_like_nib(delegate_class: &str) -> Vec<u8> {
        use NibTestValue::*;
        let s = |t: &str| Data(t.as_bytes().to_vec());
        write_nib(&[
            ("NSObject", vec![
                ("UINibTopLevelObjectsKey", Object(1)),
                ("UINibObjectsKey", Object(1)),
                ("UINibConnectionsKey", Object(11)),
                ("UINibVisibleWindowsKey", Object(16)),
            ]),
            ("NSArray", vec![("UINibEncoderEmptyKey", Object(2)), ("UINibEncoderEmptyKey", Object(4)), ("UINibEncoderEmptyKey", Object(6)), ("UINibEncoderEmptyKey", Object(9))]),
            ("UIProxyObject", vec![("UIProxiedObjectIdentifier", Object(3))]),
            ("NSString", vec![("NS.bytes", s("IBFilesOwner"))]),
            ("UIProxyObject", vec![("UIProxiedObjectIdentifier", Object(5))]),
            ("NSString", vec![("NS.bytes", s("IBFirstResponder"))]),
            ("UIClassSwapper", vec![("UIClassName", Object(7)), ("UIOriginalClassName", Object(8))]),
            ("NSString", vec![("NS.bytes", s(delegate_class))]),
            ("NSString", vec![("NS.bytes", s("UICustomObject"))]),
            ("UIWindow", vec![("UIResizesToFullScreen", True)]),
            ("NSArray", vec![]),
            ("NSArray", vec![("UINibEncoderEmptyKey", Object(12)), ("UINibEncoderEmptyKey", Object(14))]),
            ("UIRuntimeOutletConnection", vec![("UILabel", Object(13)), ("UISource", Object(2)), ("UIDestination", Object(6))]),
            ("NSString", vec![("NS.bytes", s("delegate"))]),
            ("UIRuntimeOutletConnection", vec![("UILabel", Object(15)), ("UISource", Object(6)), ("UIDestination", Object(9))]),
            ("NSString", vec![("NS.bytes", s("window"))]),
            ("NSArray", vec![("UINibEncoderEmptyKey", Object(9))]),
        ])
    }
    #[test]
    fn main_nib_yields_delegate_window_and_visibility() {
        let plan = parse_nib(&coromon_like_nib("AppDelegate")).unwrap();
        assert_eq!(
            plan,
            LaunchPlan {
                delegate_class: Some("AppDelegate".into()),
                window_class: Some("UIWindow".into()),
                window_visible: true,
            }
        );
        assert!(parse_nib(b"NIBArchive").is_err());
        let mut truncated = coromon_like_nib("AppDelegate");
        truncated.truncate(80);
        assert!(parse_nib(&truncated).is_err());
    }
    /// Real Coromon MainWindow.nib, when available locally (never in git):
    /// PLAYCOVER_COROMON_NIB=/path/to/MainWindow.nib
    #[test]
    #[ignore]
    fn actual_coromon_main_window_nib() {
        let path = std::env::var("PLAYCOVER_COROMON_NIB").expect("PLAYCOVER_COROMON_NIB");
        let plan = parse_nib(&std::fs::read(path).unwrap()).unwrap();
        echo!("PLAYCOVER_COROMON_NIB plan: {plan:?}");
        assert_eq!(plan.delegate_class.as_deref(), Some("AppDelegate"));
        assert_eq!(plan.window_class.as_deref(), Some("UIWindow"));
        assert!(plan.window_visible);
    }
}
