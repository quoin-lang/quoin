use crate::error::QuoinError;
use crate::recv;
use crate::value::{NativeClassBuilder, Value};

/// Convert an already-whole `f64` (the result of `floor`/`ceil`/`round`/`trunc`) to an `i64`,
/// erroring if its magnitude falls outside the Integer range. There is no auto-promotion to
/// BigInteger, so an unrepresentable result is an `ArithmeticError`, never a silent wrap.
fn whole_to_i64(f: f64) -> Result<i64, QuoinError> {
    // i64::MIN is exactly -2^63; i64::MAX rounds *up* to 2^63 as f64, so the upper bound is strict.
    if f.is_finite() && f >= -(2f64.powi(63)) && f < 2f64.powi(63) {
        Ok(f as i64)
    } else {
        Err(QuoinError::ArithmeticError(format!(
            "{f} is out of Integer range"
        )))
    }
}

/// Generate `[Integer]` and `[Double]` typed variants for a binary operator on a
/// `Double` receiver. Simpler than `int_binop!` (integer.rs): a Double receiver
/// makes *every* arithmetic result a `Double` (both operands coerce via `as_f64`),
/// so the two variants share a body; `/:`/`%:` need no zero-guard under f64
/// semantics. A non-numeric RHS matches no variant and (until the comparison
/// natives are demoted) falls through to the rekeyed global fallback in native.rs.
macro_rules! double_binop {
    ($b:expr, $sel:literal, arith $op:tt) => {
        $b.sdk_typed_instance_method($sel, &["Integer"], |host, receiver, args| {
            Ok(host.new_double(receiver.as_f64().unwrap() $op args[0].as_f64().unwrap()))
        })
        .sdk_typed_instance_method($sel, &["Double"], |host, receiver, args| {
            Ok(host.new_double(receiver.as_f64().unwrap() $op args[0].as_f64().unwrap()))
        })
    };
    ($b:expr, $sel:literal, cmp $op:tt) => {
        $b.sdk_typed_instance_method($sel, &["Integer"], |host, receiver, args| {
            Ok(host.new_bool(receiver.as_f64().unwrap() $op args[0].as_f64().unwrap()))
        })
        .sdk_typed_instance_method($sel, &["Double"], |host, receiver, args| {
            Ok(host.new_bool(receiver.as_f64().unwrap() $op args[0].as_f64().unwrap()))
        })
    };
}

pub fn build_double_class() -> NativeClassBuilder {
    // Binary operators are the `:` keyword selectors (`a + b` -> `Send(a, "+:", [b])`).
    // Only `<:` is provided natively; `>:`/`<=:`/`>=:` derive from it as shared Quoin.
    let b = NativeClassBuilder::new("Double", Some("Object"))
        .sdk_instance_method("sqrt", |host, receiver, _args| {
            let val = recv!(receiver, Double);
            if val < 0.0 {
                return Err(QuoinError::ArithmeticError(
                    "sqrt of a negative Double".to_string(),
                ));
            }
            Ok(host.new_double(val.sqrt()))
        })
        // Rounding to a whole number yields an Integer (range-guarded). `round` is
        // half-away-from-zero (`f64::round`); `truncate` drops the fraction toward zero.
        .sdk_instance_method("floor", |host, receiver, _args| {
            Ok(host.new_int(whole_to_i64(recv!(receiver, Double).floor())?))
        })
        .sdk_instance_method("ceil", |host, receiver, _args| {
            Ok(host.new_int(whole_to_i64(recv!(receiver, Double).ceil())?))
        })
        .sdk_instance_method("round", |host, receiver, _args| {
            Ok(host.new_int(whole_to_i64(recv!(receiver, Double).round())?))
        })
        .sdk_instance_method("truncate", |host, receiver, _args| {
            Ok(host.new_int(whole_to_i64(recv!(receiver, Double).trunc())?))
        })
        // -1.0 / 0.0 / 1.0 by sign (NaN -> NaN; `f64::signum` would call +0.0 positive).
        .sdk_instance_method("sign", |host, receiver, _args| {
            let val = recv!(receiver, Double);
            let s = if val.is_nan() {
                f64::NAN
            } else if val > 0.0 {
                1.0
            } else if val < 0.0 {
                -1.0
            } else {
                0.0
            };
            Ok(host.new_double(s))
        })
        // Human string form. Explicit so `.s` never routes through the Rust Display impl.
        .sdk_instance_method("s", |host, receiver, _args| {
            let val = recv!(receiver, Double);
            Ok(host.new_string(format!("{val}")))
        });
    let b = double_binop!(b, "+:", arith+);
    let b = double_binop!(b, "-:", arith -);
    let b = double_binop!(b, "*:", arith *);
    let b = double_binop!(b, "/:", arith /);
    let b = double_binop!(b, "%:", arith %);
    let b = double_binop!(b, "<:", cmp <);
    // pow: — a Double base always yields a Double (both arg types coerce via `as_f64`).
    let b = b
        .sdk_typed_instance_method("pow:", &["Integer"], |host, receiver, args| {
            Ok(host.new_double(receiver.as_f64().unwrap().powf(args[0].as_f64().unwrap())))
        })
        .sdk_typed_instance_method("pow:", &["Double"], |host, receiver, args| {
            Ok(host.new_double(receiver.as_f64().unwrap().powf(args[0].as_f64().unwrap())))
        });
    // min:/max: select the winning operand and return it in its own type (see integer.rs); a
    // Double receiver compares on the f64 scale regardless of the argument's type.
    let b = b
        .sdk_typed_instance_method("min:", &["Integer"], |_host, receiver, args| {
            Ok(if receiver.as_f64().unwrap() <= args[0].as_f64().unwrap() {
                receiver
            } else {
                args[0]
            })
        })
        .sdk_typed_instance_method("min:", &["Double"], |_host, receiver, args| {
            Ok(if receiver.as_f64().unwrap() <= args[0].as_f64().unwrap() {
                receiver
            } else {
                args[0]
            })
        })
        .sdk_typed_instance_method("max:", &["Integer"], |_host, receiver, args| {
            Ok(if receiver.as_f64().unwrap() >= args[0].as_f64().unwrap() {
                receiver
            } else {
                args[0]
            })
        })
        .sdk_typed_instance_method("max:", &["Double"], |_host, receiver, args| {
            Ok(if receiver.as_f64().unwrap() >= args[0].as_f64().unwrap() {
                receiver
            } else {
                args[0]
            })
        });
    b.sdk_instance_method("==:", |host, receiver, args| {
        Ok(host.new_bool(receiver == args[0]))
    })
}
