use crate::error::QuoinError;
use crate::recv;
use crate::value::{NativeClassBuilder, Value};

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
    let b = NativeClassBuilder::new("Integer", Some("Object")).instance_method(
        "sqrt",
        |vm, mc, receiver, _args| {
            let val = recv!(receiver, Int);
            Ok(vm.new_double(mc, (val as f64).sqrt()))
        },
    );
    let b = int_binop!(b, "+:", arith+);
    let b = int_binop!(b, "-:", arith -);
    let b = int_binop!(b, "*:", arith *);
    let b = int_binop!(b, "/:", divop /);
    let b = int_binop!(b, "%:", divop %);
    // Only `<:` is native; `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
    let b = int_binop!(b, "<:", cmp <);
    b.instance_method("==:", |vm, mc, receiver, args| {
        Ok(vm.new_bool(mc, receiver == args[0]))
    })
}
