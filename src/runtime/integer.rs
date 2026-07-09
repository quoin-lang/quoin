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
        // `floor`/`ceil`/`round`/`truncate` are identities on a whole number — return the
        // receiver unchanged, so the surface matches Double's (where they round to Integer).
        .sdk_instance_method("floor", |_host, receiver, _args| Ok(receiver))
        .returns("Integer")
        .sdk_instance_method("ceil", |_host, receiver, _args| Ok(receiver))
        .returns("Integer")
        .sdk_instance_method("round", |_host, receiver, _args| Ok(receiver))
        .returns("Integer")
        .sdk_instance_method("truncate", |_host, receiver, _args| Ok(receiver))
        .returns("Integer")
        // -1 / 0 / 1 by sign.
        .sdk_instance_method("sign", |host, receiver, _args| {
            let val = recv!(receiver, Int);
            Ok(host.new_int(val.signum()))
        })
        .returns("Integer")
        // Human string form — the decimal digits. Explicit so `.s` never routes through the
        // Rust Display impl (which is the default `Object.s` fallback this replaces).
        .sdk_instance_method("s", |host, receiver, _args| {
            let val = recv!(receiver, Int);
            Ok(host.new_string(val.to_string()))
        });
    let b = int_binop!(b, "+:", IntBinKind::Add);
    let b = int_binop!(b, "-:", IntBinKind::Sub);
    let b = int_binop!(b, "*:", IntBinKind::Mul);
    let b = int_binop!(b, "/:", IntBinKind::Div);
    let b = int_binop!(b, "%:", IntBinKind::Mod);
    // Only `<:` is native; `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
    let b = int_binop!(b, "<:", IntBinKind::Lt);
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
        .sdk_typed_instance_method("max:", &["Double"], |_host, receiver, args| {
            Ok(if receiver.as_f64().unwrap() >= args[0].as_f64().unwrap() {
                receiver
            } else {
                args[0]
            })
        });
    let b = b.sdk_instance_method("==:", |host, receiver, args| {
        Ok(host.new_bool(receiver == args[0]))
    });
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
}
