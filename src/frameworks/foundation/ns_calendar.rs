/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSCalendar` and `NSDateComponents`.
//!
//! A minimal implementation: only the Gregorian calendar, always in UTC (time
//! zone and locale settings are accepted and ignored), and only the date and
//! time fields that fit in a plain year/month/day/hour/minute/second/weekday
//! breakdown.

use super::{NSInteger, NSTimeInterval, NSUInteger};
use crate::objc::{
    autorelease, id, msg, msg_class, nil, objc_classes, ClassExports, HostObject, NSZonePtr,
};

/// `NSDateComponentUndefined`
const NSDateComponentUndefined: NSInteger = i32::MAX;

// `NSCalendarUnit` flags (iOS 4-7 values).
const NSYearCalendarUnit: NSUInteger = 1 << 2;
const NSMonthCalendarUnit: NSUInteger = 1 << 3;
const NSDayCalendarUnit: NSUInteger = 1 << 4;
const NSHourCalendarUnit: NSUInteger = 1 << 5;
const NSMinuteCalendarUnit: NSUInteger = 1 << 6;
const NSSecondCalendarUnit: NSUInteger = 1 << 7;
const NSWeekdayCalendarUnit: NSUInteger = 1 << 9;

/// Seconds from the Unix epoch (1970-01-01) to the Cocoa reference date
/// (2001-01-01), both at 00:00:00 UTC.
const UNIX_TO_REFERENCE_SECONDS: i64 = 978_307_200;

/// Days since 1970-01-01 for a proleptic Gregorian date.
/// Algorithm from <https://howardhinnant.github.io/date_algorithms.html>.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [days_from_civil]: returns (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

struct NSDateComponentsHostObject {
    year: NSInteger,
    month: NSInteger,
    day: NSInteger,
    hour: NSInteger,
    minute: NSInteger,
    second: NSInteger,
    weekday: NSInteger,
}
impl HostObject for NSDateComponentsHostObject {}
impl Default for NSDateComponentsHostObject {
    fn default() -> Self {
        NSDateComponentsHostObject {
            year: NSDateComponentUndefined,
            month: NSDateComponentUndefined,
            day: NSDateComponentUndefined,
            hour: NSDateComponentUndefined,
            minute: NSDateComponentUndefined,
            second: NSDateComponentUndefined,
            weekday: NSDateComponentUndefined,
        }
    }
}

struct NSCalendarHostObject;
impl HostObject for NSCalendarHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSDateComponents: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<NSDateComponentsHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (NSInteger)year { env.objc.borrow::<NSDateComponentsHostObject>(this).year }
- (NSInteger)month { env.objc.borrow::<NSDateComponentsHostObject>(this).month }
- (NSInteger)day { env.objc.borrow::<NSDateComponentsHostObject>(this).day }
- (NSInteger)hour { env.objc.borrow::<NSDateComponentsHostObject>(this).hour }
- (NSInteger)minute { env.objc.borrow::<NSDateComponentsHostObject>(this).minute }
- (NSInteger)second { env.objc.borrow::<NSDateComponentsHostObject>(this).second }
- (NSInteger)weekday { env.objc.borrow::<NSDateComponentsHostObject>(this).weekday }

- (())setYear:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).year = v; }
- (())setMonth:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).month = v; }
- (())setDay:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).day = v; }
- (())setHour:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).hour = v; }
- (())setMinute:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).minute = v; }
- (())setSecond:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).second = v; }
- (())setWeekday:(NSInteger)v { env.objc.borrow_mut::<NSDateComponentsHostObject>(this).weekday = v; }

@end

@implementation NSCalendar: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::new(NSCalendarHostObject), &mut env.mem)
}

+ (id)currentCalendar {
    let calendar: id = msg![env; this new];
    autorelease(env, calendar)
}
+ (id)autoupdatingCurrentCalendar {
    msg![env; this currentCalendar]
}

- (id)initWithCalendarIdentifier:(id)_identifier { // NSString *
    // Only the Gregorian calendar exists here, whatever was asked for.
    this
}

// Accepted and ignored: everything is UTC with no locale-specific behaviour.
- (())setTimeZone:(id)_tz {}
- (())setLocale:(id)_locale {}
- (())setFirstWeekday:(NSUInteger)_weekday {}

- (id)dateFromComponents:(id)components { // NSDateComponents *
    let c = env.objc.borrow::<NSDateComponentsHostObject>(components);
    // Fields left undefined default to the start of the epoch-ish minimum
    // (January 1st, midnight), like Cocoa effectively does.
    let get = |v: NSInteger, default: i64| if v == NSDateComponentUndefined { default } else { v as i64 };
    let (year, month, day) = (get(c.year, 1970), get(c.month, 1), get(c.day, 1));
    let (hour, minute, second) = (get(c.hour, 0), get(c.minute, 0), get(c.second, 0));
    // Fold out-of-range months into the year, as Cocoa does.
    let month0 = month - 1;
    let (year, month) = (year + month0.div_euclid(12), month0.rem_euclid(12) + 1);
    let days = days_from_civil(year, month, 1) + (day - 1);
    let unix = days * 86_400 + hour * 3600 + minute * 60 + second;
    let reference = (unix - UNIX_TO_REFERENCE_SECONDS) as NSTimeInterval;
    let date: id = msg_class![env; NSDate dateWithTimeIntervalSinceReferenceDate:reference];
    date
}

- (id)components:(NSUInteger)unit_flags fromDate:(id)date { // NSDate *
    if date == nil {
        return nil;
    }
    let reference: NSTimeInterval = msg![env; date timeIntervalSinceReferenceDate];
    let unix = reference.floor() as i64 + UNIX_TO_REFERENCE_SECONDS;
    let days = unix.div_euclid(86_400);
    let secs_of_day = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday; Cocoa's weekday is 1 = Sunday … 7 = Saturday.
    let weekday = (days + 4).rem_euclid(7) + 1;

    let components: id = msg_class![env; NSDateComponents new];
    {
        let c = env.objc.borrow_mut::<NSDateComponentsHostObject>(components);
        if unit_flags & NSYearCalendarUnit != 0 { c.year = year as NSInteger; }
        if unit_flags & NSMonthCalendarUnit != 0 { c.month = month as NSInteger; }
        if unit_flags & NSDayCalendarUnit != 0 { c.day = day as NSInteger; }
        if unit_flags & NSHourCalendarUnit != 0 { c.hour = (secs_of_day / 3600) as NSInteger; }
        if unit_flags & NSMinuteCalendarUnit != 0 { c.minute = (secs_of_day % 3600 / 60) as NSInteger; }
        if unit_flags & NSSecondCalendarUnit != 0 { c.second = (secs_of_day % 60) as NSInteger; }
        if unit_flags & NSWeekdayCalendarUnit != 0 { c.weekday = weekday as NSInteger; }
    }
    autorelease(env, components)
}

@end

};
