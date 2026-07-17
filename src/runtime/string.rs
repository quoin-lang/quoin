use crate::arg;
use crate::compiler::Compiler;
use crate::error::QuoinError;
use crate::parser::ast::NodeValue;
use crate::parser::interp::{InterpPart, split_interpolation};
use crate::recv;
use crate::runtime::list::NativeListState;
use crate::runtime::map::NativeMapState;
use crate::runtime::regex::NativeRegexState;
use crate::value::{NativeClassBuilder, ObjectPayload, Value};
use std::sync::Arc;

use std::collections::{HashMap, HashSet};

pub fn build_string_class() -> NativeClassBuilder {
    NativeClassBuilder::new("String", Some("Object"))
        .construct_with("use string literals ('…')")
        .class_doc(
            "Immutable UTF-8 text -- the type of 'single-quoted' literals (a double quote \
             starts a comment in Quoin, so strings are single-quoted). Position-based \
             operations (`length`, `index:`, `insert:at:`) count characters, not bytes. \
             Strings concatenate with `+`, format with binary `%`, and interpolate with \
             `%'…%{expr}…'`; every operation returns a new String.\n\n\
             ```\n\
             'ab' + 'cd'       \"* -> abcd\n\
             'héllo'.length    \"* -> 5\n\
             ```",
        )
        // Human string form is the string itself (no quoting — that's `.pp`'s job). Explicit so
        // `.s` never routes through the Rust Display impl (the default `Object.s` fallback).
        .instance_method("s", |_vm, _mc, receiver, _args| Ok(receiver))
        .doc(
            "The string itself -- the human rendering of a String adds no quotes or \
             escapes (structural, quoted rendering is `.pp`'s job).",
        )
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
                    r.regex.replace_all(&s, to.as_str()).to_string()
                })?;
                Ok(vm.new_string(mc, result))
            },
        )
        .doc(
            "A copy with every match of the pattern replaced by the String argument. The \
             pattern's type selects the variant: a String replaces each literal \
             occurrence, a Regex replaces each match.\n\n\
             ```\n\
             'banana'.replace:'an' with:'AN'         \"* -> bANANa\n\
             'a1b22c'.replace:#/[0-9]+/ with:'#'     \"* -> a#b#c\n\
             ```",
        )
        .typed_instance_method(
            "replace:with:",
            &["String", "String"],
            |vm, mc, receiver, args| {
                let s = recv!(receiver, String);
                let from = arg!(args, String, 0);
                let to = arg!(args, String, 1);
                Ok(vm.new_string(mc, s.replace(from.as_str(), &to)))
            },
        )
        .instance_method("==:", |vm, mc, receiver, args| {
            Ok(vm.new_bool(mc, receiver == args[0]))
        })
        .doc(
            "Whether the argument is a character-for-character equal String; any \
             non-String value is simply unequal, never an error.\n\n\
             ```\n\
             'abc' == 'abc'     \"* -> true\n\
             ```",
        )
        // Concatenation: `a + b` -> `Send(a, "+:", [b])`. A String RHS concatenates
        // directly (the fast path); any other RHS is coerced via `.s` by the untyped
        // fallback below, so `'n = ' + 5` or `'m = ' + aMap` work.
        .typed_instance_method("+:", &["String"], |vm, mc, receiver, args| {
            let a = recv!(receiver, String);
            let b = arg!(args, String, 0);
            // Assembled straight into the payload (inline when short, one
            // sized buffer when long) — NOT `format!`: this is the hottest
            // string op and the fmt machinery (Formatter, pad, Write
            // plumbing) was ~20% of the strings bench's whole profile.
            Ok(vm.new_string_concat(mc, &a, &b))
        })
        .doc(
            "Concatenation (`a + b`). A String argument is appended directly; any other \
             value is first rendered with `.s`, so `'n = ' + 5` just works.\n\n\
             ```\n\
             'ab' + 'cd'     \"* -> abcd\n\
             'n = ' + 5      \"* -> n = 5\n\
             ```",
        )
        .instance_method("+:", |vm, mc, receiver, args| {
            // Coerce the RHS via `.s` FIRST (it re-enters the VM, so the
            // receiver borrow must not be held across it); clone the receiver
            // only after, and only once, into the sized output buffer.
            let b_val = vm.call_method(mc, args[0], "s", vec![])?;
            let a = recv!(receiver, String);
            let out = match b_val {
                Value::Object(o) => match &o.borrow().payload {
                    ObjectPayload::String(st) => return Ok(vm.new_string_concat(mc, &a, st)),
                    _ => format!("{}{}", a, b_val),
                },
                _ => format!("{}{}", a, b_val),
            };
            Ok(vm.new_string(mc, out))
        })
        // asBytes -> the string's UTF-8 bytes as a `Bytes` (infallible). The inverse
        // is `Bytes.asString` (which can fail). See `docs/internal/ASYNC_ARCH.md`.
        .instance_method("asBytes", |vm, mc, receiver, _args| {
            let s = recv!(receiver, String);
            Ok(vm.new_bytes(mc, s.as_bytes().to_vec()))
        })
        .doc(
            "The string's UTF-8 bytes as a Bytes. Never fails; the inverse, \
             `Bytes.asString`, can (not all byte sequences are valid UTF-8).\n\n\
             ```\n\
             'abc'.asBytes     \"* -> Bytes[3] 61 62 63\n\
             ```",
        )
        // `a % b` -> `Send(a, "%:", [b])`: printf-like formatting. A List RHS supplies
        // positional args (`%1`, `%2`, … and bare `%`); a Map RHS additionally supplies
        // named substitutions (`%<key>`); any other RHS is a single positional arg.
        // (Migrated verbatim from `native_mod`'s String branch.) Values stay reachable
        // through the rooted `active_native_args`, so they survive the `.s` calls below.
        .instance_method("%:", |vm, mc, receiver, _args| {
            let s_str = recv!(receiver, String).to_string();

            let arg1 = {
                let c = vm.active_native_args.last().unwrap();
                c.arg(&vm.stack, 0).unwrap()
            };
            let mut format_args_raw = Vec::new();
            if let Ok(items) =
                arg1.with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
            {
                format_args_raw.extend(items);
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
            let arg1 = {
                let c = vm.active_native_args.last().unwrap();
                c.arg(&vm.stack, 0).unwrap()
            };
            // Snapshot the map's pairs under a short borrow, THEN render: `.s` on a
            // value runs arbitrary Quoin that can cooperatively yield, and a map-state
            // borrow held across that suspend collides with any concurrent use of the
            // same map (or a `s` that touches it) — "RefCell already borrowed".
            let map_pairs: Vec<(String, Value)> = arg1
                .with_native_state::<NativeMapState, _, _>(|map_state| {
                    map_state
                        .entries()
                        .iter()
                        .filter_map(|(_, k, v)| {
                            if let Value::Object(kobj) = k
                                && let crate::value::ObjectPayload::String(s) =
                                    &kobj.borrow().payload
                            {
                                Some((s.to_string(), *v))
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
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
                        if next_c.is_ascii_digit() {
                            let mut num_str = String::new();
                            while let Some(&digit) = chars.peek() {
                                if digit.is_ascii_digit() {
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
        .doc(
            "printf-style formatting: `template % args`. A List argument fills `%1`, \
             `%2`, ... (1-based) and bare `%` placeholders in order; a Map argument fills \
             named `%<key>` placeholders; any other value serves as a single positional \
             argument. Substituted values are rendered with `.s`.\n\n\
             ```\n\
             '%1-%2' % #(42 43)            \"* -> 42-43\n\
             '%a, %b' % #{ 'a':1 'b':2 }   \"* -> 1, 2\n\
             'total: %' % 7                \"* -> total: 7\n\
             ```",
        )
        // Only `<:` is native (the compiler lowers `a < b` to `Send(a, "<:", [b])`);
        // `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
        .instance_method("<:", |vm, mc, receiver, args| {
            let lhs = recv!(receiver, String);
            let rhs = arg!(args, String, 0);
            Ok(vm.new_bool(mc, *lhs < *rhs))
        })
        .doc(
            "Lexicographic less-than against another String. The one native comparison -- \
             `>`, `<=` and `>=` derive from it.\n\n\
             ```\n\
             'abc' < 'abd'     \"* -> true\n\
             ```",
        )
        .instance_method("to_integer", |vm, mc, receiver, _args| {
            let s = recv!(receiver, String);
            Ok(vm.new_int(
                mc,
                s.parse::<i64>()
                    .map_err(|e| QuoinError::Other(e.to_string()))?,
            ))
        })
        .doc(
            "Parse the receiver as a decimal Integer; raises an error if the whole string \
             is not one.\n\n\
             ```\n\
             '42'.to_integer     \"* -> 42\n\
             ```",
        )
        .instance_method("%", |vm, mc, receiver, _args| {
            let s_borrow = recv!(receiver, String);
            let s = s_borrow.to_string();

            // The DYNAMIC interpolation path: `%` sent to a computed string.
            // A `%'…'` LITERAL never gets here — the compiler lowers it to a
            // `+` concatenation chain (`compile_interpolated_literal`). Same
            // splitter as the compiler, so the two paths can't disagree.
            let parts = split_interpolation(&s);

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
                    InterpPart::Lit(lit) => {
                        result.push_str(&lit);
                    }
                    InterpPart::Expr(expr_str) => {
                        // `%{…}` re-compiles user text at runtime — a bad
                        // expression must be a CATCHABLE ParseError, not a
                        // process abort (BUGS.md Finding 6).
                        let node = crate::parser::try_parse_quoin_string_named(
                            &expr_str,
                            "<interpolation>",
                        )
                        .map_err(|e| {
                            QuoinError::ParseError(format!("in %{{…}} interpolation: {e}"))
                        })?;
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
                        // `define_self: false`, exactly like `eval:self:`: the
                        // default top-level `self = nil` would shadow the
                        // caller's `self` in the env chain, silently rendering
                        // `%{@ivar}` / `%{self}` / `%{.send}` as nil.
                        let compiled = compiler
                            .compile_program_with(program_node, false)
                            .map_err(|e| QuoinError::Other(e.to_string()))?;
                        vm.report_type_warnings(compiler.diagnostics());

                        let block = vm.block_from_template(
                            mc,
                            Arc::new(compiled),
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
        .doc(
            "Interpolation: evaluate each `%{...}` in the receiver as a Quoin expression \
             in the surrounding scope — locals, `self`, and `@ivars` — and splice in the \
             result's `.s` rendering. A `%'…'` literal is lowered to string concatenation \
             at compile time, so a malformed expression there is a compile error; sending \
             `%` to a computed string evaluates its `%{...}` reflectively at call time \
             instead, and a malformed expression raises a catchable ParseError.\n\n\
             ```\n\
             %'x = %{1 + 2}'     \"* -> x = 3\n\
             ```",
        )
        .instance_method("length", |vm, mc, receiver, _args| {
            let s = recv!(receiver, String);
            Ok(vm.new_int(mc, s.chars().count() as i64))
        })
        .doc(
            "The number of characters (Unicode scalar values) -- not bytes.\n\n\
             ```\n\
             'héllo'.length     \"* -> 5\n\
             ```",
        )
        .instance_method("ansiEscaped", |vm, mc, receiver, _args| {
            // Escape '[' so this text can never open a tag in an #ANSI'…'
            // template. Reuses the colorizer's own escape so the two can't drift.
            let s = recv!(receiver, String);
            Ok(vm.new_string(mc, crate::ansi_colorizer::escape(&s)))
        })
        .doc(
            "A copy with each `[` doubled, so the text is safe to embed literally in an \
             `#ANSI'…'` color template (where `[…]` opens a styled span).\n\n\
             ```\n\
             '[red]cost'.ansiEscaped     \"* -> [[red]cost\n\
             ```",
        )
        .instance_method("contains?:", |vm, mc, receiver, args| {
            let s = recv!(receiver, String);
            let sub = arg!(args, String, 0);
            Ok(vm.new_bool(mc, s.contains(sub.as_str())))
        })
        .doc(
            "Whether the String argument occurs anywhere in the receiver.\n\n\
             ```\n\
             'hello'.contains?:'ell'     \"* -> true\n\
             ```",
        )
        .instance_method("ends?:", |vm, mc, receiver, args| {
            let s = recv!(receiver, String);
            let sub = arg!(args, String, 0);
            Ok(vm.new_bool(mc, s.ends_with(sub.as_str())))
        })
        .doc(
            "Whether the receiver ends with the String argument.\n\n\
             ```\n\
             'hello'.ends?:'lo'     \"* -> true\n\
             ```",
        )
        .instance_method("starts?:", |vm, mc, receiver, args| {
            let s = recv!(receiver, String);
            let sub = arg!(args, String, 0);
            Ok(vm.new_bool(mc, s.starts_with(sub.as_str())))
        })
        .doc(
            "Whether the receiver starts with the String argument.\n\n\
             ```\n\
             'hello'.starts?:'he'     \"* -> true\n\
             ```",
        )
        .instance_method("index:", |vm, mc, receiver, args| {
            let s = recv!(receiver, String);
            let sub = arg!(args, String, 0);
            if let Some(byte_idx) = s.find(sub.as_str()) {
                // Byte->char conversion: an all-ASCII prefix (the common
                // case) needs no second decode pass.
                let prefix = &s.as_bytes()[..byte_idx];
                let char_idx = if prefix.is_ascii() {
                    byte_idx as i64
                } else {
                    s[..byte_idx].chars().count() as i64
                };
                Ok(vm.new_int(mc, char_idx))
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        .doc(
            "The character index of the first occurrence of the String argument, or nil \
             if it does not occur.\n\n\
             ```\n\
             'hello'.index:'l'     \"* -> 2\n\
             ```",
        )
        .typed_instance_method(
            "sliceFrom:to:",
            &["Integer", "Integer"],
            |vm, mc, receiver, args| {
                // The String twin of List#sliceFrom:to: — CHARACTER-indexed,
                // end-exclusive, both ends clamped, inverted answers ''. The
                // char walk pays only for the non-ASCII case.
                let s = recv!(receiver, String);
                let from = arg!(args, Int, 0).max(0) as usize;
                let to = arg!(args, Int, 1).max(0) as usize;
                let out = if to <= from {
                    String::new()
                } else if s.is_ascii() {
                    let len = s.len();
                    s[from.min(len)..to.min(len)].to_string()
                } else {
                    s.chars().skip(from).take(to - from).collect()
                };
                Ok(vm.new_string(mc, out))
            },
        )
        .returns("String")
        .doc(
            "The characters from `from` (inclusive) to `to` (exclusive) — the String twin \
             of `List#sliceFrom:to:`: character-indexed, both ends clamp to the string, an \
             inverted range answers ''.\n\n\
             ```\n\
             'command'.sliceFrom:3 to:7     \"* -> mand\n\
             'command'.sliceFrom:3 to:99    \"* -> mand\n\
             ```",
        )
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
                result.push_str(&sub);
                result.push_str(&s[byte_offset..]);

                Ok(vm.new_string(mc, result))
            },
        )
        .doc(
            "A copy with the String argument inserted before the given character index; \
             an index at or past the end appends.\n\n\
             ```\n\
             'hello'.insert:'-' at:2     \"* -> he-llo\n\
             ```",
        )
        .instance_method("lower", |vm, mc, receiver, _args| {
            let s = recv!(receiver, String);
            Ok(vm.new_string(mc, s.to_lowercase()))
        })
        .doc(
            "A lowercase copy (full Unicode lowercasing).\n\n\
             ```\n\
             'Hello'.lower     \"* -> hello\n\
             ```",
        )
        .instance_method("upper", |vm, mc, receiver, _args| {
            let s = recv!(receiver, String);
            Ok(vm.new_string(mc, s.to_uppercase()))
        })
        .doc(
            "An uppercase copy (full Unicode uppercasing).\n\n\
             ```\n\
             'hello'.upper     \"* -> HELLO\n\
             ```",
        )
        .instance_method("splitString:", |vm, mc, receiver, args| {
            let s = recv!(receiver, String);
            let pat = arg!(args, String, 0);
            // `split`'s iterator is not ExactSize, so a bare collect of
            // Values regrows repeatedly; collect the cheap slices first and
            // size the Value Vec exactly.
            let slices: Vec<&str> = s.split(pat.as_str()).collect();
            let mut parts: Vec<Value> = Vec::with_capacity(slices.len());
            for part in slices {
                parts.push(vm.new_string_from_str(mc, part));
            }
            let res = vm.new_list(mc, parts);
            Ok(res)
        })
        .doc(
            "The pieces between occurrences of the literal String separator, as a List of \
             Strings (for pattern-based splitting see `Regex.split:`).\n\n\
             ```\n\
             'a,b,c'.splitString:','     \"* -> #(a b c)\n\
             ```",
        )
}
