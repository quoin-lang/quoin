use crate::error::BBError;
use crate::instruction::{Constant, Instruction};
use crate::runtime::list::NativeListState;
use crate::value::{
    AnyCollect, Block, Class, EnvFrame, GcRegex, GcUlid, NamespacedName, NativeClass, NativeFunc,
    Object, ObjectPayload, Value,
};
use crate::{gc, gcl};

use gc_arena::{lock::RefLock, Collect, Gc, Mutation};
use std::collections::HashMap;
use ulid::Ulid;

#[derive(Collect)]
#[collect(no_drop)]
pub struct Frame<'gc> {
    pub id: usize,
    pub is_nested_block: bool,
    pub enclosing_method_id: Option<usize>,
    pub block: Gc<'gc, Block<'gc>>,
    pub ip: usize,
    pub env: Gc<'gc, RefLock<EnvFrame<'gc>>>,
    pub instantiating_obj: Option<Gc<'gc, RefLock<Object<'gc>>>>,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct BuiltinCache<'gc> {
    pub nil_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub boolean_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub integer_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub double_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub string_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub list_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub map_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub regex_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub block_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub native_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub nil_val: Option<Value<'gc>>,
    pub true_val: Option<Value<'gc>>,
    pub false_val: Option<Value<'gc>>,
}

impl<'gc> BuiltinCache<'gc> {
    pub fn new() -> Self {
        Self {
            nil_class: None,
            boolean_class: None,
            integer_class: None,
            double_class: None,
            string_class: None,
            list_class: None,
            map_class: None,
            regex_class: None,
            block_class: None,
            native_class: None,
            nil_val: None,
            true_val: None,
            false_val: None,
        }
    }
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct VmState<'gc> {
    pub stack: Vec<Value<'gc>>,
    pub frames: Vec<Frame<'gc>>,
    pub globals: Gc<'gc, RefLock<HashMap<NamespacedName, Value<'gc>>>>,
    pub next_frame_id: usize,

    pub builtin_cache: Gc<'gc, RefLock<BuiltinCache<'gc>>>,
}

pub enum VmStatus<'gc> {
    Running,
    Finished(Value<'gc>),
    Yeeted(Value<'gc>), // Uncaught exception
}

pub trait Callable<'gc> {
    fn call(
        &self,
        vm: &mut VmState<'gc>,
        mc: &Mutation<'gc>,
        args: Vec<Value<'gc>>,
    ) -> Result<(), BBError>;
}

pub struct BlockCallable<'gc> {
    pub block: Gc<'gc, Block<'gc>>,
}

impl<'gc> Callable<'gc> for BlockCallable<'gc> {
    fn call(
        &self,
        vm: &mut VmState<'gc>,
        mc: &Mutation<'gc>,
        args: Vec<Value<'gc>>,
    ) -> Result<(), BBError> {
        if args.is_empty() {
            return Err(BBError::Other(
                "Method call arguments is empty (missing receiver)".to_string(),
            ));
        }
        let receiver = args[0];
        let method_args = args[1..].to_vec();
        vm.start_block_as_method(mc, self.block, receiver, method_args);
        Ok(())
    }
}

pub struct MetaCallable<'gc> {
    pub class_obj: Gc<'gc, RefLock<Class<'gc>>>,
}

impl<'gc> Callable<'gc> for MetaCallable<'gc> {
    fn call(
        &self,
        vm: &mut VmState<'gc>,
        _mc: &Mutation<'gc>,
        _args: Vec<Value<'gc>>,
    ) -> Result<(), BBError> {
        vm.push(Value::ClassMeta(self.class_obj));
        Ok(())
    }
}

pub struct NewCallable<'gc> {
    pub class_obj: Gc<'gc, RefLock<Class<'gc>>>,
}

impl<'gc> Callable<'gc> for NewCallable<'gc> {
    fn call(
        &self,
        vm: &mut VmState<'gc>,
        mc: &Mutation<'gc>,
        args: Vec<Value<'gc>>,
    ) -> Result<(), BBError> {
        if args.len() != 2 {
            return Err(BBError::Other(
                "new: expects receiver and a block".to_string(),
            ));
        }
        let block = if let Value::Object(obj) = args[1]
            && let ObjectPayload::Block(b) = &obj.borrow().payload
        {
            *b
        } else {
            return Err(BBError::TypeError {
                expected: "Block".to_string(),
                got: args[1].type_name().to_string(),
                msg: "new: expects a Block".to_string(),
            });
        };

        // Create the new object
        let obj = vm.new_object(mc, self.class_obj);

        vm.start_block_for_instantiation(mc, block, obj);
        Ok(())
    }
}

pub struct NewNoBlockCallable<'gc> {
    pub class_obj: Gc<'gc, RefLock<Class<'gc>>>,
}

impl<'gc> Callable<'gc> for NewNoBlockCallable<'gc> {
    fn call(
        &self,
        vm: &mut VmState<'gc>,
        mc: &Mutation<'gc>,
        args: Vec<Value<'gc>>,
    ) -> Result<(), BBError> {
        if args.len() != 1 {
            return Err(BBError::Other("new expects only the receiver".to_string()));
        }

        // Create the new object
        let obj = vm.new_object(mc, self.class_obj);

        let has_init = vm.lookup_in_class_hierarchy(self.class_obj, "init", false).is_some();
        if has_init {
            vm.call_method(mc, Value::Object(obj), "init", Vec::new())?;
        }

        vm.push(Value::Object(obj));
        Ok(())
    }
}

pub struct NativeCallable(pub NativeFunc);

impl<'gc> Callable<'gc> for NativeCallable {
    fn call(
        &self,
        vm: &mut VmState<'gc>,
        mc: &Mutation<'gc>,
        args: Vec<Value<'gc>>,
    ) -> Result<(), BBError> {
        let ret = self.0.0(vm, mc, args)?;
        vm.push(ret);
        Ok(())
    }
}

impl<'gc> VmState<'gc> {
    pub fn new(mc: &Mutation<'gc>) -> Self {
        Self {
            stack: Vec::new(),
            frames: Vec::new(),
            globals: gcl!(mc, HashMap::new()),
            next_frame_id: 1,
            builtin_cache: gcl!(mc, BuiltinCache::new()),
        }
    }

    pub fn new_object(
        &self,
        mc: &Mutation<'gc>,
        class_obj: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Gc<'gc, RefLock<Object<'gc>>> {
        let mut fields = HashMap::new();
        let nil_val = self.new_nil(mc);
        for var in self.get_all_instance_vars(class_obj) {
            fields.insert(var.clone(), nil_val);
        }
        gcl!(
            mc,
            Object {
                id: GcUlid(Ulid::new()),
                class: class_obj,
                fields,
                payload: ObjectPayload::Instance,
            }
        )
    }

    pub fn new_native_state<T: AnyCollect + 'static>(
        &self,
        mc: &Mutation<'gc>,
        class_obj: Gc<'gc, RefLock<Class<'gc>>>,
        state: T,
    ) -> Value<'gc> {
        let payload = ObjectPayload::NativeState(gcl!(mc, Box::new(state) as Box<dyn AnyCollect>));
        let obj = gcl!(
            mc,
            Object {
                id: GcUlid(Ulid::new()),
                class: class_obj,
                fields: HashMap::new(),
                payload,
            }
        );
        Value::Object(obj)
    }

    pub fn new_nil(&self, mc: &Mutation<'gc>) -> Value<'gc> {
        let cached = self.builtin_cache.borrow().nil_val;
        if let Some(v) = cached {
            v
        } else {
            let class = self.builtin_cache.borrow().nil_class;
            let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Nil"));
            let v = Value::Object(gcl!(
                mc,
                Object {
                    id: GcUlid(Ulid::new()),
                    class,
                    fields: HashMap::new(),
                    payload: ObjectPayload::Nil,
                }
            ));
            self.builtin_cache.borrow_mut(mc).nil_val = Some(v);
            v
        }
    }

    pub fn new_bool(&self, mc: &Mutation<'gc>, b: bool) -> Value<'gc> {
        let cached = if b {
            self.builtin_cache.borrow().true_val
        } else {
            self.builtin_cache.borrow().false_val
        };
        if let Some(v) = cached {
            v
        } else {
            let class = self.builtin_cache.borrow().boolean_class;
            let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Boolean"));
            let v = Value::Object(gcl!(
                mc,
                Object {
                    id: GcUlid(Ulid::new()),
                    class,
                    fields: HashMap::new(),
                    payload: ObjectPayload::Bool(b),
                }
            ));
            if b {
                self.builtin_cache.borrow_mut(mc).true_val = Some(v);
            } else {
                self.builtin_cache.borrow_mut(mc).false_val = Some(v);
            }
            v
        }
    }

    pub fn new_int(&self, mc: &Mutation<'gc>, i: i64) -> Value<'gc> {
        let class = self.builtin_cache.borrow().integer_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Integer"));
        Value::Object(gcl!(
            mc,
            Object {
                id: GcUlid(Ulid::new()),
                class,
                fields: HashMap::new(),
                payload: ObjectPayload::Int(i),
            }
        ))
    }

    pub fn new_double(&self, mc: &Mutation<'gc>, f: f64) -> Value<'gc> {
        let class = self.builtin_cache.borrow().double_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Double"));
        Value::Object(gcl!(
            mc,
            Object {
                id: GcUlid(Ulid::new()),
                class,
                fields: HashMap::new(),
                payload: ObjectPayload::Double(f),
            }
        ))
    }

    pub fn new_string(&self, mc: &Mutation<'gc>, s: String) -> Value<'gc> {
        let class = self.builtin_cache.borrow().string_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "String"));
        Value::Object(gcl!(
            mc,
            Object {
                id: GcUlid(Ulid::new()),
                class,
                fields: HashMap::new(),
                payload: ObjectPayload::String(gc!(mc, s)),
            }
        ))
    }

    pub fn new_list(&self, mc: &Mutation<'gc>, list: Vec<Value<'gc>>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().list_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "List"));
        let state = NativeListState::new(list);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                id: GcUlid(Ulid::new()),
                class,
                fields: HashMap::new(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    pub fn new_map(&self, mc: &Mutation<'gc>, map: HashMap<String, Value<'gc>>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().map_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Map"));
        Value::Object(gcl!(
            mc,
            Object {
                id: GcUlid(Ulid::new()),
                class,
                fields: HashMap::new(),
                payload: ObjectPayload::Map(gcl!(mc, map)),
            }
        ))
    }

    pub fn new_regex(&self, mc: &Mutation<'gc>, regex: regex::Regex) -> Value<'gc> {
        let class = self.builtin_cache.borrow().regex_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Regex"));
        Value::Object(gcl!(
            mc,
            Object {
                id: GcUlid(Ulid::new()),
                class,
                fields: HashMap::new(),
                payload: ObjectPayload::Regex(gc!(mc, GcRegex(regex))),
            }
        ))
    }

    pub fn new_block(&self, mc: &Mutation<'gc>, block: Block<'gc>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().block_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Block"));
        Value::Object(gcl!(
            mc,
            Object {
                id: GcUlid(Ulid::new()),
                class,
                fields: HashMap::new(),
                payload: ObjectPayload::Block(gc!(mc, block)),
            }
        ))
    }

    pub fn new_native(&self, mc: &Mutation<'gc>, func: NativeFunc) -> Value<'gc> {
        let class = self.builtin_cache.borrow().native_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Native"));
        Value::Object(gcl!(
            mc,
            Object {
                id: GcUlid(Ulid::new()),
                class,
                fields: HashMap::new(),
                payload: ObjectPayload::Native(func),
            }
        ))
    }

    pub fn new_method(
        &self,
        mc: &Mutation<'gc>,
        selector: String,
        block: Value<'gc>,
        is_extension: bool,
    ) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Method");
        let state = crate::runtime::method::NativeMethodState::new(selector, block, is_extension);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                id: GcUlid(Ulid::new()),
                class,
                fields: HashMap::new(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    fn finalize_instantiation(
        &mut self,
        mc: &Mutation<'gc>,
        obj: Gc<'gc, RefLock<Object<'gc>>>,
        env_borrow: &EnvFrame<'gc>,
    ) -> Result<(), BBError> {
        let vars = self.get_all_instance_vars(obj.borrow().class);
        for var in &vars {
            if let Some(val) = env_borrow.vars.get(var) {
                obj.borrow_mut(mc).fields.insert(var.clone(), *val);
            }
        }

        // Call init: if it exists
        let init_val = self.lookup_in_class_hierarchy(obj.borrow().class, "init:", false);
        let mut init_block = None;
        if let Some(val) = init_val {
            if let Value::Object(ref io) = val {
                match &io.borrow().payload {
                    ObjectPayload::Block(b) => {
                        init_block = Some(b.clone());
                    }
                    ObjectPayload::NativeState(state_cell) => {
                        let state_ref = state_cell.borrow();
                        let any_ref = (**state_ref).as_any();
                        if let Some(method_state) = any_ref.downcast_ref::<crate::runtime::method::NativeMethodState>() {
                            let block_val = method_state.get_block();
                            if let Value::Object(block_obj) = block_val
                                && let ObjectPayload::Block(b) = &block_obj.borrow().payload
                            {
                                init_block = Some(b.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if let Some(block) = init_block {
            let mut init_args = Vec::new();
            for param in &block.param_names {
                let val = env_borrow.vars.get(param).copied().unwrap_or_else(|| self.new_nil(mc));
                init_args.push(val);
            }
            self.call_method(mc, Value::Object(obj), "init:", init_args)?;
        } else {
            // Call init if it exists
            let has_init = self.lookup_in_class_hierarchy(obj.borrow().class, "init", false).is_some();
            if has_init {
                self.call_method(mc, Value::Object(obj), "init", Vec::new())?;
            }
        }

        Ok(())
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
                    instance_methods: HashMap::new(),
                    class_methods: HashMap::new(),
                    mixin_classes: Vec::new(),
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
                "Native" => cache.native_class = Some(class_obj),
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
        let parent_class = if let Some(parent_name) = native_class.parent_name() {
            Some(self.get_or_create_builtin_class(mc, parent_name))
        } else {
            None
        };

        let mut inst_methods = HashMap::new();
        for (name, func) in native_class.instance_methods() {
            inst_methods.insert(name, self.new_native(mc, func));
        }

        let mut cls_methods = HashMap::new();
        for (name, func) in native_class.class_methods() {
            cls_methods.insert(name, self.new_native(mc, func));
        }

        let name = native_class.name();
        let ns_name = NamespacedName::parse(name);
        let existing = self.globals.borrow().get(&ns_name).copied();
        if let Some(Value::Class(existing_class)) = existing {
            let mut borrowed = existing_class.borrow_mut(mc);
            borrowed.parent = parent_class;
            borrowed.instance_methods = inst_methods;
            borrowed.class_methods = cls_methods;
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
                "Native" => cache.native_class = Some(class_obj),
                _ => {}
            }
        }
    }

    pub fn call_method(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, BBError> {
        let method = self.lookup_method(receiver, selector);
        if let Some(method) = method {
            let mut all_args = vec![receiver];
            all_args.extend(args);
            let initial_frame_count = self.frames.len();
            method.call(self, mc, all_args)?;

            // let the VM catch up
            if self.frames.len() > initial_frame_count {
                while self.frames.len() > initial_frame_count {
                    match self.step(mc)? {
                        VmStatus::Running => {}
                        VmStatus::Finished(_) => {
                            break;
                        }
                        VmStatus::Yeeted(val) => {
                            return Err(BBError::Other(format!(
                                "Uncaught exception during method call: {}",
                                val
                            )));
                        }
                    }
                }
            }

            Ok(self.pop()?)
        } else {
            Ok(self.new_nil(mc))
        }
    }

    pub fn execute_block(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        args: Vec<Value<'gc>>,
        self_val: Option<Value<'gc>>,
    ) -> Result<Value<'gc>, BBError> {
        let initial_frame_count = self.frames.len();
        if let Some(receiver) = self_val {
            self.start_block_as_method(mc, block, receiver, args);
        } else {
            self.start_block(mc, block, args);
        }

        if self.frames.len() > initial_frame_count {
            while self.frames.len() > initial_frame_count {
                match self.step(mc)? {
                    VmStatus::Running => {}
                    VmStatus::Finished(_) => {
                        break;
                    }
                    VmStatus::Yeeted(val) => {
                        return Err(BBError::Other(format!(
                            "Uncaught exception during block execution: {}",
                            val
                        )));
                    }
                }
            }
        }

        Ok(self.pop()?)
    }

    pub fn lookup_method(
        &self,
        receiver: Value<'gc>,
        selector: &str,
    ) -> Option<Box<dyn Callable<'gc> + 'gc>> {
        if selector == "meta" {
            if let Value::Class(c) = receiver {
                return Some(Box::new(MetaCallable { class_obj: c }));
            }
        }
        if selector == "new:" {
            if let Value::Class(c) = receiver {
                return Some(Box::new(NewCallable { class_obj: c }));
            }
        }
        if selector == "new" {
            if let Value::Class(c) = receiver {
                return Some(Box::new(NewNoBlockCallable { class_obj: c }));
            }
        }
        let selector_key = NamespacedName::new(Vec::new(), selector.to_string());
        let method_val = match receiver {
            Value::Class(class_obj) => {
                if let Some(m) = self.lookup_in_class_hierarchy(class_obj, selector, true) {
                    Some(m)
                } else {
                    let class_key = NamespacedName::new(Vec::new(), "Class".to_string());
                    if let Some(Value::Class(class_class)) =
                        self.globals.borrow().get(&class_key).copied()
                    {
                        if let Some(m) =
                            self.lookup_in_class_hierarchy(class_class, selector, false)
                        {
                            Some(m)
                        } else {
                            self.globals.borrow().get(&selector_key).copied()
                        }
                    } else {
                        self.globals.borrow().get(&selector_key).copied()
                    }
                }
            }
            Value::ClassMeta(class_obj) => {
                if let Some(m) = self.lookup_in_class_hierarchy(class_obj, selector, true) {
                    Some(m)
                } else {
                    self.globals.borrow().get(&selector_key).copied()
                }
            }
            Value::Object(obj) => {
                let class_obj = obj.borrow().class;
                if let Some(m) = self.lookup_in_class_hierarchy(class_obj, selector, false) {
                    Some(m)
                } else {
                    self.globals.borrow().get(&selector_key).copied()
                }
            }
        }?;

        match method_val {
            Value::Object(obj) => match &obj.borrow().payload {
                ObjectPayload::Block(block) => Some(Box::new(BlockCallable { block: *block })),
                ObjectPayload::Native(native_fn) => Some(Box::new(NativeCallable(*native_fn))),
                ObjectPayload::NativeState(state_cell) => {
                    let state_ref = state_cell.borrow();
                    let any_ref = (**state_ref).as_any();
                    if let Some(method_state) =
                        any_ref.downcast_ref::<crate::runtime::method::NativeMethodState>()
                    {
                        let block_val = method_state.get_block();
                        if let Value::Object(block_obj) = block_val
                            && let ObjectPayload::Block(block) = &block_obj.borrow().payload
                        {
                            Some(Box::new(BlockCallable { block: *block }))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }

    pub fn lookup_in_class_hierarchy(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        selector: &str,
        class_side: bool,
    ) -> Option<Value<'gc>> {
        let mut visited = Vec::new();
        self.lookup_in_class_hierarchy_rec(class_ref, selector, class_side, &mut visited)
    }

    fn lookup_in_class_hierarchy_rec(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        selector: &str,
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
        if let Some(method) = methods.get(selector).copied() {
            return Some(method);
        }
        for mixin in &class_borrow.mixin_classes {
            if let Some(method) =
                self.lookup_in_class_hierarchy_rec(*mixin, selector, class_side, visited)
            {
                return Some(method);
            }
        }
        if let Some(parent) = class_borrow.parent {
            if let Some(method) =
                self.lookup_in_class_hierarchy_rec(parent, selector, class_side, visited)
            {
                return Some(method);
            }
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

    pub fn start_block(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        args: Vec<Value<'gc>>,
    ) {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let mut env_frame = EnvFrame::new(block.parent_env);
        // Bind parameters
        for (name, val) in block.param_names.iter().zip(args.into_iter()) {
            env_frame.vars.insert(name.clone(), val);
        }
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.is_nested_block;
        let enclosing_method_id = if is_nested_block {
            block.enclosing_method_id
        } else {
            Some(frame_id)
        };

        self.frames.push(Frame {
            id: frame_id,
            is_nested_block,
            enclosing_method_id,
            block,
            ip: 0,
            env: env_ref,
            instantiating_obj: None,
        });
    }

    pub fn start_block_as_method(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        receiver: Value<'gc>,
        args: Vec<Value<'gc>>,
    ) {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let mut env_frame = EnvFrame::new(block.parent_env);
        // Bind self
        env_frame.vars.insert("self".to_string(), receiver);
        // Bind parameters
        for (name, val) in block.param_names.iter().zip(args.into_iter()) {
            env_frame.vars.insert(name.clone(), val);
        }
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.is_nested_block;
        let enclosing_method_id = Some(frame_id);

        self.frames.push(Frame {
            id: frame_id,
            is_nested_block,
            enclosing_method_id,
            block,
            ip: 0,
            env: env_ref,
            instantiating_obj: None,
        });
    }

    pub fn start_block_for_instantiation(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        obj: Gc<'gc, RefLock<Object<'gc>>>,
    ) {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let mut env_frame = EnvFrame::new(block.parent_env);
        // Bind all instance variables as local variables in this block, initialized from the parent env or current fields
        let vars = self.get_all_instance_vars(obj.borrow().class);
        for var in &vars {
            let val = if let Some(parent) = block.parent_env {
                EnvFrame::get(parent, var).unwrap_or_else(|| {
                    obj.borrow()
                        .fields
                        .get(var)
                        .copied()
                        .unwrap_or_else(|| self.new_nil(mc))
                })
            } else {
                obj.borrow()
                    .fields
                    .get(var)
                    .copied()
                    .unwrap_or_else(|| self.new_nil(mc))
            };
            env_frame.vars.insert(var.clone(), val);
        }
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.is_nested_block;
        let enclosing_method_id = if is_nested_block {
            block.enclosing_method_id
        } else {
            Some(frame_id)
        };

        self.frames.push(Frame {
            id: frame_id,
            is_nested_block,
            enclosing_method_id,
            block,
            ip: 0,
            env: env_ref,
            instantiating_obj: Some(obj),
        });
    }

    pub fn get_class_for_lookup(
        &self,
        receiver: Value<'gc>,
    ) -> Option<Gc<'gc, RefLock<Class<'gc>>>> {
        match receiver {
            Value::Object(obj) => Some(obj.borrow().class),
            Value::Class(c) => Some(c),
            Value::ClassMeta(c) => Some(c),
        }
    }

    pub fn get_target_class_for_def(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<Gc<'gc, RefLock<Class<'gc>>>, String> {
        match receiver {
            Value::Class(c) => Ok(c),
            Value::ClassMeta(c) => Ok(c),
            Value::Object(obj) => {
                let class_ref = obj.borrow().class;
                if class_ref.borrow().name.name.starts_with('$') {
                    Ok(class_ref)
                } else {
                    let singleton_name = match &obj.borrow().payload {
                        ObjectPayload::Nil => {
                            NamespacedName::new(Vec::new(), "$NilClass".to_string())
                        }
                        ObjectPayload::Bool(true) => {
                            NamespacedName::new(Vec::new(), "$TrueClass".to_string())
                        }
                        ObjectPayload::Bool(false) => {
                            NamespacedName::new(Vec::new(), "$FalseClass".to_string())
                        }
                        _ => {
                            let mut ns_name = class_ref.borrow().name.clone();
                            ns_name.name = format!("${}", ns_name.name);
                            ns_name
                        }
                    };
                    let s = gcl!(
                        mc,
                        Class {
                            name: singleton_name,
                            parent: Some(class_ref),
                            instance_vars: Vec::new(),
                            instance_methods: HashMap::new(),
                            class_methods: HashMap::new(),
                            mixin_classes: Vec::new(),
                        }
                    );
                    obj.borrow_mut(mc).class = s;
                    Ok(s)
                }
            }
        }
    }

    pub fn annotate_error(&self, error: BBError) -> BBError {
        if matches!(error, BBError::WithSourceInfo { .. }) {
            return error;
        }
        if let Some(frame) = self.frames.last() {
            if let Some(source_info) = &frame.block.source_info {
                return BBError::WithSourceInfo {
                    error: Box::new(error),
                    source_info: source_info.clone(),
                };
            }
        }
        error
    }

    pub fn step(&mut self, mc: &Mutation<'gc>) -> Result<VmStatus<'gc>, BBError> {
        let res = self.step_internal(mc);
        if let Err(e) = res {
            return Err(self.annotate_error(e));
        }
        res
    }

    fn step_internal(&mut self, mc: &Mutation<'gc>) -> Result<VmStatus<'gc>, BBError> {
        if self.frames.is_empty() {
            let ret = self.pop().unwrap_or_else(|_| self.new_nil(mc));
            // assert_eq!(self.stack.len(), 0, "Stack is not empty! {:?}", self.stack);
            return Ok(VmStatus::Finished(ret));
        }

        let frame_idx = self.frames.len() - 1;
        let inst = {
            let frame = &self.frames[frame_idx];
            frame.block.bytecode.get(frame.ip).cloned()
        };

        let inst = match inst {
            Some(i) => i,
            None => {
                // Implicit return Nil
                let ret_val = self.new_nil(mc);
                self.frames.pop();
                self.push(ret_val);
                return Ok(VmStatus::Running);
            }
        };

        match inst {
            Instruction::LoadLocal(name) => {
                let frame = &self.frames[frame_idx];
                let val = EnvFrame::get(frame.env, &name).unwrap_or_else(|| self.new_nil(mc));
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::DefineLocal(name) => {
                let val = self.pop()?;
                let frame = &mut self.frames[frame_idx];
                frame.env.borrow_mut(mc).vars.insert(name, val);
                frame.ip += 1;
            }
            Instruction::StoreLocal(name) => {
                let val = self.pop()?;
                let frame = &mut self.frames[frame_idx];
                if !EnvFrame::set(frame.env, mc, &name, val) {
                    frame.env.borrow_mut(mc).vars.insert(name, val);
                }
                frame.ip += 1;
            }
            Instruction::LoadGlobal(name) => {
                let val = self
                    .globals
                    .borrow()
                    .get(&name)
                    .copied()
                    .unwrap_or_else(|| self.new_nil(mc));
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::StoreGlobal(name) => {
                let val = self.pop()?;
                self.globals.borrow_mut(mc).insert(name, val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::Push(constant) => {
                let val = match constant {
                    Constant::Nil => self.new_nil(mc),
                    Constant::Bool(b) => self.new_bool(mc, b),
                    Constant::Int(i) => self.new_int(mc, i),
                    Constant::Double(f) => self.new_double(mc, f),
                    Constant::String(s) => self.new_string(mc, s.clone()),
                    Constant::Block(sb) => {
                        let parent_env = self.frames.last().map(|f| f.env);
                        let enclosing_method_id =
                            self.frames.last().and_then(|f| f.enclosing_method_id);
                        let block = Block { name: sb.name.clone(),
                            is_nested_block: sb.is_nested_block,
                            param_names: sb.param_names.clone(),
                            bytecode: sb.bytecode.clone(),
                            parent_env,
                            enclosing_method_id,
                            source_info: sb.source_info.clone(),
                        };
                        self.new_block(mc, block)
                    }
                };
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::Pop => {
                self.pop()?;
                self.frames[frame_idx].ip += 1;
            }
            Instruction::Dup => {
                let val = self.peek()?;
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::Send(selector, num_args) => {
                let mut args = Vec::new();
                for _ in 0..num_args {
                    args.push(self.pop()?);
                }
                args.reverse();

                let receiver = self.pop()?;
                self.frames[frame_idx].ip += 1; // Advance caller frame IP

                if let Value::Object(obj) = receiver
                    && let ObjectPayload::Block(block) = &obj.borrow().payload
                {
                    if selector == "value" || selector == "value:" {
                        self.start_block(mc, *block, args);
                        return Ok(VmStatus::Running);
                    }
                }

                let method_opt = self.lookup_method(receiver, &selector);
                if let Some(callable) = method_opt {
                    let mut all_args = vec![receiver];
                    all_args.extend(args);
                    callable.call(self, mc, all_args)?;
                } else {
                    return Err(format!(
                        "Message not understood: receiver={:?}, selector='{}', args={:?}",
                        receiver, selector, args
                    )
                    .into());
                }
            }
            Instruction::Return | Instruction::BlockReturn => {
                let mut ret_val = self.pop()?;
                let popped_frame = self.frames.pop().unwrap();
                if let Some(obj) = popped_frame.instantiating_obj {
                    let env_borrow = popped_frame.env.borrow();
                    self.finalize_instantiation(mc, obj, &env_borrow)?;
                    ret_val = Value::Object(obj);
                }
                self.push(ret_val);
            }
            Instruction::MethodReturn => {
                let ret_val = self.pop()?;
                let enclosing_id = self.frames[frame_idx].enclosing_method_id;
                if let Some(target_id) = enclosing_id {
                    while let Some(f) = self.frames.pop() {
                        if let Some(obj) = f.instantiating_obj {
                            let env_borrow = f.env.borrow();
                            self.finalize_instantiation(mc, obj, &env_borrow)?;
                        }
                        if f.id == target_id {
                            break;
                        }
                    }
                    self.push(ret_val);
                } else {
                    return Err("MethodReturn executed outside of a method context".into());
                }
            }
            Instruction::Yeet => {
                let yeeted_val = self.pop()?;
                self.frames.clear();
                return Ok(VmStatus::Yeeted(yeeted_val));
            }
            Instruction::Jump(offset) => {
                let frame = &mut self.frames[frame_idx];
                frame.ip = (frame.ip as isize + offset) as usize;
            }
            Instruction::IfJump(offset) => {
                let cond = self.pop()?;
                let frame = &mut self.frames[frame_idx];
                if cond.is_truthy() {
                    frame.ip = (frame.ip as isize + offset) as usize;
                } else {
                    frame.ip += 1;
                }
            }
            Instruction::ElseJump(offset) => {
                let cond = self.pop()?;
                let frame = &mut self.frames[frame_idx];
                if !cond.is_truthy() {
                    frame.ip = (frame.ip as isize + offset) as usize;
                } else {
                    frame.ip += 1;
                }
            }
            Instruction::NewList(n) => {
                let mut elements = Vec::new();
                for _ in 0..n {
                    elements.push(self.pop()?);
                }
                elements.reverse();
                let list = self.new_list(mc, elements);
                self.push(list);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::NewMap(n) => {
                let mut map = HashMap::new();
                for _ in 0..n {
                    let val = self.pop()?;
                    let key_val = self.pop()?;
                    if let Value::Object(obj) = key_val
                        && let ObjectPayload::String(s) = &obj.borrow().payload
                    {
                        map.insert((**s).clone(), val);
                    } else {
                        return Err(BBError::TypeError {
                            expected: "String".to_string(),
                            got: key_val.type_name().to_string(),
                            msg: format!("Map keys must be Strings, got: {:?}", key_val),
                        });
                    }
                }
                let map_val = self.new_map(mc, map);
                self.push(map_val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::NewRegex => {
                let pattern_val = self.pop()?;
                if let Value::Object(obj) = pattern_val
                    && let ObjectPayload::String(s) = &obj.borrow().payload
                {
                    let re =
                        regex::Regex::new(&**s).map_err(|e| format!("Invalid regex: {}", e))?;
                    let regex_val = self.new_regex(mc, re);
                    self.push(regex_val);
                } else {
                    return Err(BBError::TypeError {
                        expected: "String".to_string(),
                        got: pattern_val.type_name().to_string(),
                        msg: format!("Regex pattern must be a String, got: {:?}", pattern_val),
                    });
                }
                self.frames[frame_idx].ip += 1;
            }
            Instruction::DefineClass {
                name,
                parent_name,
                instance_vars,
            } => {
                let parent = if let Some(p_name) = &parent_name {
                    let val = self
                        .globals
                        .borrow()
                        .get(p_name)
                        .copied()
                        .ok_or_else(|| format!("Parent class {} not found", p_name))?;
                    if let Value::Class(parent_class) = val {
                        Some(parent_class)
                    } else {
                        return Err(format!("Parent {} is not a Class", p_name).into());
                    }
                } else {
                    if !(name.path.is_empty() && name.name == "Object") {
                        let obj_key = NamespacedName::new(Vec::new(), "Object".to_string());
                        if let Some(Value::Class(obj_class)) =
                            self.globals.borrow().get(&obj_key).copied()
                        {
                            Some(obj_class)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(existing_val) = self.globals.borrow().get(&name).copied() {
                    if let Value::Class(_) = existing_val {
                        return Err(format!(
                            "Cannot redefine class {} because it already exists",
                            name.to_explicit_string()
                        )
                        .into());
                    }
                }

                let class_obj = gcl!(
                    mc,
                    Class {
                        name: name.clone(),
                        parent,
                        instance_vars: instance_vars.clone(),
                        instance_methods: HashMap::new(),
                        class_methods: HashMap::new(),
                        mixin_classes: Vec::new(),
                    }
                );
                self.globals
                    .borrow_mut(mc)
                    .insert(name.clone(), Value::Class(class_obj));
                self.push(Value::Class(class_obj));
                self.frames[frame_idx].ip += 1;
            }
            Instruction::ExecuteBlockWithSelf => {
                let block_val = self.pop()?;
                let self_val = self.pop()?;
                if self_val.is_nil() {
                    return Err(BBError::Other(
                        "Cannot extend nil or non-existent class/object".to_string()
                    ));
                }
                return if let Value::Object(obj) = block_val
                    && let ObjectPayload::Block(block) = &obj.borrow().payload
                {
                    self.frames[frame_idx].ip += 1;
                    self.start_block_as_method(mc, *block, self_val, Vec::new());
                    Ok(VmStatus::Running)
                } else {
                    Err(BBError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: format!("ExecuteBlockWithSelf expects a Block, got {:?}", block_val),
                    })
                };
            }
            Instruction::DefineMethod(selector) => {
                let block_val = self.pop()?;
                if let Value::Object(obj) = block_val
                    && let ObjectPayload::Block(_) = &obj.borrow().payload
                {
                    let self_val = EnvFrame::get(self.frames[frame_idx].env, "self")
                        .unwrap_or_else(|| self.new_nil(mc));
                    let target_class = self
                        .get_target_class_for_def(mc, self_val)
                        .map_err(|e| BBError::Other(e))?;

                    let method_obj = self.new_method(mc, selector.clone(), block_val, false);
                    let is_class_side = matches!(self_val, Value::ClassMeta(_));
                    if is_class_side {
                        if target_class.borrow().class_methods.contains_key(&selector) {
                            return Err(BBError::Other(format!(
                                "Method {} already exists on Class {}",
                                selector,
                                target_class.borrow().name
                            )));
                        }
                        target_class
                            .borrow_mut(mc)
                            .class_methods
                            .insert(selector, method_obj);
                    } else {
                        if target_class
                            .borrow()
                            .instance_methods
                            .contains_key(&selector)
                        {
                            return Err(BBError::Other(format!(
                                "Method {} already exists on Class {}",
                                selector,
                                target_class.borrow().name
                            )));
                        }
                        target_class
                            .borrow_mut(mc)
                            .instance_methods
                            .insert(selector, method_obj);
                    }
                    self.push(method_obj);
                    self.frames[frame_idx].ip += 1;
                } else {
                    return Err(BBError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: format!("DefineMethod expects a Block, got {:?}", block_val),
                    });
                }
            }
            Instruction::OverrideMethod(selector) => {
                let block_val = self.pop()?;
                if let Value::Object(obj) = block_val
                    && let ObjectPayload::Block(_) = &obj.borrow().payload
                {
                    let self_val = EnvFrame::get(self.frames[frame_idx].env, "self")
                        .unwrap_or_else(|| self.new_nil(mc));
                    let target_class = self
                        .get_target_class_for_def(mc, self_val)
                        .map_err(|e| BBError::Other(e))?;

                    let method_obj = self.new_method(mc, selector.clone(), block_val, true);
                    let is_class_side = matches!(self_val, Value::ClassMeta(_));
                    let exists = self
                        .lookup_in_class_hierarchy(target_class, &selector, is_class_side)
                        .is_some();
                    if !exists {
                        return Err(BBError::Other(format!(
                            "Method {} does not exist in hierarchy of Class {} to override",
                            selector,
                            target_class.borrow().name
                        )));
                    }

                    if is_class_side {
                        target_class
                            .borrow_mut(mc)
                            .class_methods
                            .insert(selector, method_obj);
                    } else {
                        target_class
                            .borrow_mut(mc)
                            .instance_methods
                            .insert(selector, method_obj);
                    }
                    self.push(method_obj);
                    self.frames[frame_idx].ip += 1;
                } else {
                    return Err(BBError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: format!("OverrideMethod expects a Block, got {:?}", block_val),
                    });
                }
            }
            Instruction::LoadField(name) => {
                let frame = &self.frames[frame_idx];
                let self_val = EnvFrame::get(frame.env, "self").unwrap_or_else(|| self.new_nil(mc));
                let val = if let Value::Object(obj) = self_val {
                    obj.borrow()
                        .fields
                        .get(&name)
                        .copied()
                        .unwrap_or_else(|| self.new_nil(mc))
                } else {
                    self.new_nil(mc)
                };
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::StoreField(name) => {
                let val = self.pop()?;
                let frame = &self.frames[frame_idx];
                let self_val = EnvFrame::get(frame.env, "self").unwrap_or_else(|| self.new_nil(mc));
                if let Value::Object(obj) = self_val {
                    obj.borrow_mut(mc).fields.insert(name, val);
                } else {
                    return Err(BBError::Other(format!(
                        "Cannot set field '{}' on non-object {:?}",
                        name, self_val
                    )));
                }
                self.frames[frame_idx].ip += 1;
            }
        }

        Ok(VmStatus::Running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::{Constant, StaticBlock};
    use crate::value::{NativeClassBuilder, OpaqueState};
    use crate::parser::ast_visitor::NodeValue;
    use gc_arena::{Arena, Rootable};

    fn native_add<'gc>(
        vm: &mut VmState<'gc>,
        mc: &Mutation<'gc>,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, BBError> {
        let a = match args[0] {
            Value::Object(obj) => match obj.borrow().payload {
                ObjectPayload::Int(i) => i,
                _ => return Err(BBError::Other("Invalid types".to_string())),
            },
            _ => return Err(BBError::Other("Invalid types".to_string())),
        };
        let b = match args[1] {
            Value::Object(obj) => match obj.borrow().payload {
                ObjectPayload::Int(i) => i,
                _ => return Err(BBError::Other("Invalid types".to_string())),
            },
            _ => return Err(BBError::Other("Invalid types".to_string())),
        };
        Ok(vm.new_int(mc, a + b))
    }

    #[derive(Debug, PartialEq, Clone)]
    enum ValueSpec {
        Nil,
        Bool(bool),
        Int(i64),
        Double(f64),
        String(String),
        Class(String),
        ClassMeta(String),
        List(Vec<ValueSpec>),
        Map(HashMap<String, ValueSpec>),
        Regex(String),
        Block(Option<String>),
        Native,
        Instance(String),
    }

    fn to_spec(val: Value<'_>) -> ValueSpec {
        match val {
            Value::Class(c) => ValueSpec::Class(c.borrow().name.to_string()),
            Value::ClassMeta(c) => ValueSpec::ClassMeta(c.borrow().name.to_string()),
            Value::Object(obj) => {
                let borrowed = obj.borrow();
                match &borrowed.payload {
                    ObjectPayload::Nil => ValueSpec::Nil,
                    ObjectPayload::Bool(b) => ValueSpec::Bool(*b),
                    ObjectPayload::Int(i) => ValueSpec::Int(*i),
                    ObjectPayload::Double(d) => ValueSpec::Double(*d),
                    ObjectPayload::String(s) => ValueSpec::String((**s).clone()),
                    _ if borrowed.class_name() == "List" => {
                        let res = val.with_native_state::<NativeListState, _, _>(|l| {
                            let list_specs = l.get_vec().iter().map(|&v| to_spec(v)).collect();
                            ValueSpec::List(list_specs)
                        });
                        res.unwrap_or_else(|_| ValueSpec::Instance("List".to_string()))
                    }
                    ObjectPayload::Map(m) => {
                        let map_specs = m
                            .borrow()
                            .iter()
                            .map(|(k, &v)| (k.clone(), to_spec(v)))
                            .collect();
                        ValueSpec::Map(map_specs)
                    }
                    ObjectPayload::Regex(r) => ValueSpec::Regex(r.0.as_str().to_string()),
                    ObjectPayload::Block(b) => ValueSpec::Block(b.name.clone()),
                    ObjectPayload::Native(_) => ValueSpec::Native,
                    ObjectPayload::Instance | ObjectPayload::NativeState(_) => {
                        ValueSpec::Instance(borrowed.class.borrow().name.to_string())
                    }
                }
            }
        }
    }

    #[derive(Debug, PartialEq, Clone)]
    enum VmStatusSpec {
        Running,
        Finished(ValueSpec),
        Yeeted(ValueSpec),
    }

    fn to_status_spec(status: VmStatus<'_>) -> VmStatusSpec {
        match status {
            VmStatus::Running => VmStatusSpec::Running,
            VmStatus::Finished(val) => VmStatusSpec::Finished(to_spec(val)),
            VmStatus::Yeeted(val) => VmStatusSpec::Yeeted(to_spec(val)),
        }
    }

    fn stack_spec(vm: &VmState<'_>) -> Vec<ValueSpec> {
        vm.stack.iter().copied().map(to_spec).collect()
    }

    fn run_test_steps<F>(instructions: Vec<Instruction>, check_steps: F)
    where
        F: for<'gc> FnOnce(&mut VmState<'gc>, &Mutation<'gc>),
    {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc);

            // Register standard classes first, so that they exist when new_xxx helper methods are called.
            vm.register_native_class(mc, crate::runtime::object::build_object_class());
            vm.register_native_class(mc, crate::runtime::class::build_class_class());
            vm.register_native_class(mc, crate::runtime::boolean::build_boolean_class());
            vm.register_native_class(mc, crate::runtime::block::build_block_class());
            vm.register_native_class(mc, crate::runtime::list::build_list_class());
            vm.register_native_class(mc, crate::runtime::double::build_double_class());
            vm.register_native_class(mc, crate::runtime::integer::build_integer_class());
            vm.register_native_class(mc, crate::runtime::string::build_string_class());

            for t in [
                "Nil",
                "Map",
                "Regex",
                "Method",
                "Native",
            ] {
                vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
            }

            // Register standard native functions we might need
            let native_val = vm.new_native(mc, NativeFunc(native_add));
            vm.globals
                .borrow_mut(mc)
                .insert(NamespacedName::new(Vec::new(), "+".to_string()), native_val);

            let static_block = StaticBlock { source_info: None,
                name: Some("test_main".to_string()),
                is_nested_block: false,
                param_names: Vec::new(),
                bytecode: instructions,
            };
            let block = gc!(
                mc,
                Block { source_info: None, name: static_block.name.clone(),
                    is_nested_block: static_block.is_nested_block,
                    param_names: static_block.param_names.clone(),
                    bytecode: static_block.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                }
            );
            vm.start_block(mc, block, Vec::new());
            vm
        });

        arena.mutate_root(|mc, vm| {
            check_steps(vm, mc);
        });
    }

    #[test]
    fn test_push_pop_dup() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Int(10)),
                Instruction::Push(Constant::Int(20)),
                Instruction::Pop,
                Instruction::Dup,
            ],
            |vm, mc| {
                // Initial: Stack = []
                assert_eq!(vm.stack.len(), 0);

                // Step 1: Push(10)
                let status = vm.step(mc).unwrap();
                assert_eq!(to_status_spec(status), VmStatusSpec::Running);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10)]);

                // Step 2: Push(20)
                let status = vm.step(mc).unwrap();
                assert_eq!(to_status_spec(status), VmStatusSpec::Running);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10), ValueSpec::Int(20)]);

                // Step 3: Pop
                let status = vm.step(mc).unwrap();
                assert_eq!(to_status_spec(status), VmStatusSpec::Running);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10)]);

                // Step 4: Dup
                let status = vm.step(mc).unwrap();
                assert_eq!(to_status_spec(status), VmStatusSpec::Running);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10), ValueSpec::Int(10)]);

                // Step 5: Implicit return Nil
                let status = vm.step(mc).unwrap();
                assert_eq!(to_status_spec(status), VmStatusSpec::Running);
                assert_eq!(
                    stack_spec(vm),
                    vec![ValueSpec::Int(10), ValueSpec::Int(10), ValueSpec::Nil]
                );

                // Pop the remaining values left on stack for testing to satisfy the stack-empty assertion
                vm.pop().unwrap(); // Nil
                vm.pop().unwrap(); // 10
                vm.pop().unwrap(); // 10
                let nil_val = vm.new_nil(mc);
                vm.push(nil_val); // Push it back as the return value

                // Step 6: Finished
                let status = vm.step(mc).unwrap();
                assert_eq!(
                    to_status_spec(status),
                    VmStatusSpec::Finished(ValueSpec::Nil)
                );
            },
        );
    }

    #[test]
    fn test_local_variables() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Int(42)),
                Instruction::DefineLocal("a".to_string()),
                Instruction::LoadLocal("a".to_string()),
                Instruction::Push(Constant::Int(100)),
                Instruction::StoreLocal("a".to_string()),
                Instruction::LoadLocal("a".to_string()),
            ],
            |vm, mc| {
                // Step 1: Push(42) -> [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

                // Step 2: DefineLocal("a") -> []
                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 0);

                // Step 3: LoadLocal("a") -> [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

                // Step 4: Push(100) -> [Int(42), Int(100)]
                vm.step(mc).unwrap();
                assert_eq!(
                    stack_spec(vm),
                    vec![ValueSpec::Int(42), ValueSpec::Int(100)]
                );

                // Step 5: StoreLocal("a") -> [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

                // Step 6: LoadLocal("a") -> [Int(42), Int(100)]
                vm.step(mc).unwrap();
                assert_eq!(
                    stack_spec(vm),
                    vec![ValueSpec::Int(42), ValueSpec::Int(100)]
                );
            },
        );
    }

    #[test]
    fn test_global_variables() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Int(77)),
                Instruction::StoreGlobal(NamespacedName::new(Vec::new(), "g_var".to_string())),
                Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "g_var".to_string())),
            ],
            |vm, mc| {
                // Step 1: Push(77)
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(77)]);

                // Step 2: StoreGlobal("g_var")
                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 0);

                // Step 3: LoadGlobal("g_var")
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(77)]);
            },
        );
    }

    #[test]
    fn test_constants() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Nil),
                Instruction::Push(Constant::Bool(true)),
                Instruction::Push(Constant::Double(3.14)),
                Instruction::Push(Constant::String("hello".to_string())),
            ],
            |vm, mc| {
                // Nil
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Nil]);

                // Bool
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Nil, ValueSpec::Bool(true)]);

                // Float
                vm.step(mc).unwrap();
                assert_eq!(
                    stack_spec(vm),
                    vec![
                        ValueSpec::Nil,
                        ValueSpec::Bool(true),
                        ValueSpec::Double(3.14)
                    ]
                );

                // String
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm).len(), 4);
                assert_eq!(stack_spec(vm)[3], ValueSpec::String("hello".to_string()));
            },
        );
    }

    #[test]
    fn test_jump_if_else() {
        run_test_steps(
            vec![
                // 0: Push true
                Instruction::Push(Constant::Bool(true)),
                // 1: IfJump to 4 (offset +3 -> 4)
                Instruction::IfJump(3),
                // 2: Push 99 (should be skipped)
                Instruction::Push(Constant::Int(99)),
                // 3: Jump to 5 (offset +2 -> 5)
                Instruction::Jump(2),
                // 4: Push 42 (target of IfJump)
                Instruction::Push(Constant::Int(42)),
                // 5: Push false
                Instruction::Push(Constant::Bool(false)),
                // 6: ElseJump to 9 (offset +3 -> 9)
                Instruction::ElseJump(3),
                // 7: Push 88 (should be skipped)
                Instruction::Push(Constant::Int(88)),
                // 8: Jump to 10 (offset +2 -> 10)
                Instruction::Jump(2),
                // 9: Push 55 (target of ElseJump)
                Instruction::Push(Constant::Int(55)),
            ],
            |vm, mc| {
                // Push true -> [Bool(true)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(true)]);

                // IfJump(3) -> condition true -> jump to index 4 (Push 42). Stack becomes []
                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 0);
                assert_eq!(vm.frames[0].ip, 4);

                // Push 42 -> [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

                // Push false -> [Int(42), Bool(false)]
                vm.step(mc).unwrap();
                assert_eq!(
                    stack_spec(vm),
                    vec![ValueSpec::Int(42), ValueSpec::Bool(false)]
                );

                // ElseJump(3) -> condition false -> jump to index 9 (Push 55). Stack becomes [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
                assert_eq!(vm.frames[0].ip, 9);

                // Push 55 -> [Int(42), Int(55)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42), ValueSpec::Int(55)]);
            },
        );
    }

    #[test]
    fn test_list_map_regex() {
        run_test_steps(
            vec![
                // List of 2 elements: Push 1, Push 2, NewList(2)
                Instruction::Push(Constant::Int(1)),
                Instruction::Push(Constant::Int(2)),
                Instruction::NewList(2),
                // Map of 1 pair: Push key "a", Push val 10, NewMap(1)
                Instruction::Push(Constant::String("a".to_string())),
                Instruction::Push(Constant::Int(10)),
                Instruction::NewMap(1),
                // Regex: Push pattern "^ab$", NewRegex
                Instruction::Push(Constant::String("^ab$".to_string())),
                Instruction::NewRegex,
            ],
            |vm, mc| {
                // List creation
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                vm.step(mc).unwrap(); // NewList(2)
                assert_eq!(vm.stack.len(), 1);
                assert_eq!(
                    stack_spec(vm),
                    vec![ValueSpec::List(vec![ValueSpec::Int(1), ValueSpec::Int(2)])]
                );

                // Map creation
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                vm.step(mc).unwrap(); // NewMap(1)
                assert_eq!(vm.stack.len(), 2);
                let mut expected_map = HashMap::new();
                expected_map.insert("a".to_string(), ValueSpec::Int(10));
                assert_eq!(stack_spec(vm)[1], ValueSpec::Map(expected_map));

                // Regex creation
                vm.step(mc).unwrap();
                vm.step(mc).unwrap(); // NewRegex
                assert_eq!(vm.stack.len(), 3);
                assert_eq!(stack_spec(vm)[2], ValueSpec::Regex("^ab$".to_string()));
            },
        );
    }

    #[test]
    fn test_send_message() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Int(5)),
                Instruction::Push(Constant::Int(10)),
                // Send "+" with 1 argument (selector: "+", receiver: Int(5), arg: Int(10))
                Instruction::Send("+".to_string(), 1),
            ],
            |vm, mc| {
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                // Send -> receiver 5 + argument 10 -> returns Int(15)
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(15)]);
            },
        );
    }

    #[test]
    fn test_block_execution_and_returns() {
        // We will push a block constant, then send "value" to it.
        // The block bytecode will load its parameter, add 1 to it, and return.
        let block_static = StaticBlock { source_info: None,
            name: Some("test_block".to_string()),
            is_nested_block: false,
            param_names: vec!["x".to_string()],
            bytecode: vec![
                Instruction::LoadLocal("x".to_string()),
                Instruction::Push(Constant::Int(1)),
                Instruction::Send("+".to_string(), 1),
                Instruction::Return,
            ],
        };

        run_test_steps(
            vec![
                Instruction::Push(Constant::Block(block_static)),
                Instruction::Push(Constant::Int(41)),
                // Send "value:" with 1 arg
                Instruction::Send("value:".to_string(), 1),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push block -> [Block]
                vm.step(mc).unwrap(); // Push 41 -> [Block, Int(41)]
                assert_eq!(vm.frames.len(), 1);

                // Send -> starts block frame -> [Block]
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 2);
                assert_eq!(vm.frames[1].block.name, Some("test_block".to_string()));

                // Inside block: LoadLocal("x") -> push 41 -> [41]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(41)]);

                // Inside block: Push(1) -> [41, 1]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(41), ValueSpec::Int(1)]);

                // Inside block: Send("+", 1) -> [42]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

                // Inside block: Return -> pops block frame, leaves return value on stack -> [42]
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 1);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
            },
        );
    }

    #[test]
    fn test_yeet_exception() {
        run_test_steps(
            vec![Instruction::Push(Constant::Int(500)), Instruction::Yeet],
            |vm, mc| {
                vm.step(mc).unwrap();
                let status = vm.step(mc).unwrap();
                assert_eq!(
                    to_status_spec(status),
                    VmStatusSpec::Yeeted(ValueSpec::Int(500))
                );
                assert_eq!(vm.frames.len(), 0);
            },
        );
    }

    #[test]
    fn test_method_return() {
        // Block 1 is the method context.
        // Block 2 is a nested block context.
        // If Block 2 executes MethodReturn, it should unwind all frames up to and including the method context (Block 1).

        // Block 2: nested block
        // Bytecode: Push(999), MethodReturn
        let block_nested = StaticBlock { source_info: None,
            name: Some("nested".to_string()),
            is_nested_block: true,
            param_names: Vec::new(),
            bytecode: vec![
                Instruction::Push(Constant::Int(999)),
                Instruction::MethodReturn,
            ],
        };

        // Block 1: method
        // Bytecode: Push(Block(nested)), Send("value", 0), Push(100), Return
        let block_method = StaticBlock { source_info: None,
            name: Some("method".to_string()),
            is_nested_block: false, // enclosing_method_id will be this frame's ID
            param_names: Vec::new(),
            bytecode: vec![
                Instruction::Push(Constant::Block(block_nested)),
                Instruction::Send("value".to_string(), 0),
                Instruction::Push(Constant::Int(100)), // this should be skipped due to MethodReturn
                Instruction::Return,
            ],
        };

        run_test_steps(
            vec![
                Instruction::Push(Constant::Block(block_method)),
                Instruction::Send("value".to_string(), 0),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push block_method
                vm.step(mc).unwrap(); // Send "value" -> starts block_method frame (frame id = 2, enclosing_method_id = 2)
                assert_eq!(vm.frames.len(), 2);
                assert_eq!(vm.frames[1].enclosing_method_id, Some(vm.frames[1].id));

                vm.step(mc).unwrap(); // Inside block_method: Push(block_nested)
                vm.step(mc).unwrap(); // Inside block_method: Send("value", 0) -> starts block_nested frame (frame id = 3, enclosing_method_id = 2)
                assert_eq!(vm.frames.len(), 3);
                assert_eq!(vm.frames[2].enclosing_method_id, Some(vm.frames[1].id));

                vm.step(mc).unwrap(); // Inside block_nested: Push(999) -> Stack has [999]
                // Inside block_nested: MethodReturn.
                // It should pop frame 3 (nested) and frame 2 (method), leaving only the main frame (frame 1),
                // and pushing 999 to the stack.
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 1);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(999)]);
            },
        );
    }

    #[test]
    fn test_non_local_return_callback() {
        // block_nested: Push(777), MethodReturn
        let block_nested = StaticBlock { source_info: None,
            name: Some("nested".to_string()),
            is_nested_block: true,
            param_names: Vec::new(),
            bytecode: vec![
                Instruction::Push(Constant::Int(777)),
                Instruction::MethodReturn,
            ],
        };

        // block_bar: blk.value, Push(111), Return
        let block_bar = StaticBlock { source_info: None,
            name: Some("bar".to_string()),
            is_nested_block: false,
            param_names: vec!["blk".to_string()],
            bytecode: vec![
                Instruction::LoadLocal("blk".to_string()),
                Instruction::Send("value".to_string(), 0),
                Instruction::Push(Constant::Int(111)),
                Instruction::Return,
            ],
        };

        // block_foo: bar.value: block_nested, Push(222), Return
        let block_foo = StaticBlock { source_info: None,
            name: Some("foo".to_string()),
            is_nested_block: false,
            param_names: Vec::new(),
            bytecode: vec![
                Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "bar_func".to_string())),
                Instruction::Push(Constant::Block(block_nested)),
                Instruction::Send("value:".to_string(), 1),
                Instruction::Push(Constant::Int(222)),
                Instruction::Return,
            ],
        };

        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc);
            let bar_block = Block { source_info: None, name: block_bar.name.clone(),
                is_nested_block: block_bar.is_nested_block,
                param_names: block_bar.param_names.clone(),
                bytecode: block_bar.bytecode.clone(),
                parent_env: None,
                enclosing_method_id: None,
            };
            let bar_block_val = vm.new_block(mc, bar_block);
            vm.globals.borrow_mut(mc).insert(
                NamespacedName::new(Vec::new(), "bar_func".to_string()),
                bar_block_val,
            );

            let foo_block = gc!(
                mc,
                Block { source_info: None, name: block_foo.name.clone(),
                    is_nested_block: block_foo.is_nested_block,
                    param_names: block_foo.param_names.clone(),
                    bytecode: block_foo.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                }
            );
            vm.start_block(mc, foo_block, Vec::new());
            vm
        });

        arena.mutate_root(|mc, vm| {
            // Step 1: Inside foo: LoadGlobal(bar_func)
            vm.step(mc).unwrap();
            // Step 2: Inside foo: Push(block_nested)
            vm.step(mc).unwrap();
            // Step 3: Inside foo: Send(value:) -> starts block_bar frame
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 2);
            assert_eq!(vm.frames[1].block.name, Some("bar".to_string()));

            // Step 4: Inside bar: LoadLocal(blk)
            vm.step(mc).unwrap();
            // Step 5: Inside bar: Send(value) -> starts block_nested frame
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 3);
            assert_eq!(vm.frames[2].block.name, Some("nested".to_string()));

            // Step 6: Inside nested: Push(777)
            vm.step(mc).unwrap();
            // Step 7: Inside nested: MethodReturn -> unwinds nested, bar, and foo frames!
            vm.step(mc).unwrap();

            // All frames should be unwound.
            assert_eq!(vm.frames.len(), 0);
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(777)]);
        });
    }

    #[test]
    fn test_class_and_method_definition_vm() {
        let class_block = StaticBlock { source_info: None,
            name: Some("class_block".to_string()),
            is_nested_block: false,
            param_names: Vec::new(),
            bytecode: vec![
                // 1. Define inst method x
                Instruction::Push(Constant::Block(StaticBlock { source_info: None,
                    name: Some("x".to_string()),
                    is_nested_block: false,
                    param_names: Vec::new(),
                    bytecode: vec![
                        Instruction::LoadLocal("self".to_string()),
                        Instruction::Return,
                    ],
                })),
                Instruction::DefineMethod("x".to_string()),
                // 2. Override inst method x
                Instruction::Push(Constant::Block(StaticBlock { source_info: None,
                    name: Some("x".to_string()),
                    is_nested_block: false,
                    param_names: Vec::new(),
                    bytecode: vec![Instruction::Push(Constant::Int(42)), Instruction::Return],
                })),
                Instruction::OverrideMethod("x".to_string()),
                Instruction::Return,
            ],
        };

        run_test_steps(
            vec![
                // Define class Point
                Instruction::DefineClass {
                    name: NamespacedName::new(Vec::new(), "Point".to_string()),
                    parent_name: None,
                    instance_vars: vec!["x".to_string(), "y".to_string()],
                },
                // Push class block
                Instruction::Push(Constant::Block(class_block)),
                // Execute block with Point as self
                Instruction::ExecuteBlockWithSelf,
                // Send "meta" to Point
                Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "Point".to_string())),
                Instruction::Send("meta".to_string(), 0),
            ],
            |vm, mc| {
                // Step DefineClass
                vm.step(mc).unwrap();
                let class_val = vm.peek().unwrap();
                if let Value::Class(c) = class_val {
                    assert_eq!(c.borrow().name.to_string(), "Point");
                    assert_eq!(
                        c.borrow().instance_vars,
                        vec!["x".to_string(), "y".to_string()]
                    );
                } else {
                    panic!("Expected Class value");
                }

                // Step Push Block
                vm.step(mc).unwrap();
                // Step ExecuteBlockWithSelf -> frame for class_block starts
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 2);
                assert_eq!(EnvFrame::get(vm.frames[1].env, "self").unwrap(), class_val);

                // Inside class_block: Push(x_block)
                vm.step(mc).unwrap();
                // Inside class_block: DefineMethod("x")
                vm.step(mc).unwrap();

                // Verify method x exists in instance_methods
                if let Value::Class(c) = class_val {
                    assert!(c.borrow().instance_methods.contains_key("x"));
                }

                // Inside class_block: Push(override_x_block)
                vm.step(mc).unwrap();
                // Inside class_block: OverrideMethod("x")
                vm.step(mc).unwrap();

                // Inside class_block: Return -> pops class_block frame, pushes Nil to main stack
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 1);

                // Step LoadGlobal Point
                vm.step(mc).unwrap();
                // Step Send meta
                vm.step(mc).unwrap();

                // Stack should have [Point, Nil, ClassMeta(Point)]
                let meta_val = vm.peek().unwrap();
                if let Value::ClassMeta(c) = meta_val {
                    assert_eq!(c.borrow().name.to_string(), "Point");
                } else {
                    panic!("Expected ClassMeta, got {:?}", meta_val);
                }
            },
        );
    }

    #[test]
    fn test_class_method_lookup_fallback() {
        run_test_steps(
            vec![
                Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "Point".to_string())),
                Instruction::Send("name".to_string(), 0),
            ],
            |vm, mc| {
                let point_class = gcl!(
                    mc,
                    Class {
                        name: NamespacedName::new(Vec::new(), "Point".to_string()),
                        parent: None,
                        instance_vars: Vec::new(),
                        instance_methods: HashMap::new(),
                        class_methods: HashMap::new(),
                        mixin_classes: Vec::new(),
                    }
                );
                vm.globals.borrow_mut(mc).insert(
                    NamespacedName::new(Vec::new(), "Point".to_string()),
                    Value::Class(point_class),
                );

                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 1);

                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 1);

                assert_eq!(stack_spec(vm), vec![ValueSpec::String("Point".to_string())]);
            },
        );
    }

    #[test]
    fn test_primitive_methods_and_overrides() {
        let custom_true_method = StaticBlock { source_info: None,
            name: Some("custom_true_method".to_string()),
            is_nested_block: false,
            param_names: Vec::new(),
            bytecode: vec![Instruction::Push(Constant::Int(42)), Instruction::Return],
        };

        let class_extension_block = StaticBlock { source_info: None,
            name: Some("class_extension_block".to_string()),
            is_nested_block: false,
            param_names: Vec::new(),
            bytecode: vec![
                Instruction::Push(Constant::Block(custom_true_method)),
                Instruction::DefineMethod("custom_true".to_string()),
                Instruction::Push(Constant::Nil),
                Instruction::Return,
            ],
        };

        run_test_steps(
            vec![
                Instruction::Push(Constant::Bool(true)),
                Instruction::Send("class".to_string(), 0),
                Instruction::Push(Constant::Bool(true)),
                Instruction::Push(Constant::Block(class_extension_block)),
                Instruction::ExecuteBlockWithSelf,
                Instruction::Push(Constant::Bool(true)),
                Instruction::Send("class".to_string(), 0),
                Instruction::Push(Constant::Bool(true)),
                Instruction::Send("custom_true".to_string(), 0),
                Instruction::Push(Constant::Bool(false)),
                Instruction::Send("class".to_string(), 0),
            ],
            |vm, mc| {
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                let class_val = vm.pop().unwrap();
                if let Value::Class(c) = class_val {
                    assert_eq!(c.borrow().name.to_string(), "Boolean");
                } else {
                    panic!("Expected Class Boolean, got {:?}", class_val);
                }

                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 2);

                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 1);
                assert_eq!(to_spec(vm.pop().unwrap()), ValueSpec::Nil);

                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                let class_val = vm.pop().unwrap();
                if let Value::Class(c) = class_val {
                    assert_eq!(c.borrow().name.to_string(), "$TrueClass");
                } else {
                    panic!("Expected Class $TrueClass, got {:?}", class_val);
                }

                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 2);
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 1);
                assert_eq!(to_spec(vm.pop().unwrap()), ValueSpec::Int(42));

                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                let class_val = vm.pop().unwrap();
                if let Value::Class(c) = class_val {
                    assert_eq!(c.borrow().name.to_string(), "Boolean");
                } else {
                    panic!("Expected Class Boolean, got {:?}", class_val);
                }
            },
        );
    }

    #[test]
    fn test_class_new() {
        run_test_steps(
            vec![
                Instruction::DefineClass {
                    name: NamespacedName::new(Vec::new(), "Point".to_string()),
                    parent_name: None,
                    instance_vars: vec!["x".to_string(), "y".to_string()],
                },
                Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "Point".to_string())),
                Instruction::Send("new".to_string(), 0),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // DefineClass
                vm.step(mc).unwrap(); // LoadGlobal
                vm.step(mc).unwrap(); // Send "new"
                let obj_val = vm.pop().unwrap();
                if let Value::Object(obj) = obj_val {
                    assert_eq!(obj.borrow().class.borrow().name.to_string(), "Point");
                    let fields = &obj.borrow().fields;
                    assert_eq!(to_spec(*fields.get("x").unwrap()), ValueSpec::Nil);
                    assert_eq!(to_spec(*fields.get("y").unwrap()), ValueSpec::Nil);
                } else {
                    panic!("Expected Object, got {:?}", obj_val);
                }
            },
        );
    }

    #[test]
    fn test_namespaced_native_class() {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc);

            // Build a namespaced native class [IO]File
            let file_builder = NativeClassBuilder::new("[IO]File", Some("Object"))
                .instance_method("path", |vm, mc, _args| {
                    Ok(vm.new_string(mc, "/etc/passwd".to_string()))
                });
            vm.register_native_class(mc, file_builder);
            vm
        });

        arena.mutate_root(|_mc, vm| {
            // Verify [IO]File class is in globals
            let file_key = NamespacedName::new(vec!["IO".to_string()], "File".to_string());
            let val = vm.globals.borrow().get(&file_key).copied().unwrap();
            if let Value::Class(c) = val {
                assert_eq!(c.borrow().name.to_string(), "[IO]File");
                assert_eq!(c.borrow().name.path, vec!["IO".to_string()]);
                assert_eq!(c.borrow().name.name, "File");
            } else {
                panic!("Expected Class, got {:?}", val);
            }
        });
    }

    #[derive(Debug)]
    struct MyCustomResource {
        counter: i32,
    }

    #[test]
    fn test_native_state_holding_rust_state() {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc);

            // Build native class [IO]Resource
            let resource_builder = NativeClassBuilder::new("[IO]Resource", Some("Object"))
                .class_method("create", |vm, mc, _args| {
                    let class_obj = vm.get_builtin_class("[IO]Resource");
                    let state = OpaqueState(MyCustomResource { counter: 10 });
                    Ok(vm.new_native_state(mc, class_obj, state))
                })
                .instance_method("get", |vm, mc, args| {
                    let val = args[0]
                        .with_native_state::<MyCustomResource, _, _>(|res| res.counter)
                        .unwrap();
                    Ok(vm.new_int(mc, val as i64))
                })
                .instance_method("inc:", |_vm, mc, args| {
                    let val = match args[1] {
                        Value::Object(obj) => match obj.borrow().payload {
                            ObjectPayload::Int(i) => i as i32,
                            _ => panic!("Expected Int"),
                        },
                        _ => panic!("Expected Int"),
                    };
                    args[0]
                        .with_native_state_mut::<MyCustomResource, _, _>(mc, |res| {
                            res.counter += val;
                        })
                        .unwrap();
                    Ok(args[0])
                });
            vm.register_native_class(mc, resource_builder);
            vm
        });

        arena.mutate_root(|mc, vm| {
            // Instantiate [IO]Resource via sending "create"
            let resource_class = vm.get_builtin_class("[IO]Resource");
            let instance = vm
                .call_method(mc, Value::Class(resource_class), "create", vec![])
                .unwrap();

            // Check counter is 10
            let counter_val = vm.call_method(mc, instance, "get", vec![]).unwrap();
            assert_eq!(to_spec(counter_val), ValueSpec::Int(10));

            // Increment by 5
            let five = vm.new_int(mc, 5);
            vm.call_method(mc, instance, "inc:", vec![five]).unwrap();

            // Check counter is 15
            let counter_val = vm.call_method(mc, instance, "get", vec![]).unwrap();
            assert_eq!(to_spec(counter_val), ValueSpec::Int(15));
        });
    }

    #[test]
    fn test_mixin_method_lookup_and_instance_vars() {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let vm = VmState::new(mc);
            vm
        });

        arena.mutate_root(|mc, vm| {
            // Define mixin class Point
            let point_class = gcl!(
                mc,
                Class {
                    name: NamespacedName::new(Vec::new(), "Point".to_string()),
                    parent: None,
                    instance_vars: vec!["x".to_string(), "y".to_string()],
                    instance_methods: {
                        let mut m = HashMap::new();
                        m.insert(
                            "name".to_string(),
                            vm.new_native(
                                mc,
                                NativeFunc::new(|vm, mc, _args| {
                                    Ok(vm.new_string(mc, "Point".to_string()))
                                }),
                            ),
                        );
                        m
                    },
                    class_methods: HashMap::new(),
                    mixin_classes: Vec::new(),
                }
            );

            // Define class PType which mixes in Point
            let ptype_class = gcl!(
                mc,
                Class {
                    name: NamespacedName::new(Vec::new(), "PType".to_string()),
                    parent: None,
                    instance_vars: vec!["z".to_string()],
                    instance_methods: HashMap::new(),
                    class_methods: HashMap::new(),
                    mixin_classes: vec![point_class],
                }
            );

            // Check instance variables (should contain z, x, y)
            let vars = vm.get_all_instance_vars(ptype_class);
            assert!(vars.contains(&"x".to_string()));
            assert!(vars.contains(&"y".to_string()));
            assert!(vars.contains(&"z".to_string()));

            // Instantiate PType
            let obj = vm.new_object(mc, ptype_class);

            // Look up "name" on PType instance -> should find Point's name method
            let _method = vm.lookup_method(Value::Object(obj), "name").unwrap();

            // Execute method
            let ret = vm
                .call_method(mc, Value::Object(obj), "name", vec![])
                .unwrap();
            assert_eq!(to_spec(ret), ValueSpec::String("Point".to_string()));
        });
    }

    #[test]
    fn test_execute_block_helper() {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let vm = VmState::new(mc);
            vm
        });

        arena.mutate_root(|mc, vm| {
            // Build a block that adds two arguments (a, b) and returns self + a + b
            let block = gc!(
                mc,
                Block { source_info: None, name: Some("test_block".to_string()),
                    is_nested_block: false,
                    param_names: vec!["a".to_string(), "b".to_string()],
                    bytecode: vec![
                        Instruction::LoadLocal("self".to_string()),
                        Instruction::LoadLocal("a".to_string()),
                        Instruction::Send("+".to_string(), 1),
                        Instruction::LoadLocal("b".to_string()),
                        Instruction::Send("+".to_string(), 1),
                        Instruction::Return,
                    ],
                    parent_env: None,
                    enclosing_method_id: None,
                }
            );

            // Register standard native functions we need (+ operator)
            let native_val = vm.new_native(mc, NativeFunc(native_add));
            vm.globals
                .borrow_mut(mc)
                .insert(NamespacedName::new(Vec::new(), "+".to_string()), native_val);

            // Execute block with args [10, 20] and self = 100
            let self_val = vm.new_int(mc, 100);
            let arg1 = vm.new_int(mc, 10);
            let arg2 = vm.new_int(mc, 20);

            let res = vm
                .execute_block(mc, block, vec![arg1, arg2], Some(self_val))
                .unwrap();

            assert_eq!(to_spec(res), ValueSpec::Int(130));

            // Execute block without self: a + b
            let block2 = gc!(
                mc,
                Block { source_info: None, name: Some("test_block_no_self".to_string()),
                    is_nested_block: false,
                    param_names: vec!["a".to_string(), "b".to_string()],
                    bytecode: vec![
                        Instruction::LoadLocal("a".to_string()),
                        Instruction::LoadLocal("b".to_string()),
                        Instruction::Send("+".to_string(), 1),
                        Instruction::Return,
                    ],
                    parent_env: None,
                    enclosing_method_id: None,
                }
            );

            let res2 = vm
                .execute_block(mc, block2, vec![arg1, arg2], None)
                .unwrap();

            assert_eq!(to_spec(res2), ValueSpec::Int(30));
        });
    }

    #[test]
    fn test_cannot_redefine_existing_class() {
        run_test_steps(
            vec![
                Instruction::DefineClass {
                    name: NamespacedName::new(Vec::new(), "Object".to_string()),
                    parent_name: None,
                    instance_vars: Vec::new(),
                },
            ],
            |vm, mc| {
                let res = vm.step(mc);
                assert!(res.is_err());
                let err_msg = format!("{}", res.err().unwrap());
                assert!(err_msg.contains("Cannot redefine class [/]Object because it already exists"));
            },
        );
    }

    #[test]
    fn test_cannot_extend_non_existent_class() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Nil),
                Instruction::Push(Constant::Block(StaticBlock { source_info: None,
                    name: Some("ext_block".to_string()),
                    is_nested_block: false,
                    param_names: Vec::new(),
                    bytecode: vec![Instruction::Push(Constant::Nil), Instruction::Return],
                })),
                Instruction::ExecuteBlockWithSelf,
            ],
            |vm, mc| {
                let res = vm.step(mc);
                assert!(res.is_ok());
                let res = vm.step(mc);
                assert!(res.is_ok());
                let res = vm.step(mc);
                assert!(res.is_err());
                let err_msg = format!("{}", res.err().unwrap());
                assert!(err_msg.contains("Cannot extend nil or non-existent class/object"));
            },
        );
    }

    #[test]
    fn test_short_circuit_and_or() {
        // Test: false && (panic!)
        run_test_steps(
            vec![
                Instruction::Push(Constant::Bool(false)),
                Instruction::Dup,
                Instruction::ElseJump(3),
                Instruction::Pop,
                Instruction::Push(Constant::Int(99)),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push false
                vm.step(mc).unwrap(); // Dup
                vm.step(mc).unwrap(); // ElseJump (should jump to end, i.e., index 5)
                assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(false)]);
                assert_eq!(vm.frames[0].ip, 5);
            },
        );

        // Test: true && 42
        run_test_steps(
            vec![
                Instruction::Push(Constant::Bool(true)),
                Instruction::Dup,
                Instruction::ElseJump(3),
                Instruction::Pop,
                Instruction::Push(Constant::Int(42)),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push true
                vm.step(mc).unwrap(); // Dup
                vm.step(mc).unwrap(); // ElseJump (should not jump, IP becomes 3)
                assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(true)]);
                assert_eq!(vm.frames[0].ip, 3);
                vm.step(mc).unwrap(); // Pop
                assert_eq!(vm.stack.len(), 0);
                vm.step(mc).unwrap(); // Push 42
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
            },
        );

        // Test: true || (panic!)
        run_test_steps(
            vec![
                Instruction::Push(Constant::Bool(true)),
                Instruction::Dup,
                Instruction::IfJump(3),
                Instruction::Pop,
                Instruction::Push(Constant::Int(99)),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push true
                vm.step(mc).unwrap(); // Dup
                vm.step(mc).unwrap(); // IfJump (should jump to end, i.e. index 5)
                assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(true)]);
                assert_eq!(vm.frames[0].ip, 5);
            },
        );

        // Test: false || 42
        run_test_steps(
            vec![
                Instruction::Push(Constant::Bool(false)),
                Instruction::Dup,
                Instruction::IfJump(3),
                Instruction::Pop,
                Instruction::Push(Constant::Int(42)),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push false
                vm.step(mc).unwrap(); // Dup
                vm.step(mc).unwrap(); // IfJump (should not jump, IP becomes 3)
                assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(false)]);
                assert_eq!(vm.frames[0].ip, 3);
                vm.step(mc).unwrap(); // Pop
                assert_eq!(vm.stack.len(), 0);
                vm.step(mc).unwrap(); // Push 42
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
            },
        );
    }

    #[test]
    fn test_error_annotation_and_display() {
        use crate::parser::parser::parse_building_blocks_string;
        use crate::compiler::Compiler;

        let code = "1.foo;";
        let ast = parse_building_blocks_string(code);
        let mut compiler = Compiler::new();
        let compiled = compiler.compile_program(match &ast.value {
            NodeValue::Program(p) => p,
            _ => unreachable!(),
        }).unwrap();

        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            VmState::new(mc)
        });

        arena.mutate_root(|mc, vm| {
            let block = gc!(
                mc,
                Block {
                    source_info: compiled.source_info.clone(), name: compiled.name.clone(),
                    is_nested_block: compiled.is_nested_block,
                    param_names: compiled.param_names.clone(),
                    bytecode: compiled.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                }
            );
            vm.start_block(mc, block, Vec::new());

            // Run until error. It should fail because Integer/Nil does not have 'foo' method.
            let mut err = None;
            loop {
                match vm.step(mc) {
                    Ok(VmStatus::Running) => {}
                    Ok(_) => break,
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                }
            }

            let err = err.expect("Expected execution error");
            let err_str = err.to_string();
            
            // Check that the error message displays the source information
            assert!(err_str.contains("at <string>:1:1"));
            assert!(err_str.contains("1.foo;"));
        });
    }
}

