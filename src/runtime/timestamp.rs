use crate::arg;
use crate::error::QuoinError;
use crate::runtime::date_time::make_date_time;
use crate::runtime::duration::{duration_of, make_duration};
use crate::runtime::time_zone::tz_of;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
use gc_arena::collect::Trace;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use std::any::Any;

/// Native backing state for a `Timestamp`: an absolute instant in time (jiff `Timestamp`, UTC,
/// nanosecond precision). The wall-clock counterpart to the monotonic `Instant`. `Copy`, no `Gc`,
/// no reap.
#[derive(Debug)]
pub struct NativeTimestamp(pub Timestamp);

impl AnyCollect for NativeTimestamp {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

pub fn ts_of(v: Value, who: &str) -> Result<Timestamp, QuoinError> {
    v.with_native_state::<NativeTimestamp, _, _>(|t| t.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "Timestamp".to_string(),
            got: "a non-Timestamp value".to_string(),
            msg: format!("{who} requires a Timestamp operand"),
        })
}

pub fn make_timestamp<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, ts: Timestamp) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "Timestamp");
    vm.new_native_state(mc, class, NativeTimestamp(ts))
}

/// Signed elapsed time `a - b` as a `SignedDuration` (the backing of a `Duration`). Computed from
/// the nanosecond difference; `secs` can't overflow i64 (jiff timestamps span ~±9999 years).
pub fn duration_between(a: Timestamp, b: Timestamp) -> SignedDuration {
    let nanos = a.as_nanosecond() - b.as_nanosecond();
    SignedDuration::new(
        (nanos / 1_000_000_000) as i64,
        (nanos % 1_000_000_000) as i32,
    )
}

pub fn build_timestamp_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("Timestamp", Some("Object"))
        // Timestamp.now -> the current absolute instant (UTC).
        .class_method("now", |vm, mc, _r, _a| {
            Ok(make_timestamp(vm, mc, Timestamp::now()))
        })
        // Timestamp.parse:'2024-01-01T00:00:00Z' -> parse an RFC 3339 timestamp.
        .typed_class_method("parse:", &["String"], |vm, mc, _r, args| {
            let s = arg!(args, String, 0);
            match s.as_str().parse::<Timestamp>() {
                Ok(ts) => Ok(make_timestamp(vm, mc, ts)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "Timestamp.parse:: not an RFC 3339 timestamp: '{}'",
                    s.as_str()
                ))),
            }
        })
        // From a Unix epoch count.
        .typed_class_method("fromUnixSeconds:", &["Integer"], |vm, mc, _r, args| {
            match Timestamp::from_second(arg!(args, Int, 0)) {
                Ok(ts) => Ok(make_timestamp(vm, mc, ts)),
                Err(_) => Err(QuoinError::ValueError(
                    "Timestamp.fromUnixSeconds:: out of range".to_string(),
                )),
            }
        })
        .typed_class_method("fromUnixMilliseconds:", &["Integer"], |vm, mc, _r, args| {
            match Timestamp::from_millisecond(arg!(args, Int, 0)) {
                Ok(ts) => Ok(make_timestamp(vm, mc, ts)),
                Err(_) => Err(QuoinError::ValueError(
                    "Timestamp.fromUnixMilliseconds:: out of range".to_string(),
                )),
            }
        });
    let b = b
        // ts + Duration / ts - Duration -> a shifted Timestamp; ts - ts -> the Duration between.
        .typed_instance_method("+:", &["Duration"], |vm, mc, receiver, args| {
            ts_of(receiver, "+:")?
                .checked_add(duration_of(args[0], "+:")?)
                .map(|ts| make_timestamp(vm, mc, ts))
                .map_err(|e| QuoinError::ArithmeticError(format!("Timestamp +:: {e}")))
        })
        .typed_instance_method("-:", &["Duration"], |vm, mc, receiver, args| {
            ts_of(receiver, "-:")?
                .checked_sub(duration_of(args[0], "-:")?)
                .map(|ts| make_timestamp(vm, mc, ts))
                .map_err(|e| QuoinError::ArithmeticError(format!("Timestamp -:: {e}")))
        })
        .typed_instance_method("-:", &["Timestamp"], |vm, mc, receiver, args| {
            let d = duration_between(ts_of(receiver, "-:")?, ts_of(args[0], "-:")?);
            Ok(make_duration(vm, mc, d))
        })
        // Only `<:` is native; the other comparisons derive from it on Object.
        .typed_instance_method("<:", &["Timestamp"], |vm, mc, receiver, args| {
            Ok(vm.new_bool(mc, ts_of(receiver, "<:")? < ts_of(args[0], "<:")?))
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            let a = ts_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeTimestamp, _, _>(|t| t.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(vm.new_bool(mc, eq))
        });
    b.instance_method("asUnixSeconds", |vm, mc, receiver, _args| {
        Ok(vm.new_int(mc, ts_of(receiver, "asUnixSeconds")?.as_second()))
    })
    .instance_method("asUnixMilliseconds", |vm, mc, receiver, _args| {
        Ok(vm.new_int(mc, ts_of(receiver, "asUnixMilliseconds")?.as_millisecond()))
    })
    // RFC 3339 string (e.g. '2024-01-01T00:00:00Z').
    .instance_method("s", |vm, mc, receiver, _args| {
        Ok(vm.new_string(mc, ts_of(receiver, "s")?.to_string()))
    })
    // The zoned DateTime for this instant in a given zone / in UTC.
    .typed_instance_method("inZone:", &["TimeZone"], |vm, mc, receiver, args| {
        let zoned = ts_of(receiver, "inZone:")?.to_zoned(tz_of(args[0], "inZone:")?);
        Ok(make_date_time(vm, mc, zoned))
    })
    .instance_method("utc", |vm, mc, receiver, _args| {
        let zoned = ts_of(receiver, "utc")?.to_zoned(TimeZone::UTC);
        Ok(make_date_time(vm, mc, zoned))
    })
}
