/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Settings changed while an app is running.
//!
//! The Android in-game menu writes `settings_cmd.txt` in the user data folder:
//! a sequence number, then `key=value` lines. Each command is applied once per
//! new sequence number. Keys:
//! - `view=default|stretch|16:9|blur` (see `--widescreen`)
//! - `ads=block|allow` (see `--allow-ads`)
//! - `network=on|off` (see `--no-network-access`)

use crate::Environment;
use std::sync::atomic::{AtomicU64, Ordering};

static LAST_SEQ: AtomicU64 = AtomicU64::new(0);
static FIRST_POLL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn poll(env: &mut Environment) {
    let path = crate::paths::user_data_base_path().join("settings_cmd.txt");
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let mut lines = text.lines();
    let Some(seq) = lines.next().and_then(|l| l.trim().parse::<u64>().ok()) else {
        return;
    };
    // A command left over from an earlier run is not applied: the app's own
    // options (which the menu keeps in sync) already say how it should start.
    let first = FIRST_POLL.swap(false, Ordering::Relaxed);
    if LAST_SEQ.swap(seq, Ordering::Relaxed) == seq || first {
        return;
    }
    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "view" => {
                let (widescreen, blur) = match value {
                    "stretch" => (Some(0.0), false),
                    "16:9" => (Some(16.0 / 9.0), false),
                    "blur" => (None, true),
                    _ => (None, false),
                };
                if env.window.is_some() {
                    env.window_mut().set_view_mode(widescreen, blur);
                }
                log!("Settings: view = {}", value);
            }
            "ads" => {
                crate::ad_blocklist::set_allow_ads(value == "allow");
                log!("Settings: ads = {}", value);
            }
            "network" => {
                env.options.network_access = value != "off";
                log!("Settings: network = {}", value);
            }
            _ => {}
        }
    }
}
