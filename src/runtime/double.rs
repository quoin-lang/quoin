use crate::arg;
use crate::error::QuoinError;
use crate::value::{NativeClassBuilder, Value};

/// Generate `[Integer]` and `[Double]` typed variants for a binary operator on a
/// `Double` receiver. Simpler than `int_binop!` (integer.rs): a Double receiver
/// makes *every* arithmetic result a `Double` (both operands coerce via `as_f64`),
/// so the two variants share a body; `/:`/`%:` need no zero-guard under f64
/// semantics. A non-numeric RHS matches no variant and (until the comparison
/// natives are demoted) falls through to the rekeyed global fallback in native.rs.
macro_rules! double_binop {
    ($b:expr, $sel:literal, arith $op:tt) => {
        $b.typed_instance_method($sel, &["Integer"], |vm, mc, args| {
            Ok(vm.new_double(mc, args[0].as_f64().unwrap() $op args[1].as_f64().unwrap()))
        })
        .typed_instance_method($sel, &["Double"], |vm, mc, args| {
            Ok(vm.new_double(mc, args[0].as_f64().unwrap() $op args[1].as_f64().unwrap()))
        })
    };
    ($b:expr, $sel:literal, cmp $op:tt) => {
        $b.typed_instance_method($sel, &["Integer"], |vm, mc, args| {
            Ok(vm.new_bool(mc, args[0].as_f64().unwrap() $op args[1].as_f64().unwrap()))
        })
        .typed_instance_method($sel, &["Double"], |vm, mc, args| {
            Ok(vm.new_bool(mc, args[0].as_f64().unwrap() $op args[1].as_f64().unwrap()))
        })
    };
}

pub fn build_double_class() -> NativeClassBuilder {
    // Binary operators are the `:` keyword selectors (`a + b` -> `Send(a, "+:", [b])`).
    // Only `<:` is provided natively; `>:`/`<=:`/`>=:` derive from it as shared Quoin.
    let b = NativeClassBuilder::new("Double", Some("Object")).instance_method("sqrt", |vm, mc, args| {
        if args.is_empty() {
            return Err(QuoinError::Other("sqrt expects a receiver".to_string()));
        }
        let val = arg!(args, Double, 0);
        Ok(vm.new_double(mc, val.sqrt()))
    });
    let b = double_binop!(b, "+:", arith +);
    let b = double_binop!(b, "-:", arith -);
    let b = double_binop!(b, "*:", arith *);
    let b = double_binop!(b, "/:", arith /);
    let b = double_binop!(b, "%:", arith %);
    let b = double_binop!(b, "<:", cmp <);
    b.instance_method("==:", |vm, mc, args| Ok(vm.new_bool(mc, args[0] == args[1])))
}
