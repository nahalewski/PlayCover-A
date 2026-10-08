/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIDevice`.

use crate::dyld::ConstantExports;
use crate::dyld::HostConstant;
use crate::environment::Environment;
use crate::frameworks::foundation::ns_string::get_static_str;
use crate::frameworks::foundation::{ns_string, NSInteger};
use crate::msg_class;
use crate::objc::{
    id, msg, objc_classes, todo_objc_setter, ClassExports, NSZonePtr, TrivialHostObject,
};
use crate::window::{get_battery_status, BatteryState, DeviceFamily, DeviceOrientation};

/// A bounded numeric iOS version, for reporting metadata rather than promising
/// framework support. Reject suffixes and normalize leading zeroes.
pub(crate) fn canonical_ios_version(value: &str) -> Option<String> {
    if value.len() > 11 {
        return None;
    }
    let parts: Vec<_> = value.split('.').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut numbers = Vec::new();
    for part in parts {
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let number: u16 = part.parse().ok()?;
        if number > 255 {
            return None;
        }
        numbers.push(number);
    }
    if numbers[0] == 0 {
        return None;
    }
    Some(
        numbers
            .into_iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("."),
    )
}

#[cfg(test)]
mod version_tests {
    use super::canonical_ios_version;

    #[test]
    fn canonicalizes_bounded_metadata_versions() {
        assert_eq!(canonical_ios_version("3.2").as_deref(), Some("3.2"));
        assert_eq!(
            canonical_ios_version("013.00.007").as_deref(),
            Some("13.0.7")
        );
        assert_eq!(
            canonical_ios_version("255.255.255").as_deref(),
            Some("255.255.255")
        );
    }

    #[test]
    fn rejects_malformed_or_unbounded_versions() {
        for version in [
            "", "13", "13.0.0.1", "0.1", "256.0", "1.256", "1.0.256", "0001.2", "13..1",
            "13.0beta", "13.-1", " 13.0", "13.0\n", "１３.0",
        ] {
            assert_eq!(canonical_ios_version(version), None, "{version:?}");
        }
    }
}

pub const UIDeviceOrientationDidChangeNotification: &str =
    "UIDeviceOrientationDidChangeNotification";

pub type UIDeviceOrientation = NSInteger;
#[allow(dead_code)]
pub const UIDeviceOrientationUnknown: UIDeviceOrientation = 0;
pub const UIDeviceOrientationPortrait: UIDeviceOrientation = 1;
pub const UIDeviceOrientationPortraitUpsideDown: UIDeviceOrientation = 2;
pub const UIDeviceOrientationLandscapeLeft: UIDeviceOrientation = 3;
pub const UIDeviceOrientationLandscapeRight: UIDeviceOrientation = 4;
#[allow(dead_code)]
pub const UIDeviceOrientationFaceUp: UIDeviceOrientation = 5;
#[allow(dead_code)]
pub const UIDeviceOrientationFaceDown: UIDeviceOrientation = 6;

pub type UIDeviceBatteryState = NSInteger;
pub const UIDeviceBatteryStateUnknown: UIDeviceBatteryState = 0;
pub const UIDeviceBatteryStateUnplugged: UIDeviceBatteryState = 1;
pub const UIDeviceBatteryStateCharging: UIDeviceBatteryState = 2;
pub const UIDeviceBatteryStateFull: UIDeviceBatteryState = 3;

type UIUserInterfaceIdiom = NSInteger;
#[allow(dead_code)]
const UIUserInterfaceIdiomUnspecified: UIUserInterfaceIdiom = -1;
const UIUserInterfaceIdiomPhone: UIUserInterfaceIdiom = 0;
const UIUserInterfaceIdiomPad: UIUserInterfaceIdiom = 1;

#[derive(Default)]
pub struct State {
    current_device: Option<id>,
    is_generating_device_orientation_notifications: bool,
}
impl State {
    pub fn is_generating_device_orientation_notifications(&self) -> bool {
        self.is_generating_device_orientation_notifications
    }
}

pub const CONSTANTS: ConstantExports = &[(
    "_UIDeviceOrientationDidChangeNotification",
    HostConstant::NSString(UIDeviceOrientationDidChangeNotification),
)];

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIDevice: NSObject

+ (id)currentDevice {
    if let Some(device) = env.framework_state.uikit.ui_device.current_device {
        device
    } else {
        let new = msg_class![env; _touchHLE_UIDevice_Static alloc];
        env.framework_state.uikit.ui_device.current_device = Some(new);
        new
    }
}

- (())beginGeneratingDeviceOrientationNotifications {
    log_dbg!("[UIDevice beginGeneratingDeviceOrientationNotifications]");
    env.framework_state.uikit.ui_device.is_generating_device_orientation_notifications = true;
}
- (())endGeneratingDeviceOrientationNotifications {
    log_dbg!("[UIDevice endGeneratingDeviceOrientationNotifications]");
    env.framework_state.uikit.ui_device.is_generating_device_orientation_notifications = false;
}
- (bool)isGeneratingDeviceOrientationNotifications {
    let res = env.framework_state.uikit.ui_device.is_generating_device_orientation_notifications;
    log_dbg!("[UIDevice isGeneratingDeviceOrientationNotifications] -> {}", res);
    res
}

- (id)model {
    // TODO: Hardcoded to iPhone for now
    ns_string::get_static_str(env, "iPhone")
}
- (id)localizedModel {
    // TODO: localization
    msg![env; this model]
}

- (id)name {
    // TODO: Hardcoded to iPhone for now
    ns_string::get_static_str(env, "iPhone")
}

- (id)systemName {
    ns_string::get_static_str(env, "iPhone OS")
}

// NSString
- (id)systemVersion {
    let version = env.options.reported_ios_version.clone()
        .or_else(|| env.bundle.minimum_os_version().and_then(canonical_ios_version))
        .unwrap_or_else(|| "2.0".into());
    let string = ns_string::from_rust_string(env, version);
    crate::objc::autorelease(env, string)
}

- (id)uniqueIdentifier {
    // Aspen Simulator returns (null) here
    // A device unique identifier must be 40 characters long
    ns_string::get_static_str(env, "touchHLEdevice..........................")
}

- (bool)isMultitaskingSupported {
    false
}

- (UIDeviceOrientation)orientation {
    match env.window().current_rotation() {
        DeviceOrientation::Portrait => UIDeviceOrientationPortrait,
        DeviceOrientation::PortraitUpsideDown => UIDeviceOrientationPortraitUpsideDown,
        DeviceOrientation::LandscapeLeft => UIDeviceOrientationLandscapeLeft,
        DeviceOrientation::LandscapeRight => UIDeviceOrientationLandscapeRight
    }
}
- (())setOrientation:(UIDeviceOrientation)orientation {
    let prev_orientation = env.window().current_rotation();
    env.on_parent_stack_in_coroutine(|window, _| {window.rotate_device(match orientation {
        UIDeviceOrientationPortrait => DeviceOrientation::Portrait,
        UIDeviceOrientationPortraitUpsideDown => DeviceOrientation::PortraitUpsideDown,
        UIDeviceOrientationLandscapeLeft => DeviceOrientation::LandscapeLeft,
        UIDeviceOrientationLandscapeRight => DeviceOrientation::LandscapeRight,
        _ => unimplemented!("Orientation {} not handled yet", orientation),
    })});
    if prev_orientation != env.window().current_rotation() {
        generate_device_orientation_notification(env);
    }
}

- (bool)isBatteryMonitoringEnabled {
    true
}
- (())setBatteryMonitoringEnabled:(bool)enabled {
    todo_objc_setter!(this, enabled);
    assert!(enabled);
}
- (f32)batteryLevel {
    let pct = get_battery_status().0;
    if pct < 0 {
        log_dbg!("batteryLevel percentage could not be determined, returning 100% for compatibility");
        return 1.0
    }
    pct as f32 / 100.0 // narrow down to 0.0 - 1.0
}
- (UIDeviceBatteryState)batteryState {
    match get_battery_status().1 {
        BatteryState::Unknown => UIDeviceBatteryStateUnknown,
        BatteryState::OnBattery => UIDeviceBatteryStateUnplugged,
        BatteryState::NoBattery | BatteryState::Charging => UIDeviceBatteryStateCharging,
        BatteryState::Full => UIDeviceBatteryStateFull,
    }
}

- (UIUserInterfaceIdiom)userInterfaceIdiom {
    match env.window().device_family() {
        DeviceFamily::iPhone => UIUserInterfaceIdiomPhone,
        DeviceFamily::iPad => UIUserInterfaceIdiomPad,
    }
}

@end

// Private static implementation of UIDevice, used for the current device
@implementation _touchHLE_UIDevice_Static: UIDevice

+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_static_object(
        this,
        Box::new(TrivialHostObject),
        &mut env.mem
    )
}

- (id) retain { this }
- (()) release {}
- (id) autorelease { this }

@end

};

pub fn generate_device_orientation_notification(env: &mut Environment) {
    let center: id = msg_class![env; NSNotificationCenter defaultCenter];
    let name = get_static_str(env, UIDeviceOrientationDidChangeNotification);
    let device: id = msg_class![env; UIDevice currentDevice];
    let _: () = msg![env; center postNotificationName:name object:device];
}
