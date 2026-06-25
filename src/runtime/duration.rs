use crate::arg;
use crate::error::QuoinError;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
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

pub fn make_duration<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, d: SignedDuration) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "Duration");
    vm.new_native_state(mc, class, NativeDuration(d))
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
        // Unit constructors (Integer counts). `Duration.zero` is the identity.
        .class_method("zero", |vm, mc, _r, _a| {
            Ok(make_duration(vm, mc, SignedDuration::ZERO))
        })
        .typed_class_method("nanoseconds:", &["Integer"], |vm, mc, _r, args| {
            Ok(make_duration(
                vm,
                mc,
                SignedDuration::from_nanos(arg!(args, Int, 0)),
            ))
        })
        .typed_class_method("microseconds:", &["Integer"], |vm, mc, _r, args| {
            Ok(make_duration(
                vm,
                mc,
                SignedDuration::from_micros(arg!(args, Int, 0)),
            ))
        })
        .typed_class_method("milliseconds:", &["Integer"], |vm, mc, _r, args| {
            Ok(make_duration(
                vm,
                mc,
                SignedDuration::from_millis(arg!(args, Int, 0)),
            ))
        })
        .typed_class_method("seconds:", &["Integer"], |vm, mc, _r, args| {
            Ok(make_duration(
                vm,
                mc,
                SignedDuration::from_secs(arg!(args, Int, 0)),
            ))
        })
        .typed_class_method("minutes:", &["Integer"], |vm, mc, _r, args| {
            Ok(make_duration(
                vm,
                mc,
                SignedDuration::from_mins(arg!(args, Int, 0)),
            ))
        })
        .typed_class_method("hours:", &["Integer"], |vm, mc, _r, args| {
            Ok(make_duration(
                vm,
                mc,
                SignedDuration::from_hours(arg!(args, Int, 0)),
            ))
        });
    // Arithmetic is Duration-only (explicit), except `*:` which scales by an Integer; overflow
    // (a duration past ~292 billion years) is a checked ArithmeticError.
    let b = b
        .typed_instance_method("+:", &["Duration"], |vm, mc, receiver, args| {
            duration_of(receiver, "+:")?
                .checked_add(duration_of(args[0], "+:")?)
                .map(|d| make_duration(vm, mc, d))
                .ok_or_else(|| QuoinError::ArithmeticError("Duration +: overflow".to_string()))
        })
        .typed_instance_method("-:", &["Duration"], |vm, mc, receiver, args| {
            duration_of(receiver, "-:")?
                .checked_sub(duration_of(args[0], "-:")?)
                .map(|d| make_duration(vm, mc, d))
                .ok_or_else(|| QuoinError::ArithmeticError("Duration -: overflow".to_string()))
        })
        .typed_instance_method("*:", &["Integer"], |vm, mc, receiver, args| {
            let factor = i32::try_from(arg!(args, Int, 0)).map_err(|_| {
                QuoinError::ArithmeticError("Duration *: factor out of range".to_string())
            })?;
            duration_of(receiver, "*:")?
                .checked_mul(factor)
                .map(|d| make_duration(vm, mc, d))
                .ok_or_else(|| QuoinError::ArithmeticError("Duration *: overflow".to_string()))
        })
        // Only `<:` is native; `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
        .typed_instance_method("<:", &["Duration"], |vm, mc, receiver, args| {
            Ok(vm.new_bool(
                mc,
                duration_of(receiver, "<:")? < duration_of(args[0], "<:")?,
            ))
        })
        // `==:` accepts any argument: a non-Duration is simply unequal (never an error).
        .instance_method("==:", |vm, mc, receiver, args| {
            let a = duration_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeDuration, _, _>(|d| d.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(vm.new_bool(mc, eq))
        });
    b.instance_method("negate", |vm, mc, receiver, _args| {
        duration_of(receiver, "negate")?
            .checked_neg()
            .map(|d| make_duration(vm, mc, d))
            .ok_or_else(|| QuoinError::ArithmeticError("Duration negate overflow".to_string()))
    })
    .instance_method("abs", |vm, mc, receiver, _args| {
        // unsigned_abs can't overflow (returns the magnitude); re-sign as positive.
        let d = duration_of(receiver, "abs")?;
        Ok(make_duration(vm, mc, if d.is_negative() { -d } else { d }))
    })
    // Total length in a unit. `asSeconds` is fractional (Double); the rest are whole counts
    // truncated toward zero (Integer), range-checked.
    .instance_method("asSeconds", |vm, mc, receiver, _args| {
        Ok(vm.new_double(mc, duration_of(receiver, "asSeconds")?.as_secs_f64()))
    })
    .instance_method("asMilliseconds", |vm, mc, receiver, _args| {
        Ok(vm.new_int(
            mc,
            narrow(
                duration_of(receiver, "asMilliseconds")?.as_millis(),
                "asMilliseconds",
            )?,
        ))
    })
    .instance_method("asMicroseconds", |vm, mc, receiver, _args| {
        Ok(vm.new_int(
            mc,
            narrow(
                duration_of(receiver, "asMicroseconds")?.as_micros(),
                "asMicroseconds",
            )?,
        ))
    })
    .instance_method("asNanoseconds", |vm, mc, receiver, _args| {
        Ok(vm.new_int(
            mc,
            narrow(
                duration_of(receiver, "asNanoseconds")?.as_nanos(),
                "asNanoseconds",
            )?,
        ))
    })
    // Human-readable string — jiff's friendly form via the alternate flag (e.g. "1h 30m").
    .instance_method("s", |vm, mc, receiver, _args| {
        Ok(vm.new_string(mc, format!("{:#}", duration_of(receiver, "s")?)))
    })
    // The canonical ISO 8601 duration string (e.g. "PT1H30M"), for serialization.
    .instance_method("iso8601", |vm, mc, receiver, _args| {
        Ok(vm.new_string(mc, duration_of(receiver, "iso8601")?.to_string()))
    })
}
