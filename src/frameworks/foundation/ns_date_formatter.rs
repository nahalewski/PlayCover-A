/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSDateFormatter`.
//!
//! Resources:
//! - Apple's [Introduction to Data Formatting Programming Guide For Cocoa](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/DataFormatting/DataFormatting.html)
//! - [Unicode Technical Standard #35](https://unicode.org/reports/tr35/tr35-10.html#Date_Format_Patterns)

use crate::frameworks::core_foundation::time::CFAbsoluteTimeGetGregorianDate;
use crate::frameworks::foundation::{ns_string, NSTimeInterval};
use crate::objc::{
    autorelease, id, msg, nil, objc_classes, todo_objc_setter, ClassExports, HostObject, NSZonePtr,
};

struct NSDateFormatterHostObject {
    date_format: Option<id>,
}
impl HostObject for NSDateFormatterHostObject {}

const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September",
    "October", "November", "December",
];
const WEEKDAY_NAMES: [&str; 7] = [
    "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
];

/// Day of the week (0 = Sunday) of a Gregorian date.
fn weekday(year: i32, month: u32, day: u32) -> usize {
    // Sakamoto's algorithm
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    ((y + y / 4 - y / 100 + y / 400 + T[(month as usize).clamp(1, 12) - 1] + day as i32)
        .rem_euclid(7)) as usize
}

/// Formats a date with the common Unicode date format patterns (the time
/// zone is always GMT). Quoted text is copied; unknown letters are kept.
fn format_date(format: &str, year: i32, month: u32, day: u32, hour: u32, minute: u32, second: f64) -> String {
    let chars: Vec<char> = format.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // quoted literal ('' is a single quote)
            i += 1;
            if i < chars.len() && chars[i] == '\'' {
                out.push('\'');
                i += 1;
                continue;
            }
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += 1;
            continue;
        }
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        let month_name = MONTH_NAMES[(month as usize).clamp(1, 12) - 1];
        let weekday_name = WEEKDAY_NAMES[weekday(year, month, day)];
        let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
        let whole_seconds = second.floor() as u32;
        match c {
            'y' | 'Y' => {
                if n == 2 {
                    out.push_str(&format!("{:02}", year.rem_euclid(100)));
                } else {
                    out.push_str(&format!("{:0width$}", year, width = n.max(1)));
                }
            }
            'M' | 'L' => match n {
                1 => out.push_str(&month.to_string()),
                2 => out.push_str(&format!("{month:02}")),
                3 => out.push_str(&month_name[..3]),
                _ => out.push_str(month_name),
            },
            'd' => out.push_str(&format!("{:0width$}", day, width = n)),
            'E' => {
                if n >= 4 {
                    out.push_str(weekday_name);
                } else {
                    out.push_str(&weekday_name[..3]);
                }
            }
            'H' => out.push_str(&format!("{:0width$}", hour, width = n)),
            'h' => out.push_str(&format!("{:0width$}", hour12, width = n)),
            'm' => out.push_str(&format!("{:0width$}", minute, width = n)),
            's' => out.push_str(&format!("{:0width$}", whole_seconds, width = n)),
            'S' => {
                let frac = format!("{:03}", ((second - second.floor()) * 1000.0) as u32);
                out.push_str(&frac[..n.min(3)]);
            }
            'a' => out.push_str(if hour < 12 { "AM" } else { "PM" }),
            'z' => out.push_str("GMT"),
            'Z' => out.push_str("+0000"),
            other => {
                log!("NSDateFormatter: unsupported pattern letter {:?}, copied as is", other);
                for _ in 0..n {
                    out.push(other);
                }
            }
        }
        i += n;
    }
    out
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSDateFormatter: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(NSDateFormatterHostObject {
        date_format: None,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (())setDateFormat:(id)format { // NSString *
    let date_format: id = msg![env; format copy];
    env.objc.borrow_mut::<NSDateFormatterHostObject>(this).date_format = Some(date_format);
}

- (())setTimeZone:(id)time_zone {
    todo_objc_setter!(this, time_zone);
}

- (id)stringFromDate:(id)date {
    let &NSDateFormatterHostObject {
        date_format
    } = env.objc.borrow(this);
    // Apps that only set dateStyle/timeStyle (not implemented) never set a
    // format: fall back to a plain numeric date and time.
    let mut format = match date_format {
        Some(date_format) => ns_string::to_rust_string(env, date_format).to_string().clone(),
        None => "yyyy-MM-dd HH:mm:ss".to_string(),
    };
    log_dbg!("date_format before: {:?}", format);

    let ti: NSTimeInterval = msg![env; date timeIntervalSinceReferenceDate];
    let greg_date = CFAbsoluteTimeGetGregorianDate(env, ti, nil);
    let year = greg_date.year;
    let month = greg_date.month;
    let day = greg_date.day;
    let hour = greg_date.hours;
    let minute = greg_date.minutes;
    let second = greg_date.seconds;

    format = format_date(&format, year, month as u32, day as u32, hour as u32, minute as u32, second);
    log_dbg!("date_format after: {:?}", format);

    let res = ns_string::from_rust_string(env, format);
    autorelease(env, res)
}

@end

};
