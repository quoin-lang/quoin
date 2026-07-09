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
        // Constants.
        .sdk_class_method("pi", |host, _r, _a| {
            Ok(host.new_double(std::f64::consts::PI))
        })
        .sdk_class_method("e", |host, _r, _a| Ok(host.new_double(std::f64::consts::E)))
        .sdk_class_method("tau", |host, _r, _a| {
            Ok(host.new_double(std::f64::consts::TAU))
        })
        // Trigonometry.
        .sdk_class_method("sin:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.sin:")?.sin()))
        })
        .sdk_class_method("cos:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.cos:")?.cos()))
        })
        .sdk_class_method("tan:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.tan:")?.tan()))
        })
        .sdk_class_method("asin:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.asin:")?.asin()))
        })
        .sdk_class_method("acos:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.acos:")?.acos()))
        })
        .sdk_class_method("atan:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.atan:")?.atan()))
        })
        // atan2(y, x): the angle of the vector (x, y), reading `Math.atan: y over: x`.
        .sdk_class_method("atan:over:", |host, _r, args| {
            let y = num(&args, 0, "Math.atan:over:")?;
            let x = num(&args, 1, "Math.atan:over:")?;
            Ok(host.new_double(y.atan2(x)))
        })
        // Exponential / logarithmic. ln/log domains are guarded to x > 0.
        .sdk_class_method("exp:", |host, _r, args| {
            Ok(host.new_double(num(&args, 0, "Math.exp:")?.exp()))
        })
        .sdk_class_method("ln:", |host, _r, args| {
            let x = num(&args, 0, "Math.ln:")?;
            require_positive(x, "Math.ln:")?;
            Ok(host.new_double(x.ln()))
        })
        .sdk_class_method("log:", |host, _r, args| {
            let x = num(&args, 0, "Math.log:")?;
            require_positive(x, "Math.log:")?;
            Ok(host.new_double(x.log10()))
        })
        .sdk_class_method("log2:", |host, _r, args| {
            let x = num(&args, 0, "Math.log2:")?;
            require_positive(x, "Math.log2:")?;
            Ok(host.new_double(x.log2()))
        })
        .sdk_class_method("log:base:", |host, _r, args| {
            let x = num(&args, 0, "Math.log:base:")?;
            let base = num(&args, 1, "Math.log:base:")?;
            require_positive(x, "Math.log:base:")?;
            Ok(host.new_double(x.log(base)))
        })
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
        .sdk_class_method("pow:to:", |host, _r, args| {
            let base = num(&args, 0, "Math.pow:to:")?;
            let exp = num(&args, 1, "Math.pow:to:")?;
            Ok(host.new_double(base.powf(exp)))
        })
        .sdk_class_method("hypot:over:", |host, _r, args| {
            let a = num(&args, 0, "Math.hypot:over:")?;
            let b = num(&args, 1, "Math.hypot:over:")?;
            Ok(host.new_double(a.hypot(b)))
        })
}
