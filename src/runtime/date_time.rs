use crate::arg;
use crate::error::QuoinError;
use crate::ext_sdk::{Host, HostExt};
use crate::runtime::duration::{duration_of, make_duration};
use crate::runtime::pretty::{PpChild, PpRole, PpShape, PrettyPrint};
use crate::runtime::time_zone::{make_time_zone, tz_of, zone_name};
use crate::runtime::timestamp::{duration_between, make_timestamp};
use crate::value::{AnyCollect, NativeClassBuilder, Value};

use gc_arena::collect::Trace;
use jiff::civil::Weekday;
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Zoned};
use std::any::Any;

/// Native backing state for a `DateTime`: a zone-aware date+time (jiff `Zoned` — an instant plus
/// its `TimeZone`, so components and DST are correct). `Clone` (not `Copy`), extracted by cloning.
/// No `Gc` fields / no OS resource — `trace_gc` is empty, no reap.
#[derive(Debug)]
pub struct NativeDateTime(pub Zoned);

impl AnyCollect for NativeDateTime {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

impl PrettyPrint for NativeDateTime {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        let z = &self.0;
        PpShape::Record {
            name: "DateTime",
            fields: vec![
                (
                    "year".to_string(),
                    PpChild::Val(Value::Int(z.year() as i64)),
                ),
                (
                    "month".to_string(),
                    PpChild::Val(Value::Int(z.month() as i64)),
                ),
                ("day".to_string(), PpChild::Val(Value::Int(z.day() as i64))),
                (
                    "hour".to_string(),
                    PpChild::Val(Value::Int(z.hour() as i64)),
                ),
                (
                    "minute".to_string(),
                    PpChild::Val(Value::Int(z.minute() as i64)),
                ),
                (
                    "second".to_string(),
                    PpChild::Val(Value::Int(z.second() as i64)),
                ),
                (
                    "nanosecond".to_string(),
                    PpChild::Val(Value::Int(z.subsec_nanosecond() as i64)),
                ),
                (
                    "zone".to_string(),
                    PpChild::Text(zone_name(z.time_zone()), PpRole::Str),
                ),
            ],
        }
    }
}

fn zoned_of(v: Value, who: &str) -> Result<Zoned, QuoinError> {
    v.with_native_state::<NativeDateTime, _, _>(|z| z.0.clone())
        .map_err(|_| QuoinError::TypeError {
            expected: "DateTime".to_string(),
            got: "a non-DateTime value".to_string(),
            msg: format!("{who} requires a DateTime operand"),
        })
}

pub fn make_date_time<'gc>(host: &dyn Host<'gc>, z: Zoned) -> Value<'gc> {
    let class = host.get_or_create_builtin_class("DateTime");
    host.new_native_state(class, NativeDateTime(z))
}

fn weekday_name(w: Weekday) -> &'static str {
    match w {
        Weekday::Monday => "Monday",
        Weekday::Tuesday => "Tuesday",
        Weekday::Wednesday => "Wednesday",
        Weekday::Thursday => "Thursday",
        Weekday::Friday => "Friday",
        Weekday::Saturday => "Saturday",
        Weekday::Sunday => "Sunday",
    }
}

/// Guard a calendar-unit count against jiff's per-unit `Span` limits (its setters panic if a unit
/// is out of range), turning an absurd count into a clean error instead.
fn span_count(n: i64, limit: i64, who: &str) -> Result<i64, QuoinError> {
    if n.abs() <= limit {
        Ok(n)
    } else {
        Err(QuoinError::ArithmeticError(format!(
            "{who}: count {n} out of range (max ±{limit})"
        )))
    }
}

/// Add (or, with `sub`, subtract) a calendar `Span` to the receiver DateTime — DST- and
/// end-of-month-aware via jiff. Errors if the result leaves jiff's supported date range.
fn shift<'gc>(
    host: &dyn Host<'gc>,
    receiver: Value<'gc>,
    span: Span,
    sub: bool,
    who: &str,
) -> Result<Value<'gc>, QuoinError> {
    let z = zoned_of(receiver, who)?;
    let result = if sub {
        z.checked_sub(span)
    } else {
        z.checked_add(span)
    };
    result
        .map(|z2| make_date_time(host, z2))
        .map_err(|e| QuoinError::ArithmeticError(format!("{who}: {e}")))
}

pub fn build_date_time_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("DateTime", Some("Object"))
        .construct_with("use DateTime.now / DateTime.parse:")
        // DateTime.now -> the current date+time in the host's local zone.
        .sdk_class_method("now", |host, _r, _a| Ok(make_date_time(host, Zoned::now())))
        // DateTime.nowUtc -> the current date+time in UTC.
        .sdk_class_method("nowUtc", |host, _r, _a| {
            Ok(make_date_time(
                host,
                Timestamp::now().to_zoned(TimeZone::UTC),
            ))
        })
        // DateTime.nowIn:aTimeZone -> the current date+time in a given zone.
        .sdk_typed_class_method("nowIn:", &["TimeZone"], |host, _r, args| {
            let z = Timestamp::now().to_zoned(tz_of(args[0], "nowIn:")?);
            Ok(make_date_time(host, z))
        })
        // DateTime.parse:'2024-01-01T12:00:00-05:00[America/New_York]' -> parse a zoned datetime.
        .sdk_typed_class_method("parse:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            match s.as_str().parse::<Zoned>() {
                Ok(z) => Ok(make_date_time(host, z)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "DateTime.parse:: not a zoned datetime: '{}'",
                    s.as_str()
                ))),
            }
        });
    // Components.
    let b = b
        .sdk_instance_method("year", |host, r, _a| {
            Ok(host.new_int(zoned_of(r, "year")?.year() as i64))
        })
        .sdk_instance_method("month", |host, r, _a| {
            Ok(host.new_int(zoned_of(r, "month")?.month() as i64))
        })
        .sdk_instance_method("day", |host, r, _a| {
            Ok(host.new_int(zoned_of(r, "day")?.day() as i64))
        })
        .sdk_instance_method("hour", |host, r, _a| {
            Ok(host.new_int(zoned_of(r, "hour")?.hour() as i64))
        })
        .sdk_instance_method("minute", |host, r, _a| {
            Ok(host.new_int(zoned_of(r, "minute")?.minute() as i64))
        })
        .sdk_instance_method("second", |host, r, _a| {
            Ok(host.new_int(zoned_of(r, "second")?.second() as i64))
        })
        .sdk_instance_method("nanosecond", |host, r, _a| {
            Ok(host.new_int(zoned_of(r, "nanosecond")?.subsec_nanosecond() as i64))
        })
        .sdk_instance_method("weekday", |host, r, _a| {
            Ok(host.new_string(weekday_name(zoned_of(r, "weekday")?.weekday()).to_string()))
        })
        .sdk_instance_method("timeZone", |host, r, _a| {
            Ok(make_time_zone(
                host,
                zoned_of(r, "timeZone")?.time_zone().clone(),
            ))
        })
        .sdk_instance_method("timestamp", |host, r, _a| {
            Ok(make_timestamp(host, zoned_of(r, "timestamp")?.timestamp()))
        })
        // RFC 9557 string (RFC 3339 + an [IANA/Zone] suffix), e.g.
        // '2024-01-01T12:00:00-05:00[America/New_York]'.
        .sdk_instance_method("s", |host, r, _a| {
            Ok(host.new_string(zoned_of(r, "s")?.to_string()))
        });
    // Absolute arithmetic (Duration) and difference.
    let b = b
        .sdk_typed_instance_method("+:", &["Duration"], |host, receiver, args| {
            zoned_of(receiver, "+:")?
                .checked_add(duration_of(args[0], "+:")?)
                .map(|z| make_date_time(host, z))
                .map_err(|e| QuoinError::ArithmeticError(format!("DateTime +:: {e}")))
        })
        .sdk_typed_instance_method("-:", &["Duration"], |host, receiver, args| {
            zoned_of(receiver, "-:")?
                .checked_sub(duration_of(args[0], "-:")?)
                .map(|z| make_date_time(host, z))
                .map_err(|e| QuoinError::ArithmeticError(format!("DateTime -:: {e}")))
        })
        .sdk_typed_instance_method("-:", &["DateTime"], |host, receiver, args| {
            let d = duration_between(
                zoned_of(receiver, "-:")?.timestamp(),
                zoned_of(args[0], "-:")?.timestamp(),
            );
            Ok(make_duration(host, d))
        })
        // Comparison is by instant (the underlying timestamp), regardless of zone.
        .sdk_typed_instance_method("<:", &["DateTime"], |host, receiver, args| {
            let lt = zoned_of(receiver, "<:")?.timestamp() < zoned_of(args[0], "<:")?.timestamp();
            Ok(host.new_bool(lt))
        })
        .sdk_instance_method("==:", |host, receiver, args| {
            let a = zoned_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeDateTime, _, _>(|z| z.0.clone()) {
                Ok(b) => a.timestamp() == b.timestamp(),
                Err(_) => false,
            };
            Ok(host.new_bool(eq))
        });
    // Calendar arithmetic (DST/end-of-month aware). Span unit limits guarded; see span_count.
    b.sdk_typed_instance_method("plusDays:", &["Integer"], |host, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 7_304_484, "plusDays:")?;
        shift(host, receiver, Span::new().days(n), false, "plusDays:")
    })
    .sdk_typed_instance_method("plusWeeks:", &["Integer"], |host, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 1_043_497, "plusWeeks:")?;
        shift(host, receiver, Span::new().weeks(n), false, "plusWeeks:")
    })
    .sdk_typed_instance_method("plusMonths:", &["Integer"], |host, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 239_976, "plusMonths:")?;
        shift(host, receiver, Span::new().months(n), false, "plusMonths:")
    })
    .sdk_typed_instance_method("plusYears:", &["Integer"], |host, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 19_998, "plusYears:")?;
        shift(host, receiver, Span::new().years(n), false, "plusYears:")
    })
    .sdk_typed_instance_method("minusDays:", &["Integer"], |host, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 7_304_484, "minusDays:")?;
        shift(host, receiver, Span::new().days(n), true, "minusDays:")
    })
    .sdk_typed_instance_method("minusWeeks:", &["Integer"], |host, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 1_043_497, "minusWeeks:")?;
        shift(host, receiver, Span::new().weeks(n), true, "minusWeeks:")
    })
    .sdk_typed_instance_method("minusMonths:", &["Integer"], |host, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 239_976, "minusMonths:")?;
        shift(host, receiver, Span::new().months(n), true, "minusMonths:")
    })
    .sdk_typed_instance_method("minusYears:", &["Integer"], |host, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 19_998, "minusYears:")?;
        shift(host, receiver, Span::new().years(n), true, "minusYears:")
    })
    // The same instant viewed in another zone.
    .sdk_typed_instance_method("inZone:", &["TimeZone"], |host, receiver, args| {
        let z = zoned_of(receiver, "inZone:")?.with_time_zone(tz_of(args[0], "inZone:")?);
        Ok(make_date_time(host, z))
    })
}
