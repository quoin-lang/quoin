use crate::arg;
use crate::error::QuoinError;
use crate::ext_sdk::{Host, HostExt};
use crate::runtime::pretty::{PpChild, PpRole, PpShape, PrettyPrint};
use crate::value::{AnyCollect, NativeClassBuilder, Value};

use gc_arena::collect::Trace;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use std::any::Any;
use std::str::FromStr;

/// Native backing state for a `BigDecimal`: one exact base-10 `rust_decimal::Decimal` (28-29
/// significant digits). Plain `Copy` data — no `Gc` fields and no OS resource — so `trace_gc`
/// is empty and there is no reap-on-drop (unlike `NativeSocket`).
#[derive(Debug)]
pub struct NativeBigDecimal(pub Decimal);

impl AnyCollect for NativeBigDecimal {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

impl PrettyPrint for NativeBigDecimal {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        // value = mantissa × 10^-scale; mantissa is an i128 (too wide for an `Int`), so a leaf.
        PpShape::Record {
            name: "BigDecimal",
            fields: vec![
                (
                    "mantissa".to_string(),
                    PpChild::Text(self.0.mantissa().to_string(), PpRole::Number),
                ),
                (
                    "scale".to_string(),
                    PpChild::Val(Value::Int(self.0.scale() as i64)),
                ),
            ],
        }
    }
}

/// The `Decimal` behind a `BigDecimal` value (the receiver, or — for the typed operators — a
/// same-typed operand). Errors clearly if `v` is not a `BigDecimal`; arithmetic requires
/// explicit conversion, so a foreign operand never silently coerces.
fn decimal_of(v: Value, who: &str) -> Result<Decimal, QuoinError> {
    v.with_native_state::<NativeBigDecimal, _, _>(|d| d.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "BigDecimal".to_string(),
            got: "a non-BigDecimal value".to_string(),
            msg: format!("{who} requires a BigDecimal operand (convert with BigDecimal.of:)"),
        })
}

pub fn make_decimal<'gc>(host: &dyn Host<'gc>, d: Decimal) -> Value<'gc> {
    let class = host.get_or_create_builtin_class("BigDecimal");
    host.new_native_state(class, NativeBigDecimal(d))
}

pub fn build_big_decimal_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("BigDecimal", Some("Object"))
        // BigDecimal.of:'1.50' — parse exactly from a string. (A Double is intentionally not
        // accepted: convert via a string so the value isn't already corrupted by binary float
        // rounding.)
        .sdk_typed_class_method("of:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            match Decimal::from_str(s.as_str()) {
                Ok(d) => Ok(make_decimal(host, d)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "BigDecimal.of:: not a decimal number: '{}'",
                    s.as_str()
                ))),
            }
        })
        // BigDecimal.of:42 — exact from an Integer.
        .sdk_typed_class_method("of:", &["Integer"], |host, _r, args| {
            Ok(make_decimal(host, Decimal::from(arg!(args, Int, 0))))
        })
        // BigDecimal.of:150 scale:2 -> 1.50  (mantissa + number of fractional digits).
        .sdk_typed_class_method("of:scale:", &["Integer", "Integer"], |host, _r, args| {
            let mantissa = arg!(args, Int, 0);
            let scale = arg!(args, Int, 1);
            if !(0..=28).contains(&scale) {
                return Err(QuoinError::ValueError(
                    "BigDecimal.of:scale:: scale must be 0..=28".to_string(),
                ));
            }
            Ok(make_decimal(host, Decimal::new(mantissa, scale as u32)))
        });
    // Arithmetic is BigDecimal-only (explicit conversion); a foreign operand matches no typed
    // variant and surfaces as a "message not understood" naming the `:BigDecimal` signature.
    let b = b
        .sdk_typed_instance_method("+:", &["BigDecimal"], |host, receiver, args| {
            checked(
                host,
                decimal_of(receiver, "+:")?.checked_add(decimal_of(args[0], "+:")?),
                "+:",
            )
        })
        .sdk_typed_instance_method("-:", &["BigDecimal"], |host, receiver, args| {
            checked(
                host,
                decimal_of(receiver, "-:")?.checked_sub(decimal_of(args[0], "-:")?),
                "-:",
            )
        })
        .sdk_typed_instance_method("*:", &["BigDecimal"], |host, receiver, args| {
            checked(
                host,
                decimal_of(receiver, "*:")?.checked_mul(decimal_of(args[0], "*:")?),
                "*:",
            )
        })
        .sdk_typed_instance_method("/:", &["BigDecimal"], |host, receiver, args| {
            let divisor = decimal_of(args[0], "/:")?;
            if divisor == Decimal::ZERO {
                return Err(QuoinError::ArithmeticError(
                    "BigDecimal division by zero".to_string(),
                ));
            }
            checked(host, decimal_of(receiver, "/:")?.checked_div(divisor), "/:")
        })
        // Only `<:` is native; `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
        .sdk_typed_instance_method("<:", &["BigDecimal"], |host, receiver, args| {
            Ok(host.new_bool(decimal_of(receiver, "<:")? < decimal_of(args[0], "<:")?))
        })
        // `==:` accepts any argument: a non-BigDecimal is simply unequal (never an error),
        // matching Integer/Double `==:` semantics.
        .sdk_instance_method("==:", |host, receiver, args| {
            let a = decimal_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeBigDecimal, _, _>(|d| d.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(host.new_bool(eq))
        });
    b.sdk_instance_method("abs", |host, receiver, _args| {
        Ok(make_decimal(host, decimal_of(receiver, "abs")?.abs()))
    })
    // The number of fractional digits (e.g. 1.50 -> 2).
    .sdk_instance_method("scale", |host, receiver, _args| {
        Ok(host.new_int(decimal_of(receiver, "scale")?.scale() as i64))
    })
    // Round to `n` fractional digits, half-away-from-zero (matching `Double.round`, rather than
    // rust_decimal's default banker's rounding — one rounding rule across the number types).
    .sdk_typed_instance_method("round:", &["Integer"], |host, receiver, args| {
        let dp = arg!(args, Int, 0);
        if !(0..=28).contains(&dp) {
            return Err(QuoinError::ValueError(
                "BigDecimal round:: places must be 0..=28".to_string(),
            ));
        }
        let rounded = decimal_of(receiver, "round:")?
            .round_dp_with_strategy(dp as u32, RoundingStrategy::MidpointAwayFromZero);
        Ok(make_decimal(host, rounded))
    })
    // Lossy conversion to a Double.
    .sdk_instance_method("asDouble", |host, receiver, _args| {
        match decimal_of(receiver, "asDouble")?.to_f64() {
            Some(f) => Ok(host.new_double(f)),
            None => Err(QuoinError::ArithmeticError(
                "BigDecimal asDouble: not representable as a Double".to_string(),
            )),
        }
    })
    // Truncate toward zero to an Integer (errors if out of i64 range).
    .sdk_instance_method("asInteger", |host, receiver, _args| {
        match decimal_of(receiver, "asInteger")?.trunc().to_i64() {
            Some(n) => Ok(host.new_int(n)),
            None => Err(QuoinError::ArithmeticError(
                "BigDecimal asInteger: out of Integer range".to_string(),
            )),
        }
    })
    // Canonical decimal string.
    .sdk_instance_method("s", |host, receiver, _args| {
        Ok(host.new_string(decimal_of(receiver, "s")?.to_string()))
    })
}

/// Wrap a `checked_*` result: `Some` -> a `BigDecimal`, `None` -> an overflow `ArithmeticError`.
fn checked<'gc>(
    host: &dyn Host<'gc>,
    result: Option<Decimal>,
    who: &str,
) -> Result<Value<'gc>, QuoinError> {
    match result {
        Some(d) => Ok(make_decimal(host, d)),
        None => Err(QuoinError::ArithmeticError(format!(
            "BigDecimal {who} overflow"
        ))),
    }
}
