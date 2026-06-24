use crate::arg;
use crate::error::QuoinError;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
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

fn make_decimal<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, d: Decimal) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "BigDecimal");
    vm.new_native_state(mc, class, NativeBigDecimal(d))
}

pub fn build_big_decimal_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("BigDecimal", Some("Object"))
        // BigDecimal.of:'1.50' — parse exactly from a string. (A Double is intentionally not
        // accepted: convert via a string so the value isn't already corrupted by binary float
        // rounding.)
        .typed_class_method("of:", &["String"], |vm, mc, _r, args| {
            let s = arg!(args, String, 0);
            match Decimal::from_str(s.as_str()) {
                Ok(d) => Ok(make_decimal(vm, mc, d)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "BigDecimal.of:: not a decimal number: '{}'",
                    s.as_str()
                ))),
            }
        })
        // BigDecimal.of:42 — exact from an Integer.
        .typed_class_method("of:", &["Integer"], |vm, mc, _r, args| {
            Ok(make_decimal(vm, mc, Decimal::from(arg!(args, Int, 0))))
        })
        // BigDecimal.of:150 scale:2 -> 1.50  (mantissa + number of fractional digits).
        .typed_class_method("of:scale:", &["Integer", "Integer"], |vm, mc, _r, args| {
            let mantissa = arg!(args, Int, 0);
            let scale = arg!(args, Int, 1);
            if !(0..=28).contains(&scale) {
                return Err(QuoinError::ValueError(
                    "BigDecimal.of:scale:: scale must be 0..=28".to_string(),
                ));
            }
            Ok(make_decimal(vm, mc, Decimal::new(mantissa, scale as u32)))
        });
    // Arithmetic is BigDecimal-only (explicit conversion); a foreign operand matches no typed
    // variant and surfaces as a "message not understood" naming the `:BigDecimal` signature.
    let b = b
        .typed_instance_method("+:", &["BigDecimal"], |vm, mc, receiver, args| {
            checked(
                vm,
                mc,
                decimal_of(receiver, "+:")?.checked_add(decimal_of(args[0], "+:")?),
                "+:",
            )
        })
        .typed_instance_method("-:", &["BigDecimal"], |vm, mc, receiver, args| {
            checked(
                vm,
                mc,
                decimal_of(receiver, "-:")?.checked_sub(decimal_of(args[0], "-:")?),
                "-:",
            )
        })
        .typed_instance_method("*:", &["BigDecimal"], |vm, mc, receiver, args| {
            checked(
                vm,
                mc,
                decimal_of(receiver, "*:")?.checked_mul(decimal_of(args[0], "*:")?),
                "*:",
            )
        })
        .typed_instance_method("/:", &["BigDecimal"], |vm, mc, receiver, args| {
            let divisor = decimal_of(args[0], "/:")?;
            if divisor == Decimal::ZERO {
                return Err(QuoinError::ArithmeticError(
                    "BigDecimal division by zero".to_string(),
                ));
            }
            checked(
                vm,
                mc,
                decimal_of(receiver, "/:")?.checked_div(divisor),
                "/:",
            )
        })
        // Only `<:` is native; `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
        .typed_instance_method("<:", &["BigDecimal"], |vm, mc, receiver, args| {
            Ok(vm.new_bool(mc, decimal_of(receiver, "<:")? < decimal_of(args[0], "<:")?))
        })
        // `==:` accepts any argument: a non-BigDecimal is simply unequal (never an error),
        // matching Integer/Double `==:` semantics.
        .instance_method("==:", |vm, mc, receiver, args| {
            let a = decimal_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeBigDecimal, _, _>(|d| d.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(vm.new_bool(mc, eq))
        });
    b.instance_method("abs", |vm, mc, receiver, _args| {
        Ok(make_decimal(vm, mc, decimal_of(receiver, "abs")?.abs()))
    })
    // The number of fractional digits (e.g. 1.50 -> 2).
    .instance_method("scale", |vm, mc, receiver, _args| {
        Ok(vm.new_int(mc, decimal_of(receiver, "scale")?.scale() as i64))
    })
    // Round to `n` fractional digits, half-away-from-zero (matching `Double.round`, rather than
    // rust_decimal's default banker's rounding — one rounding rule across the number types).
    .typed_instance_method("round:", &["Integer"], |vm, mc, receiver, args| {
        let dp = arg!(args, Int, 0);
        if !(0..=28).contains(&dp) {
            return Err(QuoinError::ValueError(
                "BigDecimal round:: places must be 0..=28".to_string(),
            ));
        }
        let rounded = decimal_of(receiver, "round:")?
            .round_dp_with_strategy(dp as u32, RoundingStrategy::MidpointAwayFromZero);
        Ok(make_decimal(vm, mc, rounded))
    })
    // Lossy conversion to a Double.
    .instance_method("asDouble", |vm, mc, receiver, _args| {
        match decimal_of(receiver, "asDouble")?.to_f64() {
            Some(f) => Ok(vm.new_double(mc, f)),
            None => Err(QuoinError::ArithmeticError(
                "BigDecimal asDouble: not representable as a Double".to_string(),
            )),
        }
    })
    // Truncate toward zero to an Integer (errors if out of i64 range).
    .instance_method("asInteger", |vm, mc, receiver, _args| {
        match decimal_of(receiver, "asInteger")?.trunc().to_i64() {
            Some(n) => Ok(vm.new_int(mc, n)),
            None => Err(QuoinError::ArithmeticError(
                "BigDecimal asInteger: out of Integer range".to_string(),
            )),
        }
    })
    // Canonical decimal string.
    .instance_method("s", |vm, mc, receiver, _args| {
        Ok(vm.new_string(mc, decimal_of(receiver, "s")?.to_string()))
    })
}

/// Wrap a `checked_*` result: `Some` -> a `BigDecimal`, `None` -> an overflow `ArithmeticError`.
fn checked<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    result: Option<Decimal>,
    who: &str,
) -> Result<Value<'gc>, QuoinError> {
    match result {
        Some(d) => Ok(make_decimal(vm, mc, d)),
        None => Err(QuoinError::ArithmeticError(format!(
            "BigDecimal {who} overflow"
        ))),
    }
}
