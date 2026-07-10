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
        .construct_with("use BigDecimal.of:")
        .class_doc(
            "An exact base-10 decimal number with up to 28 significant digits -- for \
             money and other quantities where binary floating point drifts. Construct \
             with `BigDecimal.of:` (a String or an Integer; deliberately not a Double, \
             which would already carry rounding error). Arithmetic never mixes silently: \
             a non-BigDecimal operand is 'message not understood'.\n\n\
             ```\n\
             0.1 + 0.2                                       \"* -> 0.30000000000000004\n\
             (BigDecimal.of:'0.1') + (BigDecimal.of:'0.2')   \"* -> 0.3\n\
             ```",
        )
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
        .doc(
            "A BigDecimal parsed exactly from a decimal String (trailing zeros keep their \
             scale: '1.50' has scale 2) or converted exactly from an Integer. A Double is \
             intentionally not accepted -- go through a String so the value is not \
             already corrupted by binary rounding. A non-numeric String raises a \
             ValueError.\n\n\
             ```\n\
             BigDecimal.of:'1.50'     \"* -> 1.50\n\
             ```",
        )
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
        })
        .doc(
            "A BigDecimal from an integer mantissa and a scale -- the number of \
             fractional digits, 0 through 28.\n\n\
             ```\n\
             BigDecimal.of:150 scale:2     \"* -> 1.50\n\
             ```",
        );
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
        .doc(
            "The exact sum of two BigDecimals; raises an ArithmeticError if the result \
             overflows the 28-digit capacity.\n\n\
             ```\n\
             (BigDecimal.of:'0.1') + (BigDecimal.of:'0.2')     \"* -> 0.3\n\
             ```",
        )
        .sdk_typed_instance_method("-:", &["BigDecimal"], |host, receiver, args| {
            checked(
                host,
                decimal_of(receiver, "-:")?.checked_sub(decimal_of(args[0], "-:")?),
                "-:",
            )
        })
        .doc("The exact difference of two BigDecimals (overflow-checked like `+`).")
        .sdk_typed_instance_method("*:", &["BigDecimal"], |host, receiver, args| {
            checked(
                host,
                decimal_of(receiver, "*:")?.checked_mul(decimal_of(args[0], "*:")?),
                "*:",
            )
        })
        .doc("The exact product of two BigDecimals (overflow-checked like `+`).")
        .sdk_typed_instance_method("/:", &["BigDecimal"], |host, receiver, args| {
            let divisor = decimal_of(args[0], "/:")?;
            if divisor == Decimal::ZERO {
                return Err(QuoinError::ArithmeticError(
                    "BigDecimal division by zero".to_string(),
                ));
            }
            checked(host, decimal_of(receiver, "/:")?.checked_div(divisor), "/:")
        })
        .doc(
            "The quotient of two BigDecimals; a non-terminating quotient is rounded at \
             the 28-digit precision limit. Raises an ArithmeticError for a zero divisor \
             or on overflow.",
        )
        // Only `<:` is native; `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
        .sdk_typed_instance_method("<:", &["BigDecimal"], |host, receiver, args| {
            Ok(host.new_bool(decimal_of(receiver, "<:")? < decimal_of(args[0], "<:")?))
        })
        .doc(
            "Whether the receiver is less than the BigDecimal argument. The one native \
             comparison -- `>`, `<=` and `>=` derive from it.",
        )
        // `==:` accepts any argument: a non-BigDecimal is simply unequal (never an error),
        // matching Integer/Double `==:` semantics.
        .sdk_instance_method("==:", |host, receiver, args| {
            let a = decimal_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeBigDecimal, _, _>(|d| d.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(host.new_bool(eq))
        })
        .doc(
            "Whether the argument is a numerically equal BigDecimal -- scale is ignored, \
             so 1.50 equals 1.5. Any other type is simply unequal, never an error.\n\n\
             ```\n\
             (BigDecimal.of:'1.50') == (BigDecimal.of:'1.5')     \"* -> true\n\
             ```",
        );
    b.sdk_instance_method("abs", |host, receiver, _args| {
        Ok(make_decimal(host, decimal_of(receiver, "abs")?.abs()))
    })
    .doc(
        "The absolute value.\n\n\
         ```\n\
         (BigDecimal.of:'-1.5').abs     \"* -> 1.5\n\
         ```",
    )
    // The number of fractional digits (e.g. 1.50 -> 2).
    .sdk_instance_method("scale", |host, receiver, _args| {
        Ok(host.new_int(decimal_of(receiver, "scale")?.scale() as i64))
    })
    .doc(
        "The number of fractional digits.\n\n\
         ```\n\
         (BigDecimal.of:'1.50').scale     \"* -> 2\n\
         ```",
    )
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
    .doc(
        "Round to the given number of fractional digits (0 through 28), halves away from \
         zero -- the same rule as `Double.round`, not banker's rounding.\n\n\
         ```\n\
         (BigDecimal.of:'2.345').round:2     \"* -> 2.35\n\
         (BigDecimal.of:'-2.5').round:0      \"* -> -3\n\
         ```",
    )
    // Lossy conversion to a Double.
    .sdk_instance_method("asDouble", |host, receiver, _args| {
        match decimal_of(receiver, "asDouble")?.to_f64() {
            Some(f) => Ok(host.new_double(f)),
            None => Err(QuoinError::ArithmeticError(
                "BigDecimal asDouble: not representable as a Double".to_string(),
            )),
        }
    })
    .doc(
        "Convert to a Double, accepting binary rounding error.\n\n\
         ```\n\
         (BigDecimal.of:'1.5').asDouble     \"* -> 1.5\n\
         ```",
    )
    // Truncate toward zero to an Integer (errors if out of i64 range).
    .sdk_instance_method("asInteger", |host, receiver, _args| {
        match decimal_of(receiver, "asInteger")?.trunc().to_i64() {
            Some(n) => Ok(host.new_int(n)),
            None => Err(QuoinError::ArithmeticError(
                "BigDecimal asInteger: out of Integer range".to_string(),
            )),
        }
    })
    .doc(
        "Truncate toward zero to a 64-bit Integer; raises an ArithmeticError if the \
         result is out of range.\n\n\
         ```\n\
         (BigDecimal.of:'3.99').asInteger     \"* -> 3\n\
         ```",
    )
    // Canonical decimal string.
    .sdk_instance_method("s", |host, receiver, _args| {
        Ok(host.new_string(decimal_of(receiver, "s")?.to_string()))
    })
    .doc("The exact decimal digits, preserving scale (a value of scale 2 prints as e.g. '1.50').")
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
