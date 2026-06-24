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
        // Constants.
        .class_method("pi", |vm, mc, _r, _a| {
            Ok(vm.new_double(mc, std::f64::consts::PI))
        })
        .class_method("e", |vm, mc, _r, _a| {
            Ok(vm.new_double(mc, std::f64::consts::E))
        })
        .class_method("tau", |vm, mc, _r, _a| {
            Ok(vm.new_double(mc, std::f64::consts::TAU))
        })
        // Trigonometry.
        .class_method("sin:", |vm, mc, _r, args| {
            Ok(vm.new_double(mc, num(&args, 0, "Math.sin:")?.sin()))
        })
        .class_method("cos:", |vm, mc, _r, args| {
            Ok(vm.new_double(mc, num(&args, 0, "Math.cos:")?.cos()))
        })
        .class_method("tan:", |vm, mc, _r, args| {
            Ok(vm.new_double(mc, num(&args, 0, "Math.tan:")?.tan()))
        })
        .class_method("asin:", |vm, mc, _r, args| {
            Ok(vm.new_double(mc, num(&args, 0, "Math.asin:")?.asin()))
        })
        .class_method("acos:", |vm, mc, _r, args| {
            Ok(vm.new_double(mc, num(&args, 0, "Math.acos:")?.acos()))
        })
        .class_method("atan:", |vm, mc, _r, args| {
            Ok(vm.new_double(mc, num(&args, 0, "Math.atan:")?.atan()))
        })
        // atan2(y, x): the angle of the vector (x, y), reading `Math.atan: y over: x`.
        .class_method("atan:over:", |vm, mc, _r, args| {
            let y = num(&args, 0, "Math.atan:over:")?;
            let x = num(&args, 1, "Math.atan:over:")?;
            Ok(vm.new_double(mc, y.atan2(x)))
        })
        // Exponential / logarithmic. ln/log domains are guarded to x > 0.
        .class_method("exp:", |vm, mc, _r, args| {
            Ok(vm.new_double(mc, num(&args, 0, "Math.exp:")?.exp()))
        })
        .class_method("ln:", |vm, mc, _r, args| {
            let x = num(&args, 0, "Math.ln:")?;
            require_positive(x, "Math.ln:")?;
            Ok(vm.new_double(mc, x.ln()))
        })
        .class_method("log:", |vm, mc, _r, args| {
            let x = num(&args, 0, "Math.log:")?;
            require_positive(x, "Math.log:")?;
            Ok(vm.new_double(mc, x.log10()))
        })
        .class_method("log2:", |vm, mc, _r, args| {
            let x = num(&args, 0, "Math.log2:")?;
            require_positive(x, "Math.log2:")?;
            Ok(vm.new_double(mc, x.log2()))
        })
        .class_method("log:base:", |vm, mc, _r, args| {
            let x = num(&args, 0, "Math.log:base:")?;
            let base = num(&args, 1, "Math.log:base:")?;
            require_positive(x, "Math.log:base:")?;
            Ok(vm.new_double(mc, x.log(base)))
        })
        // Mirrors of number-instance operations, for a free-function style.
        .class_method("sqrt:", |vm, mc, _r, args| {
            let x = num(&args, 0, "Math.sqrt:")?;
            if x < 0.0 {
                return Err(QuoinError::ArithmeticError(
                    "Math.sqrt: of a negative number".to_string(),
                ));
            }
            Ok(vm.new_double(mc, x.sqrt()))
        })
        .class_method("pow:to:", |vm, mc, _r, args| {
            let base = num(&args, 0, "Math.pow:to:")?;
            let exp = num(&args, 1, "Math.pow:to:")?;
            Ok(vm.new_double(mc, base.powf(exp)))
        })
        .class_method("hypot:over:", |vm, mc, _r, args| {
            let a = num(&args, 0, "Math.hypot:over:")?;
            let b = num(&args, 1, "Math.hypot:over:")?;
            Ok(vm.new_double(mc, a.hypot(b)))
        })
}
