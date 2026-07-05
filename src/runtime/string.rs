use crate::arg;
use crate::compiler::Compiler;
use crate::error::QuoinError;
use crate::parser::ast::NodeValue;
use crate::parser::parse_quoin_string;
use crate::recv;
use crate::runtime::list::NativeListState;
use crate::runtime::map::NativeMapState;
use crate::runtime::regex::NativeRegexState;
use crate::value::{NativeClassBuilder, ObjectPayload, Value};
use std::rc::Rc;

use std::collections::{HashMap, HashSet};

pub fn build_string_class() -> NativeClassBuilder {
    NativeClassBuilder::new("String", Some("Object"))
        // Human string form is the string itself (no quoting — that's `.pp`'s job). Explicit so
        // `.s` never routes through the Rust Display impl (the default `Object.s` fallback).
        .instance_method("s", |_vm, _mc, receiver, _args| Ok(receiver))
        // `replace:with:` is a multimethod: the pattern's type selects the variant
        // (a non-String/non-Regex pattern matches neither → MessageNotUnderstood).
        // The replacement is always a String.
        .typed_instance_method(
            "replace:with:",
            &["Regex", "String"],
            |vm, mc, receiver, args| {
                let s = recv!(receiver, String);
                let to = arg!(args, String, 1);
                let result = args[0].with_native_state::<NativeRegexState, _, _>(|r| {
                    r.regex.replace_all(&*s, &**to).to_string()
                })?;
                Ok(vm.new_string(mc, result))
            },
        )
        .typed_instance_method(
            "replace:with:",
            &["String", "String"],
            |vm, mc, receiver, args| {
                let s = recv!(receiver, String);
                let from = arg!(args, String, 0);
                let to = arg!(args, String, 1);
                Ok(vm.new_string(mc, s.replace(&**from, &**to)))
            },
        )
        .instance_method("==:", |vm, mc, receiver, args| {
            Ok(vm.new_bool(mc, receiver == args[0]))
        })
        // Concatenation: `a + b` -> `Send(a, "+:", [b])`. A String RHS concatenates
        // directly (the fast path); any other RHS is coerced via `.s` by the untyped
        // fallback below, so `'n = ' + 5` or `'m = ' + aMap` work.
        .typed_instance_method("+:", &["String"], |vm, mc, receiver, args| {
            let a = recv!(receiver, String);
            let b = arg!(args, String, 0);
            Ok(vm.new_string(mc, format!("{}{}", *a, *b)))
        })
        .instance_method("+:", |vm, mc, receiver, args| {
            let a = recv!(receiver, String).to_string();
            let b_val = vm.call_method(mc, args[0], "s", vec![])?;
            let b = match b_val {
                Value::Object(o) => match &o.borrow().payload {
                    ObjectPayload::String(st) => st.to_string(),
                    _ => format!("{}", b_val),
                },
                _ => format!("{}", b_val),
            };
            Ok(vm.new_string(mc, format!("{}{}", a, b)))
        })
        // asBytes -> the string's UTF-8 bytes as a `Bytes` (infallible). The inverse
        // is `Bytes.asString` (which can fail). See `docs/ASYNC_ARCH.md`.
        .instance_method("asBytes", |vm, mc, receiver, _args| {
            let s = recv!(receiver, String);
            Ok(vm.new_bytes(mc, s.as_bytes().to_vec()))
        })
        // `a % b` -> `Send(a, "%:", [b])`: printf-like formatting. A List RHS supplies
        // positional args (`%1`, `%2`, … and bare `%`); a Map RHS additionally supplies
        // named substitutions (`%<key>`); any other RHS is a single positional arg.
        // (Migrated verbatim from `native_mod`'s String branch.) Values stay reachable
        // through the rooted `active_native_args`, so they survive the `.s` calls below.
        .instance_method("%:", |vm, mc, receiver, _args| {
            let s_str = recv!(receiver, String).to_string();

            let arg1 = vm.active_native_args.last().unwrap().args[0];
            let mut format_args_raw = Vec::new();
            if let Value::Object(o) = arg1 {
                let oref = o.borrow();
                match &oref.payload {
                    ObjectPayload::NativeState(state_cell) => {
                        let state_ref = state_cell.borrow();
                        if let Some(list_state) =
                            state_ref.as_any().downcast_ref::<NativeListState>()
                        {
                            for val in list_state.get_vec() {
                                format_args_raw.push(*val);
                            }
                        } else {
                            format_args_raw.push(arg1);
                        }
                    }
                    _ => format_args_raw.push(arg1),
                }
            } else {
                format_args_raw.push(arg1);
            }

            let mut format_args_strings = Vec::new();
            for val in format_args_raw {
                let val_str_val = vm.call_method(mc, val, "s", vec![])?;
                let val_str = match val_str_val {
                    Value::Object(o) => match &o.borrow().payload {
                        ObjectPayload::String(st) => st.to_string(),
                        _ => format!("{}", val_str_val),
                    },
                    _ => format!("{}", val_str_val),
                };
                format_args_strings.push(val_str);
            }

            let mut map_formatted_args = HashMap::new();
            let arg1 = vm.active_native_args.last().unwrap().args[0];
            // Snapshot the map's pairs under a short borrow, THEN render: `.s` on a
            // value runs arbitrary Quoin that can cooperatively yield, and a map-state
            // borrow held across that suspend collides with any concurrent use of the
            // same map (or a `s` that touches it) — "RefCell already borrowed".
            let map_pairs: Vec<(String, Value)> = if let Value::Object(obj) = arg1
                && let ObjectPayload::NativeState(state_cell) = &obj.borrow().payload
                && let Some(map_state) = state_cell
                    .borrow()
                    .as_any()
                    .downcast_ref::<NativeMapState>()
            {
                map_state
                    .get_map()
                    .iter()
                    .map(|(k, &v)| (k.clone(), v))
                    .collect()
            } else {
                Vec::new()
            };
            for (k, v) in map_pairs {
                let val_str_val = vm.call_method(mc, v, "s", vec![])?;
                let val_str = match val_str_val {
                    Value::Object(o) => match &o.borrow().payload {
                        ObjectPayload::String(st) => st.to_string(),
                        _ => format!("{}", val_str_val),
                    },
                    _ => format!("{}", val_str_val),
                };
                map_formatted_args.insert(k, val_str);
            }

            let mut result = String::new();
            let mut chars = s_str.chars().peekable();
            let mut arg_idx = 0;

            while let Some(c) = chars.next() {
                if c == '%' {
                    if let Some(&next_c) = chars.peek() {
                        if next_c.is_digit(10) {
                            let mut num_str = String::new();
                            while let Some(&digit) = chars.peek() {
                                if digit.is_digit(10) {
                                    num_str.push(digit);
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                            let idx: usize = num_str.parse().unwrap();
                            if idx > 0 && idx <= format_args_strings.len() {
                                result.push_str(&format_args_strings[idx - 1]);
                            }
                        } else if next_c.is_alphabetic() && !map_formatted_args.is_empty() {
                            let key_char = next_c;
                            chars.next();

                            let key_str = key_char.to_string();
                            if let Some(val_str) = map_formatted_args.get(&key_str) {
                                result.push_str(val_str);
                            } else {
                                result.push('%');
                                result.push(key_char);
                            }
                        } else {
                            if arg_idx < format_args_strings.len() {
                                result.push_str(&format_args_strings[arg_idx]);
                                arg_idx += 1;
                            }
                        }
                    } else {
                        if arg_idx < format_args_strings.len() {
                            result.push_str(&format_args_strings[arg_idx]);
                            arg_idx += 1;
                        } else {
                            result.push('%');
                        }
                    }
                } else {
                    result.push(c);
                }
            }
            Ok(vm.new_string(mc, result))
        })
        // Only `<:` is native (the compiler lowers `a < b` to `Send(a, "<:", [b])`);
        // `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
        .instance_method("<:", |vm, mc, receiver, args| {
            let lhs = recv!(receiver, String);
            let rhs = arg!(args, String, 0);
            Ok(vm.new_bool(mc, *lhs < *rhs))
        })
        .instance_method("to_integer", |vm, mc, receiver, _args| {
            let s = recv!(receiver, String);
            Ok(vm.new_int(
                mc,
                s.parse::<i64>()
                    .map_err(|e| QuoinError::Other(e.to_string()))?,
            ))
        })
        .instance_method("mod", |vm, mc, receiver, _args| {
            let s_borrow = recv!(receiver, String);
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
                    QuoinError::Other("No caller frame found for string interpolation".to_string())
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
                        let node = parse_quoin_string(&expr_str);
                        let program_node = match &node.value {
                            NodeValue::Program(p) => p,
                            _ => {
                                return Err(QuoinError::Other(
                                    "Parsed node is not a ProgramNode".to_string(),
                                ));
                            }
                        };

                        let mut local_names = HashSet::new();
                        let mut current_env = Some(caller_env);
                        while let Some(env) = current_env {
                            for (sym, _) in &env.borrow().vars {
                                local_names.insert(sym.as_str().to_string());
                            }
                            current_env = env.borrow().parent;
                        }

                        let mut compiler = Compiler::new_with_locals(local_names);
                        compiler.set_seen_types(vm.options.seen_types.clone());
                        compiler.set_class_table(vm.options.class_table.clone());
                        crate::class_table::populate_from_vm(vm, &vm.options.class_table);
                        let compiled = compiler
                            .compile_program(program_node)
                            .map_err(|e| QuoinError::Other(e))?;
                        vm.report_type_warnings(compiler.diagnostics());

                        let block = vm.block_from_template(
                            mc,
                            Rc::new(compiled),
                            Some(caller_env),
                            enclosing_method_id,
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
        .instance_method("length", |vm, mc, receiver, _args| {
            let s = recv!(receiver, String);
            Ok(vm.new_int(mc, s.chars().count() as i64))
        })
        .instance_method("ansiEscaped", |vm, mc, receiver, _args| {
            // Escape '$' so this text is safe to embed in an #ANSI'…' color
            // template. Reuses the colorizer's own escape so the two can't drift.
            let s = recv!(receiver, String);
            Ok(vm.new_string(mc, crate::ansi_colorizer::escape(&s)))
        })
        .instance_method("contains?:", |vm, mc, receiver, args| {
            let s = recv!(receiver, String);
            let sub = arg!(args, String, 0);
            Ok(vm.new_bool(mc, s.contains(&**sub)))
        })
        .instance_method("ends?:", |vm, mc, receiver, args| {
            let s = recv!(receiver, String);
            let sub = arg!(args, String, 0);
            Ok(vm.new_bool(mc, s.ends_with(&**sub)))
        })
        .instance_method("starts?:", |vm, mc, receiver, args| {
            let s = recv!(receiver, String);
            let sub = arg!(args, String, 0);
            Ok(vm.new_bool(mc, s.starts_with(&**sub)))
        })
        .instance_method("index:", |vm, mc, receiver, args| {
            let s = recv!(receiver, String);
            let sub = arg!(args, String, 0);
            if let Some(byte_idx) = s.find(&**sub) {
                let char_idx = s[..byte_idx].chars().count() as i64;
                Ok(vm.new_int(mc, char_idx))
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        // Both args are typed: the substring (String) and the index (Integer); a
        // wrong-typed arg matches no variant -> MNU (dispatch enforces the types).
        .typed_instance_method(
            "insert:at:",
            &["String", "Integer"],
            |vm, mc, receiver, args| {
                let s = recv!(receiver, String);
                let sub = arg!(args, String, 0);
                let char_idx = arg!(args, Int, 1) as usize;

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
            },
        )
        .instance_method("lower", |vm, mc, receiver, _args| {
            let s = recv!(receiver, String);
            Ok(vm.new_string(mc, s.to_lowercase()))
        })
        .instance_method("upper", |vm, mc, receiver, _args| {
            let s = recv!(receiver, String);
            Ok(vm.new_string(mc, s.to_uppercase()))
        })
        .instance_method("splitString:", |vm, mc, receiver, args| {
            let s = recv!(receiver, String);
            let pat = arg!(args, String, 0);
            let parts: Vec<Value> = s
                .split(&**pat)
                .map(|part| vm.new_string(mc, part.to_string()))
                .collect();
            let res = vm.new_list(mc, parts);
            Ok(res)
        })
}
