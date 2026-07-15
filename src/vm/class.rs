//! The class/object model: method and service-class construction, extension-class
//! installation, instantiation plans and init chains, the builtin/native class
//! registry, hierarchy walks, and class-definition guards. Extends `VmState`.

use super::*;

impl<'gc> VmState<'gc> {
    pub fn new_method(
        &self,
        mc: &Mutation<'gc>,
        selector: String,
        block: Value<'gc>,
        is_extension: bool,
    ) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Method");
        let state = NativeMethodState::new(selector, block, is_extension);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    /// Wrap a native fn as a `Method` chain node, so native methods are chainable,
    /// scored, and override-able just like user methods.
    pub fn new_native_method(
        &self,
        mc: &Mutation<'gc>,
        selector: String,
        func: NativeFunc,
        param_types: Option<Vec<String>>,
        ret_type: Option<String>,
        doc: Option<String>,
    ) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Method");
        let state = NativeMethodState::new_native(selector, func, param_types, ret_type, doc);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    /// Wrap an extension-backed selector as a `Method` chain node (Phase 3): the method dispatches
    /// over the socket to `ext` (the owning `Extension` value, kept GC-rooted via the method table).
    pub fn new_ext_method(
        &self,
        mc: &Mutation<'gc>,
        selector: String,
        ext: Value<'gc>,
    ) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Method");
        let state = NativeMethodState::new_ext(selector, ext);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    /// Install an extension-provided class (Phase 3) as a host global: a real Quoin class whose
    /// selectors dispatch over the socket to `ext`. Class-side selectors become class methods,
    /// instance-side become instance methods; each is an `ExtDispatch` node carrying `ext`. A
    /// re-declared name is overwritten (last spawn wins).
    pub fn install_ext_class(
        &mut self,
        mc: &Mutation<'gc>,
        ext: Value<'gc>,
        name: &str,
        instance_selectors: &[String],
        class_selectors: &[String],
    ) {
        let mut instance_methods: FxHashMap<Symbol, Value<'gc>> = FxHashMap::default();
        for sel in instance_selectors {
            let node = self.new_ext_method(mc, sel.clone(), ext);
            instance_methods.insert(Symbol::intern(sel), node);
        }
        let mut class_methods: FxHashMap<Symbol, Value<'gc>> = FxHashMap::default();
        for sel in class_selectors {
            let node = self.new_ext_method(mc, sel.clone(), ext);
            class_methods.insert(Symbol::intern(sel), node);
        }
        let parent = self.get_or_create_builtin_class(mc, "Object");
        let ns_name = NamespacedName::parse(name);
        let class_obj = gcl!(
            mc,
            Class {
                name: ns_name.clone(),
                parent: Some(parent),
                instance_vars: Vec::new(),
                instance_methods,
                class_methods,
                mixin_classes: Vec::new(),
                field_slots: FxHashMap::default(),
                init_plan: None,
                is_eigenclass: false,
                is_sealed: false,
                is_abstract: false,
                native_new_refusal: None,
            }
        );
        self.globals
            .borrow_mut(mc)
            .insert(ns_name, Value::Class(class_obj));
        self.invalidate_method_cache();
        // An ext class can shadow/extend a name already baked into a compiled
        // entry's direct self-calls — the redefinition epoch is what Bails
        // those stale entries (the contract codegen's epoch doc promises for
        // extension installs, matching the DefineMethod arms).
        crate::codegen::bump_redef_epoch();
    }

    /// A hosted-service dispatch node (ACTOR_OBJECTS.md §2 manifests): the
    /// selector forwards to the worker behind the receiver; `service` (the
    /// root proxy) rides along for class-side sends and roots the service
    /// through the method table, mirroring `new_ext_method`.
    pub fn new_service_method(
        &self,
        mc: &Mutation<'gc>,
        selector: String,
        service: Value<'gc>,
    ) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Method");
        let state = NativeMethodState::new_service(selector, service);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    /// A plain native-method node (for the proxy-owned selectors installed on
    /// service classes — `serviceStop`, `==:`).
    pub fn new_native_method_value(
        &self,
        mc: &Mutation<'gc>,
        selector: &str,
        func: crate::value::LegacyNativeFn,
    ) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Method");
        let state = NativeMethodState::new_native(
            selector.to_string(),
            crate::value::NativeFunc::new(func),
            None,
            None,
            None,
        );
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    /// An EMPTY class shell for a hosted-service class, deliberately NOT bound
    /// as a global (the parent's own class of the same name is untouched; the
    /// value lives in `service_classes`). Populated in place once the root
    /// proxy exists — the method nodes carry the proxy, and the proxy is an
    /// instance of this class, so creation is two-step by construction.
    pub fn make_service_class_shell(
        &mut self,
        mc: &Mutation<'gc>,
        name: &str,
    ) -> Gc<'gc, RefLock<Class<'gc>>> {
        let parent = self.get_or_create_builtin_class(mc, "Object");
        gcl!(
            mc,
            Class {
                name: NamespacedName::parse(name),
                parent: Some(parent),
                instance_vars: Vec::new(),
                instance_methods: FxHashMap::default(),
                class_methods: FxHashMap::default(),
                mixin_classes: Vec::new(),
                field_slots: FxHashMap::default(),
                init_plan: None,
                is_eigenclass: false,
                is_sealed: false,
                is_abstract: false,
                native_new_refusal: None,
            }
        )
    }

    /// Fill a service class shell from its manifest: every declared selector
    /// becomes a `ServiceDispatch` node carrying `service` (the root proxy),
    /// and the proxy-owned selectors (`serviceStop`, `==:`) are installed
    /// last, shadowing same-named hosted methods by design.
    pub fn populate_service_class(
        &mut self,
        mc: &Mutation<'gc>,
        shell: Gc<'gc, RefLock<Class<'gc>>>,
        service: Value<'gc>,
        instance_selectors: &[String],
        class_selectors: &[String],
        owned: &[(&str, crate::value::LegacyNativeFn)],
    ) {
        {
            let mut class = shell.borrow_mut(mc);
            for sel in instance_selectors {
                let node = self.new_service_method(mc, sel.clone(), service);
                class.instance_methods.insert(Symbol::intern(sel), node);
            }
            for sel in class_selectors {
                let node = self.new_service_method(mc, sel.clone(), service);
                class.class_methods.insert(Symbol::intern(sel), node);
            }
            for (sel, func) in owned {
                let node = self.new_native_method_value(mc, sel, *func);
                class.instance_methods.insert(Symbol::intern(sel), node);
            }
        }
        self.invalidate_method_cache();
        crate::codegen::bump_redef_epoch();
    }

    /// The memoized instantiation recipe for `class` (see [`InitPlan`]),
    /// rebuilt whenever the dispatch epoch has moved — every method-table,
    /// mixin, or extension mutation bumps it (including `mix:`, fixed
    /// alongside this cache), so a stale plan cannot survive a hierarchy
    /// change. Field layout is append-only (`field_slots`), so resolved
    /// slots never go stale within an epoch.
    pub(super) fn instantiation_plan(
        &mut self,
        mc: &Mutation<'gc>,
        class: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Gc<'gc, InitPlan<'gc>> {
        if let Some((epoch, plan)) = class.borrow().init_plan
            && epoch == self.dispatch_epoch
        {
            return plan;
        }
        let vars = self.get_all_instance_vars(class);
        let ivar_slots: Vec<(String, usize)> = vars
            .into_iter()
            .filter_map(|v| self.field_slot(class, &v).map(|slot| (v, slot)))
            .collect();
        let mut classes = Vec::new();
        let mut visited = Vec::new();
        self.collect_classes_for_init(class, &mut classes, &mut visited);
        let mut inits = Vec::new();
        for clz in classes {
            let init_colon = clz
                .borrow()
                .instance_methods
                .get(&init_colon_symbol())
                .copied()
                .map(|m| (m, self.init_param_names(m).unwrap_or_default()));
            let init_plain = clz.borrow().instance_methods.get(&init_symbol()).copied();
            if init_colon.is_some() || init_plain.is_some() {
                inits.push(InitEntry {
                    init_colon,
                    init_plain,
                });
            }
        }
        let plan = gc!(mc, InitPlan { ivar_slots, inits });
        class.borrow_mut(mc).init_plan = Some((self.dispatch_epoch, plan));
        plan
    }

    /// NO borrow may be held while an initializer runs: `call_method_value` executes
    /// arbitrary Quoin that can cooperatively yield (an `init` that resumes a fiber or
    /// does I/O parks the whole task mid-call), and a Class/env borrow living on this
    /// suspended stack collides with any other task touching the same cell — e.g.
    /// `ensure_field_layout`'s `borrow_mut` when instantiating the same class
    /// ("RefCell already borrowed"). So the env rides in as a `Gc` and is borrowed
    /// transiently per lookup, and method lookups are hoisted OUT of `if let`
    /// scrutinees (a scrutinee temporary lives through the success branch — even in
    /// edition 2024, whose rescope only shortened the `else` path).
    pub(super) fn finalize_instantiation(
        &mut self,
        mc: &Mutation<'gc>,
        obj: Gc<'gc, RefLock<Object<'gc>>>,
        env: Gc<'gc, RefLock<EnvFrame<'gc>>>,
    ) -> Result<(), QuoinError> {
        let class = obj.borrow().class;
        let plan = self.instantiation_plan(mc, class);
        for (name, slot) in &plan.ivar_slots {
            let val = env.borrow().lookup_str(name);
            if let Some(val) = val {
                obj.borrow_mut(mc).fields[*slot] = val;
            }
        }

        // Run each class's initializer base->derived (parents, then mixins,
        // then self). A class that defines `init:` receives the block fields
        // it names (matched by param name); otherwise its zero-arg `init`
        // runs. Running the whole chain means an ancestor or mixin
        // initializer is never skipped just because a more derived class
        // happens to define `init:`. The plan is rooted for the chain's
        // duration (a user init can park AND replace the cached plan).
        let receiver = Value::Object(obj);
        self.active_init_plans.push(plan);
        let result = self.run_init_chain_planned(mc, receiver, plan, Some(env));
        self.active_init_plans.pop();
        result
    }

    /// The init-chain body shared by [`Self::finalize_instantiation`]
    /// (`with_env` = the `new:{}` block env feeding `init:` params) and
    /// [`Self::run_all_inits`] (`None`: the plain-`new` path runs `init`
    /// ONLY, exactly as before the plan existed).
    // The caller has pushed `plan` onto `active_init_plans` for this whole
    // call; the method Values read from it are rooted by that contract
    // across the user init calls (which can park).
    #[allow(no_gc_across_yield)]
    pub(super) fn run_init_chain_planned(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        plan: Gc<'gc, InitPlan<'gc>>,
        with_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    ) -> Result<(), QuoinError> {
        for idx in 0..plan.inits.len() {
            let entry = &plan.inits[idx];
            match (with_env, &entry.init_colon) {
                (Some(env), Some((method_val, param_names))) => {
                    let method_val = *method_val;
                    let mut init_args = Vec::with_capacity(param_names.len());
                    for param in param_names {
                        let val = env
                            .borrow()
                            .lookup_str(param)
                            .unwrap_or_else(|| self.new_nil(mc));
                        init_args.push(val);
                    }
                    self.call_method_value(mc, receiver, method_val, "init:", init_args)?;
                }
                _ => {
                    if let Some(method_val) = entry.init_plain {
                        self.call_method_value(mc, receiver, method_val, "init", Vec::new())?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Parameter names of a method's underlying block, used so `init:` can be fed
    /// the `new:{}` block fields it declares by name. Handles both plain block
    /// methods and native-wrapped method state.
    fn init_param_names(&self, method_val: Value<'gc>) -> Option<Vec<String>> {
        let Value::Object(io) = method_val else {
            return None;
        };
        let io_ref = io.borrow();
        match &io_ref.payload {
            ObjectPayload::Block(b) => Some(
                b.template
                    .param_syms
                    .iter()
                    .map(|s| s.as_str().to_string())
                    .collect(),
            ),
            ObjectPayload::NativeState(state_cell) => {
                let state_ref = state_cell.borrow();
                let any_ref = (**state_ref).as_any();
                let method_state = any_ref.downcast_ref::<NativeMethodState>()?;
                if let Some(Value::Object(block_obj)) = method_state.get_block()
                    && let ObjectPayload::Block(b) = &block_obj.borrow().payload
                {
                    Some(
                        b.template
                            .param_syms
                            .iter()
                            .map(|s| s.as_str().to_string())
                            .collect(),
                    )
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn get_or_create_builtin_class(
        &self,
        mc: &Mutation<'gc>,
        name: &str,
    ) -> Gc<'gc, RefLock<Class<'gc>>> {
        let ns_name = NamespacedName::parse(name);
        let existing = self.globals.borrow().get(&ns_name).copied();
        if let Some(Value::Class(c)) = existing {
            c
        } else {
            let parent = if name == "Object" {
                None
            } else {
                Some(self.get_or_create_builtin_class(mc, "Object"))
            };
            let class_obj = gcl!(
                mc,
                Class {
                    name: ns_name.clone(),
                    parent,
                    instance_vars: Vec::new(),
                    instance_methods: FxHashMap::default(),
                    class_methods: FxHashMap::default(),
                    mixin_classes: Vec::new(),
                    field_slots: FxHashMap::default(),
                    init_plan: None,
                    is_eigenclass: false,
                    is_sealed: false,
                    is_abstract: false,
                    native_new_refusal: None,
                }
            );
            self.globals
                .borrow_mut(mc)
                .insert(ns_name, Value::Class(class_obj));

            let mut cache = self.builtin_cache.borrow_mut(mc);
            match name {
                "Nil" => cache.nil_class = Some(class_obj),
                "Boolean" => cache.boolean_class = Some(class_obj),
                "Integer" => cache.integer_class = Some(class_obj),
                "Double" => cache.double_class = Some(class_obj),
                "String" => cache.string_class = Some(class_obj),
                "List" => cache.list_class = Some(class_obj),
                "Map" => cache.map_class = Some(class_obj),
                "Regex" => cache.regex_class = Some(class_obj),
                "Block" => cache.block_class = Some(class_obj),
                _ => {}
            }
            class_obj
        }
    }

    pub fn get_builtin_class(&self, name: &str) -> Gc<'gc, RefLock<Class<'gc>>> {
        let ns_name = NamespacedName::parse(name);
        let existing = self.globals.borrow().get(&ns_name).copied();
        if let Some(Value::Class(c)) = existing {
            c
        } else {
            panic!("Builtin class {} not found in globals!", name);
        }
    }

    pub fn register_native_class<T: NativeClass>(&mut self, mc: &Mutation<'gc>, native_class: T) {
        if let Some(doc) = native_class.class_doc() {
            self.class_meta
                .entry(NamespacedName::parse(native_class.name()))
                .or_default()
                .doc = Some(doc.to_string());
        }
        let parent_class = native_class
            .parent_name()
            .map(|parent_name| self.get_or_create_builtin_class(mc, parent_name));

        // Several defs may share a selector (typed multimethod variants); chain
        // them in declaration order so the scorer routes by argument type and ties
        // resolve to the first-declared.
        let mut inst_methods: FxHashMap<Symbol, Value<'gc>> = FxHashMap::default();
        for def in native_class.instance_methods() {
            let sym = Symbol::intern(&def.selector);
            let node = self.new_native_method(
                mc,
                def.selector.clone(),
                def.func,
                def.param_types,
                def.ret_type,
                def.doc,
            );
            if let Some(head) = inst_methods.get(&sym).copied() {
                let _ = Self::append_method_to_chain(mc, head, node);
            } else {
                inst_methods.insert(sym, node);
            }
        }

        let mut cls_methods: FxHashMap<Symbol, Value<'gc>> = FxHashMap::default();
        for def in native_class.class_methods() {
            let sym = Symbol::intern(&def.selector);
            let node = self.new_native_method(
                mc,
                def.selector.clone(),
                def.func,
                def.param_types,
                def.ret_type,
                def.doc,
            );
            if let Some(head) = cls_methods.get(&sym).copied() {
                let _ = Self::append_method_to_chain(mc, head, node);
            } else {
                cls_methods.insert(sym, node);
            }
        }

        let (is_abstract, native_new_refusal) = match native_class.new_policy() {
            NativeNewPolicy::Abstract => (true, None),
            NativeNewPolicy::Refuse(hint) => (false, Some(hint.unwrap_or(NATIVE_NEW_GENERIC_HINT))),
        };

        let name = native_class.name();
        let ns_name = NamespacedName::parse(name);
        let existing = self.globals.borrow().get(&ns_name).copied();
        if let Some(Value::Class(existing_class)) = existing {
            let mut borrowed = existing_class.borrow_mut(mc);
            borrowed.parent = parent_class;
            borrowed.instance_methods = inst_methods;
            borrowed.class_methods = cls_methods;
            borrowed.instance_vars = Vec::new();
            borrowed.is_abstract = is_abstract;
            borrowed.native_new_refusal = native_new_refusal;
        } else {
            let class_obj = gcl!(
                mc,
                Class {
                    name: ns_name.clone(),
                    parent: parent_class,
                    instance_vars: Vec::new(),
                    instance_methods: inst_methods,
                    class_methods: cls_methods,
                    mixin_classes: Vec::new(),
                    field_slots: FxHashMap::default(),
                    init_plan: None,
                    is_eigenclass: false,
                    is_sealed: false,
                    is_abstract,
                    native_new_refusal,
                }
            );

            self.globals
                .borrow_mut(mc)
                .insert(ns_name, Value::Class(class_obj));

            let mut cache = self.builtin_cache.borrow_mut(mc);
            match name {
                "Nil" => cache.nil_class = Some(class_obj),
                "Boolean" => cache.boolean_class = Some(class_obj),
                "Integer" => cache.integer_class = Some(class_obj),
                "Double" => cache.double_class = Some(class_obj),
                "String" => cache.string_class = Some(class_obj),
                "List" => cache.list_class = Some(class_obj),
                "Map" => cache.map_class = Some(class_obj),
                "Regex" => cache.regex_class = Some(class_obj),
                "Block" => cache.block_class = Some(class_obj),
                _ => {}
            }
        }
        // A class's method tables just changed — drop any memoized resolutions.
        self.invalidate_method_cache();
    }

    pub fn is_subclass_of_clz(
        &self,
        sub: Gc<'gc, RefLock<Class<'gc>>>,
        sup: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> bool {
        let mut curr = Some(sub);
        while let Some(clz) = curr {
            if Gc::ptr_eq(clz, sup) {
                return true;
            }
            for mixin in &clz.borrow().mixin_classes {
                if Gc::ptr_eq(*mixin, sup) {
                    return true;
                }
            }
            curr = clz.borrow().parent;
        }
        false
    }

    pub fn is_instance_of(&self, val: Value<'gc>, class_obj: Gc<'gc, RefLock<Class<'gc>>>) -> bool {
        if let Some(val_class) = self.get_class_for_lookup(val) {
            self.is_subclass_of_clz(val_class, class_obj)
        } else {
            false
        }
    }

    pub fn append_method_to_chain(
        mc: &Mutation<'gc>,
        chain_start: Value<'gc>,
        new_method: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let mut curr = chain_start;
        loop {
            if let Value::Object(obj) = curr {
                let payload = &obj.borrow().payload;
                if let ObjectPayload::NativeState(state_cell) = payload {
                    let mut state_ref = state_cell.borrow_mut(mc);
                    let any_mut = state_ref.as_any_mut();
                    if let Some(method_state) = any_mut.downcast_mut::<NativeMethodState>() {
                        if let Some(next_val) = method_state.next {
                            let next_val_gc: Value<'gc> = unsafe { transmute(next_val) };
                            drop(state_ref);
                            curr = next_val_gc;
                            continue;
                        } else {
                            let new_method_static: Value<'static> =
                                unsafe { transmute(new_method) };
                            method_state.next = Some(new_method_static);
                            return Ok(());
                        }
                    }
                }
            }
            return Err(QuoinError::Other(
                "Invalid method object in chain".to_string(),
            ));
        }
    }

    /// Add `new_method` to a selector's method chain. A plain *unguarded* variant
    /// (no `decl_block`) whose parameter types match an existing unguarded variant
    /// *replaces* that variant's block in place — a true redefinition, so a later
    /// `-->` (or a repeated `->`) overrides instead of silently shadowing. Guarded
    /// and type-differentiated variants are appended, preserving definition order
    /// for multimethod dispatch.
    pub(super) fn replace_or_append_method_in_chain(
        &self,
        mc: &Mutation<'gc>,
        chain_start: Value<'gc>,
        new_method: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let new_block = self.get_block_from_method(new_method);
        if let Some(nb) = new_block
            && nb.decl_block.is_none()
            && let Some(new_block_val) =
                new_method.with_native_state::<NativeMethodState, _, _>(|m| m.get_block())?
        {
            let new_param_types = nb.template.param_types.clone();
            // Element-tag requirements are part of a variant's identity:
            // `|l: List(Integer)|` and `|l: List(String)|` share the erased
            // base signature ["List"] but are distinct multimethod variants
            // (GENERICS_ARCH.md §5), not a redefinition.
            let new_elem_tags = nb.template.param_elem_tags.clone();
            let mut curr = Some(chain_start);
            while let Some(node) = curr {
                let is_match = self
                    .get_block_from_method(node)
                    .map(|eb| {
                        eb.decl_block.is_none()
                            && eb.template.param_types == new_param_types
                            && eb.template.param_elem_tags == new_elem_tags
                    })
                    .unwrap_or(false);
                if is_match {
                    if let Value::Object(obj) = node {
                        let obj_ref = obj.borrow();
                        if let ObjectPayload::NativeState(state_cell) = &obj_ref.payload {
                            let mut state_ref = state_cell.borrow_mut(mc);
                            if let Some(ms) =
                                state_ref.as_any_mut().downcast_mut::<NativeMethodState>()
                            {
                                ms.body = MethodBody::UserBlock(unsafe {
                                    transmute::<Value<'gc>, Value<'static>>(new_block_val)
                                });
                            }
                        }
                    }
                    return Ok(());
                }
                curr = self.get_next_method_in_chain(node);
            }
        }
        Self::append_method_to_chain(mc, chain_start, new_method)
    }

    pub fn lookup_in_class_hierarchy(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        selector: &str,
        class_side: bool,
    ) -> Option<Value<'gc>> {
        // Intern once at the boundary; the recursive walk probes by Symbol.
        let selector = Symbol::intern(selector);
        let mut visited = Vec::new();
        self.lookup_in_class_hierarchy_rec(class_ref, selector, class_side, &mut visited)
    }

    fn lookup_in_class_hierarchy_rec(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        selector: Symbol,
        class_side: bool,
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    ) -> Option<Value<'gc>> {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            return None;
        }
        visited.push(class_ref);

        let class_borrow = class_ref.borrow();
        let methods = if class_side {
            &class_borrow.class_methods
        } else {
            &class_borrow.instance_methods
        };
        if let Some(method) = methods.get(&selector).copied() {
            return Some(method);
        }
        for mixin in &class_borrow.mixin_classes {
            if let Some(method) =
                self.lookup_in_class_hierarchy_rec(*mixin, selector, class_side, visited)
            {
                return Some(method);
            }
        }
        if let Some(parent) = class_borrow.parent
            && let Some(method) =
                self.lookup_in_class_hierarchy_rec(parent, selector, class_side, visited)
        {
            return Some(method);
        }
        None
    }

    pub fn get_all_instance_vars(&self, class_ref: Gc<'gc, RefLock<Class<'gc>>>) -> Vec<String> {
        let mut vars = Vec::new();
        let mut visited = Vec::new();
        self.collect_instance_vars(class_ref, &mut vars, &mut visited);
        vars
    }

    fn collect_instance_vars(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        vars: &mut Vec<String>,
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    ) {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            return;
        }
        visited.push(class_ref);

        let class_borrow = class_ref.borrow();
        for var in &class_borrow.instance_vars {
            if !vars.contains(var) {
                vars.push(var.clone());
            }
        }
        for mixin in &class_borrow.mixin_classes {
            self.collect_instance_vars(*mixin, vars, visited);
        }
        if let Some(parent) = class_borrow.parent {
            self.collect_instance_vars(parent, vars, visited);
        }
    }

    /// Error if `class` is `sealed!` — refuses extension (`<--` / `->` / `-->` /
    /// `.mix:`) and subclassing of a sealed class (or an instance's sealed eigenclass).
    pub(crate) fn ensure_not_sealed(
        &self,
        class: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Result<(), QuoinError> {
        let c = class.borrow();
        if c.is_sealed {
            return Err(QuoinError::ClassError(if c.is_eigenclass {
                "Cannot extend a sealed instance".to_string()
            } else {
                format!("Cannot extend sealed class {}", c.name.to_explicit_string())
            }));
        }
        Ok(())
    }

    /// Error if `class` refuses `new` / `new:` on the class itself — either
    /// `abstract!`, or a native class whose generic instantiation fallback would
    /// mint a payload-less shell (`Class::native_new_refusal`). Concrete
    /// subclasses are unaffected, since neither flag is inherited.
    pub(crate) fn ensure_instantiable(
        &self,
        class: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Result<(), QuoinError> {
        let c = class.borrow();
        if c.is_abstract {
            return Err(QuoinError::ClassError(format!(
                "Cannot instantiate abstract class {}",
                c.name.to_explicit_string()
            )));
        }
        if let Some(hint) = c.native_new_refusal {
            return Err(QuoinError::ClassError(format!(
                "Cannot construct {} with new — {}",
                c.name.to_explicit_string(),
                hint
            )));
        }
        Ok(())
    }

    pub fn get_target_class_for_def(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<Gc<'gc, RefLock<Class<'gc>>>, String> {
        match receiver {
            Value::Class(c) => Ok(c),
            Value::ClassMeta(c) => Ok(c),
            // Extending a value type (`5 <-- {…}`, `Integer <-- {…}`) extends the
            // type itself — value types have no per-instance eigenclass.
            Value::Int(_) => Ok(self.get_or_create_builtin_class(mc, "Integer")),
            Value::Double(_) => Ok(self.get_or_create_builtin_class(mc, "Double")),
            Value::Nil => Ok(self.get_or_create_builtin_class(mc, "Nil")),
            // `true` and `false` carry distinct methods, so each gets its own
            // singleton class (parent `Boolean`), synthesized once and cached.
            Value::Bool(b) => {
                let existing = if b {
                    self.builtin_cache.borrow().true_class
                } else {
                    self.builtin_cache.borrow().false_class
                };
                if let Some(c) = existing {
                    return Ok(c);
                }
                let boolean = self.get_or_create_builtin_class(mc, "Boolean");
                let name = if b { "$TrueClass" } else { "$FalseClass" };
                let ns = NamespacedName::new(Vec::new(), name.to_string());
                let s = gcl!(
                    mc,
                    Class {
                        name: ns.clone(),
                        parent: Some(boolean),
                        instance_vars: Vec::new(),
                        instance_methods: FxHashMap::default(),
                        class_methods: FxHashMap::default(),
                        mixin_classes: Vec::new(),
                        field_slots: FxHashMap::default(),
                        init_plan: None,
                        is_eigenclass: false,
                        is_sealed: false,
                        is_abstract: false,
                        native_new_refusal: None,
                    }
                );
                self.globals.borrow_mut(mc).insert(ns, Value::Class(s));
                if b {
                    self.builtin_cache.borrow_mut(mc).true_class = Some(s);
                } else {
                    self.builtin_cache.borrow_mut(mc).false_class = Some(s);
                }
                Ok(s)
            }
            Value::Object(obj) => {
                let class_ref = obj.borrow().class;
                if class_ref.borrow().name.name.starts_with('$') {
                    Ok(class_ref)
                } else {
                    let mut singleton_name = class_ref.borrow().name.clone();
                    singleton_name.name = format!("${}", singleton_name.name);
                    // The eigenclass declares no new ivars, so it shares its base
                    // class's instance layout: it must carry the same field-slot map,
                    // or `@ivar` access on the instance (now of the eigenclass) can't
                    // resolve the inherited slots and reads them as nil.
                    let field_slots = class_ref.borrow().field_slots.clone();
                    let s = gcl!(
                        mc,
                        Class {
                            name: singleton_name,
                            parent: Some(class_ref),
                            instance_vars: Vec::new(),
                            instance_methods: FxHashMap::default(),
                            class_methods: FxHashMap::default(),
                            mixin_classes: Vec::new(),
                            field_slots,
                            init_plan: None,
                            is_eigenclass: true,
                            is_sealed: false,
                            is_abstract: false,
                            native_new_refusal: None,
                        }
                    );
                    obj.borrow_mut(mc).class = s;
                    Ok(s)
                }
            }
        }
    }
}
