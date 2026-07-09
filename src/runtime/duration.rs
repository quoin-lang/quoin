use crate::arg;
use crate::error::QuoinError;
use crate::ext_sdk::{Host, HostExt};
use crate::runtime::pretty::{PpChild, PpShape, PrettyPrint};
use crate::value::{AnyCollect, NativeClassBuilder, Value};

use gc_arena::collect::Trace;
use jiff::SignedDuration;
use std::any::Any;

/// Native backing state for a `Duration`: a signed, fixed length of time (jiff `SignedDuration`,
/// i64 seconds + i32 nanoseconds). Signed so a DateTime difference (Phase 2) can be negative.
/// Plain `Copy` data — no `Gc` fields, no OS resource — so `trace_gc` is empty and there is no
/// reap-on-drop.
#[derive(Debug)]
pub struct NativeDuration(pub SignedDuration);

impl AnyCollect for NativeDuration {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

impl PrettyPrint for NativeDuration {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        // The faithful internal repr — whole seconds + subsecond nanos (same sign), mirroring
        // `Timestamp`'s decomposition.
        let d = self.0;
        PpShape::Record {
            name: "Duration",
            fields: vec![
                ("seconds".to_string(), PpChild::Val(Value::Int(d.as_secs()))),
                (
                    "nanoseconds".to_string(),
                    PpChild::Val(Value::Int(d.subsec_nanos() as i64)),
                ),
            ],
        }
    }
}

/// The `SignedDuration` behind a `Duration` value (the receiver, or — for the typed operators — a
/// same-typed operand). Errors clearly if `v` is not a `Duration`.
pub fn duration_of(v: Value, who: &str) -> Result<SignedDuration, QuoinError> {
    v.with_native_state::<NativeDuration, _, _>(|d| d.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "Duration".to_string(),
            got: "a non-Duration value".to_string(),
            msg: format!("{who} requires a Duration operand"),
        })
}

pub fn make_duration<'gc>(host: &dyn Host<'gc>, d: SignedDuration) -> Value<'gc> {
    let class = host.get_or_create_builtin_class("Duration");
    host.new_native_state(class, NativeDuration(d))
}

/// Total whole milliseconds of a `Duration` value, clamped to a non-negative `i64` — for the
/// scheduler's ms-based `sleep:`/`timeout:` (a negative or absurd span clamps rather than errors).
pub fn duration_to_millis(v: Value, who: &str) -> Result<i64, QuoinError> {
    Ok(duration_of(v, who)?.as_millis().clamp(0, i64::MAX as i128) as i64)
}

/// `i128` (jiff's total-unit return) narrowed to an `i64`, erroring if out of range — a duration
/// long enough to overflow i64 nanoseconds is ~292 years, far past anything practical.
fn narrow(total: i128, unit: &str) -> Result<i64, QuoinError> {
    i64::try_from(total)
        .map_err(|_| QuoinError::ArithmeticError(format!("Duration {unit}: value out of range")))
}

pub fn build_duration_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("Duration", Some("Object"))
        .construct_with("use Duration.seconds: / Duration.milliseconds: / …")
        // Unit constructors (Integer counts). `Duration.zero` is the identity.
        .sdk_class_method("zero", |host, _r, _a| {
            Ok(make_duration(host, SignedDuration::ZERO))
        })
        .sdk_typed_class_method("nanoseconds:", &["Integer"], |host, _r, args| {
            Ok(make_duration(
                host,
                SignedDuration::from_nanos(arg!(args, Int, 0)),
            ))
        })
        .sdk_typed_class_method("microseconds:", &["Integer"], |host, _r, args| {
            Ok(make_duration(
                host,
                SignedDuration::from_micros(arg!(args, Int, 0)),
            ))
        })
        .sdk_typed_class_method("milliseconds:", &["Integer"], |host, _r, args| {
            Ok(make_duration(
                host,
                SignedDuration::from_millis(arg!(args, Int, 0)),
            ))
        })
        .sdk_typed_class_method("seconds:", &["Integer"], |host, _r, args| {
            Ok(make_duration(
                host,
                SignedDuration::from_secs(arg!(args, Int, 0)),
            ))
        })
        .sdk_typed_class_method("minutes:", &["Integer"], |host, _r, args| {
            Ok(make_duration(
                host,
                SignedDuration::from_mins(arg!(args, Int, 0)),
            ))
        })
        .sdk_typed_class_method("hours:", &["Integer"], |host, _r, args| {
            Ok(make_duration(
                host,
                SignedDuration::from_hours(arg!(args, Int, 0)),
            ))
        });
    // Arithmetic is Duration-only (explicit), except `*:` which scales by an Integer; overflow
    // (a duration past ~292 billion years) is a checked ArithmeticError.
    let b = b
        .sdk_typed_instance_method("+:", &["Duration"], |host, receiver, args| {
            duration_of(receiver, "+:")?
                .checked_add(duration_of(args[0], "+:")?)
                .map(|d| make_duration(host, d))
                .ok_or_else(|| QuoinError::ArithmeticError("Duration +: overflow".to_string()))
        })
        .sdk_typed_instance_method("-:", &["Duration"], |host, receiver, args| {
            duration_of(receiver, "-:")?
                .checked_sub(duration_of(args[0], "-:")?)
                .map(|d| make_duration(host, d))
                .ok_or_else(|| QuoinError::ArithmeticError("Duration -: overflow".to_string()))
        })
        .sdk_typed_instance_method("*:", &["Integer"], |host, receiver, args| {
            let factor = i32::try_from(arg!(args, Int, 0)).map_err(|_| {
                QuoinError::ArithmeticError("Duration *: factor out of range".to_string())
            })?;
            duration_of(receiver, "*:")?
                .checked_mul(factor)
                .map(|d| make_duration(host, d))
                .ok_or_else(|| QuoinError::ArithmeticError("Duration *: overflow".to_string()))
        })
        // Only `<:` is native; `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
        .sdk_typed_instance_method("<:", &["Duration"], |host, receiver, args| {
            Ok(host.new_bool(duration_of(receiver, "<:")? < duration_of(args[0], "<:")?))
        })
        // `==:` accepts any argument: a non-Duration is simply unequal (never an error).
        .sdk_instance_method("==:", |host, receiver, args| {
            let a = duration_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeDuration, _, _>(|d| d.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(host.new_bool(eq))
        });
    b.sdk_instance_method("negate", |host, receiver, _args| {
        duration_of(receiver, "negate")?
            .checked_neg()
            .map(|d| make_duration(host, d))
            .ok_or_else(|| QuoinError::ArithmeticError("Duration negate overflow".to_string()))
    })
    .sdk_instance_method("abs", |host, receiver, _args| {
        // unsigned_abs can't overflow (returns the magnitude); re-sign as positive.
        let d = duration_of(receiver, "abs")?;
        Ok(make_duration(host, if d.is_negative() { -d } else { d }))
    })
    // Total length in a unit. `asSeconds` is fractional (Double); the rest are whole counts
    // truncated toward zero (Integer), range-checked.
    .sdk_instance_method("asSeconds", |host, receiver, _args| {
        Ok(host.new_double(duration_of(receiver, "asSeconds")?.as_secs_f64()))
    })
    .sdk_instance_method("asMilliseconds", |host, receiver, _args| {
        Ok(host.new_int(narrow(
            duration_of(receiver, "asMilliseconds")?.as_millis(),
            "asMilliseconds",
        )?))
    })
    .sdk_instance_method("asMicroseconds", |host, receiver, _args| {
        Ok(host.new_int(narrow(
            duration_of(receiver, "asMicroseconds")?.as_micros(),
            "asMicroseconds",
        )?))
    })
    .sdk_instance_method("asNanoseconds", |host, receiver, _args| {
        Ok(host.new_int(narrow(
            duration_of(receiver, "asNanoseconds")?.as_nanos(),
            "asNanoseconds",
        )?))
    })
    // Human-readable string — jiff's friendly form via the alternate flag (e.g. "1h 30m").
    .sdk_instance_method("s", |host, receiver, _args| {
        Ok(host.new_string(format!("{:#}", duration_of(receiver, "s")?)))
    })
    // The canonical ISO 8601 duration string (e.g. "PT1H30M"), for serialization.
    .sdk_instance_method("iso8601", |host, receiver, _args| {
        Ok(host.new_string(duration_of(receiver, "iso8601")?.to_string()))
    })
}
