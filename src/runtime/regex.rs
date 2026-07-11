use crate::error::QuoinError;
use crate::runtime::pretty::{PpChild, PpRole, PpShape, PrettyPrint};
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};

use gc_arena::Gc;
use gc_arena::collect::Trace;
use gc_arena::lock::RefLock;
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
        .construct_with("use regex literals (#/…/)")
        .class_doc(
            "A compiled regular expression -- the type of `#/pattern/` literals. Match \
             with `~`, split with `split:`, substitute via `String.replace:with:`. \
             Patterns use Rust's regex syntax.\n\n\
             ```\n\
             #/ab+c/ ~ 'xabbbcy'     \"* -> true\n\
             ```",
        )
        .sdk_instance_method("==:", |host, receiver, args| {
            let lhs_pat =
                receiver.with_native_state(|r: &NativeRegexState| r.regex.as_str().to_string())?;
            let rhs_pat =
                args[0].with_native_state(|r: &NativeRegexState| r.regex.as_str().to_string());
            match rhs_pat {
                Ok(rhs_pat) => Ok(host.new_bool(lhs_pat == rhs_pat)),
                Err(_) => Ok(host.new_bool(false)),
            }
        })
        .doc(
            "Whether the argument is a Regex with the identical pattern text -- \
             equivalent regexes written differently are unequal. A non-Regex is simply \
             unequal, never an error.\n\n\
             ```\n\
             #/a/ == #/a/     \"* -> true\n\
             #/a/ == 'a'      \"* -> false\n\
             ```",
        )
        // `pattern ~ str` -> `Send(pattern, "~:", [str])`: true if the regex matches.
        // A non-String operand never matches (mirrors the old `native_match`).
        .sdk_instance_method("~:", |host, receiver, args| {
            let matched = receiver.with_native_state(|r: &NativeRegexState| {
                if let Value::Object(o) = args[0]
                    && let ObjectPayload::String(s) = &o.borrow().payload
                {
                    r.regex.is_match(&**s)
                } else {
                    false
                }
            })?;
            Ok(host.new_bool(matched))
        })
        .doc(
            "Whether the pattern matches anywhere in the String argument (`pattern ~ \
             str`). A non-String argument never matches (false, not an error).\n\n\
             ```\n\
             #/\\d+/ ~ 'order 66'     \"* -> true\n\
             #/ab+c/ ~ 'xyz'         \"* -> false\n\
             ```",
        )
        .sdk_instance_method("split:", |host, receiver, args| {
            let s = crate::arg!(args, String, 0);
            let parts: Vec<Value> = receiver.with_native_state(|r: &NativeRegexState| {
                r.regex
                    .split(&**s)
                    .map(|part| host.new_string(part.to_string()))
                    .collect()
            })?;
            Ok(host.new_list(parts))
        })
        .doc(
            "The pieces of the String argument between matches of the pattern, as a List \
             of Strings (the pattern-based counterpart of `String.splitString:`).\n\n\
             ```\n\
             #/,\\s*/.split:'a, b,c'     \"* -> #(a b c)\n\
             ```",
        )
        .instance_method("match:", |vm, mc, receiver, args| {
            let s = crate::arg!(args, String, 0);
            let state = receiver.with_native_state(|r: &NativeRegexState| {
                r.regex.captures(&s).map(|caps| {
                    let groups = caps
                        .iter()
                        .map(|g| g.map(|g| g.as_str().to_string()))
                        .collect();
                    let names = r
                        .regex
                        .capture_names()
                        .enumerate()
                        .filter_map(|(i, n)| n.map(|n| (n.to_string(), i)))
                        .collect();
                    NativeMatchState { groups, names }
                })
            })?;
            match state {
                None => Ok(vm.new_nil(mc)),
                Some(state) => {
                    let class = vm.get_or_create_builtin_class(mc, "Match");
                    let obj = vm.new_object(mc, class);
                    let boxed: Box<dyn AnyCollect> = Box::new(state);
                    obj.borrow_mut(mc).payload =
                        ObjectPayload::NativeState(crate::gc!(mc, RefLock::new(boxed)));
                    Ok(Value::Object(obj))
                }
            }
        })
        .returns("Match?")
        .doc(
            "The first match of the pattern in the String argument, as a Match — or nil \
             when the pattern does not match, so a nil-guard is the miss test. Capture \
             groups (positional and named) ride on the Match.\n\n\
             ```\n\
             (#/(\\d+)-(\\d+)/.match:'10-20').s     \"* -> 10-20\n\
             #/x/.match:'abc'                      \"* -> nil\n\
             ```",
        )
}

/// One resolved regex match, detached from both the pattern and the subject
/// string: the group texts are owned copies, so the state holds no GC
/// references and no borrow of the subject.
#[derive(Debug)]
pub struct NativeMatchState {
    /// Capture-group texts by index — 0 is the whole match (always present),
    /// `None` a group that did not participate in the match.
    pub groups: Vec<Option<String>>,
    /// Named-group table: group name → index into `groups`.
    pub names: Vec<(String, usize)>,
}

impl NativeMatchState {
    fn group_text(&self, idx: usize) -> Option<String> {
        self.groups.get(idx).cloned().flatten()
    }

    fn named_text(&self, name: &str) -> Option<String> {
        self.names
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, i)| self.group_text(*i))
    }
}

impl AnyCollect for NativeMatchState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {
        // Owned strings only — no GC references.
    }
}

impl PrettyPrint for NativeMatchState {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        // The whole match plus every participating group, labeled by name when
        // the group has one, by index otherwise (non-participating groups are
        // omitted rather than rendered as a pseudo-nil).
        let mut fields = vec![(
            "s".to_string(),
            PpChild::Text(self.groups[0].clone().unwrap_or_default(), PpRole::Str),
        )];
        for (i, g) in self.groups.iter().enumerate().skip(1) {
            let Some(text) = g else { continue };
            let label = self
                .names
                .iter()
                .find(|(_, gi)| *gi == i)
                .map(|(n, _)| n.clone())
                .unwrap_or_else(|| i.to_string());
            fields.push((label, PpChild::Text(text.clone(), PpRole::Str)));
        }
        PpShape::Record {
            name: "Match",
            fields,
        }
    }
}

pub fn build_match_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Match", Some("Object"))
        .construct_with("use Regex match: (#/…/.match:'…')")
        .class_doc(
            "One resolved regex match — what `Regex.match:` answers. `s` is the matched \
             text, `at:` reads a capture group by index (1-based; 0 is the whole match) \
             or by name, `captures` lists them all, and `bind:` destructures them into a \
             block's parameters.\n\n\
             ```\n\
             (#/(?<hour>\\d+):(?<min>\\d+)/.match:'at 9:40').at:'min'     \"* -> 40\n\
             ```",
        )
        .instance_method("s", |vm, mc, receiver, _args| {
            let text = receiver
                .with_native_state(|m: &NativeMatchState| m.groups[0].clone().unwrap_or_default())?;
            Ok(vm.new_string(mc, text))
        })
        .returns("String")
        .doc(
            "The text the whole pattern matched (capture group 0).\n\n\
             ```\n\
             (#/b+/.match:'abbbc').s     \"* -> bbb\n\
             ```",
        )
        .instance_method("at:", |vm, mc, receiver, args| {
            enum Key {
                Idx(i64),
                Name(String),
            }
            let key = match args.first() {
                Some(&Value::Int(i)) => Key::Idx(i),
                Some(&Value::Object(o)) => match &o.borrow().payload {
                    ObjectPayload::String(s) => Key::Name(s.to_string()),
                    ObjectPayload::Symbol(s) => Key::Name(s.to_string()),
                    _ => {
                        return Err(QuoinError::TypeError {
                            expected: "Integer, String, or Symbol".to_string(),
                            got: args
                                .first()
                                .map_or("None".to_string(), |v| v.type_name().to_string()),
                            msg: "Match at: takes a group index or a group name".to_string(),
                        });
                    }
                },
                _ => {
                    return Err(QuoinError::TypeError {
                        expected: "Integer, String, or Symbol".to_string(),
                        got: args
                            .first()
                            .map_or("None".to_string(), |v| v.type_name().to_string()),
                        msg: "Match at: takes a group index or a group name".to_string(),
                    });
                }
            };
            let text = receiver.with_native_state(|m: &NativeMatchState| match &key {
                Key::Idx(i) => usize::try_from(*i).ok().and_then(|i| m.group_text(i)),
                Key::Name(n) => m.named_text(n),
            })?;
            Ok(text.map_or_else(|| vm.new_nil(mc), |t| vm.new_string(mc, t)))
        })
        .returns("String?")
        .doc(
            "A capture group's text: an Integer reads by position (1-based; 0 is the whole \
             match), a String or Symbol by group name. Nil for a group that is out of \
             range, unknown, or did not participate in the match.\n\n\
             ```\n\
             (#/(\\d+)-(\\d+)/.match:'10-20').at:2                \"* -> 20\n\
             (#/(?<a>x)|(?<b>y)/.match:'y').at:'a'              \"* -> nil\n\
             ```",
        )
        .instance_method("captures", |vm, mc, receiver, _args| {
            let groups =
                receiver.with_native_state(|m: &NativeMatchState| m.groups[1..].to_vec())?;
            let items = groups
                .into_iter()
                .map(|g| g.map_or_else(|| vm.new_nil(mc), |t| vm.new_string(mc, t)))
                .collect();
            Ok(vm.new_list(mc, items))
        })
        .returns("List(String?)")
        .doc(
            "Every capture group's text in position order (group 0, the whole match, is \
             `s` — not listed here). A group that did not participate is nil.\n\n\
             ```\n\
             (#/(\\d+)-(\\d+)/.match:'10-20').captures     \"* -> #(10 20)\n\
             ```",
        )
        .instance_method("bind:", |vm, mc, receiver, args| {
            let block = crate::arg!(args, Block, 0);
            let texts = receiver.with_native_state(|m: &NativeMatchState| {
                block
                    .template
                    .param_syms
                    .iter()
                    .enumerate()
                    .map(|(i, sym)| {
                        // A parameter whose name is a named group binds that
                        // group; anything else binds positionally (param i →
                        // group i+1).
                        let name = sym.as_str();
                        if m.names.iter().any(|(n, _)| n == name) {
                            m.named_text(name)
                        } else {
                            m.group_text(i + 1)
                        }
                    })
                    .collect::<Vec<_>>()
            })?;
            let block_args = texts
                .into_iter()
                .map(|t| t.map_or_else(|| vm.new_nil(mc), |t| vm.new_string(mc, t)))
                .collect();
            vm.execute_block(mc, block, block_args, None)
        })
        .doc(
            "Destructure the capture groups into a block: a parameter named after a named \
             group gets that group's text, any other parameter binds positionally (first \
             parameter → group 1, and so on). A group that did not participate binds nil. \
             Answers the block's value.\n\n\
             ```\n\
             (#/(\\d+)-(\\d+)/.match:'10-20').bind:{ |lo hi| hi.to_integer - lo.to_integer }     \"* -> 10\n\
             ```",
        )
}
