//! Value and collection constructors (objects, scalars, strings, lists/maps/sets,
//! regexes, blocks), field-layout interning, write-stream tracking, and the raw
//! stack primitives. Extends `VmState` exactly like `scheduler`.

use super::*;

use crate::value::Str;

impl<'gc> VmState<'gc> {
    pub fn new_object(
        &self,
        mc: &Mutation<'gc>,
        class_obj: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Gc<'gc, RefLock<Object<'gc>>> {
        let count = self.ensure_field_layout(mc, class_obj);
        let nil_val = self.new_nil(mc);
        let fields = Fields::new(count, nil_val);
        gcl!(
            mc,
            Object {
                class: class_obj,
                fields,
                payload: ObjectPayload::Instance,
            }
        )
    }

    /// Ensure `class.field_slots` covers the full current hierarchy (own + mixins +
    /// parent) and return the field count. Append-only: a newly-seen ivar gets a
    /// fresh trailing slot, so existing slots stay stable across runtime mixins.
    fn ensure_field_layout(
        &self,
        mc: &Mutation<'gc>,
        class: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> usize {
        let all = self.get_all_instance_vars(class);
        let mut c = class.borrow_mut(mc);
        for name in all {
            if !c.field_slots.contains_key(&name) {
                let slot = c.field_slots.len();
                c.field_slots.insert(name, slot);
            }
        }
        c.field_slots.len()
    }

    /// The absolute slot of instance variable `name` for instances of `class`
    /// (the layout is populated at instantiation), or `None` if it's not a declared
    /// ivar of the class.
    pub(super) fn field_slot(
        &self,
        class: Gc<'gc, RefLock<Class<'gc>>>,
        name: &str,
    ) -> Option<usize> {
        class.borrow().field_slots.get(name).copied()
    }

    pub fn new_native_state<T: AnyCollect + 'static>(
        &self,
        mc: &Mutation<'gc>,
        class_obj: Gc<'gc, RefLock<Class<'gc>>>,
        state: T,
    ) -> Value<'gc> {
        self.new_native_state_boxed(mc, class_obj, Box::new(state))
    }

    /// The dyn-safe core of [`new_native_state`](Self::new_native_state): takes an
    /// already-boxed payload, so it can sit on the `ext_sdk::Host` trait (which can't
    /// carry the generic form). The generic wrapper lives on `ext_sdk::HostExt`.
    pub fn new_native_state_boxed(
        &self,
        mc: &Mutation<'gc>,
        class_obj: Gc<'gc, RefLock<Class<'gc>>>,
        mut state: Box<dyn AnyCollect>,
    ) -> Value<'gc> {
        // Collections have dedicated payload variants and must never ride the
        // Box path (`with_native_state` & GC tracing assume the split). The
        // SDK's generic constructor boxes before this function can see the
        // type, so re-route by downcast — `mem::take` lifts the state out of
        // the box without copying the backing storage.
        let payload = if let Some(l) = state.as_any_mut().downcast_mut::<NativeListState>() {
            ObjectPayload::List(gcl!(mc, std::mem::take(l)))
        } else if let Some(m) = state.as_any_mut().downcast_mut::<NativeMapState>() {
            ObjectPayload::Map(gcl!(mc, std::mem::take(m)))
        } else if let Some(s) = state.as_any_mut().downcast_mut::<NativeSetState>() {
            ObjectPayload::Set(gcl!(mc, std::mem::take(s)))
        } else {
            ObjectPayload::NativeState(gcl!(mc, state))
        };
        let obj = gcl!(
            mc,
            Object {
                class: class_obj,
                fields: Fields::default(),
                payload,
            }
        );
        Value::Object(obj)
    }

    /// Start flushing this buffered write stream at program exit.
    pub fn track_write_stream(&mut self, stream: Value<'gc>) {
        self.open_write_streams.push(stream);
    }

    /// Stop tracking `stream` — it was closed (and so already flushed), or consumed by a
    /// `stringStream` that took over its buffer.
    pub fn untrack_write_stream(&mut self, mc: &Mutation<'gc>, stream: Value<'gc>) {
        let Ok(id) = stream.with_native_state::<NativeStream, _, _>(|s| s.stream_id()) else {
            return;
        };
        let _ = mc;
        self.open_write_streams.retain(|v| {
            v.with_native_state::<NativeStream, _, _>(|s| s.stream_id())
                .map(|other| other != id)
                .unwrap_or(true)
        });
    }

    /// Take every still-buffered byte from the tracked write streams. The driver writes these
    /// out when the program ends. Returns `(id, bytes)` pairs in the order the streams were
    /// opened; a stream with nothing pending contributes nothing.
    ///
    /// A stream that is still *open* stays tracked: the REPL drives — and so flushes — once per
    /// line, and a stream opened on one line is written on the next. Emptying the registry here
    /// would leave that stream untracked and lose its bytes. Closed streams are dropped; they
    /// were flushed on the way out.
    pub fn take_pending_writes(&mut self, mc: &Mutation<'gc>) -> Vec<(StreamId, Vec<u8>)> {
        let mut pending = Vec::new();
        self.open_write_streams.retain(|v| {
            match v.with_native_state_mut::<NativeStream, _, _>(mc, |s| {
                (!s.is_stream_closed(), s.take_pending())
            }) {
                Ok((open, bytes)) => {
                    if let Some(b) = bytes {
                        pending.push(b);
                    }
                    open
                }
                Err(_) => false, // no longer a stream: stop tracking it
            }
        });
        pending
    }

    // Scalar value types are immediate `Value` variants — no GC allocation. `mc`
    // is kept in the signatures so the many call sites stay unchanged.
    pub fn new_nil(&self, _mc: &Mutation<'gc>) -> Value<'gc> {
        Value::Nil
    }

    pub fn new_bool(&self, _mc: &Mutation<'gc>, b: bool) -> Value<'gc> {
        Value::Bool(b)
    }

    pub fn new_int(&self, _mc: &Mutation<'gc>, i: i64) -> Value<'gc> {
        Value::Int(i)
    }

    pub fn new_double(&self, _mc: &Mutation<'gc>, f: f64) -> Value<'gc> {
        Value::Double(f)
    }

    /// The one string-object assembler: every string VALUE is an Object whose
    /// payload is a `Str` (inline bytes or a `Gc<String>` buffer — see `Str`).
    fn string_object(&self, mc: &Mutation<'gc>, s: Str<'gc>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().string_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "String"));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::String(s),
            }
        ))
    }

    pub fn new_string(&self, mc: &Mutation<'gc>, s: String) -> Value<'gc> {
        self.string_object(mc, Str::from_string(mc, s))
    }

    /// A fresh SHORT string straight from a `&str`: the bytes go inline in
    /// the Object payload — one GC alloc, no inner buffer, no `String`.
    /// Caller guarantees `s.len() <= INLINE_STR_CAP`.
    pub fn new_string_inline(&self, mc: &Mutation<'gc>, s: &str) -> Value<'gc> {
        self.string_object(mc, Str::inline(s))
    }

    /// A fresh string from a borrowed str: inline when short (no allocation
    /// beyond the Object), heap copy when long. Use this instead of
    /// `new_string(mc, s.to_string())` wherever the source is a slice.
    pub fn new_string_from_str(&self, mc: &Mutation<'gc>, s: &str) -> Value<'gc> {
        self.string_object(mc, Str::new(mc, s))
    }

    /// Concatenation constructor: `a` + `b` assembled straight into the
    /// payload — inline with NO allocation when the result fits, else one
    /// exactly-sized heap buffer (never `format!` — see the `+:` native).
    pub fn new_string_concat(&self, mc: &Mutation<'gc>, a: &str, b: &str) -> Value<'gc> {
        if a.len() + b.len() <= crate::value::INLINE_STR_CAP {
            self.string_object(mc, Str::inline2(a, b))
        } else {
            let mut out = String::with_capacity(a.len() + b.len());
            out.push_str(a);
            out.push_str(b);
            self.string_object(mc, Str::Heap(gc!(mc, out)))
        }
    }

    /// A fresh string VALUE over an already-GC'd shared buffer — the
    /// long-literal materialization fast path (see `string_literal_buffers`;
    /// short literals inline instead).
    pub fn new_string_shared(&self, mc: &Mutation<'gc>, buf: Gc<'gc, String>) -> Value<'gc> {
        self.string_object(mc, Str::Heap(buf))
    }

    /// The shared buffer for literal content `s`, minting it on first use.
    pub fn literal_string_buffer(&mut self, mc: &Mutation<'gc>, s: &str) -> Gc<'gc, String> {
        if let Some(g) = self.string_literal_buffers.get(s) {
            return *g;
        }
        let g = gc!(mc, s.to_string());
        self.string_literal_buffers.insert(s.to_string(), g);
        g
    }

    /// Build an immutable `Bytes` value from raw bytes (mirrors `new_string`). One
    /// copy at the native boundary; the inner `Vec<u8>` is a GC leaf.
    pub fn new_bytes(&self, mc: &Mutation<'gc>, bytes: Vec<u8>) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Bytes");
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::Bytes(gc!(mc, bytes)),
            }
        ))
    }

    /// Return the interned `Symbol` value for `name`, creating it on first use.
    /// All occurrences of the same name share one value, so symbols compare by
    /// identity.
    pub fn new_symbol(&self, mc: &Mutation<'gc>, name: String) -> Value<'gc> {
        let existing = self.symbol_table.borrow().get(&name).copied();
        if let Some(sym) = existing {
            return sym;
        }
        let class = self.get_or_create_builtin_class(mc, "Symbol");
        let sym = Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::Symbol(gc!(mc, name.clone())),
            }
        ));
        self.symbol_table.borrow_mut(mc).insert(name, sym);
        sym
    }

    #[allow(clippy::wrong_self_convention)]
    pub fn to_s(
        &mut self,
        mc: &Mutation<'gc>,
        value: Value<'gc>,
    ) -> Result<Value<'gc>, QuoinError> {
        match value {
            Value::Class(_) | Value::ClassMeta(_) => {
                let display = value.to_string();
                Ok(self.new_string(mc, display))
            }
            // Object + immediate value types dispatch their `s` method.
            _ => self.call_method(mc, value, "s", vec![]),
        }
    }

    /// Verify every element of a FRESH collection literal against `tag`, then
    /// stamp the tag (`TagCollection` — annotation-driven tagged literals,
    /// docs/internal/GENERICS_ARCH.md §4.2). Safe to stamp in place: the literal has no
    /// aliases yet.
    pub(crate) fn tag_fresh_collection(
        &self,
        mc: &Mutation<'gc>,
        v: Value<'gc>,
        tag: elem_tag::ElemTag,
    ) -> Result<(), QuoinError> {
        use crate::runtime::map::NativeMapState;
        use crate::runtime::set::NativeSetState;
        if let Ok(vec) = v.with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec()) {
            for (i, e) in vec.iter().enumerate() {
                elem_tag::check_insert(Some(tag), "List", e, Some(i as i64), |val, n| {
                    self.value_matches_type(*val, n)
                })?;
            }
            let _ = v.with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                l.elem = Some(tag);
            });
            return Ok(());
        }
        if let Ok(vals) = v.with_native_state::<NativeMapState, _, _>(|m| {
            m.entries().iter().map(|(_, _, v)| *v).collect::<Vec<_>>()
        }) {
            for e in &vals {
                elem_tag::check_insert(Some(tag), "Map String", e, None, |val, n| {
                    self.value_matches_type(*val, n)
                })?;
            }
            let _ = v.with_native_state_mut::<NativeMapState, _, _>(mc, |m| {
                m.elem = Some(tag);
            });
            return Ok(());
        }
        if let Ok(vec) = v.with_native_state::<NativeSetState, _, _>(|s| s.values()) {
            for (i, e) in vec.iter().enumerate() {
                elem_tag::check_insert(Some(tag), "Set", e, Some(i as i64), |val, n| {
                    self.value_matches_type(*val, n)
                })?;
            }
            let _ = v.with_native_state_mut::<NativeSetState, _, _>(mc, |s| {
                s.elem = Some(tag);
            });
            return Ok(());
        }
        Err(QuoinError::Other(
            "TagCollection on a non-collection value".to_string(),
        ))
    }

    /// Checked write into a TAGGED list (the cold side of the ListPush arm).
    #[inline(never)]
    pub(crate) fn tagged_list_push(
        &self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let tag = receiver
            .with_native_state::<NativeListState, _, _>(|l| l.elem)
            .map_err(QuoinError::Other)?;
        elem_tag::check_insert(tag, "List", &value, None, |v, n| {
            self.value_matches_type(*v, n)
        })?;
        let _ = receiver
            .with_native_state_mut::<NativeListState, _, _>(mc, |l| l.get_vec_mut().push(value));
        Ok(())
    }

    /// Checked write into a TAGGED list (the cold side of the ListSet arm).
    /// The tag check precedes the bounds check — the VALUE is illegal
    /// regardless of index.
    #[inline(never)]
    pub(crate) fn tagged_list_set(
        &self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        i: i64,
        value: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let tag = receiver
            .with_native_state::<NativeListState, _, _>(|l| l.elem)
            .map_err(QuoinError::Other)?;
        elem_tag::check_insert(tag, "List", &value, Some(i), |v, n| {
            self.value_matches_type(*v, n)
        })?;
        receiver
            .with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                devirt_ops::list_set(l.get_vec_mut(), i, value)
            })
            .map_err(QuoinError::Other)?
    }

    pub fn new_list(&self, mc: &Mutation<'gc>, list: Vec<Value<'gc>>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().list_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "List"));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::List(gcl!(mc, NativeListState::new(list))),
            }
        ))
    }

    pub fn new_map(&self, mc: &Mutation<'gc>, pairs: Vec<(String, Value<'gc>)>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().map_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Map"));
        // Constructor for the string-shaped native callers (JSON/wire/CSV/
        // stats): ordered pairs straight into the any-key storage, duplicate
        // keys last-wins (what the IndexMap intermediary this replaced did —
        // it cost a second hash of every key, plus SipHash and a table build
        // per map, measured on the json bench).
        let mut state = NativeMapState::new_empty();
        for (k, v) in pairs {
            let k = self.new_string(mc, k);
            state
                .insert_scalar(k, v)
                .expect("String keys are native-exact");
        }
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::Map(gcl!(mc, state)),
            }
        ))
    }

    pub fn new_set(&self, mc: &Mutation<'gc>, set: Vec<Value<'gc>>) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Set");
        // Sole caller passes an empty vec (the NewSet literal dedups via
        // set_add); accept scalar-hashable elements defensively.
        let mut state = NativeSetState::new_empty();
        for v in set {
            let h = crate::value::value_hash_scalar(&v)
                .expect("new_set elements must be scalar-hashable; use set_add for instances");
            state.append(h, v);
        }
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::Set(gcl!(mc, state)),
            }
        ))
    }

    /// True if `set_val` already contains a value equal (by Quoin `==:`) to `value`.
    pub fn set_contains(
        &mut self,
        mc: &Mutation<'gc>,
        set_val: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<bool, QuoinError> {
        let (_, found) = crate::runtime::set::set_find(self, mc, set_val, value)?;
        Ok(found.is_some())
    }

    /// Insert `value` into `set_val` unless an equal element is already present.
    /// Returns whether a new element was added.
    pub fn set_add(
        &mut self,
        mc: &Mutation<'gc>,
        set_val: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<bool, QuoinError> {
        let (h, found) = crate::runtime::set::set_find(self, mc, set_val, value)?;
        if found.is_some() {
            Ok(false)
        } else {
            set_val
                .with_native_state_mut::<NativeSetState, _, _>(mc, |s| s.append(h, value))
                .map_err(QuoinError::Other)?;
            Ok(true)
        }
    }

    /// Remove the first element of `set_val` equal (by `==:`) to `value`.
    /// Returns whether an element was removed.
    pub fn set_remove(
        &mut self,
        mc: &Mutation<'gc>,
        set_val: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<bool, QuoinError> {
        let (_, found) = crate::runtime::set::set_find(self, mc, set_val, value)?;
        match found {
            Some(idx) => {
                set_val
                    .with_native_state_mut::<NativeSetState, _, _>(mc, |s| s.remove_at(idx))
                    .map_err(QuoinError::Other)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn new_regex(&self, mc: &Mutation<'gc>, regex: Regex) -> Value<'gc> {
        let class = self.builtin_cache.borrow().regex_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Regex"));
        let boxed_state: Box<dyn AnyCollect> = Box::new(NativeRegexState::new(regex));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    pub fn new_block(&self, mc: &Mutation<'gc>, block: Block<'gc>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().block_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Block"));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::Block(gc!(mc, block)),
            }
        ))
    }

    pub fn push(&mut self, val: Value<'gc>) {
        self.stack.push(val);
    }

    pub fn pop(&mut self) -> Result<Value<'gc>, String> {
        self.stack
            .pop()
            .ok_or_else(|| "Stack underflow".to_string())
    }

    pub fn peek(&self) -> Result<Value<'gc>, String> {
        self.stack
            .last()
            .copied()
            .ok_or_else(|| "Stack is empty".to_string())
    }
}
