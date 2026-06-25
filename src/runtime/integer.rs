use crate::error::QuoinError;
use crate::value::{NativeClassBuilder, Value};
use crate::{arg, recv};

/// Generate `[Integer]` and `[Double]` typed variants for a binary numeric
/// operator on an `Integer` receiver. `Int op Int` stays `Int`; a `Double` RHS
/// promotes the result to `Double` (`as_i64`/`as_f64` are the coercion helpers).
/// `divop` additionally guards Integer division/modulo by zero. A non-numeric RHS
/// matches no variant and falls through to the rekeyed global fallback in
/// `native.rs`. (Receiver and arg are scorer-guaranteed, so the coercions are total.)
macro_rules! int_binop {
    ($b:expr, $sel:literal, arith $op:tt) => {
        $b.typed_instance_method($sel, &["Integer"], |vm, mc, receiver, args| {
            Ok(vm.new_int(mc, receiver.as_i64().unwrap() $op args[0].as_i64().unwrap()))
        })
        .typed_instance_method($sel, &["Double"], |vm, mc, receiver, args| {
            Ok(vm.new_double(mc, receiver.as_f64().unwrap() $op args[0].as_f64().unwrap()))
        })
    };
    ($b:expr, $sel:literal, divop $op:tt) => {
        $b.typed_instance_method($sel, &["Integer"], |vm, mc, receiver, args| {
            let divisor = args[0].as_i64().unwrap();
            if divisor == 0 {
                return Err(QuoinError::ArithmeticError("Division by zero".to_string()));
            }
            Ok(vm.new_int(mc, receiver.as_i64().unwrap() $op divisor))
        })
        .typed_instance_method($sel, &["Double"], |vm, mc, receiver, args| {
            Ok(vm.new_double(mc, receiver.as_f64().unwrap() $op args[0].as_f64().unwrap()))
        })
    };
    ($b:expr, $sel:literal, cmp $op:tt) => {
        $b.typed_instance_method($sel, &["Integer"], |vm, mc, receiver, args| {
            Ok(vm.new_bool(mc, receiver.as_i64().unwrap() $op args[0].as_i64().unwrap()))
        })
        .typed_instance_method($sel, &["Double"], |vm, mc, receiver, args| {
            Ok(vm.new_bool(mc, receiver.as_f64().unwrap() $op args[0].as_f64().unwrap()))
        })
    };
}

pub fn build_integer_class() -> NativeClassBuilder {
    // Binary operators are the `:` keyword selectors (`a + b` -> `Send(a, "+:", [b])`);
    // the bare forms are reserved for unary operators.
    let b = NativeClassBuilder::new("Integer", Some("Object"))
        .instance_method("sqrt", |vm, mc, receiver, _args| {
            let val = recv!(receiver, Int);
            if val < 0 {
                return Err(QuoinError::ArithmeticError(
                    "sqrt of a negative Integer".to_string(),
                ));
            }
            Ok(vm.new_double(mc, (val as f64).sqrt()))
        })
        // `floor`/`ceil`/`round`/`truncate` are identities on a whole number — return the
        // receiver unchanged, so the surface matches Double's (where they round to Integer).
        .instance_method("floor", |_vm, _mc, receiver, _args| Ok(receiver))
        .instance_method("ceil", |_vm, _mc, receiver, _args| Ok(receiver))
        .instance_method("round", |_vm, _mc, receiver, _args| Ok(receiver))
        .instance_method("truncate", |_vm, _mc, receiver, _args| Ok(receiver))
        // -1 / 0 / 1 by sign.
        .instance_method("sign", |vm, mc, receiver, _args| {
            let val = recv!(receiver, Int);
            Ok(vm.new_int(mc, val.signum()))
        })
        // Human string form — the decimal digits. Explicit so `.s` never routes through the
        // Rust Display impl (which is the default `Object.s` fallback this replaces).
        .instance_method("s", |vm, mc, receiver, _args| {
            let val = recv!(receiver, Int);
            Ok(vm.new_string(mc, val.to_string()))
        });
    let b = int_binop!(b, "+:", arith+);
    let b = int_binop!(b, "-:", arith -);
    let b = int_binop!(b, "*:", arith *);
    let b = int_binop!(b, "/:", divop /);
    let b = int_binop!(b, "%:", divop %);
    // Only `<:` is native; `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
    let b = int_binop!(b, "<:", cmp <);
    // pow: — Int**Int stays Integer but is *checked* (overflow -> ArithmeticError, since there is
    // no auto-promotion to BigInteger); a negative exponent leaves the integer domain and returns
    // a Double (`2.pow: -1` -> 0.5). A Double exponent always yields a Double.
    let b = b
        .typed_instance_method("pow:", &["Integer"], |vm, mc, receiver, args| {
            let base = receiver.as_i64().unwrap();
            let exp = args[0].as_i64().unwrap();
            if exp < 0 {
                return Ok(vm.new_double(mc, (base as f64).powf(exp as f64)));
            }
            let e = u32::try_from(exp)
                .map_err(|_| QuoinError::ArithmeticError(format!("exponent {exp} too large")))?;
            match base.checked_pow(e) {
                Some(r) => Ok(vm.new_int(mc, r)),
                None => Err(QuoinError::ArithmeticError(format!(
                    "{base} ** {exp} overflows Integer"
                ))),
            }
        })
        .typed_instance_method("pow:", &["Double"], |vm, mc, receiver, args| {
            Ok(vm.new_double(
                mc,
                (receiver.as_i64().unwrap() as f64).powf(args[0].as_f64().unwrap()),
            ))
        });
    // min:/max: *select* the winning operand and return it in its own type, so a mixed
    // Integer/Double comparison keeps the winner's type (`5.max: 3.0` -> 5; `5.max: 7.0` -> 7.0).
    // Same-type compares natively (i64 for two Integers, avoiding f64 precision loss); a mixed
    // compare promotes to f64 only to decide the winner, never to build the result.
    let b = b
        .typed_instance_method("min:", &["Integer"], |_vm, _mc, receiver, args| {
            Ok(if receiver.as_i64().unwrap() <= args[0].as_i64().unwrap() {
                receiver
            } else {
                args[0]
            })
        })
        .typed_instance_method("min:", &["Double"], |_vm, _mc, receiver, args| {
            Ok(if receiver.as_f64().unwrap() <= args[0].as_f64().unwrap() {
                receiver
            } else {
                args[0]
            })
        })
        .typed_instance_method("max:", &["Integer"], |_vm, _mc, receiver, args| {
            Ok(if receiver.as_i64().unwrap() >= args[0].as_i64().unwrap() {
                receiver
            } else {
                args[0]
            })
        })
        .typed_instance_method("max:", &["Double"], |_vm, _mc, receiver, args| {
            Ok(if receiver.as_f64().unwrap() >= args[0].as_f64().unwrap() {
                receiver
            } else {
                args[0]
            })
        });
    let b = b.instance_method("==:", |vm, mc, receiver, args| {
        Ok(vm.new_bool(mc, receiver == args[0]))
    });
    // Integer.fromHex: 'ff' -> 255. Parses a hexadecimal string (surrounding whitespace
    // ignored; an optional '0x'/'0X' prefix accepted); throws on a non-hex string. Used
    // e.g. for HTTP chunk sizes so that logic can stay in Quoin.
    b.typed_class_method("fromHex:", &["String"], |vm, mc, _receiver, args| {
        let s = arg!(args, String, 0);
        let trimmed = s.trim();
        let hex = trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
            .unwrap_or(trimmed);
        match i64::from_str_radix(hex, 16) {
            Ok(n) => Ok(vm.new_int(mc, n)),
            Err(_) => Err(QuoinError::ValueError(format!(
                "Integer.fromHex:: not a hexadecimal integer: '{}'",
                s.as_str()
            ))),
        }
    })
}
