use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};

use gc_arena::collect::Trace;
use regex::Regex;
use std::any::Any;

#[derive(Debug)]
pub struct NativeRegexState {
    pub regex: Regex,
}

impl NativeRegexState {
    pub fn new(regex: Regex) -> Self {
        Self { regex }
    }
}

impl AnyCollect for NativeRegexState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {
        // No GC references in Regex
    }
}
pub fn build_regex_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Regex", Some("Object"))
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_pat =
                receiver.with_native_state(|r: &NativeRegexState| r.regex.as_str().to_string())?;
            let rhs_pat =
                args[0].with_native_state(|r: &NativeRegexState| r.regex.as_str().to_string());
            match rhs_pat {
                Ok(rhs_pat) => Ok(vm.new_bool(mc, lhs_pat == rhs_pat)),
                Err(_) => Ok(vm.new_bool(mc, false)),
            }
        })
        // `pattern ~ str` -> `Send(pattern, "~:", [str])`: true if the regex matches.
        // A non-String operand never matches (mirrors the old `native_match`).
        .instance_method("~:", |vm, mc, receiver, args| {
            let matched = receiver.with_native_state(|r: &NativeRegexState| {
                if let Value::Object(o) = args[0]
                    && let ObjectPayload::String(s) = &o.borrow().payload
                {
                    r.regex.is_match(&**s)
                } else {
                    false
                }
            })?;
            Ok(vm.new_bool(mc, matched))
        })
        .instance_method("split:", |vm, mc, receiver, args| {
            let s = crate::arg!(args, String, 0);
            let parts: Vec<Value> = receiver.with_native_state(|r: &NativeRegexState| {
                r.regex
                    .split(&**s)
                    .map(|part| vm.new_string(mc, part.to_string()))
                    .collect()
            })?;
            Ok(vm.new_list(mc, parts))
        })
}
