use crate::error::QuoinError;
use crate::runtime::elem_tag::{ElemTag, check_insert};
use crate::runtime::pretty::{PpChild, PpShape, PrettyPrint};
use crate::value::{
    AnyCollect, NativeClassBuilder, ObjectPayload, Value, hash_bytes, key_native_exact,
    value_hash_scalar,
};
use crate::vm::{VmState, VmStatus};

use gc_arena::Gc;
use gc_arena::collect::{DynCollect, Trace};
use gc_arena::lock::RefLock;
use indexmap::IndexMap;
use rustc_hash::FxHashMap;
use std::any::Any;
use std::mem::transmute;

/// The `Map` store: insertion-ordered entries with ANY value as key, plus a
/// hash index for O(1) lookup. Each entry caches its key's hash — computed
/// ONCE at insert (dispatching a user instance's `hash` method there, never
/// from inside a Rust `Hash`/`Eq` impl) — so removal/re-indexing and `==:`
/// never re-dispatch.
///
/// The key contract (docs in `value_hash_scalar`): scalars and
/// content-payloads key by value; user instances by identity unless their
/// class overrides BOTH `hash` and `==:`; mutable built-in collections by
/// identity (content-hashing a mutable key is the classic footgun).
/// Iteration, pretty-printing, and serialization keep insertion order, so a
/// parse → generate round-trip doesn't reshuffle a document.
#[derive(Debug)]
pub struct NativeMapState {
    /// `(cached key hash, key, value)`, insertion-ordered.
    entries: Vec<(u64, Value<'static>, Value<'static>)>,
    /// hash → entry indices (buckets are almost always length 1).
    index: FxHashMap<u64, Vec<u32>>,
    /// Checked *value* type (`Map(String V)`).
    /// `None` = untagged, no checks (docs/GENERICS_ARCH.md).
    pub elem: Option<ElemTag>,
}

impl NativeMapState {
    pub fn new_empty() -> Self {
        Self {
            entries: Vec::new(),
            index: FxHashMap::default(),
            elem: None,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The insertion-ordered `(hash, key, value)` entries.
    pub fn entries<'gc>(&self) -> &[(u64, Value<'gc>, Value<'gc>)] {
        unsafe { transmute(self.entries.as_slice()) }
    }

    /// The bucket for `hash`, as owned `(index, key)` pairs — cloned OUT so
    /// the caller can drop the state borrow before dispatching guest `==:`
    /// (a hook could re-enter this very map).
    pub fn bucket<'gc>(&self, hash: u64) -> Vec<(u32, Value<'gc>)> {
        self.index
            .get(&hash)
            .map(|ixs| {
                ixs.iter()
                    .map(|&i| {
                        let (_, k, _) = self.entries[i as usize];
                        (i, unsafe { transmute::<Value<'static>, Value<'gc>>(k) })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn value_at<'gc>(&self, idx: u32) -> Value<'gc> {
        unsafe { transmute(self.entries[idx as usize].2) }
    }

    pub fn set_value_at(&mut self, idx: u32, v: Value<'_>) {
        self.entries[idx as usize].2 = unsafe { transmute(v) };
    }

    /// Append a NEW entry (caller has established the key is absent).
    pub fn append(&mut self, hash: u64, key: Value<'_>, value: Value<'_>) {
        let i = self.entries.len() as u32;
        self.entries
            .push((hash, unsafe { transmute(key) }, unsafe { transmute(value) }));
        self.index.entry(hash).or_default().push(i);
    }

    /// Remove the entry at `idx`, preserving order. O(n): later indices
    /// shift, so the index rebuilds from the cached hashes (no dispatch).
    pub fn remove_at<'gc>(&mut self, idx: u32) -> Value<'gc> {
        let (_, _, v) = self.entries.remove(idx as usize);
        self.index.clear();
        for (i, (h, _, _)) in self.entries.iter().enumerate() {
            self.index.entry(*h).or_default().push(i as u32);
        }
        unsafe { transmute(v) }
    }

    /// Native `&str` lookup (CSV columns, `%` interpolation, serialization):
    /// content-hashes the str — the SAME hash a String value gets — and
    /// compares String payloads only. No guest dispatch, no vm needed.
    pub fn get_str<'gc>(&self, key: &str) -> Option<Value<'gc>> {
        let h = hash_bytes(key.as_bytes());
        let ixs = self.index.get(&h)?;
        for &i in ixs {
            let (_, k, v) = &self.entries[i as usize];
            if let Value::Object(obj) = k
                && let ObjectPayload::String(s) = &obj.borrow().payload
                && s.as_str() == key
            {
                return Some(unsafe { transmute::<Value<'static>, Value<'gc>>(*v) });
            }
        }
        None
    }

    /// Scalar-exact lookup: `Some(hit)` when the key's native `==` is
    /// authoritative (the devirt ops' inline path); `None` = needs the
    /// dispatching path (`map_get_any`).
    pub fn get_scalar<'gc>(&self, key: &Value<'gc>) -> Option<Option<Value<'gc>>> {
        if !key_native_exact(key) {
            return None;
        }
        let h = value_hash_scalar(key)?;
        let hit = self.index.get(&h).and_then(|ixs| {
            ixs.iter().find_map(|&i| {
                let (_, k, v) = &self.entries[i as usize];
                let k: &Value<'gc> = unsafe { transmute(k) };
                if k == key {
                    Some(unsafe { transmute::<Value<'static>, Value<'gc>>(*v) })
                } else {
                    None
                }
            })
        });
        Some(hit)
    }

    /// Scalar-exact insert (devirt fast path); `None` = key needs dispatch.
    pub fn insert_scalar<'gc>(&mut self, key: Value<'gc>, value: Value<'gc>) -> Option<()> {
        if !key_native_exact(&key) {
            return None;
        }
        let h = value_hash_scalar(&key)?;
        if let Some(ixs) = self.index.get(&h) {
            for &i in ixs {
                let k: &Value<'gc> = unsafe { transmute(&self.entries[i as usize].1) };
                if *k == key {
                    self.set_value_at(i, value);
                    return Some(());
                }
            }
        }
        self.append(h, key, value);
        Some(())
    }
}

/// Hash a map key: scalars in Rust; user instances dispatch their `hash`
/// method (defaulting to `Object.hash` = identity).
pub(crate) fn map_hash_key<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    key: Value<'gc>,
) -> Result<u64, QuoinError> {
    if let Some(h) = value_hash_scalar(&key) {
        return Ok(h);
    }
    match vm.call_method(mc, key, "hash", vec![])? {
        Value::Int(i) => Ok(i as u64),
        other => Err(QuoinError::Other(format!(
            "hash must answer an Integer (got {})",
            other.class_name()
        ))),
    }
}

/// Key equality: native `==` first (covers every exact type, incl. the
/// Int↔Double coercion); definitively unequal when both are exact; guest
/// `==:` otherwise (instances, big numerics) — same semantics as
/// `Set.contains?:`, same reentry bound.
pub(crate) fn keys_equal<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    a: Value<'gc>,
    b: Value<'gc>,
) -> Result<bool, QuoinError> {
    if a == b {
        return Ok(true);
    }
    if key_native_exact(&a) && key_native_exact(&b) {
        return Ok(false);
    }
    Ok(vm.call_method(mc, a, "==:", vec![b])?.is_true())
}

/// Find `key`'s entry: `(hash, Some(index))` on a hit. `hash` may come from
/// a guest `hash` dispatch, so callers can reuse it (`map_find_prehashed`).
pub(crate) fn map_find<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    map_val: Value<'gc>,
    key: Value<'gc>,
) -> Result<(u64, Option<u32>), QuoinError> {
    let h = map_hash_key(vm, mc, key)?;
    Ok((h, map_find_prehashed(vm, mc, map_val, key, h)?))
}

pub(crate) fn map_find_prehashed<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    map_val: Value<'gc>,
    key: Value<'gc>,
    h: u64,
) -> Result<Option<u32>, QuoinError> {
    let bucket = map_val
        .with_native_state::<NativeMapState, _, _>(|m| m.bucket(h))
        .map_err(QuoinError::Other)?;
    // keys_equal may dispatch guest ==:, which can PARK — root the candidate
    // keys on the VM stack for the duration (a hook that mutates this very
    // map could otherwise leave them collectible in Rust locals).
    let base = vm.stack.len();
    for (_, k) in &bucket {
        vm.push(*k);
    }
    let mut hit = None;
    for (i, (idx, _)) in bucket.iter().enumerate() {
        let k = vm.stack[base + i];
        match keys_equal(vm, mc, k, key) {
            Ok(true) => {
                hit = Some(*idx);
                break;
            }
            Ok(false) => {}
            Err(e) => {
                vm.stack.truncate(base);
                return Err(e);
            }
        }
    }
    vm.stack.truncate(base);
    Ok(hit)
}

/// `at:` semantics: absent → `None` (the method answers nil).
pub(crate) fn map_get_any<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    map_val: Value<'gc>,
    key: Value<'gc>,
) -> Result<Option<Value<'gc>>, QuoinError> {
    let (_, found) = map_find(vm, mc, map_val, key)?;
    Ok(found.map(|i| {
        map_val
            .with_native_state::<NativeMapState, _, _>(|m| m.value_at(i))
            .expect("map vanished mid-lookup")
    }))
}

/// `at:put:` semantics (incl. the checked-generics value-tag).
pub(crate) fn map_put_any<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    map_val: Value<'gc>,
    key: Value<'gc>,
    val: Value<'gc>,
) -> Result<(), QuoinError> {
    let tag = map_val
        .with_native_state::<NativeMapState, _, _>(|m| m.elem)
        .map_err(QuoinError::Other)?;
    check_insert(tag, "Map String", &val, None, |v, n| {
        vm.value_matches_type(*v, n)
    })?;
    let (h, found) = map_find(vm, mc, map_val, key)?;
    map_val
        .with_native_state_mut::<NativeMapState, _, _>(mc, |m| match found {
            Some(i) => m.set_value_at(i, val),
            None => m.append(h, key, val),
        })
        .map_err(QuoinError::Other)?;
    Ok(())
}

impl PrettyPrint for NativeMapState {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        // Insertion order. String keys keep their raw text (the renderer
        // quotes them, exactly as before); any other key pre-renders through
        // the structural renderer, unquoted.
        let entries: Vec<(String, bool, Value<'gc>)> = self
            .entries()
            .iter()
            .map(|(_, k, v)| {
                if let Value::Object(obj) = k
                    && let ObjectPayload::String(s) = &obj.borrow().payload
                {
                    ((**s).clone(), true, *v)
                } else {
                    (
                        crate::runtime::pretty::render(*k, usize::MAX, false),
                        false,
                        *v,
                    )
                }
            })
            .collect();
        PpShape::Entries {
            open: "#{",
            close: "}",
            entries,
        }
    }
}

impl AnyCollect for NativeMapState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>) {
        for (_, key, val) in &self.entries {
            let key_gc: &Value<'gc> = unsafe { transmute(key) };
            key_gc.dyn_trace(cc);
            let val_gc: &Value<'gc> = unsafe { transmute(val) };
            val_gc.dyn_trace(cc);
        }
    }
}

pub fn build_map_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Map", Some("Object"))
        //
        .instance_method("containsKey?:", |vm, mc, receiver, args| {
            let (_, found) = map_find(vm, mc, receiver, args[0])?;
            Ok(vm.new_bool(mc, found.is_some()))
        })
        .returns("Boolean")
        .instance_method("at:", |vm, mc, receiver, args| {
            let value = map_get_any(vm, mc, receiver, args[0])?;
            Ok(value.unwrap_or_else(|| vm.new_nil(mc)))
        })
        .returns("V?") // value-typed read on a Map(String V) receiver
        .instance_method("at:put:", |vm, mc, receiver, args| {
            map_put_any(vm, mc, receiver, args[0], args[1])?;
            Ok(receiver)
        })
        .instance_method("remove:", |vm, mc, receiver, args| {
            let (_, found) = map_find(vm, mc, receiver, args[0])?;
            match found {
                Some(i) => receiver
                    .with_native_state_mut::<NativeMapState, _, _>(mc, |m| m.remove_at(i))
                    .map_err(QuoinError::Other),
                None => Ok(vm.new_nil(mc)),
            }
        })
        // --- checked generics (docs/GENERICS_ARCH.md §4.2/§6): the VALUE type
        // is generic (`Map(String V)`). ---
        .class_method("new", |vm, mc, _receiver, _args| {
            Ok(vm.new_map(mc, IndexMap::new()))
        })
        // `Map.new:` — a config block on a native map is meaningless; refuse
        // clearly instead of minting a payload-less shell (QUOIN_TODO.md).
        .class_method("new:", |_vm, _mc, _receiver, _args| {
            Err(QuoinError::Other(
                "Map has no instance fields — construct with `#{}`, `Map.new`, or `Map.of:`"
                    .to_string(),
            ))
        })
        .class_method("of:", |vm, mc, _receiver, args| {
            let tag = ElemTag::from_class_value(&args[0]).ok_or_else(|| QuoinError::TypeError {
                expected: "Class".to_string(),
                got: args[0].type_name().to_string(),
                msg: "Map.of: expects a value class (e.g. `Map.of:Integer`)".to_string(),
            })?;
            let v = vm.new_map(mc, IndexMap::new());
            let _ = v.with_native_state_mut::<NativeMapState, _, _>(mc, |m| m.elem = Some(tag));
            Ok(v)
        })
        .instance_method("ensure:", |vm, mc, receiver, args| {
            let tag = ElemTag::from_class_value(&args[0]).ok_or_else(|| QuoinError::TypeError {
                expected: "Class".to_string(),
                got: args[0].type_name().to_string(),
                msg: "ensure: expects a value class (e.g. `m.ensure:Integer`)".to_string(),
            })?;
            let entries: Vec<(u64, Value, Value)> =
                receiver.with_native_state(|m: &NativeMapState| m.entries().to_vec())?;
            for (_, k, v) in &entries {
                check_insert(Some(tag), "Map String", v, None, |v, n| {
                    vm.value_matches_type(*v, n)
                })
                .map_err(|e| match e {
                    QuoinError::TypeError { expected, got, msg } => QuoinError::TypeError {
                        expected,
                        got,
                        msg: format!("{msg} (key {})", k.class_name()),
                    },
                    other => other,
                })?;
            }
            let v = vm.new_map(mc, IndexMap::new());
            let _ = v.with_native_state_mut::<NativeMapState, _, _>(mc, |m| {
                for (h, k, val) in entries {
                    m.append(h, k, val);
                }
                m.elem = Some(tag);
            });
            Ok(v)
        })
        .instance_method("emptyLike", |vm, mc, receiver, _args| {
            let tag = receiver.with_native_state(|m: &NativeMapState| m.elem)?;
            let v = vm.new_map(mc, IndexMap::new());
            if tag.is_some() {
                let _ = v.with_native_state_mut::<NativeMapState, _, _>(mc, |m| m.elem = tag);
            }
            Ok(v)
        })
        .returns("Map(String V)") // emptyLike: same shape, same tag, empty
        .instance_method("elementType", |vm, mc, receiver, _args| {
            let tag = receiver.with_native_state(|m: &NativeMapState| m.elem)?;
            Ok(match tag {
                Some(t) => vm.new_symbol(mc, t.name().to_string()),
                None => Value::Nil,
            })
        })
        .instance_method("count", |vm, mc, receiver, _args| {
            Ok(vm.new_int(
                mc,
                receiver.with_native_state(|m: &NativeMapState| m.len())? as i64,
            ))
        })
        .returns("Integer")
        .instance_method("keys", |vm, mc, receiver, _args| {
            let keys_vec = receiver.with_native_state(|m: &NativeMapState| {
                m.entries().iter().map(|(_, k, _)| *k).collect::<Vec<_>>()
            })?;
            Ok(vm.new_list(mc, keys_vec))
        })
        .instance_method("values", |vm, mc, receiver, _args| {
            let values_vec = receiver.with_native_state(|m: &NativeMapState| {
                m.entries().iter().map(|(_, _, v)| *v).collect::<Vec<_>>()
            })?;
            Ok(vm.new_list(mc, values_vec))
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_len = receiver.with_native_state::<NativeMapState, _, _>(|m| m.len());
            let rhs_len = args[0].with_native_state::<NativeMapState, _, _>(|m| m.len());
            let (Ok(lhs_len), Ok(rhs_len)) = (lhs_len, rhs_len) else {
                return Ok(vm.new_bool(mc, false));
            };
            if lhs_len != rhs_len {
                return Ok(vm.new_bool(mc, false));
            }
            let entries: Vec<(u64, Value, Value)> = receiver
                .with_native_state::<NativeMapState, _, _>(|m| m.entries().to_vec())
                .map_err(QuoinError::Other)?;
            for (h, k, lhs_val) in entries {
                // Reuse the cached hash — an instance key's `hash` method
                // ran once at insert, not again per comparison.
                let Some(idx) = map_find_prehashed(vm, mc, args[0], k, h)? else {
                    return Ok(vm.new_bool(mc, false));
                };
                let rhs_val = args[0]
                    .with_native_state::<NativeMapState, _, _>(|m| m.value_at(idx))
                    .map_err(QuoinError::Other)?;
                if !vm.call_method(mc, lhs_val, "==:", vec![rhs_val])?.is_true() {
                    return Ok(vm.new_bool(mc, false));
                }
            }
            Ok(vm.new_bool(mc, true))
        })
}

#[derive(Debug)]
pub struct NativeKeyValuePairState {
    pub key: Value<'static>,
    pub value: Value<'static>,
}

impl NativeKeyValuePairState {
    pub fn new(key: Value<'_>, value: Value<'_>) -> Self {
        let key_static: Value<'static> = unsafe { transmute(key) };
        let value_static: Value<'static> = unsafe { transmute(value) };
        Self {
            key: key_static,
            value: value_static,
        }
    }

    pub fn get_key<'gc>(&self) -> Value<'gc> {
        unsafe { transmute(self.key) }
    }

    pub fn get_value<'gc>(&self) -> Value<'gc> {
        unsafe { transmute(self.value) }
    }
}

impl AnyCollect for NativeKeyValuePairState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>) {
        let key_gc: &Value<'gc> = unsafe { transmute(&self.key) };
        key_gc.dyn_trace(cc);
        let value_gc: &Value<'gc> = unsafe { transmute(&self.value) };
        value_gc.dyn_trace(cc);
    }
}

impl PrettyPrint for NativeKeyValuePairState {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        PpShape::Record {
            name: "KeyValuePair",
            fields: vec![
                ("key".to_string(), PpChild::Val(self.get_key())),
                ("value".to_string(), PpChild::Val(self.get_value())),
            ],
        }
    }
}

pub fn build_key_value_pair_class() -> NativeClassBuilder {
    NativeClassBuilder::new("KeyValuePair", Some("Object"))
        .class_method("new:", |vm, mc, receiver, args| {
            if !matches!(receiver, Value::Class(_)) {
                return Err(QuoinError::TypeError {
                    expected: "Class".to_string(),
                    got: receiver.type_name().to_string(),
                    msg: "new: expects Class receiver".to_string(),
                });
            }
            let block = if let Value::Object(obj) = args[0]
                && let ObjectPayload::Block(b) = &obj.borrow().payload
            {
                *b
            } else {
                return Err(QuoinError::TypeError {
                    expected: "Block".to_string(),
                    got: args[0].type_name().to_string(),
                    msg: "new: expects a Block".to_string(),
                });
            };

            let initial_frame_count = vm.frames.len();
            vm.start_block(mc, block, Vec::new(), None, None);

            while vm.frames.len() > initial_frame_count {
                match vm.step_internal(mc) {
                    Ok(VmStatus::Running) => {}
                    Ok(VmStatus::Finished(_)) => break,
                    Ok(VmStatus::Yeeted(val)) => {
                        return Err(QuoinError::Other(format!(
                            "Uncaught exception during block execution: {}",
                            val
                        )));
                    }
                    Err(QuoinError::NonLocalReturn) => {
                        // The ONE absorb/propagate decision (see the
                        // predicate's doc — this hand-rolled loop predates
                        // `run_nested` and deliberately never yields).
                        if vm.nlr_must_propagate(initial_frame_count) {
                            return Err(QuoinError::NonLocalReturn);
                        }
                        if vm.frames.len() > initial_frame_count {
                            continue;
                        }
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }

            // Pop the block's return value to clean up the stack
            let _block_ret = vm.pop().map_err(|e| QuoinError::Other(e))?;

            // Retrieve environment from the last popped frame recorded in VmState
            let env_ref = vm.last_popped_env.ok_or_else(|| {
                QuoinError::Other("Missing environment from block execution".to_string())
            })?;
            let env_borrow = env_ref.borrow();
            let key = env_borrow
                .lookup_str("key")
                .unwrap_or_else(|| vm.new_nil(mc));
            let value = env_borrow
                .lookup_str("value")
                .unwrap_or_else(|| vm.new_nil(mc));

            let state = NativeKeyValuePairState::new(key, value);
            let boxed_state: Box<dyn AnyCollect> = Box::new(state);
            let active_class_val = vm.active_native_args.last().unwrap().receiver;
            let class_obj = match active_class_val {
                Value::Class(c) => c,
                _ => {
                    return Err(QuoinError::TypeError {
                        expected: "Class".to_string(),
                        got: active_class_val.type_name().to_string(),
                        msg: "new: expects Class receiver".to_string(),
                    });
                }
            };
            let obj = vm.new_object(mc, class_obj);
            obj.borrow_mut(mc).payload =
                ObjectPayload::NativeState(crate::gc!(mc, RefLock::new(boxed_state)));

            Ok(Value::Object(obj))
        })
        .instance_method("key", |_vm, _mc, receiver, _args| {
            let key = receiver.with_native_state(|kvp: &NativeKeyValuePairState| kvp.get_key())?;
            Ok(key)
        })
        .instance_method("value", |_vm, _mc, receiver, _args| {
            let value =
                receiver.with_native_state(|kvp: &NativeKeyValuePairState| kvp.get_value())?;
            Ok(value)
        })
        .instance_method("s", |vm, mc, receiver, _args| {
            let key =
                receiver.with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_key())?;

            let key_s_val = vm.call_method(mc, key, "s", vec![])?;
            let key_s = if let Value::Object(obj) = key_s_val
                && let ObjectPayload::String(s) = &obj.borrow().payload
            {
                s.to_string()
            } else {
                format!("{}", key_s_val)
            };

            let active_receiver = vm.active_native_args.last().unwrap().receiver;
            let value = active_receiver
                .with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_value())?;

            let val_s_val = vm.call_method(mc, value, "s", vec![])?;
            let val_s = if let Value::Object(obj) = val_s_val
                && let ObjectPayload::String(s) = &obj.borrow().payload
            {
                s.to_string()
            } else {
                format!("{}", val_s_val)
            };

            Ok(vm.new_string(mc, format!("{}:{}", key_s, val_s)))
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_key =
                receiver.with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_key())?;
            let rhs_key_res =
                args[0].with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_key());
            let rhs_key = match rhs_key_res {
                Ok(k) => k,
                Err(_) => return Ok(vm.new_bool(mc, false)),
            };

            let keys_eq = vm.call_method(mc, lhs_key, "==:", vec![rhs_key])?.is_true();
            if !keys_eq {
                return Ok(vm.new_bool(mc, false));
            }

            let active_lhs = vm.active_native_args.last().unwrap().receiver;
            let active_rhs = {
                let c = vm.active_native_args.last().unwrap();
                c.arg(&vm.stack, 0).unwrap()
            };

            let lhs_val = active_lhs
                .with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_value())?;
            let rhs_val = active_rhs
                .with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_value())?;

            let vals_eq = vm.call_method(mc, lhs_val, "==:", vec![rhs_val])?.is_true();
            Ok(vm.new_bool(mc, vals_eq))
        })
}
