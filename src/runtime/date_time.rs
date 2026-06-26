use crate::arg;
use crate::error::QuoinError;
use crate::runtime::duration::{duration_of, make_duration};
use crate::runtime::pretty::{PpChild, PpRole, PpShape, PrettyPrint};
use crate::runtime::time_zone::{make_time_zone, tz_of, zone_name};
use crate::runtime::timestamp::{duration_between, make_timestamp};
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
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

pub fn make_date_time<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, z: Zoned) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "DateTime");
    vm.new_native_state(mc, class, NativeDateTime(z))
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
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
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
        .map(|z2| make_date_time(vm, mc, z2))
        .map_err(|e| QuoinError::ArithmeticError(format!("{who}: {e}")))
}

pub fn build_date_time_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("DateTime", Some("Object"))
        // DateTime.now -> the current date+time in the host's local zone.
        .class_method("now", |vm, mc, _r, _a| {
            Ok(make_date_time(vm, mc, Zoned::now()))
        })
        // DateTime.nowUtc -> the current date+time in UTC.
        .class_method("nowUtc", |vm, mc, _r, _a| {
            Ok(make_date_time(
                vm,
                mc,
                Timestamp::now().to_zoned(TimeZone::UTC),
            ))
        })
        // DateTime.nowIn:aTimeZone -> the current date+time in a given zone.
        .typed_class_method("nowIn:", &["TimeZone"], |vm, mc, _r, args| {
            let z = Timestamp::now().to_zoned(tz_of(args[0], "nowIn:")?);
            Ok(make_date_time(vm, mc, z))
        })
        // DateTime.parse:'2024-01-01T12:00:00-05:00[America/New_York]' -> parse a zoned datetime.
        .typed_class_method("parse:", &["String"], |vm, mc, _r, args| {
            let s = arg!(args, String, 0);
            match s.as_str().parse::<Zoned>() {
                Ok(z) => Ok(make_date_time(vm, mc, z)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "DateTime.parse:: not a zoned datetime: '{}'",
                    s.as_str()
                ))),
            }
        });
    // Components.
    let b = b
        .instance_method("year", |vm, mc, r, _a| {
            Ok(vm.new_int(mc, zoned_of(r, "year")?.year() as i64))
        })
        .instance_method("month", |vm, mc, r, _a| {
            Ok(vm.new_int(mc, zoned_of(r, "month")?.month() as i64))
        })
        .instance_method("day", |vm, mc, r, _a| {
            Ok(vm.new_int(mc, zoned_of(r, "day")?.day() as i64))
        })
        .instance_method("hour", |vm, mc, r, _a| {
            Ok(vm.new_int(mc, zoned_of(r, "hour")?.hour() as i64))
        })
        .instance_method("minute", |vm, mc, r, _a| {
            Ok(vm.new_int(mc, zoned_of(r, "minute")?.minute() as i64))
        })
        .instance_method("second", |vm, mc, r, _a| {
            Ok(vm.new_int(mc, zoned_of(r, "second")?.second() as i64))
        })
        .instance_method("nanosecond", |vm, mc, r, _a| {
            Ok(vm.new_int(mc, zoned_of(r, "nanosecond")?.subsec_nanosecond() as i64))
        })
        .instance_method("weekday", |vm, mc, r, _a| {
            Ok(vm.new_string(
                mc,
                weekday_name(zoned_of(r, "weekday")?.weekday()).to_string(),
            ))
        })
        .instance_method("timeZone", |vm, mc, r, _a| {
            Ok(make_time_zone(
                vm,
                mc,
                zoned_of(r, "timeZone")?.time_zone().clone(),
            ))
        })
        .instance_method("timestamp", |vm, mc, r, _a| {
            Ok(make_timestamp(
                vm,
                mc,
                zoned_of(r, "timestamp")?.timestamp(),
            ))
        })
        // RFC 9557 string (RFC 3339 + an [IANA/Zone] suffix), e.g.
        // '2024-01-01T12:00:00-05:00[America/New_York]'.
        .instance_method("s", |vm, mc, r, _a| {
            Ok(vm.new_string(mc, zoned_of(r, "s")?.to_string()))
        });
    // Absolute arithmetic (Duration) and difference.
    let b = b
        .typed_instance_method("+:", &["Duration"], |vm, mc, receiver, args| {
            zoned_of(receiver, "+:")?
                .checked_add(duration_of(args[0], "+:")?)
                .map(|z| make_date_time(vm, mc, z))
                .map_err(|e| QuoinError::ArithmeticError(format!("DateTime +:: {e}")))
        })
        .typed_instance_method("-:", &["Duration"], |vm, mc, receiver, args| {
            zoned_of(receiver, "-:")?
                .checked_sub(duration_of(args[0], "-:")?)
                .map(|z| make_date_time(vm, mc, z))
                .map_err(|e| QuoinError::ArithmeticError(format!("DateTime -:: {e}")))
        })
        .typed_instance_method("-:", &["DateTime"], |vm, mc, receiver, args| {
            let d = duration_between(
                zoned_of(receiver, "-:")?.timestamp(),
                zoned_of(args[0], "-:")?.timestamp(),
            );
            Ok(make_duration(vm, mc, d))
        })
        // Comparison is by instant (the underlying timestamp), regardless of zone.
        .typed_instance_method("<:", &["DateTime"], |vm, mc, receiver, args| {
            let lt = zoned_of(receiver, "<:")?.timestamp() < zoned_of(args[0], "<:")?.timestamp();
            Ok(vm.new_bool(mc, lt))
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            let a = zoned_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeDateTime, _, _>(|z| z.0.clone()) {
                Ok(b) => a.timestamp() == b.timestamp(),
                Err(_) => false,
            };
            Ok(vm.new_bool(mc, eq))
        });
    // Calendar arithmetic (DST/end-of-month aware). Span unit limits guarded; see span_count.
    b.typed_instance_method("plusDays:", &["Integer"], |vm, mc, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 7_304_484, "plusDays:")?;
        shift(vm, mc, receiver, Span::new().days(n), false, "plusDays:")
    })
    .typed_instance_method("plusWeeks:", &["Integer"], |vm, mc, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 1_043_497, "plusWeeks:")?;
        shift(vm, mc, receiver, Span::new().weeks(n), false, "plusWeeks:")
    })
    .typed_instance_method("plusMonths:", &["Integer"], |vm, mc, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 239_976, "plusMonths:")?;
        shift(
            vm,
            mc,
            receiver,
            Span::new().months(n),
            false,
            "plusMonths:",
        )
    })
    .typed_instance_method("plusYears:", &["Integer"], |vm, mc, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 19_998, "plusYears:")?;
        shift(vm, mc, receiver, Span::new().years(n), false, "plusYears:")
    })
    .typed_instance_method("minusDays:", &["Integer"], |vm, mc, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 7_304_484, "minusDays:")?;
        shift(vm, mc, receiver, Span::new().days(n), true, "minusDays:")
    })
    .typed_instance_method("minusWeeks:", &["Integer"], |vm, mc, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 1_043_497, "minusWeeks:")?;
        shift(vm, mc, receiver, Span::new().weeks(n), true, "minusWeeks:")
    })
    .typed_instance_method("minusMonths:", &["Integer"], |vm, mc, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 239_976, "minusMonths:")?;
        shift(
            vm,
            mc,
            receiver,
            Span::new().months(n),
            true,
            "minusMonths:",
        )
    })
    .typed_instance_method("minusYears:", &["Integer"], |vm, mc, receiver, args| {
        let n = span_count(arg!(args, Int, 0), 19_998, "minusYears:")?;
        shift(vm, mc, receiver, Span::new().years(n), true, "minusYears:")
    })
    // The same instant viewed in another zone.
    .typed_instance_method("inZone:", &["TimeZone"], |vm, mc, receiver, args| {
        let z = zoned_of(receiver, "inZone:")?.with_time_zone(tz_of(args[0], "inZone:")?);
        Ok(make_date_time(vm, mc, z))
    })
}
