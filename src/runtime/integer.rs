use crate::error::QuoinError;
use crate::instruction::IntBinKind;
use crate::value::{NativeClassBuilder, Value};
use crate::{arg, recv};

// A binary operator on an Integer receiver, for the given `IntBinKind`. The Integer-arg variant
// and the DoubleXxx-arg (coercing) variant both dispatch through the shared `devirt_ops` verbs —
// the SAME functions the devirtualized `IntAdd`/`DoubleAdd` VM ops call — so the native and
// devirt implementations can't drift (div-by-zero, wrap, IEEE semantics all live in one place).
macro_rules! int_binop {
    ($b:expr, $sel:literal, $kind:expr) => {
        $b.sdk_typed_instance_method($sel, &["Integer"], |host, receiver, args| {
            match crate::devirt_ops::int_bin(
                $kind,
                receiver.as_i64().unwrap(),
                args[0].as_i64().unwrap(),
            )? {
                crate::devirt_ops::IntBinOut::Int(i) => Ok(host.new_int(i)),
                crate::devirt_ops::IntBinOut::Bool(b) => Ok(host.new_bool(b)),
            }
        })
        // Integer op with a Double argument: both coerce to f64, so the result is a Double.
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

pub fn build_integer_class() -> NativeClassBuilder {
    // Binary operators are the `:` keyword selectors (`a + b` -> `Send(a, "+:", [b])`);
    // the bare forms are reserved for unary operators.
    let b = NativeClassBuilder::new("Integer", Some("Object"))
        .construct_with("use integer literals (42)")
        .class_doc(
            "A 64-bit signed whole number -- the type of integer literals like `42`.\n\n\
             Arithmetic between two Integers stays an Integer (`/` truncates toward zero); \
             an operation with a Double operand is carried out in floating point and yields \
             a Double. There is no automatic promotion to arbitrary precision -- convert \
             explicitly with `BigInteger.of:` when a value can outgrow 64 bits.\n\n\
             ```\n\
             7 / 2       \"* -> 3\n\
             7 / 2.0     \"* -> 3.5\n\
             ```",
        )
        .sdk_instance_method("sqrt", |host, receiver, _args| {
            let val = recv!(receiver, Int);
            if val < 0 {
                return Err(QuoinError::ArithmeticError(
                    "sqrt of a negative Integer".to_string(),
                ));
            }
            Ok(host.new_double((val as f64).sqrt()))
        })
        .returns("Double")
        .doc(
            "The square root, as a Double. Raises an ArithmeticError for a negative \
             receiver.\n\n\
             ```\n\
             2.sqrt     \"* -> 1.4142135623730951\n\
             ```",
        )
        // `floor`/`ceil`/`round`/`truncate` are identities on a whole number — return the
        // receiver unchanged, so the surface matches Double's (where they round to Integer).
        .sdk_instance_method("floor", |_host, receiver, _args| Ok(receiver))
        .returns("Integer")
        .doc(
            "The receiver itself: a whole number is its own floor. Present so Integer \
             mirrors Double's rounding surface (where `floor` rounds down to an Integer).",
        )
        .sdk_instance_method("ceil", |_host, receiver, _args| Ok(receiver))
        .returns("Integer")
        .doc("The receiver itself: a whole number is its own ceiling. Mirrors `Double.ceil`.")
        .sdk_instance_method("round", |_host, receiver, _args| Ok(receiver))
        .returns("Integer")
        .doc("The receiver itself: a whole number is already rounded. Mirrors `Double.round`.")
        .sdk_instance_method("truncate", |_host, receiver, _args| Ok(receiver))
        .returns("Integer")
        .doc(
            "The receiver itself: a whole number has no fraction to drop. Mirrors \
             `Double.truncate`.",
        )
        // -1 / 0 / 1 by sign.
        .sdk_instance_method("sign", |host, receiver, _args| {
            let val = recv!(receiver, Int);
            Ok(host.new_int(val.signum()))
        })
        .returns("Integer")
        .doc(
            "-1, 0, or 1 by the receiver's sign.\n\n\
             ```\n\
             (0 - 5).sign     \"* -> -1\n\
             0.sign           \"* -> 0\n\
             ```",
        )
        // Human string form — the decimal digits. Explicit so `.s` never routes through the
        // Rust Display impl (which is the default `Object.s` fallback this replaces).
        .sdk_instance_method("s", |host, receiver, _args| {
            let val = recv!(receiver, Int);
            Ok(host.new_string(val.to_string()))
        })
        .doc(
            "The decimal digits as a String.\n\n\
             ```\n\
             1024.s     \"* -> 1024\n\
             ```",
        );
    let b = int_binop!(b, "+:", IntBinKind::Add).doc(
        "Addition (`a + b`). Integer + Integer yields an Integer; a Double operand makes \
         the result a Double.\n\n\
         ```\n\
         1 + 2.5     \"* -> 3.5\n\
         ```",
    );
    let b = int_binop!(b, "-:", IntBinKind::Sub).doc(
        "Subtraction (`a - b`). Integer - Integer yields an Integer; a Double operand makes \
         the result a Double.\n\n\
         ```\n\
         10 - 3     \"* -> 7\n\
         ```",
    );
    let b = int_binop!(b, "*:", IntBinKind::Mul).doc(
        "Multiplication (`a * b`). Integer * Integer yields an Integer; a Double operand \
         makes the result a Double.\n\n\
         ```\n\
         6 * 7     \"* -> 42\n\
         ```",
    );
    let b = int_binop!(b, "/:", IntBinKind::Div).doc(
        "Division (`a / b`). Between two Integers the quotient truncates toward zero, and \
         dividing by zero raises an ArithmeticError; a Double operand gives true \
         floating-point division.\n\n\
         ```\n\
         7 / 2           \"* -> 3\n\
         (0 - 7) / 2     \"* -> -3\n\
         7 / 2.0         \"* -> 3.5\n\
         ```",
    );
    let b = int_binop!(b, "%:", IntBinKind::Mod).doc(
        "The remainder after truncating division (`a % b`); the result takes the \
         dividend's sign. A zero Integer divisor raises an ArithmeticError; a Double \
         operand yields a Double remainder.\n\n\
         ```\n\
         7 % 3           \"* -> 1\n\
         (0 - 7) % 3     \"* -> -1\n\
         ```",
    );
    // Only `<:` is native; `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
    let b = int_binop!(b, "<:", IntBinKind::Lt).doc(
        "Whether the receiver is less than the argument (`a < b`, Integer or Double). The \
         one native comparison -- `>`, `<=` and `>=` all derive from it.\n\n\
         ```\n\
         2 < 3     \"* -> true\n\
         ```",
    );
    // pow: — Int**Int stays Integer but is *checked* (overflow -> ArithmeticError, since there is
    // no auto-promotion to BigInteger); a negative exponent leaves the integer domain and returns
    // a Double (`2.pow: -1` -> 0.5). A Double exponent always yields a Double.
    let b = b
        .sdk_typed_instance_method("pow:", &["Integer"], |host, receiver, args| {
            let base = receiver.as_i64().unwrap();
            let exp = args[0].as_i64().unwrap();
            if exp < 0 {
                return Ok(host.new_double((base as f64).powf(exp as f64)));
            }
            let e = u32::try_from(exp)
                .map_err(|_| QuoinError::ArithmeticError(format!("exponent {exp} too large")))?;
            match base.checked_pow(e) {
                Some(r) => Ok(host.new_int(r)),
                None => Err(QuoinError::ArithmeticError(format!(
                    "{base} ** {exp} overflows Integer"
                ))),
            }
        })
        .doc(
            "The receiver raised to the argument's power. A non-negative Integer exponent \
             stays an Integer and is overflow-checked (an out-of-range result raises an \
             ArithmeticError -- there is no auto-promotion to BigInteger); a negative or \
             Double exponent leaves the integer domain and yields a Double.\n\n\
             ```\n\
             2.pow: 10        \"* -> 1024\n\
             2.pow: 0 - 1     \"* -> 0.5\n\
             ```",
        )
        .sdk_typed_instance_method("pow:", &["Double"], |host, receiver, args| {
            Ok(
                host.new_double(
                    (receiver.as_i64().unwrap() as f64).powf(args[0].as_f64().unwrap()),
                ),
            )
        });
    // min:/max: *select* the winning operand and return it in its own type, so a mixed
    // Integer/Double comparison keeps the winner's type (`5.max: 3.0` -> 5; `5.max: 7.0` -> 7.0).
    // Same-type compares natively (i64 for two Integers, avoiding f64 precision loss); a mixed
    // compare promotes to f64 only to decide the winner, never to build the result.
    let b = b
        .sdk_typed_instance_method("min:", &["Integer"], |_host, receiver, args| {
            Ok(if receiver.as_i64().unwrap() <= args[0].as_i64().unwrap() {
                receiver
            } else {
                args[0]
            })
        })
        .doc(
            "The smaller of the receiver and the argument, returned as the winning operand \
             itself -- a mixed Integer/Double comparison keeps the winner's own type.\n\n\
             ```\n\
             3.min: 9     \"* -> 3\n\
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
            Ok(if receiver.as_i64().unwrap() >= args[0].as_i64().unwrap() {
                receiver
            } else {
                args[0]
            })
        })
        .doc(
            "The larger of the receiver and the argument, returned as the winning operand \
             itself -- a mixed Integer/Double comparison keeps the winner's own type (so \
             `5.max: 7.0` is the Double 7.0).\n\n\
             ```\n\
             5.max: 3.0     \"* -> 5\n\
             ```",
        )
        .sdk_typed_instance_method("max:", &["Double"], |_host, receiver, args| {
            Ok(if receiver.as_f64().unwrap() >= args[0].as_f64().unwrap() {
                receiver
            } else {
                args[0]
            })
        });
    let b = b
        .sdk_instance_method("==:", |host, receiver, args| {
            Ok(host.new_bool(receiver == args[0]))
        })
        .doc(
            "Numeric equality with any value. Integers and Doubles compare by numeric \
             value; a non-number is simply unequal, never an error.\n\n\
             ```\n\
             5 == 5.0     \"* -> true\n\
             5 == 'a'     \"* -> false\n\
             ```",
        );
    // Integer.fromHex: 'ff' -> 255. Parses a hexadecimal string (surrounding whitespace
    // ignored; an optional '0x'/'0X' prefix accepted); throws on a non-hex string. Used
    // e.g. for HTTP chunk sizes so that logic can stay in Quoin.
    b.sdk_typed_class_method("fromHex:", &["String"], |host, _receiver, args| {
        let s = arg!(args, String, 0);
        let trimmed = s.trim();
        let hex = trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
            .unwrap_or(trimmed);
        match i64::from_str_radix(hex, 16) {
            Ok(n) => Ok(host.new_int(n)),
            Err(_) => Err(QuoinError::ValueError(format!(
                "Integer.fromHex:: not a hexadecimal integer: '{}'",
                s.as_str()
            ))),
        }
    })
    .doc(
        "Parse a hexadecimal String into an Integer -- surrounding whitespace is ignored \
         and an optional `0x`/`0X` prefix is accepted; raises a ValueError for a non-hex \
         string. Handy where protocols carry hex, e.g. HTTP chunk sizes.\n\n\
         ```\n\
         Integer.fromHex:'ff'       \"* -> 255\n\
         Integer.fromHex:'0x1A'     \"* -> 26\n\
         ```",
    )
}
