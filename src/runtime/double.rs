use crate::error::QuoinError;
use crate::instruction::IntBinKind;
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

// A binary operator on a Double receiver, for the given `IntBinKind`. Both arg types coerce to
// f64, so both variants share the `devirt_ops::double_bin` verb — the SAME function the
// devirtualized `DoubleXxx` VM ops call, so native and devirt can't drift.
macro_rules! double_binop {
    ($b:expr, $sel:literal, $kind:expr) => {
        $b.sdk_typed_instance_method($sel, &["Integer"], |host, receiver, args| {
            match crate::devirt_ops::double_bin(
                $kind,
                receiver.as_f64().unwrap(),
                args[0].as_f64().unwrap(),
            ) {
                crate::devirt_ops::DoubleBinOut::Double(d) => Ok(host.new_double(d)),
                crate::devirt_ops::DoubleBinOut::Bool(b) => Ok(host.new_bool(b)),
            }
        })
        .sdk_typed_instance_method($sel, &["Double"], |host, receiver, args| {
            match crate::devirt_ops::double_bin(
                $kind,
                receiver.as_f64().unwrap(),
                args[0].as_f64().unwrap(),
            ) {
                crate::devirt_ops::DoubleBinOut::Double(d) => Ok(host.new_double(d)),
                crate::devirt_ops::DoubleBinOut::Bool(b) => Ok(host.new_bool(b)),
            }
        })
    };
}

pub fn build_double_class() -> NativeClassBuilder {
    // Binary operators are the `:` keyword selectors (`a + b` -> `Send(a, "+:", [b])`).
    // Only `<:` is provided natively; `>:`/`<=:`/`>=:` derive from it as shared Quoin.
    let b = NativeClassBuilder::new("Double", Some("Object"))
        .construct_with("use number literals (3.14)")
        .class_doc(
            "A 64-bit IEEE-754 floating-point number -- the type of literals like `3.14`. \
             Arithmetic with either an Integer or a Double operand happens in floating \
             point and yields a Double. Whole values print without a decimal point \
             (`4.0.s` is '4').\n\n\
             Infinity and NaN follow IEEE rules: `1.0 / 0.0` is `Double.inf`, `0.0 / 0.0` \
             is `Double.nan`, and NaN compares unequal to everything, itself included.\n\n\
             ```\n\
             0.1 + 0.2     \"* -> 0.30000000000000004\n\
             1.0 / 0.0     \"* -> inf\n\
             ```",
        )
        .sdk_instance_method("sqrt", |host, receiver, _args| {
            let val = recv!(receiver, Double);
            if val < 0.0 {
                return Err(QuoinError::ArithmeticError(
                    "sqrt of a negative Double".to_string(),
                ));
            }
            Ok(host.new_double(val.sqrt()))
        })
        .returns("Double")
        .doc(
            "The square root. Raises an ArithmeticError for a negative receiver.\n\n\
             ```\n\
             2.25.sqrt     \"* -> 1.5\n\
             ```",
        )
        // Rounding to a whole number yields an Integer (range-guarded). `round` is
        // half-away-from-zero (`f64::round`); `truncate` drops the fraction toward zero.
        .sdk_instance_method("floor", |host, receiver, _args| {
            Ok(host.new_int(whole_to_i64(recv!(receiver, Double).floor())?))
        })
        .returns("Integer")
        .doc(
            "The largest whole number at or below the receiver, as an Integer. Raises an \
             ArithmeticError if the result falls outside the 64-bit Integer range (there \
             is no auto-promotion to BigInteger).\n\n\
             ```\n\
             2.75.floor     \"* -> 2\n\
             ```",
        )
        .sdk_instance_method("ceil", |host, receiver, _args| {
            Ok(host.new_int(whole_to_i64(recv!(receiver, Double).ceil())?))
        })
        .returns("Integer")
        .doc(
            "The smallest whole number at or above the receiver, as an Integer \
             (range-guarded like `floor`).\n\n\
             ```\n\
             2.25.ceil     \"* -> 3\n\
             ```",
        )
        .sdk_instance_method("round", |host, receiver, _args| {
            Ok(host.new_int(whole_to_i64(recv!(receiver, Double).round())?))
        })
        .returns("Integer")
        .doc(
            "The nearest whole number as an Integer, rounding halves away from zero \
             (range-guarded like `floor`).\n\n\
             ```\n\
             2.5.round           \"* -> 3\n\
             (0 - 2.5).round     \"* -> -3\n\
             ```",
        )
        .sdk_instance_method("truncate", |host, receiver, _args| {
            Ok(host.new_int(whole_to_i64(recv!(receiver, Double).trunc())?))
        })
        .returns("Integer")
        .doc(
            "The whole part as an Integer, dropping the fraction toward zero \
             (range-guarded like `floor`).\n\n\
             ```\n\
             (0 - 7.5).truncate     \"* -> -7\n\
             ```",
        )
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
        .returns("Double")
        .doc(
            "-1.0, 0.0, or 1.0 by the receiver's sign; NaN stays NaN, and both zeroes \
             count as 0.0.\n\n\
             ```\n\
             (0 - 1.5).sign     \"* -> -1\n\
             0.0.sign           \"* -> 0\n\
             ```",
        )
        // Human string form. Explicit so `.s` never routes through the Rust Display impl.
        .sdk_instance_method("s", |host, receiver, _args| {
            let val = recv!(receiver, Double);
            Ok(host.new_string(format!("{val}")))
        })
        .doc(
            "The shortest decimal String that reads back as the same Double; whole values \
             print without a decimal point.\n\n\
             ```\n\
             3.14.s     \"* -> 3.14\n\
             4.0.s      \"* -> 4\n\
             ```",
        )
        // Class-side IEEE-754 constants — handy for tests and boundary math.
        .sdk_class_method("inf", |host, _receiver, _args| {
            Ok(host.new_double(f64::INFINITY))
        })
        .doc(
            "Positive infinity, the IEEE-754 value that `1.0 / 0.0` yields; negate it for \
             negative infinity. Handy for tests and boundary math.\n\n\
             ```\n\
             1.0 / 0.0     \"* -> inf\n\
             ```",
        )
        .sdk_class_method(
            "nan",
            |host, _receiver, _args| Ok(host.new_double(f64::NAN)),
        )
        .doc(
            "The IEEE-754 not-a-number value, e.g. what `0.0 / 0.0` yields. NaN is unequal \
             to everything, including itself.\n\n\
             ```\n\
             Double.nan == Double.nan     \"* -> false\n\
             ```",
        );
    let b = double_binop!(b, "+:", IntBinKind::Add).doc(
        "Addition (`a + b`); the result is always a Double, whether the operand is an \
         Integer or a Double.\n\n\
         ```\n\
         0.5 + 0.25     \"* -> 0.75\n\
         ```",
    );
    let b = double_binop!(b, "-:", IntBinKind::Sub).doc(
        "Subtraction (`a - b`); the result is always a Double.\n\n\
         ```\n\
         3.5 - 0.5     \"* -> 3\n\
         ```",
    );
    let b = double_binop!(b, "*:", IntBinKind::Mul).doc(
        "Multiplication (`a * b`); the result is always a Double.\n\n\
         ```\n\
         1.5 * 2.0     \"* -> 3\n\
         ```",
    );
    let b = double_binop!(b, "/:", IntBinKind::Div).doc(
        "Division (`a / b`); the result is always a Double. Follows IEEE-754: dividing by \
         zero gives an infinity (or NaN for `0.0 / 0.0`), never an error.\n\n\
         ```\n\
         10.0 / 4.0     \"* -> 2.5\n\
         1.0 / 0.0      \"* -> inf\n\
         ```",
    );
    let b = double_binop!(b, "%:", IntBinKind::Mod).doc(
        "The floating-point remainder after truncating division (`a % b`); the result \
         takes the dividend's sign.\n\n\
         ```\n\
         7.5 % 2     \"* -> 1.5\n\
         ```",
    );
    let b = double_binop!(b, "<:", IntBinKind::Lt).doc(
        "Whether the receiver is less than the argument (`a < b`), compared on the \
         floating-point scale. The one native comparison -- `>`, `<=` and `>=` derive \
         from it.\n\n\
         ```\n\
         1.5 < 2     \"* -> true\n\
         ```",
    );
    // pow: — a Double base always yields a Double (both arg types coerce via `as_f64`).
    let b = b
        .sdk_typed_instance_method("pow:", &["Integer"], |host, receiver, args| {
            Ok(host.new_double(receiver.as_f64().unwrap().powf(args[0].as_f64().unwrap())))
        })
        .doc(
            "The receiver raised to the argument's power (Integer or Double exponent); a \
             Double base always yields a Double.\n\n\
             ```\n\
             2.0.pow: 2     \"* -> 4\n\
             ```",
        )
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
        .doc(
            "The smaller of the receiver and the argument, returned as the winning operand \
             itself -- a mixed Double/Integer comparison keeps the winner's own type. The \
             compare happens on the floating-point scale.\n\n\
             ```\n\
             1.5.min: 2     \"* -> 1.5\n\
             ```",
        )
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
        .doc(
            "The larger of the receiver and the argument, returned as the winning operand \
             itself -- a mixed Double/Integer comparison keeps the winner's own type. The \
             compare happens on the floating-point scale.\n\n\
             ```\n\
             1.5.max: 2     \"* -> 2\n\
             ```",
        )
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
    .doc(
        "Numeric equality with any value. Doubles and Integers compare by numeric value; \
         a non-number is simply unequal, never an error. NaN is unequal to everything, \
         itself included.\n\n\
         ```\n\
         5.0 == 5     \"* -> true\n\
         ```",
    )
}
