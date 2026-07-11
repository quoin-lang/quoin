//! The civil (calendar) types: `Date` — a calendar date with no time and no zone — and
//! `Time` — a wall-clock time of day with no date and no zone. Both are jiff civil types.
//! They meet the zone-aware world through `DateTime`: `date.atTime:zone:` / `date.inZone:`
//! build one, `DateTime#date` / `#time` extract them back.

use crate::arg;
use crate::error::QuoinError;
use crate::ext_sdk::{Host, HostExt};
use crate::runtime::date_time::{make_date_time, weekday_name};
use crate::runtime::duration::{duration_of, make_duration};
use crate::runtime::pretty::{PpChild, PpShape, PrettyPrint};
use crate::runtime::span::{make_span, span_of};
use crate::runtime::time_zone::tz_of;
use crate::value::{AnyCollect, NativeClassBuilder, Value};

use gc_arena::collect::Trace;
use jiff::civil::{Date, DateDifference, Time, TimeDifference};
use jiff::{SignedDuration, Timestamp, Unit, Zoned};
use std::any::Any;

/// Native backing state for a `Date`: a civil calendar date (jiff `civil::Date` — year,
/// month, day; no time, no zone). Plain `Copy` data — `trace_gc` is empty.
#[derive(Debug)]
pub struct NativeDate(pub Date);

impl AnyCollect for NativeDate {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

impl PrettyPrint for NativeDate {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        let d = self.0;
        PpShape::Record {
            name: "Date",
            fields: vec![
                (
                    "year".to_string(),
                    PpChild::Val(Value::Int(d.year() as i64)),
                ),
                (
                    "month".to_string(),
                    PpChild::Val(Value::Int(d.month() as i64)),
                ),
                ("day".to_string(), PpChild::Val(Value::Int(d.day() as i64))),
            ],
        }
    }
}

/// Native backing state for a `Time`: a wall-clock time of day (jiff `civil::Time`).
/// Plain `Copy` data — `trace_gc` is empty.
#[derive(Debug)]
pub struct NativeTime(pub Time);

impl AnyCollect for NativeTime {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

impl PrettyPrint for NativeTime {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        let t = self.0;
        PpShape::Record {
            name: "Time",
            fields: vec![
                (
                    "hour".to_string(),
                    PpChild::Val(Value::Int(t.hour() as i64)),
                ),
                (
                    "minute".to_string(),
                    PpChild::Val(Value::Int(t.minute() as i64)),
                ),
                (
                    "second".to_string(),
                    PpChild::Val(Value::Int(t.second() as i64)),
                ),
                (
                    "nanosecond".to_string(),
                    PpChild::Val(Value::Int(t.subsec_nanosecond() as i64)),
                ),
            ],
        }
    }
}

pub fn date_of(v: Value, who: &str) -> Result<Date, QuoinError> {
    v.with_native_state::<NativeDate, _, _>(|d| d.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "Date".to_string(),
            got: "a non-Date value".to_string(),
            msg: format!("{who} requires a Date operand"),
        })
}

pub fn make_date<'gc>(host: &dyn Host<'gc>, d: Date) -> Value<'gc> {
    let class = host.get_or_create_builtin_class("Date");
    host.new_native_state(class, NativeDate(d))
}

pub fn time_of(v: Value, who: &str) -> Result<Time, QuoinError> {
    v.with_native_state::<NativeTime, _, _>(|t| t.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "Time".to_string(),
            got: "a non-Time value".to_string(),
            msg: format!("{who} requires a Time operand"),
        })
}

pub fn make_time<'gc>(host: &dyn Host<'gc>, t: Time) -> Value<'gc> {
    let class = host.get_or_create_builtin_class("Time");
    host.new_native_state(class, NativeTime(t))
}

/// An i64 narrowed to a calendar component's own integer width, mapping overflow to the
/// same ValueError an out-of-range component raises.
fn component<T: TryFrom<i64>>(n: i64, what: &str) -> Result<T, QuoinError> {
    T::try_from(n).map_err(|_| QuoinError::ValueError(format!("{what} {n} out of range")))
}

pub fn build_date_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("Date", Some("Object"))
        .construct_with("use Date.year:month:day: / Date.parse: / Date.today")
        .class_doc(
            "A civil calendar date — year, month, day; no time of day, no time zone. The \
             type for birthdays, deadlines, and schedules, where \"March 3rd\" means March \
             3rd regardless of zone.\n\n\
             Shift it by calendar units with `+:` / `-:` (Span) — end-of-month clamped — \
             and diff two dates with `until:`, which answers a calendar Span (`1y 2mo 3d`), \
             not a second count. It meets the zone-aware world through `DateTime`: \
             `atTime:zone:` / `inZone:` build one, `DateTime#date` extracts.\n\n\
             ```\n\
             var d = Date.year:2024 month:1 day:31\n\
             (d + (Span.months:1)).s     \"* -> 2024-02-29\n\
             d.weekday                   \"* -> 'Wednesday'\n\
             ```",
        )
        .sdk_typed_class_method(
            "year:month:day:",
            &["Integer", "Integer", "Integer"],
            |host, _r, args| {
                let d = Date::new(
                    component(arg!(args, Int, 0), "year")?,
                    component(arg!(args, Int, 1), "month")?,
                    component(arg!(args, Int, 2), "day")?,
                )
                .map_err(|e| QuoinError::ValueError(format!("Date year:month:day: {e}")))?;
                Ok(make_date(host, d))
            },
        )
        .doc(
            "The calendar date with the given components. A date that does not exist (month \
             13, February 30th) throws a ValueError rather than normalizing.\n\n\
             ```\n\
             (Date.year:2026 month:7 day:11).s     \"* -> 2026-07-11\n\
             ```",
        )
        .sdk_typed_class_method("parse:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            s.parse::<Date>()
                .map(|d| make_date(host, d))
                .map_err(|e| QuoinError::ValueError(format!("Date parse: {e}")))
        })
        .doc(
            "The date an ISO 8601 string (`'2026-07-11'`) denotes; not parseable → \
             ValueError.\n\n\
             ```\n\
             (Date.parse:'2026-07-11').day     \"* -> 11\n\
             ```",
        )
        .sdk_class_method("today", |host, _r, _a| {
            Ok(make_date(host, Zoned::now().date()))
        })
        .doc(
            "Today's date in the host's local time zone (\"today\" only means something in \
             a zone — for another zone use `todayIn:`).",
        )
        .sdk_typed_class_method("todayIn:", &["TimeZone"], |host, _r, args| {
            let z = Timestamp::now().to_zoned(tz_of(args[0], "todayIn:")?);
            Ok(make_date(host, z.date()))
        })
        .doc("Today's date in the given zone: `Date.todayIn:(TimeZone.of:'Asia/Tokyo')`.");
    // Components.
    let b = b
        .sdk_instance_method("year", |host, r, _a| {
            Ok(host.new_int(date_of(r, "year")?.year() as i64))
        })
        .doc("The year component.")
        .sdk_instance_method("month", |host, r, _a| {
            Ok(host.new_int(date_of(r, "month")?.month() as i64))
        })
        .doc("The month component (1–12).")
        .sdk_instance_method("day", |host, r, _a| {
            Ok(host.new_int(date_of(r, "day")?.day() as i64))
        })
        .doc("The day-of-month component (1-based).")
        .sdk_instance_method("weekday", |host, r, _a| {
            Ok(host.new_string(weekday_name(date_of(r, "weekday")?.weekday()).to_string()))
        })
        .doc(
            "The day of the week, as its English name.\n\n\
             ```\n\
             (Date.parse:'2026-07-11').weekday     \"* -> 'Saturday'\n\
             ```",
        )
        .sdk_instance_method("dayOfYear", |host, r, _a| {
            Ok(host.new_int(date_of(r, "dayOfYear")?.day_of_year() as i64))
        })
        .doc("The 1-based ordinal day within the year (1–366).")
        .sdk_instance_method("daysInMonth", |host, r, _a| {
            Ok(host.new_int(date_of(r, "daysInMonth")?.days_in_month() as i64))
        })
        .doc(
            "How many days this date's month has.\n\n\
             ```\n\
             (Date.parse:'2024-02-10').daysInMonth     \"* -> 29\n\
             ```",
        )
        .sdk_instance_method("leapYear?", |host, r, _a| {
            Ok(host.new_bool(date_of(r, "leapYear?")?.in_leap_year()))
        })
        .doc("Whether this date's year is a leap year.");
    // Calendar arithmetic, diffs, comparison.
    let b = b
        .sdk_typed_instance_method("+:", &["Span"], |host, receiver, args| {
            date_of(receiver, "+:")?
                .checked_add(span_of(args[0], "+:")?)
                .map(|d| make_date(host, d))
                .map_err(|e| QuoinError::ArithmeticError(format!("Date +: {e}")))
        })
        .doc(
            "The date a calendar Span later — month arithmetic clamps to the end of a \
             shorter month. Leaving jiff's supported range (years ±9999) throws an \
             ArithmeticError.\n\n\
             ```\n\
             ((Date.year:2024 month:1 day:31) + (Span.months:1)).s     \"* -> 2024-02-29\n\
             ```",
        )
        .sdk_typed_instance_method("-:", &["Span"], |host, receiver, args| {
            date_of(receiver, "-:")?
                .checked_sub(span_of(args[0], "-:")?)
                .map(|d| make_date(host, d))
                .map_err(|e| QuoinError::ArithmeticError(format!("Date -: {e}")))
        })
        .doc("The date a calendar Span earlier — `+:` in reverse.")
        .sdk_typed_instance_method("until:", &["Date"], |host, receiver, args| {
            let a = date_of(receiver, "until:")?;
            let b = date_of(args[0], "until:")?;
            a.until(DateDifference::new(b).largest(Unit::Year))
                .map(|s| make_span(host, s))
                .map_err(|e| QuoinError::ArithmeticError(format!("Date until: {e}")))
        })
        .doc(
            "The calendar Span from the receiver to the argument (negative when the \
             argument is earlier), in years/months/days — the diff `Duration` can't \
             express.\n\n\
             ```\n\
             ((Date.parse:'2024-01-15').until:(Date.parse:'2026-03-18')).s     \"* -> 2y 2mo 3d\n\
             ```",
        )
        .sdk_typed_instance_method("<:", &["Date"], |host, receiver, args| {
            Ok(host.new_bool(date_of(receiver, "<:")? < date_of(args[0], "<:")?))
        })
        .doc(
            "Whether the receiver is the earlier date. Only `<:` is native; `>:` / `<=:` / \
             `>=:` derive from it on Object.",
        )
        .sdk_instance_method("==:", |host, receiver, args| {
            let a = date_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeDate, _, _>(|d| d.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(host.new_bool(eq))
        })
        .doc("Whether the argument is the same calendar date. A non-Date is simply unequal.");
    // Bridges to the zone-aware world.
    b.sdk_typed_instance_method(
        "atTime:zone:",
        &["Time", "TimeZone"],
        |host, receiver, args| {
            let d = date_of(receiver, "atTime:zone:")?;
            let t = time_of(args[0], "atTime:zone:")?;
            let tz = tz_of(args[1], "atTime:zone:")?;
            d.to_datetime(t)
                .to_zoned(tz)
                .map(|z| make_date_time(host, z))
                .map_err(|e| QuoinError::ValueError(format!("Date atTime:zone: {e}")))
        },
    )
    .doc(
        "The zone-aware DateTime at this date and wall-clock time in a zone. A time that \
         does not exist in that zone (a DST gap) resolves per jiff's compatible rule \
         (shifted by the gap) rather than erroring.\n\n\
         ```\n\
         ((Date.parse:'2026-07-11').atTime:(Time.hour:9 minute:40) zone:(TimeZone.of:'UTC')).s\n\
         \"* -> 2026-07-11T09:40:00+00:00[UTC]\n\
         ```",
    )
    .sdk_typed_instance_method("inZone:", &["TimeZone"], |host, receiver, args| {
        let d = date_of(receiver, "inZone:")?;
        d.to_zoned(tz_of(args[0], "inZone:")?)
            .map(|z| make_date_time(host, z))
            .map_err(|e| QuoinError::ValueError(format!("Date inZone: {e}")))
    })
    .doc(
        "The DateTime at this date's first instant (usually midnight — later where DST \
         skips it) in a zone.",
    )
    .sdk_instance_method("s", |host, receiver, _args| {
        Ok(host.new_string(date_of(receiver, "s")?.to_string()))
    })
    .doc(
        "The ISO 8601 date string.\n\n\
         ```\n\
         (Date.year:2026 month:7 day:11).s     \"* -> 2026-07-11\n\
         ```",
    )
}

pub fn build_time_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("Time", Some("Object"))
        .construct_with("use Time.hour:minute: / Time.parse: / Time.midnight")
        .class_doc(
            "A wall-clock time of day — no date, no zone: the type for \"the shop opens at \
             9:40\". Arithmetic with a `Duration` WRAPS around midnight (a zoneless clock \
             has nowhere else to go); `until:` answers the signed Duration between two \
             clock readings within one day. It meets dates through `Date#atTime:zone:` and \
             `DateTime#time`.\n\n\
             ```\n\
             ((Time.hour:23 minute:30) + (Duration.hours:1)).s     \"* -> 00:30:00\n\
             ```",
        )
        .sdk_typed_class_method("hour:minute:", &["Integer", "Integer"], |host, _r, args| {
            let t = Time::new(
                component(arg!(args, Int, 0), "hour")?,
                component(arg!(args, Int, 1), "minute")?,
                0,
                0,
            )
            .map_err(|e| QuoinError::ValueError(format!("Time hour:minute: {e}")))?;
            Ok(make_time(host, t))
        })
        .doc(
            "The wall-clock time at the given hour (0–23) and minute (0–59), at zero \
             seconds. Out-of-range components throw a ValueError.\n\n\
             ```\n\
             (Time.hour:9 minute:40).s     \"* -> 09:40:00\n\
             ```",
        )
        .sdk_typed_class_method(
            "hour:minute:second:",
            &["Integer", "Integer", "Integer"],
            |host, _r, args| {
                let t = Time::new(
                    component(arg!(args, Int, 0), "hour")?,
                    component(arg!(args, Int, 1), "minute")?,
                    component(arg!(args, Int, 2), "second")?,
                    0,
                )
                .map_err(|e| QuoinError::ValueError(format!("Time hour:minute:second: {e}")))?;
                Ok(make_time(host, t))
            },
        )
        .doc("The wall-clock time at the given hour, minute, and second (0–59).")
        .sdk_typed_class_method("parse:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            s.parse::<Time>()
                .map(|t| make_time(host, t))
                .map_err(|e| QuoinError::ValueError(format!("Time parse: {e}")))
        })
        .doc(
            "The time an ISO 8601 string (`'09:40:00'`, fractional seconds allowed) \
             denotes; not parseable → ValueError.\n\n\
             ```\n\
             (Time.parse:'09:40:30').second     \"* -> 30\n\
             ```",
        )
        .sdk_class_method("midnight", |host, _r, _a| {
            Ok(make_time(host, Time::midnight()))
        })
        .doc("00:00:00 — the first instant of a day.");
    // Components.
    let b = b
        .sdk_instance_method("hour", |host, r, _a| {
            Ok(host.new_int(time_of(r, "hour")?.hour() as i64))
        })
        .doc("The hour component (0–23).")
        .sdk_instance_method("minute", |host, r, _a| {
            Ok(host.new_int(time_of(r, "minute")?.minute() as i64))
        })
        .doc("The minute component (0–59).")
        .sdk_instance_method("second", |host, r, _a| {
            Ok(host.new_int(time_of(r, "second")?.second() as i64))
        })
        .doc("The second component (0–59).")
        .sdk_instance_method("nanosecond", |host, r, _a| {
            Ok(host.new_int(time_of(r, "nanosecond")?.subsec_nanosecond() as i64))
        })
        .doc("The subsecond component in nanoseconds (0–999,999,999).");
    // Arithmetic wraps around midnight; comparison is within-a-day.
    b.sdk_typed_instance_method("+:", &["Duration"], |host, receiver, args| {
        let t = time_of(receiver, "+:")?.wrapping_add(duration_of(args[0], "+:")?);
        Ok(make_time(host, t))
    })
    .doc(
        "The clock reading a Duration later, WRAPPING around midnight — a zoneless clock \
         time has no date to carry into.\n\n\
         ```\n\
         ((Time.hour:23 minute:30) + (Duration.hours:1)).s     \"* -> 00:30:00\n\
         ```",
    )
    .sdk_typed_instance_method("-:", &["Duration"], |host, receiver, args| {
        let t = time_of(receiver, "-:")?.wrapping_sub(duration_of(args[0], "-:")?);
        Ok(make_time(host, t))
    })
    .doc("The clock reading a Duration earlier, wrapping below midnight.")
    .sdk_typed_instance_method("until:", &["Time"], |host, receiver, args| {
        let a = time_of(receiver, "until:")?;
        let b = time_of(args[0], "until:")?;
        let span = a
            .until(TimeDifference::new(b))
            .map_err(|e| QuoinError::ArithmeticError(format!("Time until: {e}")))?;
        let d = SignedDuration::try_from(span)
            .map_err(|e| QuoinError::ArithmeticError(format!("Time until: {e}")))?;
        Ok(make_duration(host, d))
    })
    .doc(
        "The signed Duration from the receiver to the argument, read within one day — \
         negative when the argument is the earlier clock reading (nothing wraps: 23:00 \
         until 01:00 is -22h, not 2h).\n\n\
         ```\n\
         ((Time.hour:9 minute:0).until:(Time.hour:17 minute:30)).s     \"* -> 8h 30m\n\
         ```",
    )
    .sdk_typed_instance_method("<:", &["Time"], |host, receiver, args| {
        Ok(host.new_bool(time_of(receiver, "<:")? < time_of(args[0], "<:")?))
    })
    .doc(
        "Whether the receiver is the earlier clock reading (midnight is least). Only `<:` \
         is native; the rest derive on Object.",
    )
    .sdk_instance_method("==:", |host, receiver, args| {
        let a = time_of(receiver, "==:")?;
        let eq = match args[0].with_native_state::<NativeTime, _, _>(|t| t.0) {
            Ok(b) => a == b,
            Err(_) => false,
        };
        Ok(host.new_bool(eq))
    })
    .doc("Whether the argument is the same clock reading. A non-Time is simply unequal.")
    .sdk_instance_method("s", |host, receiver, _args| {
        Ok(host.new_string(time_of(receiver, "s")?.to_string()))
    })
    .doc(
        "The ISO 8601 time string (subseconds only when present).\n\n\
         ```\n\
         (Time.hour:9 minute:40).s     \"* -> 09:40:00\n\
         ```",
    )
}
