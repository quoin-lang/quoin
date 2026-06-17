use crate::arg;
use crate::error::BBError;
use crate::runtime::regex::NativeRegexState;
use crate::value::{NativeClassBuilder, ObjectPayload, Value};

use gc_arena::Gc;

pub fn build_string_class() -> NativeClassBuilder {
    NativeClassBuilder::new("String", Some("Object"))
        .instance_method("replace:with:", |vm, mc, args| {
            if args.len() < 3 {
                return Err(BBError::Other(
                    "replace:with: expects receiver, pattern, and replacement".to_string(),
                ));
            }
            let s_borrow = arg!(args, String, 0);
            let from_val = args[1];
            let to_str = arg!(args, String, 2);

            if let Ok(result) = from_val.with_native_state::<NativeRegexState, _, _>(|r| {
                r.regex.replace_all(&*s_borrow, &**to_str).to_string()
            }) {
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
        .instance_method(
            "==:",
            |vm, mc, args| Ok(vm.new_bool(mc, args[0] == args[1])),
        )
        .instance_method("<", |vm, mc, args| {
            let lhs = arg!(args, String, 0);
            let rhs = arg!(args, String, 1);
            Ok(vm.new_bool(mc, *lhs < *rhs))
        })
        .instance_method(">", |vm, mc, args| {
            let lhs = arg!(args, String, 0);
            let rhs = arg!(args, String, 1);
            Ok(vm.new_bool(mc, *lhs > *rhs))
        })
        .instance_method("mod", |vm, mc, args| {
            let s_borrow = arg!(args, String, 0);
            let s = s_borrow.to_string();

            enum InterpolPart {
                Lit(String),
                Expr(String),
            }

            let mut parts = Vec::new();
            let chars: Vec<char> = s.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if i + 1 < chars.len() && chars[i] == '%' && chars[i + 1] == '{' {
                    let mut depth = 1;
                    let mut j = i + 2;
                    while j < chars.len() && depth > 0 {
                        if chars[j] == '{' {
                            depth += 1;
                        } else if chars[j] == '}' {
                            depth -= 1;
                        }
                        j += 1;
                    }
                    if depth == 0 {
                        let expr_str: String = chars[i + 2..j - 1].iter().collect();
                        parts.push(InterpolPart::Expr(expr_str));
                        i = j;
                    } else {
                        parts.push(InterpolPart::Lit(chars[i].to_string()));
                        i += 1;
                    }
                } else {
                    parts.push(InterpolPart::Lit(chars[i].to_string()));
                    i += 1;
                }
            }

            // Get the caller's frame context
            let (caller_env, caller_receiver, enclosing_method_id) = {
                let caller_frame = vm.frames.last().ok_or_else(|| {
                    BBError::Other("No caller frame found for string interpolation".to_string())
                })?;
                (
                    caller_frame.env,
                    caller_frame.receiver,
                    caller_frame.enclosing_method_id,
                )
            };

            let mut result = String::new();
            for part in parts {
                match part {
                    InterpolPart::Lit(lit) => {
                        result.push_str(&lit);
                    }
                    InterpolPart::Expr(expr_str) => {
                        let node = crate::parser::parser::parse_building_blocks_string(&expr_str);
                        let program_node = match &node.value {
                            crate::parser::ast_visitor::NodeValue::Program(p) => p,
                            _ => {
                                return Err(BBError::Other(
                                    "Parsed node is not a ProgramNode".to_string(),
                                ));
                            }
                        };

                        let mut local_names = std::collections::HashSet::new();
                        let mut current_env = Some(caller_env);
                        while let Some(env) = current_env {
                            for name in env.borrow().vars.keys() {
                                local_names.insert(name.clone());
                            }
                            current_env = env.borrow().parent;
                        }

                        let mut compiler = crate::compiler::Compiler::new_with_locals(local_names);
                        let compiled = compiler
                            .compile_program(program_node)
                            .map_err(|e| BBError::Other(e))?;

                        let block = crate::gc!(
                            mc,
                            crate::value::Block {
                                name: compiled.name.clone(),
                                is_nested_block: compiled.is_nested_block,
                                param_names: compiled.param_names.clone(),
                                param_types: compiled.param_types.clone(),
                                bytecode: compiled.bytecode.clone(),
                                parent_env: Some(caller_env),
                                enclosing_method_id,
                                source_info: compiled.source_info.clone(),
                            }
                        );

                        let val = vm.execute_block(mc, block, Vec::new(), caller_receiver)?;

                        let val_str_val = vm.call_method(mc, val, "s", vec![])?;
                        let val_str = match val_str_val {
                            Value::Object(o) => match &o.borrow().payload {
                                ObjectPayload::String(st) => st.to_string(),
                                _ => format!("{}", val_str_val),
                            },
                            x => format!("{}", x),
                        };
                        result.push_str(&val_str);
                    }
                }
            }

            Ok(vm.new_string(mc, result))
        })
        .instance_method("length", |vm, mc, args| {
            let s = arg!(args, String, 0);
            Ok(vm.new_int(mc, s.chars().count() as i64))
        })
        .instance_method("contains?:", |vm, mc, args| {
            let s = arg!(args, String, 0);
            let sub = arg!(args, String, 1);
            Ok(vm.new_bool(mc, s.contains(&**sub)))
        })
        .instance_method("ends?:", |vm, mc, args| {
            let s = arg!(args, String, 0);
            let sub = arg!(args, String, 1);
            Ok(vm.new_bool(mc, s.ends_with(&**sub)))
        })
        .instance_method("starts?:", |vm, mc, args| {
            let s = arg!(args, String, 0);
            let sub = arg!(args, String, 1);
            Ok(vm.new_bool(mc, s.starts_with(&**sub)))
        })
        .instance_method("index:", |vm, mc, args| {
            let s = arg!(args, String, 0);
            let sub = arg!(args, String, 1);
            if let Some(byte_idx) = s.find(&**sub) {
                let char_idx = s[..byte_idx].chars().count() as i64;
                Ok(vm.new_int(mc, char_idx))
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        .instance_method("insert:at:", |vm, mc, args| {
            let s = arg!(args, String, 0);
            let sub = arg!(args, String, 1);
            let char_idx = match args[2] {
                Value::Object(obj) => match &obj.borrow().payload {
                    ObjectPayload::Int(idx) => *idx as usize,
                    _ => {
                        return Err(BBError::TypeError {
                            expected: "Integer".to_string(),
                            got: args[2].type_name().to_string(),
                            msg: "insert:at: expected Integer index".to_string(),
                        });
                    }
                },
                _ => {
                    return Err(BBError::TypeError {
                        expected: "Integer".to_string(),
                        got: args[2].type_name().to_string(),
                        msg: "insert:at: expected Integer index".to_string(),
                    });
                }
            };

            let char_count = s.chars().count();
            let safe_idx = char_idx.min(char_count);

            let byte_offset: usize = s
                .char_indices()
                .map(|(idx, _)| idx)
                .nth(safe_idx)
                .unwrap_or(s.len());

            let mut result = s[..byte_offset].to_string();
            result.push_str(&**sub);
            result.push_str(&s[byte_offset..]);

            Ok(vm.new_string(mc, result))
        })
        .instance_method("lower", |vm, mc, args| {
            let s = arg!(args, String, 0);
            Ok(vm.new_string(mc, s.to_lowercase()))
        })
        .instance_method("upper", |vm, mc, args| {
            let s = arg!(args, String, 0);
            Ok(vm.new_string(mc, s.to_uppercase()))
        })
        .instance_method("splitString:", |vm, mc, args| {
            let s = arg!(args, String, 0);
            let pat = arg!(args, String, 1);
            let parts: Vec<Value> = s
                .split(&**pat)
                .map(|part| vm.new_string(mc, part.to_string()))
                .collect();
            let res = vm.new_list(mc, parts);
            Ok(res)
        })
}
