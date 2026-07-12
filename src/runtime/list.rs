use crate::arg;
use crate::devirt_ops;
use crate::dispatch::Callable;
use crate::error::QuoinError;
use crate::runtime::elem_tag::{ElemTag, check_insert};
use crate::runtime::pretty::{PpShape, PrettyPrint};
use crate::symbol::Symbol;
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmState;
use gc_arena::Mutation as GcMutation;

use gc_arena::Mutation;
use gc_arena::collect::{DynCollect, Trace};
use std::any::Any;
use std::mem::transmute;

#[derive(Debug)]
pub struct NativeListState {
    pub vec: Vec<Value<'static>>,
    /// Checked element type (docs/internal/GENERICS_ARCH.md). `None` — every list the
    /// pre-existing world builds — means no checks anywhere.
    pub elem: Option<ElemTag>,
}

impl NativeListState {
    pub fn new(vec: Vec<Value<'_>>) -> Self {
        let vec_static: Vec<Value<'static>> = unsafe { transmute(vec) };
        Self {
            vec: vec_static,
            elem: None,
        }
    }

    pub fn get_vec<'gc>(&self) -> &[Value<'gc>] {
        unsafe { transmute(&self.vec[..]) }
    }

    pub fn get_vec_mut<'gc>(&mut self) -> &mut Vec<Value<'gc>> {
        unsafe { transmute(&mut self.vec) }
    }
}

impl PrettyPrint for NativeListState {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        PpShape::Seq {
            open: "#(",
            close: ")",
            items: self.get_vec().to_vec(),
        }
    }
}

impl AnyCollect for NativeListState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>) {
        for val in &self.vec {
            let val_gc: &Value<'gc> = unsafe { transmute(val) };
            val_gc.dyn_trace(cc);
        }
    }
}

/// A fresh List value carrying an element tag (`List.of:`, `ensure:`,
/// tag-propagating copies like `sliceFrom:`).
pub fn new_list_with_tag<'gc>(
    vm: &VmState<'gc>,
    mc: &GcMutation<'gc>,
    vec: Vec<Value<'gc>>,
    tag: Option<ElemTag>,
) -> Value<'gc> {
    let v = vm.new_list(mc, vec);
    if tag.is_some() {
        let _ = v.with_native_state_mut::<NativeListState, _, _>(mc, |l| l.elem = tag);
    }
    v
}

/// Fetch `list[idx]` during an in-place sort as a catchable failure — never a Rust
/// index panic — in case a user comparator/key block shrank the receiver mid-sort.
/// (List exposes no shrink primitive today; defense in depth for when one lands.)
fn sort_fetch<'gc>(list: Value<'gc>, idx: usize) -> Result<Value<'gc>, QuoinError> {
    list.with_native_state::<NativeListState, _, _>(|l| l.get_vec().get(idx).copied())
        .map_err(QuoinError::Other)?
        .ok_or_else(|| {
            QuoinError::ValueError("List was shrunk by its sort block during sort".to_string())
        })
}

/// Swap `list[j-1]` and `list[j]` during an in-place sort, bounds-checked like
/// [`sort_fetch`].
fn sort_swap<'gc>(mc: &Mutation<'gc>, list: Value<'gc>, j: usize) -> Result<(), QuoinError> {
    let in_range = list
        .with_native_state_mut(mc, |l: &mut NativeListState| {
            let v = l.get_vec_mut();
            if j >= 1 && j < v.len() {
                v.swap(j - 1, j);
                true
            } else {
                false
            }
        })
        .map_err(QuoinError::Other)?;
    if in_range {
        Ok(())
    } else {
        Err(QuoinError::ValueError(
            "List was shrunk by its sort block during sort".to_string(),
        ))
    }
}

pub fn build_list_class() -> NativeClassBuilder {
    NativeClassBuilder::new("List", Some("Object"))
        .construct_with("use #( … ) literals")
        .class_doc(
            "The ordered, growable sequence — Quoin's workhorse collection, written \
             `#(10 20 30)`.\n\nIndexes are zero-based. A list holds any mix of values; one \
             checked to a single element class comes from `List.of:` / `ensure:`. The natives \
             here are the storage primitives; the wider iteration surface (`each:`, `collect:`, \
             `select:`, …) is the Iterate mixin in the standard library, built on them.",
        )
        .instance_method("count", |vm, mc, receiver, _args| {
            let len = receiver
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;
            Ok(vm.new_int(mc, len as i64))
        })
        .returns("Integer")
        .doc(
            "The number of elements.\n\n\
             ```\n\
             #(10 20 30).count     \"* -> 3\n\
             ```",
        )
        // `List.new` — a fresh empty native list. Without this, the generic
        // `Callable::NewNoBlock` fallback mints a payload-less Object shell of
        // class List whose first native method call dies with the internal
        // "Not a native state" error (QUOIN_TODO.md). A `new:` config block is
        // meaningless on a native collection — refuse it clearly rather than
        // mint the same poison object.
        .class_method("new", |vm, mc, _receiver, _args| {
            Ok(vm.new_list(mc, Vec::new()))
        })
        .doc(
            "A fresh empty list — the same value the `#()` literal builds.\n\n\
             ```\n\
             List.new.count     \"* -> 0\n\
             ```",
        )
        .class_method("new:", |_vm, _mc, _receiver, _args| {
            Err(QuoinError::Other(
                "List has no instance fields — construct with `#()`, `List.new`, or `List.of:`"
                    .to_string(),
            ))
        })
        .doc(
            "Always refused: a List has no instance fields for a `new:` config block to set. \
             Construct with `#()`, `List.new`, or `List.of:`.",
        )
        // --- checked generics (docs/internal/GENERICS_ARCH.md §4.2/§6) ---
        // `List.of:Integer` — a fresh empty list tagged with the element class.
        .class_method("of:", |vm, mc, _receiver, args| {
            let tag = ElemTag::from_class_value(&args[0]).ok_or_else(|| QuoinError::TypeError {
                expected: "Class".to_string(),
                got: args[0].type_name().to_string(),
                msg: "List.of: expects an element class (e.g. `List.of:Integer`)".to_string(),
            })?;
            Ok(new_list_with_tag(vm, mc, Vec::new(), Some(tag)))
        })
        .doc(
            "A fresh empty list tagged with an element class: every later insert is checked, \
             so an `add:` / `at:put:` of a non-matching value raises a catchable TypeError.\n\n\
             ```\n\
             (List.of:Integer).add:1     \"* -> #(1)\n\
             ```",
        )
        // `ensure:` — verify every element, return a NEW tagged list (a copy:
        // retagging an aliased list in place is spooky action; GENERICS_ARCH §4.2).
        .instance_method("ensure:", |vm, mc, receiver, args| {
            let tag = ElemTag::from_class_value(&args[0]).ok_or_else(|| QuoinError::TypeError {
                expected: "Class".to_string(),
                got: args[0].type_name().to_string(),
                msg: "ensure: expects an element class (e.g. `xs.ensure:Integer`)".to_string(),
            })?;
            let vec: Vec<Value> = receiver
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
                .map_err(QuoinError::Other)?;
            for (i, v) in vec.iter().enumerate() {
                check_insert(Some(tag), "List", v, Some(i as i64), |v, n| {
                    vm.value_matches_type(*v, n)
                })?;
            }
            Ok(new_list_with_tag(vm, mc, vec, Some(tag)))
        })
        .doc(
            "Check every element against a class and answer a NEW list carrying that element \
             tag; a non-matching element raises a catchable TypeError. The receiver is copied, \
             not retagged in place, so other holders of the untagged list are unaffected.\n\n\
             ```\n\
             (#(1 2 3).ensure:Integer).elementType     \"* -> Integer\n\
             ```",
        )
        // `emptyLike` — the species protocol (GENERICS_ARCH.md §4.5): a fresh
        // empty collection LIKE the receiver, element tag included. Iterate's
        // default is `self.class.default`; the natives override to carry tags.
        .instance_method("emptyLike", |vm, mc, receiver, _args| {
            let tag = receiver
                .with_native_state::<NativeListState, _, _>(|l| l.elem)
                .map_err(QuoinError::Other)?;
            Ok(new_list_with_tag(vm, mc, Vec::new(), tag))
        })
        .returns("List(T)") // emptyLike: same shape, same tag, empty
        .doc(
            "A fresh empty list like the receiver — element tag included. The species hook the \
             Iterate mixin uses, so transforms of a checked list stay checked.",
        )
        // The element tag as a Symbol (`#Integer`), or nil when untagged.
        .instance_method("elementType", |vm, mc, receiver, _args| {
            let tag = receiver
                .with_native_state::<NativeListState, _, _>(|l| l.elem)
                .map_err(QuoinError::Other)?;
            Ok(match tag {
                Some(t) => vm.new_symbol(mc, t.name().to_string()),
                None => Value::Nil,
            })
        })
        .doc(
            "The checked element type as a Symbol, or nil for an ordinary untagged list.\n\n\
             ```\n\
             (List.of:Integer).elementType     \"* -> Integer\n\
             ```",
        )
        .instance_method("add:", |vm, mc, receiver, args| {
            let tag = receiver
                .with_native_state::<NativeListState, _, _>(|l| l.elem)
                .map_err(QuoinError::Other)?;
            check_insert(tag, "List", &args[0], None, |v, n| {
                vm.value_matches_type(*v, n)
            })?;
            receiver
                .with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                    let vec = l.get_vec_mut();
                    vec.push(args[0]);
                })
                .map_err(|e| QuoinError::Other(e))?;
            Ok(receiver)
        })
        .doc(
            "Append a value at the end; answers the receiver, so adds chain. On a tagged list \
             (`List.of:`) the value is checked first.\n\n\
             ```\n\
             #(1 2).add:3     \"* -> #(1 2 3)\n\
             ```",
        )
        .instance_method("push:", |vm, mc, receiver, args| {
            let tag = receiver
                .with_native_state::<NativeListState, _, _>(|l| l.elem)
                .map_err(QuoinError::Other)?;
            check_insert(tag, "List", &args[0], None, |v, n| {
                vm.value_matches_type(*v, n)
            })?;
            receiver
                .with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                    let vec = l.get_vec_mut();
                    vec.insert(0, args[0]);
                })
                .map_err(|e| QuoinError::Other(e))?;
            Ok(receiver)
        })
        .doc(
            "Insert a value at the front (index 0); answers the receiver. The end-of-list \
             counterpart is `add:`.\n\n\
             ```\n\
             #(2 3).push:1     \"* -> #(1 2 3)\n\
             ```",
        )
        // `join:` — the LINEAR native for the hot List case; the Iterate
        // mixin version (qnlib core/02-iterate.qn) still serves every other
        // receiver, but it is a QUADRATIC interpreted `+` loop — the biggest
        // measured cross-language asymmetry in the strings bench (Python and
        // Ruby join in one linear native call). Semantics match the mixin
        // exactly: the separator is appended only while the ACCUMULATED
        // result is non-empty (so an empty first piece suppresses it), and
        // elements go through `.s`. Elements are re-read per iteration, so a
        // user `.s` that mutates the list sees it live, like `each:`. One
        // probe decides whether the `.s` identity dispatch can be skipped
        // for String elements: only the boot registry installs
        // `Callable::Native`, so a Native entry at (String, "s") means
        // "not overridden" — otherwise every element pays the full dispatch
        // the mixin would.
        .typed_instance_method("join:", &["String"], |vm, mc, receiver, args| {
            let sep: String = (*arg!(args, String, 0)).clone();
            let pristine_s = matches!(
                vm.lookup_method(mc, args[0], Symbol::intern("s"), &[]),
                Ok(Some(Callable::Native(_)))
            );
            let mut out = String::new();
            let mut i: usize = 0;
            loop {
                let elem = match receiver
                    .with_native_state::<NativeListState, _, _>(|l| l.get_vec().get(i).copied())
                {
                    Ok(Some(v)) => v,
                    Ok(None) => break,
                    Err(e) => return Err(QuoinError::Other(e)),
                };
                if !out.is_empty() {
                    out.push_str(&sep);
                }
                let direct = if pristine_s
                    && let Value::Object(o) = elem
                    && let ObjectPayload::String(s) = &o.borrow().payload
                {
                    Some(*s)
                } else {
                    None
                };
                match direct {
                    Some(s) => out.push_str(&s),
                    None => {
                        let v1 = vm.call_method(mc, elem, "s", vec![])?;
                        let pushed = if let Value::Object(o) = v1
                            && let ObjectPayload::String(s) = &o.borrow().payload
                        {
                            out.push_str(s);
                            true
                        } else {
                            false
                        };
                        if !pushed {
                            // Mirror the mixin's `result + x.s`: a non-String
                            // `.s` result rides the untyped `+:` coercion
                            // (one more `.s`, then Display).
                            let v2 = vm.call_method(mc, v1, "s", vec![])?;
                            if let Value::Object(o) = v2
                                && let ObjectPayload::String(s) = &o.borrow().payload
                            {
                                out.push_str(s);
                            } else {
                                out.push_str(&format!("{}", v2));
                            }
                        }
                    }
                }
                i += 1;
            }
            Ok(vm.new_string(mc, out))
        })
        .returns("String")
        .doc(
            "One string from the elements' `.s` renderings with the separator between them. \
             The separator only appears while the accumulated result is non-empty, so an empty \
             leading piece adds nothing.\n\n\
             ```\n\
             #(1 2 3).join:', '     \"* -> 1, 2, 3\n\
             ```",
        )
        // The index is typed, so a non-Integer index matches no variant -> MNU
        // (dispatch enforces the type instead of a hand-rolled TypeError).
        .typed_instance_method("at:", &["Integer"], |vm, mc, receiver, args| {
            let idx = arg!(args, Int, 0);
            let got = receiver
                .with_native_state::<NativeListState, _, _>(|l| {
                    devirt_ops::list_get(l.get_vec(), idx)
                })
                .map_err(QuoinError::Other)?;
            Ok(got.unwrap_or_else(|| vm.new_nil(mc)))
        })
        // Element-typed read: on a `List(Integer)` receiver the checker binds
        // T and sees `Integer?` (out-of-bounds is nil). `T` is the seeded
        // type parameter of the builtin List (class_table.rs).
        .returns("T?")
        .doc(
            "The element at a zero-based index; any out-of-range index (negatives included) \
             answers nil.\n\n\
             ```\n\
             #(10 20 30).at:1     \"* -> 20\n\
             ```",
        )
        // Only the index is typed (`&["Integer"]`); the value (arg 2) is any type.
        .typed_instance_method("at:put:", &["Integer"], |vm, mc, receiver, args| {
            let idx = arg!(args, Int, 0);
            let val = args[1];
            let tag = receiver
                .with_native_state::<NativeListState, _, _>(|l| l.elem)
                .map_err(QuoinError::Other)?;
            check_insert(tag, "List", &val, Some(idx), |v, n| {
                vm.value_matches_type(*v, n)
            })?;
            receiver
                .with_native_state_mut(mc, |l: &mut NativeListState| {
                    devirt_ops::list_set(l.get_vec_mut(), idx, val)
                })
                .map_err(QuoinError::Other)??;
            Ok(receiver)
        })
        .doc(
            "Replace the element at a zero-based index; answers the receiver. An out-of-range \
             index raises a catchable IndexError — unlike `at:`, writing never extends the \
             list. On a tagged list (`List.of:`) the value is checked first.\n\n\
             ```\n\
             #(1 2 3).at:1 put:99     \"* -> #(1 99 3)\n\
             ```",
        )
        .typed_instance_method("sliceFrom:", &["Integer"], |vm, mc, receiver, args| {
            let idx = arg!(args, Int, 0);
            receiver
                .with_native_state::<NativeListState, _, _>(|l| {
                    let vec = l.get_vec();
                    let start = idx.max(0) as usize;
                    let sliced = if start < vec.len() {
                        vec[start..].to_vec()
                    } else {
                        Vec::new()
                    };
                    // A slice's elements are already checked — carry the tag.
                    Ok(new_list_with_tag(vm, mc, sliced, l.elem))
                })
                .map_err(|e| QuoinError::Other(e))?
        })
        .returns("List(T)") // sliceFrom: carries the receiver's tag
        .doc(
            "A new list of the elements from a zero-based index to the end; an index past the \
             end answers `#()`. A checked receiver's element tag carries over.\n\n\
             ```\n\
             #(10 20 30 40).sliceFrom:2     \"* -> #(30 40)\n\
             ```",
        )
        .typed_instance_method(
            "sliceFrom:to:",
            &["Integer", "Integer"],
            |vm, mc, receiver, args| {
                let from = arg!(args, Int, 0);
                let to = arg!(args, Int, 1);
                receiver
                    .with_native_state::<NativeListState, _, _>(|l| {
                        let vec = l.get_vec();
                        let start = from.max(0) as usize;
                        let end = (to.max(0) as usize).min(vec.len());
                        let sliced = if start < end {
                            vec[start..end].to_vec()
                        } else {
                            Vec::new()
                        };
                        // A slice's elements are already checked — carry the tag.
                        Ok(new_list_with_tag(vm, mc, sliced, l.elem))
                    })
                    .map_err(|e| QuoinError::Other(e))?
            },
        )
        .returns("List(T)") // sliceFrom:to: carries the receiver's tag
        .doc(
            "A new list of the elements from a zero-based start index to an exclusive end \
             index, both clamped to the list's bounds (an empty or inverted range answers \
             `#()`). A checked receiver's element tag carries over.\n\n\
             ```\n\
             #(10 20 30 40).sliceFrom:1 to:3     \"* -> #(20 30)\n\
             ```",
        )
        .instance_method("s", |vm, mc, receiver, _args| {
            let len = receiver
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;

            let mut parts = Vec::new();
            for i in 0..len {
                let val = receiver
                    .with_native_state::<NativeListState, _, _>(|l| l.get_vec().get(i).copied())
                    .map_err(|e| QuoinError::Other(e))?
                    .ok_or_else(|| QuoinError::Other("Index out of bounds".to_string()))?;

                let result = vm.call_method(mc, val, "s", vec![])?;
                let part = if let Value::Object(obj) = result {
                    if let ObjectPayload::String(s) = &obj.borrow().payload {
                        s.to_string()
                    } else {
                        format!("{}", result)
                    }
                } else {
                    format!("{}", result)
                };
                parts.push(part);
            }

            Ok(vm.new_string(mc, format!("#({})", parts.join(" "))))
        })
        .doc(
            "The display string: `#(` and `)` around each element's `.s`, space-separated.\n\n\
             ```\n\
             #(1 'two' 3).s     \"* -> #(1 two 3)\n\
             ```",
        )
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_len = receiver
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;
            let rhs_len_res =
                args[0].with_native_state::<NativeListState, _, _>(|l| l.get_vec().len());
            let rhs_len = match rhs_len_res {
                Ok(len) => len,
                Err(_) => return Ok(vm.new_bool(mc, false)),
            };

            if lhs_len != rhs_len {
                return Ok(vm.new_bool(mc, false));
            }

            for i in 0..lhs_len {
                let lhs_val = receiver
                    .with_native_state::<NativeListState, _, _>(|l| l.get_vec().get(i).copied())
                    .map_err(|e| QuoinError::Other(e))?
                    .ok_or_else(|| QuoinError::Other("Index out of bounds".to_string()))?;
                let rhs_val = args[0]
                    .with_native_state::<NativeListState, _, _>(|l| l.get_vec().get(i).copied())
                    .map_err(|e| QuoinError::Other(e))?
                    .ok_or_else(|| QuoinError::Other("Index out of bounds".to_string()))?;

                let eq_res = vm.call_method(mc, lhs_val, "==:", vec![rhs_val])?.is_true();
                if !eq_res {
                    return Ok(vm.new_bool(mc, false));
                }
            }

            Ok(vm.new_bool(mc, true))
        })
        .doc(
            "Element-wise equality: true when the other value is a List of the same length \
             whose elements are pairwise `==`. Anything that is not a List answers false.\n\n\
             ```\n\
             #(1 2 3) == #(1 2 3)     \"* -> true\n\
             ```",
        )
        .instance_method("bind:", |vm, mc, receiver, args| {
            let block = arg!(args, Block, 0);
            let block_args = receiver.with_native_state(|l: &NativeListState| {
                l.get_vec()
                    .iter()
                    .take(block.template.param_syms.len())
                    .map(|v| unsafe { transmute(*v) })
                    .collect::<Vec<_>>()
            })?;

            vm.execute_block(mc, block, block_args, None)
        })
        .doc(
            "Destructure into a block: call it with the list's elements as its arguments — a \
             two-parameter block gets the first two elements, extras are ignored. Answers the \
             block's value.\n\n\
             ```\n\
             #(3 4).bind:{ |w h| w * h }     \"* -> 12\n\
             ```",
        )
        .instance_method("sort", |vm, mc, receiver, _args| {
            let len = receiver
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;

            for i in 1..len {
                let mut j = i;
                while j > 0 {
                    let active_args = vm.active_native_args.last().unwrap();
                    let recv = active_args.receiver;
                    let val_prev = sort_fetch(recv, j - 1)?;
                    let val_curr = sort_fetch(recv, j)?;

                    let gt_res = if val_prev.is_nil() {
                        !val_curr.is_nil()
                    } else if val_curr.is_nil() {
                        false
                    } else {
                        vm.call_method(mc, val_prev, ">:", vec![val_curr])?
                            .is_true()
                    };

                    if gt_res {
                        let active_args = vm.active_native_args.last().unwrap();
                        sort_swap(mc, active_args.receiver, j)?;
                        j -= 1;
                    } else {
                        break;
                    }
                }
            }

            let active_args = vm.active_native_args.last().unwrap();
            Ok(active_args.receiver)
        })
        .doc(
            "Sort in place, ascending by the elements' `>:`, and answer the receiver; nils \
             sort last.\n\n\
             ```\n\
             #(3 1 2).sort     \"* -> #(1 2 3)\n\
             ```",
        )
        .instance_method("sort:", |vm, mc, receiver, args| {
            let block_gc = arg!(args, Block, 0);
            let len = receiver
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;

            let arity = block_gc.template.param_syms.len();
            if arity == 1 {
                for i in 1..len {
                    let mut j = i;
                    while j > 0 {
                        let active_args = vm.active_native_args.last().unwrap();
                        let val_prev = sort_fetch(active_args.receiver, j - 1)?;

                        let key_lhs = vm.call_method(
                            mc,
                            active_args.arg(&vm.stack, 0).unwrap(),
                            "valueWithArgs:",
                            vec![vm.new_list(mc, vec![val_prev])],
                        )?;
                        vm.push(key_lhs);

                        let active_args = vm.active_native_args.last().unwrap();
                        let val_curr = sort_fetch(active_args.receiver, j)?;

                        let key_rhs = vm.call_method(
                            mc,
                            active_args.arg(&vm.stack, 0).unwrap(),
                            "valueWithArgs:",
                            vec![vm.new_list(mc, vec![val_curr])],
                        )?;
                        let key_lhs = vm.pop()?;

                        let gt_res = if key_lhs.is_nil() {
                            !key_rhs.is_nil()
                        } else if key_rhs.is_nil() {
                            false
                        } else {
                            vm.call_method(mc, key_lhs, ">:", vec![key_rhs])?.is_true()
                        };

                        if gt_res {
                            let active_args = vm.active_native_args.last().unwrap();
                            sort_swap(mc, active_args.receiver, j)?;
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                }
            } else {
                for i in 1..len {
                    let mut j = i;
                    while j > 0 {
                        let active_args = vm.active_native_args.last().unwrap();
                        let recv = active_args.receiver;
                        let val_prev = sort_fetch(recv, j - 1)?;
                        let val_curr = sort_fetch(recv, j)?;

                        let res = vm.call_method(
                            mc,
                            active_args.arg(&vm.stack, 0).unwrap(),
                            "valueWithArgs:",
                            vec![vm.new_list(mc, vec![val_prev, val_curr])],
                        )?;

                        if !res.is_true() {
                            let active_args = vm.active_native_args.last().unwrap();
                            sort_swap(mc, active_args.receiver, j)?;
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                }
            }

            let active_args = vm.active_native_args.last().unwrap();
            Ok(active_args.receiver)
        })
        .doc(
            "Sort in place, directed by the block, and answer the receiver. A one-parameter \
             block is a KEY: elements are ordered by their keys' `>:`, ascending. A \
             two-parameter block is a COMPARATOR: it answers true when its arguments are \
             already in order.\n\n\
             ```\n\
             #('bb' 'a' 'ccc').sort:{ |x| x.length }     \"* -> #(a bb ccc)\n\
             #(3 1 2).sort:{ |a b| a >= b }     \"* -> #(3 2 1)\n\
             ```",
        )
}
