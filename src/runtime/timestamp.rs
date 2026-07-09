use crate::arg;
use crate::error::QuoinError;
use crate::ext_sdk::{Host, HostExt};
use crate::runtime::date_time::make_date_time;
use crate::runtime::duration::{duration_of, make_duration};
use crate::runtime::pretty::{PpChild, PpShape, PrettyPrint};
use crate::runtime::time_zone::tz_of;
use crate::value::{AnyCollect, NativeClassBuilder, Value};

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

impl PrettyPrint for NativeTimestamp {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        let t = self.0;
        PpShape::Record {
            name: "Timestamp",
            fields: vec![
                (
                    "second".to_string(),
                    PpChild::Val(Value::Int(t.as_second())),
                ),
                (
                    "nanosecond".to_string(),
                    PpChild::Val(Value::Int(t.subsec_nanosecond() as i64)),
                ),
            ],
        }
    }
}

pub fn ts_of(v: Value, who: &str) -> Result<Timestamp, QuoinError> {
    v.with_native_state::<NativeTimestamp, _, _>(|t| t.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "Timestamp".to_string(),
            got: "a non-Timestamp value".to_string(),
            msg: format!("{who} requires a Timestamp operand"),
        })
}

pub fn make_timestamp<'gc>(host: &dyn Host<'gc>, ts: Timestamp) -> Value<'gc> {
    let class = host.get_or_create_builtin_class("Timestamp");
    host.new_native_state(class, NativeTimestamp(ts))
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
        .construct_with("use Timestamp.now / Timestamp.parse:")
        // Timestamp.now -> the current absolute instant (UTC).
        .sdk_class_method("now", |host, _r, _a| {
            Ok(make_timestamp(host, Timestamp::now()))
        })
        // Timestamp.parse:'2024-01-01T00:00:00Z' -> parse an RFC 3339 timestamp.
        .sdk_typed_class_method("parse:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            match s.as_str().parse::<Timestamp>() {
                Ok(ts) => Ok(make_timestamp(host, ts)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "Timestamp.parse:: not an RFC 3339 timestamp: '{}'",
                    s.as_str()
                ))),
            }
        })
        // From a Unix epoch count.
        .sdk_typed_class_method("fromUnixSeconds:", &["Integer"], |host, _r, args| {
            match Timestamp::from_second(arg!(args, Int, 0)) {
                Ok(ts) => Ok(make_timestamp(host, ts)),
                Err(_) => Err(QuoinError::ValueError(
                    "Timestamp.fromUnixSeconds:: out of range".to_string(),
                )),
            }
        })
        .sdk_typed_class_method("fromUnixMilliseconds:", &["Integer"], |host, _r, args| {
            match Timestamp::from_millisecond(arg!(args, Int, 0)) {
                Ok(ts) => Ok(make_timestamp(host, ts)),
                Err(_) => Err(QuoinError::ValueError(
                    "Timestamp.fromUnixMilliseconds:: out of range".to_string(),
                )),
            }
        });
    let b = b
        // ts + Duration / ts - Duration -> a shifted Timestamp; ts - ts -> the Duration between.
        .sdk_typed_instance_method("+:", &["Duration"], |host, receiver, args| {
            ts_of(receiver, "+:")?
                .checked_add(duration_of(args[0], "+:")?)
                .map(|ts| make_timestamp(host, ts))
                .map_err(|e| QuoinError::ArithmeticError(format!("Timestamp +:: {e}")))
        })
        .sdk_typed_instance_method("-:", &["Duration"], |host, receiver, args| {
            ts_of(receiver, "-:")?
                .checked_sub(duration_of(args[0], "-:")?)
                .map(|ts| make_timestamp(host, ts))
                .map_err(|e| QuoinError::ArithmeticError(format!("Timestamp -:: {e}")))
        })
        .sdk_typed_instance_method("-:", &["Timestamp"], |host, receiver, args| {
            let d = duration_between(ts_of(receiver, "-:")?, ts_of(args[0], "-:")?);
            Ok(make_duration(host, d))
        })
        // Only `<:` is native; the other comparisons derive from it on Object.
        .sdk_typed_instance_method("<:", &["Timestamp"], |host, receiver, args| {
            Ok(host.new_bool(ts_of(receiver, "<:")? < ts_of(args[0], "<:")?))
        })
        .sdk_instance_method("==:", |host, receiver, args| {
            let a = ts_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeTimestamp, _, _>(|t| t.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(host.new_bool(eq))
        });
    b.sdk_instance_method("asUnixSeconds", |host, receiver, _args| {
        Ok(host.new_int(ts_of(receiver, "asUnixSeconds")?.as_second()))
    })
    .sdk_instance_method("asUnixMilliseconds", |host, receiver, _args| {
        Ok(host.new_int(ts_of(receiver, "asUnixMilliseconds")?.as_millisecond()))
    })
    // RFC 3339 string (e.g. '2024-01-01T00:00:00Z').
    .sdk_instance_method("s", |host, receiver, _args| {
        Ok(host.new_string(ts_of(receiver, "s")?.to_string()))
    })
    // The zoned DateTime for this instant in a given zone / in UTC.
    .sdk_typed_instance_method("inZone:", &["TimeZone"], |host, receiver, args| {
        let zoned = ts_of(receiver, "inZone:")?.to_zoned(tz_of(args[0], "inZone:")?);
        Ok(make_date_time(host, zoned))
    })
    .sdk_instance_method("utc", |host, receiver, _args| {
        let zoned = ts_of(receiver, "utc")?.to_zoned(TimeZone::UTC);
        Ok(make_date_time(host, zoned))
    })
}
