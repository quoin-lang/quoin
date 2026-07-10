use crate::arg;
use crate::error::QuoinError;
use crate::ext_sdk::{Host, HostExt};
use crate::runtime::pretty::{PpChild, PpRole, PpShape, PrettyPrint};
use crate::value::{AnyCollect, NativeClassBuilder, Value};

use gc_arena::collect::Trace;
use num_bigint::BigInt;
use num_traits::{Pow, Signed, ToPrimitive, Zero};
use std::any::Any;
use std::str::FromStr;

/// Native backing state for a `BigInteger`: one arbitrary-precision `num_bigint::BigInt`.
/// Heap-allocated and `Clone` (not `Copy`), so it is extracted by cloning. No `Gc` fields and
/// no OS resource — `trace_gc` is empty and there is no reap-on-drop.
#[derive(Debug)]
pub struct NativeBigInteger(pub BigInt);

impl AnyCollect for NativeBigInteger {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

impl PrettyPrint for NativeBigInteger {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        // sign ∈ {-1, 0, 1}; magnitude is the unsigned base-10 digits (arbitrary width → a leaf).
        let n = &self.0;
        let sign = if n.is_negative() {
            -1
        } else if n.is_positive() {
            1
        } else {
            0
        };
        PpShape::Record {
            name: "BigInteger",
            fields: vec![
                ("sign".to_string(), PpChild::Val(Value::Int(sign))),
                (
                    "magnitude".to_string(),
                    PpChild::Text(n.magnitude().to_string(), PpRole::Number),
                ),
            ],
        }
    }
}

/// The `BigInt` behind a `BigInteger` value (cloned out). Errors clearly if `v` is not a
/// `BigInteger`; arithmetic requires explicit conversion, so a foreign operand never coerces.
fn bigint_of(v: Value, who: &str) -> Result<BigInt, QuoinError> {
    v.with_native_state::<NativeBigInteger, _, _>(|d| d.0.clone())
        .map_err(|_| QuoinError::TypeError {
            expected: "BigInteger".to_string(),
            got: "a non-BigInteger value".to_string(),
            msg: format!("{who} requires a BigInteger operand (convert with BigInteger.of:)"),
        })
}

pub fn make_bigint<'gc>(host: &dyn Host<'gc>, n: BigInt) -> Value<'gc> {
    let class = host.get_or_create_builtin_class("BigInteger");
    host.new_native_state(class, NativeBigInteger(n))
}

pub fn build_big_integer_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("BigInteger", Some("Object"))
        .construct_with("use BigInteger.of:")
        .class_doc(
            "An arbitrary-precision integer. Construct explicitly with `BigInteger.of:` \
             (a decimal String or an Integer); arithmetic never mixes silently -- a plain \
             Integer operand is 'message not understood', so convert both sides first. \
             Division truncates toward zero, like Integer.\n\n\
             ```\n\
             (BigInteger.of:2).pow:100     \"* -> 1267650600228229401496703205376\n\
             ```",
        )
        // BigInteger.of:'123456789012345678901234567890' — parse from a decimal string.
        .sdk_typed_class_method("of:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            match BigInt::from_str(s.as_str()) {
                Ok(n) => Ok(make_bigint(host, n)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "BigInteger.of:: not an integer: '{}'",
                    s.as_str()
                ))),
            }
        })
        .doc(
            "A BigInteger from a decimal String (any magnitude; raises a ValueError for a \
             non-integer string) or from an Integer.\n\n\
             ```\n\
             BigInteger.of:'123456789012345678901234567890'     \"* -> \
             123456789012345678901234567890\n\
             BigInteger.of:42                                    \"* -> 42\n\
             ```",
        )
        // BigInteger.of:42 — from an Integer (also reached via `42.asBigInteger`).
        .sdk_typed_class_method("of:", &["Integer"], |host, _r, args| {
            Ok(make_bigint(host, BigInt::from(arg!(args, Int, 0))))
        });
    // Arithmetic is BigInteger-only (explicit conversion); a foreign operand matches no typed
    // variant and surfaces as a "message not understood" naming the `:BigInteger` signature.
    let b = b
        .sdk_typed_instance_method("+:", &["BigInteger"], |host, receiver, args| {
            Ok(make_bigint(
                host,
                bigint_of(receiver, "+:")? + bigint_of(args[0], "+:")?,
            ))
        })
        .doc(
            "The sum of two BigIntegers, at any magnitude.\n\n\
             ```\n\
             (BigInteger.of:10) + (BigInteger.of:32)     \"* -> 42\n\
             ```",
        )
        .sdk_typed_instance_method("-:", &["BigInteger"], |host, receiver, args| {
            Ok(make_bigint(
                host,
                bigint_of(receiver, "-:")? - bigint_of(args[0], "-:")?,
            ))
        })
        .doc("The difference of two BigIntegers.")
        .sdk_typed_instance_method("*:", &["BigInteger"], |host, receiver, args| {
            Ok(make_bigint(
                host,
                bigint_of(receiver, "*:")? * bigint_of(args[0], "*:")?,
            ))
        })
        .doc("The product of two BigIntegers.")
        .sdk_typed_instance_method("/:", &["BigInteger"], |host, receiver, args| {
            let divisor = bigint_of(args[0], "/:")?;
            if divisor.is_zero() {
                return Err(QuoinError::ArithmeticError(
                    "BigInteger division by zero".to_string(),
                ));
            }
            // Truncates toward zero (Rust BigInt `/` semantics).
            Ok(make_bigint(host, bigint_of(receiver, "/:")? / divisor))
        })
        .doc(
            "The quotient of two BigIntegers, truncated toward zero (matching Integer \
             `/`); raises an ArithmeticError on a zero divisor.\n\n\
             ```\n\
             (BigInteger.of:100) / (BigInteger.of:7)     \"* -> 14\n\
             ```",
        )
        .sdk_typed_instance_method("%:", &["BigInteger"], |host, receiver, args| {
            let divisor = bigint_of(args[0], "%:")?;
            if divisor.is_zero() {
                return Err(QuoinError::ArithmeticError(
                    "BigInteger modulo by zero".to_string(),
                ));
            }
            Ok(make_bigint(host, bigint_of(receiver, "%:")? % divisor))
        })
        .doc(
            "The remainder after truncating division; takes the dividend's sign. Raises \
             an ArithmeticError on a zero divisor.\n\n\
             ```\n\
             (BigInteger.of:100) % (BigInteger.of:7)     \"* -> 2\n\
             ```",
        )
        // Only `<:` is native; `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
        .sdk_typed_instance_method("<:", &["BigInteger"], |host, receiver, args| {
            Ok(host.new_bool(bigint_of(receiver, "<:")? < bigint_of(args[0], "<:")?))
        })
        .doc(
            "Whether the receiver is less than the BigInteger argument. The one native \
             comparison -- `>`, `<=` and `>=` derive from it.",
        )
        // `==:` accepts any argument: a non-BigInteger is simply unequal (never an error).
        .sdk_instance_method("==:", |host, receiver, args| {
            let a = bigint_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeBigInteger, _, _>(|d| d.0.clone()) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(host.new_bool(eq))
        })
        .doc(
            "Whether the argument is an equal BigInteger. Any other type is simply \
             unequal, never an error -- including a plain Integer of the same value.\n\n\
             ```\n\
             (BigInteger.of:2) == (BigInteger.of:2)     \"* -> true\n\
             (BigInteger.of:2) == 2                     \"* -> false\n\
             ```",
        );
    b.sdk_instance_method("abs", |host, receiver, _args| {
        Ok(make_bigint(host, bigint_of(receiver, "abs")?.abs()))
    })
    .doc(
        "The absolute value.\n\n\
         ```\n\
         (BigInteger.of:'-7').abs     \"* -> 7\n\
         ```",
    )
    // pow: a non-negative Integer exponent. A negative exponent isn't an integer and there is
    // no Double escape for BigInteger, so it errors rather than silently changing type.
    .sdk_typed_instance_method("pow:", &["Integer"], |host, receiver, args| {
        let exp = arg!(args, Int, 0);
        let e = u32::try_from(exp).map_err(|_| {
            QuoinError::ArithmeticError(format!("BigInteger pow:: exponent out of range: {exp}"))
        })?;
        Ok(make_bigint(host, bigint_of(receiver, "pow:")?.pow(e)))
    })
    .doc(
        "The receiver raised to a non-negative Integer exponent, at full precision. A \
         negative exponent raises an ArithmeticError: there is no fractional escape from \
         the integer domain.\n\n\
         ```\n\
         (BigInteger.of:2).pow:100     \"* -> 1267650600228229401496703205376\n\
         ```",
    )
    // Narrow to an Integer (errors if out of i64 range).
    .sdk_instance_method("asInteger", |host, receiver, _args| {
        match bigint_of(receiver, "asInteger")?.to_i64() {
            Some(n) => Ok(host.new_int(n)),
            None => Err(QuoinError::ArithmeticError(
                "BigInteger asInteger: out of Integer range".to_string(),
            )),
        }
    })
    .doc(
        "Narrow to a 64-bit Integer; raises an ArithmeticError if the value does not \
         fit.\n\n\
         ```\n\
         (BigInteger.of:42).asInteger     \"* -> 42\n\
         ```",
    )
    // Lossy conversion to a Double.
    .sdk_instance_method("asDouble", |host, receiver, _args| {
        match bigint_of(receiver, "asDouble")?.to_f64() {
            Some(f) => Ok(host.new_double(f)),
            None => Err(QuoinError::ArithmeticError(
                "BigInteger asDouble: not representable as a Double".to_string(),
            )),
        }
    })
    .doc(
        "Convert to a Double, losing precision beyond a Double's ~15-17 significant \
         digits.\n\n\
         ```\n\
         (BigInteger.of:'12345678901234567890').asDouble     \"* -> 12345678901234567000\n\
         ```",
    )
    // Canonical decimal string.
    .sdk_instance_method("s", |host, receiver, _args| {
        Ok(host.new_string(bigint_of(receiver, "s")?.to_string()))
    })
    .doc("The value as decimal digits, with a leading `-` when negative.")
}
