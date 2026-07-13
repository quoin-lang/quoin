use crate::error::QuoinError;
use crate::ext_sdk::HostExt;
use crate::runtime::elem_tag::{ElemTag, check_insert};
use crate::runtime::map::{keys_equal, map_hash_key};
use crate::runtime::pretty::{PpShape, PrettyPrint};
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmState;

use gc_arena::collect::{DynCollect, Trace};
use rustc_hash::FxHashMap;
use std::any::Any;
use std::mem::transmute;

/// An insertion-ordered set of unique values, hash-indexed like the Map
/// (`NativeMapState`): each element caches its hash — a user instance's
/// `hash` method dispatches once at insert — so membership is O(1) bucket
/// probe + the same equality ladder Maps use (native `==`, definitive miss
/// between exact types, guest `==:` otherwise). Uniqueness therefore still
/// matches `List#uniq` semantics exactly; it just stopped costing a guest
/// send per element.
#[derive(Debug)]
pub struct NativeSetState {
    entries: Vec<(u64, Value<'static>)>,
    index: FxHashMap<u64, Vec<u32>>,
    /// Checked element type (docs/internal/GENERICS_ARCH.md). `None` = untagged.
    pub elem: Option<ElemTag>,
}

/// The linear/indexed tier boundary, mirroring `NativeMapState` exactly (see
/// the type doc there): at or below this size the index stays empty and
/// membership scans the cached hashes; the 20k-sweep workloads that motivated
/// the index cross over within their first insertions.
const SMALL_LINEAR_MAX: usize = 16;

impl NativeSetState {
    pub fn new_empty() -> Self {
        Self {
            entries: Vec::new(),
            index: FxHashMap::default(),
            elem: None,
        }
    }

    /// Whether the index tier is active — derived from the entry count,
    /// never stored (`entries.len() > SMALL_LINEAR_MAX ⇔ index populated`).
    #[inline]
    fn indexed(&self) -> bool {
        self.entries.len() > SMALL_LINEAR_MAX
    }

    /// (Re)build the index from the cached hashes — no dispatch.
    fn build_index(&mut self) {
        self.index.clear();
        for (i, (h, _)) in self.entries.iter().enumerate() {
            self.index.entry(*h).or_default().push(i as u32);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The elements in insertion order (cloned out — callers usually did
    /// `.to_vec()` on the old slice anyway).
    pub fn values<'gc>(&self) -> Vec<Value<'gc>> {
        self.entries
            .iter()
            .map(|(_, v)| unsafe { transmute::<Value<'static>, Value<'gc>>(*v) })
            .collect()
    }

    pub fn value_at<'gc>(&self, idx: u32) -> Value<'gc> {
        unsafe { transmute(self.entries[idx as usize].1) }
    }

    /// The hash's candidate `(index, element)` pairs — cloned OUT so callers
    /// drop the borrow before dispatching guest `==:`. Linear tier scans the
    /// cached hashes; indexed tier probes the bucket.
    pub fn bucket<'gc>(&self, hash: u64) -> Vec<(u32, Value<'gc>)> {
        let candidate = |i: u32| {
            (i, unsafe {
                transmute::<Value<'static>, Value<'gc>>(self.entries[i as usize].1)
            })
        };
        if !self.indexed() {
            return self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, (h, _))| *h == hash)
                .map(|(i, _)| candidate(i as u32))
                .collect();
        }
        self.index
            .get(&hash)
            .map(|ixs| ixs.iter().map(|&i| candidate(i)).collect())
            .unwrap_or_default()
    }

    /// Append a NEW element (caller has established absence).
    pub fn append(&mut self, hash: u64, value: Value<'_>) {
        let i = self.entries.len() as u32;
        self.entries.push((hash, unsafe {
            transmute::<Value<'_>, Value<'static>>(value)
        }));
        if self.indexed() {
            if self.index.is_empty() {
                self.build_index(); // this push crossed the tier boundary
            } else {
                self.index.entry(hash).or_default().push(i);
            }
        }
    }

    /// Remove by index, preserving order; rebuilds the index from cached
    /// hashes (no dispatch) — or drops it when back in the linear tier.
    pub fn remove_at(&mut self, idx: u32) {
        self.entries.remove(idx as usize);
        if self.indexed() {
            self.build_index();
        } else {
            self.index.clear();
        }
    }

    /// Copy contents (entries + index, cached hashes and all) — `ensure:`
    /// rebuilds a set from an existing one without re-hashing.
    #[allow(clippy::type_complexity)] // (hashed entries, hash->indices) snapshot
    pub fn clone_contents(&self) -> (Vec<(u64, Value<'static>)>, FxHashMap<u64, Vec<u32>>) {
        (self.entries.clone(), self.index.clone())
    }

    #[allow(clippy::type_complexity)] // mirror of clone_contents' (entries, index) snapshot
    pub fn adopt_contents(
        &mut self,
        contents: (Vec<(u64, Value<'static>)>, FxHashMap<u64, Vec<u32>>),
    ) {
        self.entries = contents.0;
        self.index = contents.1;
    }
}

impl PrettyPrint for NativeSetState {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        PpShape::Seq {
            open: "#<",
            close: ">",
            items: self.values(),
        }
    }
}

impl AnyCollect for NativeSetState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>) {
        for (_, val) in &self.entries {
            let val_gc: &Value<'gc> = unsafe { transmute(val) };
            val_gc.dyn_trace(cc);
        }
    }
}

/// Find `value`'s entry: `(hash, Some(index))` on a hit. Shares the Map's
/// hash + equality ladder (`map_hash_key`/`keys_equal`), including the
/// stack-rooting of bucket candidates across a parking `==:` hook.
pub(crate) fn set_find<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    set_val: Value<'gc>,
    value: Value<'gc>,
) -> Result<(u64, Option<u32>), QuoinError> {
    let h = map_hash_key(vm, mc, value)?;
    let bucket = set_val
        .with_native_state::<NativeSetState, _, _>(|s| s.bucket(h))
        .map_err(QuoinError::Other)?;
    let base = vm.stack.len();
    for (_, e) in &bucket {
        vm.push(*e);
    }
    let mut hit = None;
    for (i, (idx, _)) in bucket.iter().enumerate() {
        let e = vm.stack[base + i];
        match keys_equal(vm, mc, e, value) {
            Ok(true) => {
                hit = Some(*idx);
                break;
            }
            Ok(false) => {}
            Err(err) => {
                vm.stack.truncate(base);
                return Err(err);
            }
        }
    }
    vm.stack.truncate(base);
    Ok((h, hit))
}

pub fn build_set_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Set", Some("Object"))
        .construct_with("use #< … > literals")
        .class_doc(
            "The insertion-ordered collection of unique values, written `#<1 2 3>`. Adding an \
             element that is already present is a no-op.\n\nMembership is hash-indexed: an \
             element's `hash` runs once, at insert, and lookup uses the same equality ladder \
             as Map keys — by value for scalars and strings, by identity for user instances \
             unless their class overrides both `hash` and `==:`. A set with a checked element \
             type comes from `Set.of:` / `ensure:`.",
        )
        .sdk_instance_method("count", |host, receiver, _args| {
            let len = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.len())
                .map_err(QuoinError::Other)?;
            Ok(host.new_int(len as i64))
        })
        .returns("Integer")
        .doc(
            "The number of elements.\n\n\
             ```\n\
             #<1 2 3>.count     \"* -> 3\n\
             ```",
        )
        .instance_method("add:", |vm, mc, receiver, args| {
            let tag = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.elem)
                .map_err(QuoinError::Other)?;
            check_insert(tag, "Set", &args[0], None, |v, n| {
                vm.value_matches_type(*v, n)
            })?;
            vm.set_add(mc, receiver, args[0])?;
            Ok(receiver)
        })
        .doc(
            "Add a value; one already present is left alone (a set holds one of each). Answers \
             the receiver. On a tagged set (`Set.of:`) the value is checked first.\n\n\
             ```\n\
             #<1 2>.add:3     \"* -> #<1 2 3>\n\
             #<1 2>.add:2     \"* -> #<1 2>\n\
             ```",
        )
        // --- checked generics (docs/internal/GENERICS_ARCH.md §4.2/§6) ---
        .sdk_class_method("new", |host, _receiver, _args| {
            Ok(host.new_native_state(
                host.get_or_create_builtin_class("Set"),
                NativeSetState::new_empty(),
            ))
        })
        .doc("A fresh empty set — the same value the `#<>` literal builds.")
        // `Set.new:` — a config block on a native set is meaningless; refuse
        // clearly instead of minting a payload-less shell (QUOIN_TODO.md).
        .sdk_class_method("new:", |_host, _receiver, _args| {
            Err(QuoinError::Other(
                "Set has no instance fields — construct with `#< >`, `Set.new`, or `Set.of:`"
                    .to_string(),
            ))
        })
        .doc(
            "Always refused: a Set has no instance fields for a `new:` config block to set. \
             Construct with `#< >`, `Set.new`, or `Set.of:`.",
        )
        .sdk_class_method("of:", |host, _receiver, args| {
            let tag = ElemTag::from_class_value(&args[0]).ok_or_else(|| QuoinError::TypeError {
                expected: "Class".to_string(),
                got: args[0].type_name().to_string(),
                msg: "Set.of: expects an element class (e.g. `Set.of:String`)".to_string(),
            })?;
            let v = host.new_native_state(
                host.get_or_create_builtin_class("Set"),
                NativeSetState::new_empty(),
            );
            let _ = host.with_native_state_mut(v, |s: &mut NativeSetState| s.elem = Some(tag));
            Ok(v)
        })
        .doc(
            "A fresh empty set tagged with an element class: every later `add:` is checked, \
             raising a catchable TypeError on a mismatch.\n\n\
             ```\n\
             (Set.of:String).elementType     \"* -> String\n\
             ```",
        )
        .sdk_instance_method("ensure:", |host, receiver, args| {
            let tag = ElemTag::from_class_value(&args[0]).ok_or_else(|| QuoinError::TypeError {
                expected: "Class".to_string(),
                got: args[0].type_name().to_string(),
                msg: "ensure: expects an element class (e.g. `s.ensure:String`)".to_string(),
            })?;
            let vec: Vec<Value> = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.values())
                .map_err(QuoinError::Other)?;
            for (i, v) in vec.iter().enumerate() {
                check_insert(Some(tag), "Set", v, Some(i as i64), |v, n| {
                    host.value_matches_type(*v, n)
                })?;
            }
            // Copy contents wholesale — cached hashes carry over, no
            // re-dispatch of instance `hash` methods.
            let contents = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.clone_contents())
                .map_err(QuoinError::Other)?;
            let mut fresh = NativeSetState::new_empty();
            fresh.adopt_contents(contents);
            let v = host.new_native_state(host.get_or_create_builtin_class("Set"), fresh);
            let _ = host.with_native_state_mut(v, |s: &mut NativeSetState| s.elem = Some(tag));
            Ok(v)
        })
        .doc(
            "Check every element against a class and answer a NEW set carrying that element \
             tag; a non-matching element raises a catchable TypeError. Cached hashes carry \
             over, so instance `hash` methods do not re-run.\n\n\
             ```\n\
             (#<1 2>.ensure:Integer).elementType     \"* -> Integer\n\
             ```",
        )
        .sdk_instance_method("collector", |host, receiver, _args| {
            let tag = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.elem)
                .map_err(QuoinError::Other)?;
            let v = host.new_list(Vec::new());
            if tag.is_some() {
                let _ = host.with_native_state_mut(
                    v,
                    |l: &mut crate::runtime::list::NativeListState| {
                        l.elem = tag;
                    },
                );
            }
            Ok(v)
        })
        .returns("List(T)")
        .doc(
            "A fresh empty LIST carrying this set's element tag — the Iterate mixin's staging \
             collection, where transforms like `collect:` accumulate results in order before \
             they become the final collection.",
        )
        .sdk_instance_method("emptyLike", |host, receiver, _args| {
            let tag = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.elem)
                .map_err(QuoinError::Other)?;
            let v = host.new_native_state(
                host.get_or_create_builtin_class("Set"),
                NativeSetState::new_empty(),
            );
            if tag.is_some() {
                let _ = host.with_native_state_mut(v, |s: &mut NativeSetState| s.elem = tag);
            }
            Ok(v)
        })
        .returns("Set(T)") // emptyLike: same shape, same tag, empty
        .doc(
            "A fresh empty set like the receiver — element tag included. The species hook the \
             Iterate mixin uses, so transforms of a checked set stay checked.",
        )
        .sdk_instance_method("elementType", |host, receiver, _args| {
            let tag = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.elem)
                .map_err(QuoinError::Other)?;
            Ok(match tag {
                Some(t) => host.new_symbol(t.name().to_string()),
                None => host.new_nil(),
            })
        })
        .doc(
            "The checked element type as a Symbol, or nil for an ordinary untagged set.\n\n\
             ```\n\
             (Set.of:String).elementType     \"* -> String\n\
             ```",
        )
        .instance_method("remove:", |vm, mc, receiver, args| {
            let (_, found) = set_find(vm, mc, receiver, args[0])?;
            if let Some(idx) = found {
                receiver
                    .with_native_state_mut::<NativeSetState, _, _>(mc, |s| s.remove_at(idx))
                    .map_err(QuoinError::Other)?;
            }
            Ok(receiver)
        })
        .doc(
            "Remove a value if present (a miss is a no-op); answers the receiver. The \
             remaining elements keep their insertion order.\n\n\
             ```\n\
             #<1 2 3>.remove:2     \"* -> #<1 3>\n\
             ```",
        )
        .instance_method("contains?:", |vm, mc, receiver, args| {
            let (_, found) = set_find(vm, mc, receiver, args[0])?;
            Ok(vm.new_bool(mc, found.is_some()))
        })
        .returns("Boolean")
        .doc(
            "True when the value is an element — an O(1) hash probe, using the same equality \
             ladder as Map keys.\n\n\
             ```\n\
             #<1 2 3>.contains?:2     \"* -> true\n\
             #<1 2 3>.contains?:9     \"* -> false\n\
             ```",
        )
        .sdk_instance_method("each:", |host, receiver, args| {
            let len = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.len())
                .map_err(QuoinError::Other)?;
            for i in 0..len {
                let elem = receiver
                    .with_native_state::<NativeSetState, _, _>(|s| s.values().get(i).copied())
                    .map_err(QuoinError::Other)?;
                if let Some(elem) = elem {
                    host.call_method(args[0], "valueWithSelfOrArg:", vec![elem])?;
                }
            }
            Ok(receiver)
        })
        .doc(
            "Call a block once per element, in insertion order; answers the receiver. This is \
             the one iteration primitive — the whole Iterate surface (`collect:`, `select:`, \
             …) derives from it.",
        )
        .sdk_instance_method("s", |host, receiver, _args| {
            let len = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.len())
                .map_err(QuoinError::Other)?;

            let mut parts = Vec::new();
            for i in 0..len {
                let val = receiver
                    .with_native_state::<NativeSetState, _, _>(|s| s.values().get(i).copied())
                    .map_err(QuoinError::Other)?
                    .ok_or_else(|| QuoinError::Other("Index out of bounds".to_string()))?;

                let result = host.call_method(val, "s", vec![])?;
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

            Ok(host.new_string(format!("#<{}>", parts.join(" "))))
        })
        .doc(
            "The display string: `#<` and `>` around each element's `.s`, space-separated, in \
             insertion order.\n\n\
             ```\n\
             #<1 2 3>.s     \"* -> #<1 2 3>\n\
             ```",
        )
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_len = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.len())
                .map_err(QuoinError::Other)?;
            let rhs_len = match args[0].with_native_state::<NativeSetState, _, _>(|s| s.len()) {
                Ok(len) => len,
                Err(_) => return Ok(vm.new_bool(mc, false)),
            };
            if lhs_len != rhs_len {
                return Ok(vm.new_bool(mc, false));
            }
            for i in 0..lhs_len {
                let elem = receiver
                    .with_native_state::<NativeSetState, _, _>(|s| s.value_at(i as u32))
                    .map_err(QuoinError::Other)?;
                let (_, found) = set_find(vm, mc, args[0], elem)?;
                if found.is_none() {
                    return Ok(vm.new_bool(mc, false));
                }
            }
            Ok(vm.new_bool(mc, true))
        })
        .doc(
            "Same-elements equality: true when the other value is a Set of the same size \
             containing every element of this one — insertion order does not matter. Anything \
             that is not a Set answers false.\n\n\
             ```\n\
             #<1 2 3> == #<3 2 1>     \"* -> true\n\
             ```",
        )
}
