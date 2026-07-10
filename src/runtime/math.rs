use crate::error::QuoinError;
use crate::value::{NativeClassBuilder, Value};

/// Pull argument `i` as an `f64`, coercing `Integer`/`Double` and erroring on anything else.
/// `Math`'s methods are untyped class methods (one selector, any numeric argument), so they
/// coerce here rather than relying on the per-type dispatch the number classes use.
fn num(args: &[Value], i: usize, who: &str) -> Result<f64, QuoinError> {
    args.get(i)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| QuoinError::ValueError(format!("{who} expects a number")))
}

/// Reject a non-positive argument to `ln`/`log` (domain error), matching the `sqrt`-of-negative
/// convention on the number classes.
fn require_positive(x: f64, who: &str) -> Result<(), QuoinError> {
    if x > 0.0 {
        Ok(())
    } else {
        Err(QuoinError::ArithmeticError(format!(
            "{who} requires a positive argument"
        )))
    }
}

/// `Math` — a namespace of constants and the trig/transcendental free functions, where
/// `Math.sin: x` reads better than a method hung off the number. Number-centric operations
/// (`abs`/`floor`/`sqrt`/`pow:`/`min:`/`max:`) live on `Integer`/`Double`; a few are mirrored
/// here (`sqrt:`, `pow:to:`) for a free-function style. Angles are in radians.
pub fn build_math_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Math", Some("Object"))
        .abstract_class()
        .class_doc(
            "A namespace of mathematical constants and free functions -- `Math.sin: x` \
             where a method hung off the number would read awkwardly. Number-centric \
             operations (`abs`, `floor`, `sqrt`, `pow:`, `min:`, `max:`) live on \
             Integer/Double; `sqrt:` and `pow:to:` are mirrored here in free-function \
             style. Every function accepts an Integer or a Double and returns a Double; \
             angles are in radians.",
        )
        // Constants.
        .sdk_class_method("pi", |host, _r, _a| {
            Ok(host.new_double(std::f64::consts::PI))
        })
        .doc(
            "The circle constant pi.\n\n\
             ```\n\
             Math.pi     \"* -> 3.141592653589793\n\
             ```",
        )
        .sdk_class_method("e", |host, _r, _a| Ok(host.new_double(std::f64::consts::E)))
        .doc(
            "Euler's number, the base of the natural logarithm.\n\n\
             ```\n\
             Math.e     \"* -> 2.718281828459045\n\
             ```",
        )
        .sdk_class_method("tau", |host, _r, _a| {
            Ok(host.new_double(std::f64::consts::TAU))
        })
        .doc(
            "Tau = 2 pi, one full turn in radians.\n\n\
             ```\n\
             Math.tau     \"* -> 6.283185307179586\n\
             ```",
        )
        // Trigonometry.
        .sdk_class_method("sin:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.sin:")?.sin()))
        })
        .doc(
            "The sine of an angle in radians.\n\n\
             ```\n\
             Math.sin:(Math.pi / 2)     \"* -> 1\n\
             ```",
        )
        .sdk_class_method("cos:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.cos:")?.cos()))
        })
        .doc(
            "The cosine of an angle in radians.\n\n\
             ```\n\
             Math.cos:0     \"* -> 1\n\
             ```",
        )
        .sdk_class_method("tan:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.tan:")?.tan()))
        })
        .doc("The tangent of an angle in radians.")
        .sdk_class_method("asin:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.asin:")?.asin()))
        })
        .doc(
            "The arcsine, in radians in [-pi/2, pi/2]; an argument outside [-1, 1] yields \
             NaN.\n\n\
             ```\n\
             Math.asin:1     \"* -> 1.5707963267948966\n\
             ```",
        )
        .sdk_class_method("acos:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.acos:")?.acos()))
        })
        .doc(
            "The arccosine, in radians in [0, pi]; an argument outside [-1, 1] yields \
             NaN.\n\n\
             ```\n\
             Math.acos:1     \"* -> 0\n\
             ```",
        )
        .sdk_class_method("atan:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.atan:")?.atan()))
        })
        .doc(
            "The arctangent, in radians in (-pi/2, pi/2). For the angle of a vector, \
             prefer `atan:over:`, which keeps the quadrant.",
        )
        // atan2(y, x): the angle of the vector (x, y), reading `Math.atan: y over: x`.
        .sdk_class_method("atan:over:", |host, _r, args| {
            let y = num(&args, 0, "Math.atan:over:")?;
            let x = num(&args, 1, "Math.atan:over:")?;
            Ok(host.new_double(y.atan2(x)))
        })
        .doc(
            "The angle of the vector (x, y) in radians in (-pi, pi] -- atan2, reading \
             `Math.atan: y over: x`. Unlike `atan:` of y/x it keeps the quadrant and \
             handles a zero x.\n\n\
             ```\n\
             Math.atan:1 over:1     \"* -> 0.7853981633974483\n\
             ```",
        )
        // Exponential / logarithmic. ln/log domains are guarded to x > 0.
        .sdk_class_method("exp:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.exp:")?.exp()))
        })
        .doc(
            "e raised to the argument's power.\n\n\
             ```\n\
             Math.exp:1     \"* -> 2.718281828459045\n\
             ```",
        )
        .sdk_class_method("ln:", |host, _r, args| {
            let x = num(&args, 0, "Math.ln:")?;
            require_positive(x, "Math.ln:")?;
            Ok(host.new_double(x.ln()))
        })
        .doc(
            "The natural (base-e) logarithm. The domain is guarded: a zero or negative \
             argument raises an ArithmeticError rather than returning NaN.\n\n\
             ```\n\
             Math.ln:Math.e     \"* -> 1\n\
             ```",
        )
        .sdk_class_method("log:", |host, _r, args| {
            let x = num(&args, 0, "Math.log:")?;
            require_positive(x, "Math.log:")?;
            Ok(host.new_double(x.log10()))
        })
        .doc(
            "The base-10 logarithm (domain-guarded like `ln:`).\n\n\
             ```\n\
             Math.log:1000     \"* -> 3\n\
             ```",
        )
        .sdk_class_method("log2:", |host, _r, args| {
            let x = num(&args, 0, "Math.log2:")?;
            require_positive(x, "Math.log2:")?;
            Ok(host.new_double(x.log2()))
        })
        .doc(
            "The base-2 logarithm (domain-guarded like `ln:`).\n\n\
             ```\n\
             Math.log2:8     \"* -> 3\n\
             ```",
        )
        .sdk_class_method("log:base:", |host, _r, args| {
            let x = num(&args, 0, "Math.log:base:")?;
            let base = num(&args, 1, "Math.log:base:")?;
            require_positive(x, "Math.log:base:")?;
            Ok(host.new_double(x.log(base)))
        })
        .doc(
            "The logarithm of the first argument in the base given by the second (the \
             argument is domain-guarded like `ln:`).\n\n\
             ```\n\
             Math.log:8 base:2     \"* -> 3\n\
             ```",
        )
        // Mirrors of number-instance operations, for a free-function style.
        .sdk_class_method("sqrt:", |host, _r, args| {
            let x = num(&args, 0, "Math.sqrt:")?;
            if x < 0.0 {
                return Err(QuoinError::ArithmeticError(
                    "Math.sqrt: of a negative number".to_string(),
                ));
            }
            Ok(host.new_double(x.sqrt()))
        })
        .doc(
            "The square root, mirroring `sqrt` on the numbers in free-function style; a \
             negative argument raises an ArithmeticError.\n\n\
             ```\n\
             Math.sqrt:2     \"* -> 1.4142135623730951\n\
             ```",
        )
        .sdk_class_method("pow:to:", |host, _r, args| {
            let base = num(&args, 0, "Math.pow:to:")?;
            let exp = num(&args, 1, "Math.pow:to:")?;
            Ok(host.new_double(base.powf(exp)))
        })
        .doc(
            "The first argument raised to the second's power, mirroring `pow:` on the \
             numbers; always computed in floating point.\n\n\
             ```\n\
             Math.pow:2 to:10     \"* -> 1024\n\
             ```",
        )
        .sdk_class_method("hypot:over:", |host, _r, args| {
            let a = num(&args, 0, "Math.hypot:over:")?;
            let b = num(&args, 1, "Math.hypot:over:")?;
            Ok(host.new_double(a.hypot(b)))
        })
        .doc(
            "The hypotenuse sqrt(a^2 + b^2) of a right triangle with legs a and b, \
             computed without intermediate overflow.\n\n\
             ```\n\
             Math.hypot:3 over:4     \"* -> 5\n\
             ```",
        )
}
