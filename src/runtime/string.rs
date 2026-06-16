use crate::arg;
use crate::error::BBError;
use crate::value::{NativeClassBuilder, Value, ObjectPayload};

pub fn build_string_class() -> NativeClassBuilder {
    NativeClassBuilder::new("String", Some("Object"))
        .instance_method("replace:with:", |vm, mc, args| {
            if args.len() < 3 {
                return Err(BBError::Other("replace:with: expects receiver, pattern, and replacement".to_string()));
            }
            let s_borrow = arg!(args, String, 0);
            let from_val = args[1];
            let to_str = arg!(args, String, 2);

            if let Value::Object(obj) = from_val
                && let ObjectPayload::Regex(r) = &obj.borrow().payload
            {
                let result = r.0.replace_all(&*s_borrow, &**to_str).to_string();
                return Ok(vm.new_string(mc, result));
            }

            if let Value::Object(obj) = from_val
                && let ObjectPayload::String(s) = &obj.borrow().payload
            {
                let result = s_borrow.replace(&**s, &**to_str);
                return Ok(vm.new_string(mc, result));
            }

            Err(BBError::TypeError {
                expected: "Regex or String".to_string(),
                got: from_val.type_name().to_string(),
                msg: "replace:with: expected Regex or String pattern".to_string(),
            })
        })
}
