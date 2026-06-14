use crate::error::BBError;
use crate::instruction::{Constant, Instruction};
use crate::value::{BBRegex, Block, Class, EnvFrame, NativeClass, NativeFunc, Object, Value};
use crate::{gc, gcl};

use gc_arena::{Collect, Gc, Mutation, lock::RefLock};
use std::collections::HashMap;

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
pub struct VmState<'gc> {
    pub stack: Vec<Value<'gc>>,
    pub frames: Vec<Frame<'gc>>,
    pub globals: Gc<'gc, RefLock<HashMap<String, Value<'gc>>>>,
    pub next_frame_id: usize,
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
        let block = match args[1] {
            Value::Block(b) => b,
            _ => {
                return Err(BBError::TypeError {
                    expected: "Block".to_string(),
                    got: args[1].type_name().to_string(),
                    msg: "new: expects a Block".to_string(),
                });
            }
        };

        // Create the new object
        let mut fields = HashMap::new();
        for var in &self.class_obj.borrow().instance_vars {
            fields.insert(var.clone(), Value::Nil);
        }
        let obj = gcl!(
            mc,
            Object {
                class: self.class_obj,
                fields,
            }
        );

        vm.start_block_for_instantiation(mc, block, obj);
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
        }
    }

    pub fn register_native_class<T: NativeClass>(&mut self, mc: &Mutation<'gc>, native_class: T) {
        let parent_class = if let Some(parent_name) = native_class.parent_name()
            && let Some(parent_value) = self.globals.borrow().get(parent_name).copied()
            && let Value::Class(parent_class) = parent_value
        {
            Some(parent_class)
        } else {
            None
        };

        let mut inst_methods = HashMap::new();
        for (name, func) in native_class.instance_methods() {
            inst_methods.insert(name, Value::Native(func));
        }

        let mut cls_methods = HashMap::new();
        for (name, func) in native_class.class_methods() {
            cls_methods.insert(name, Value::Native(func));
        }

        let class_obj = gcl!(
            mc,
            Class {
                name: native_class.name().to_string(),
                parent: parent_class,
                instance_vars: Vec::new(),
                instance_methods: inst_methods,
                class_methods: cls_methods,
            }
        );

        self.globals
            .borrow_mut(mc)
            .insert(native_class.name().to_string(), Value::Class(class_obj));
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
            method.call(self, mc, all_args)?;
            Ok(self.pop()?)
        } else {
            Ok(Value::Nil)
        }
    }

    //noinspection DuplicatedCode
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
        let method_val = match receiver {
            Value::Object(obj) => {
                let class_ref = obj.borrow().class;
                if let Some(m) = self.lookup_in_class_hierarchy(class_ref, selector, false) {
                    Some(m)
                } else {
                    self.globals.borrow().get(selector).copied()
                }
            }
            Value::Class(class_obj) => self.lookup_in_class_hierarchy(class_obj, selector, true),
            Value::ClassMeta(class_obj) => {
                self.lookup_in_class_hierarchy(class_obj, selector, true)
            }
            _ => {
                let type_name = receiver.type_name();
                if let Some(Value::Class(class_obj)) = self.globals.borrow().get(type_name).copied()
                {
                    if let Some(m) = self.lookup_in_class_hierarchy(class_obj, selector, false) {
                        Some(m)
                    } else {
                        self.globals.borrow().get(selector).copied()
                    }
                } else {
                    self.globals.borrow().get(selector).copied()
                }
            }
        }?;

        match method_val {
            Value::Block(block) => Some(Box::new(BlockCallable { block })),
            Value::Native(native_fn) => Some(Box::new(NativeCallable(native_fn))),
            _ => None,
        }
    }

    fn lookup_in_class_hierarchy(
        &self,
        mut class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        selector: &str,
        class_side: bool,
    ) -> Option<Value<'gc>> {
        loop {
            let class_borrow = class_ref.borrow();
            let methods = if class_side {
                &class_borrow.class_methods
            } else {
                &class_borrow.instance_methods
            };
            if let Some(method) = methods.get(selector).copied() {
                return Some(method);
            }
            if let Some(parent) = class_borrow.parent {
                class_ref = parent;
            } else {
                break;
            }
        }
        None
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
            self.frames.last().and_then(|f| f.enclosing_method_id)
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
        let enclosing_method_id = if is_nested_block {
            self.frames.last().and_then(|f| f.enclosing_method_id)
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

    pub fn start_block_for_instantiation(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        obj: Gc<'gc, RefLock<Object<'gc>>>,
    ) {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let mut env_frame = EnvFrame::new(block.parent_env);
        // Bind self to the object
        env_frame
            .vars
            .insert("self".to_string(), Value::Object(obj));
        // Bind all instance variables as local variables in this block, initialized from the parent env or current fields
        for var in &obj.borrow().class.borrow().instance_vars {
            let val = if let Some(parent) = block.parent_env {
                EnvFrame::get(parent, var)
                    .unwrap_or_else(|| obj.borrow().fields.get(var).copied().unwrap_or(Value::Nil))
            } else {
                obj.borrow().fields.get(var).copied().unwrap_or(Value::Nil)
            };
            env_frame.vars.insert(var.clone(), val);
        }
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.is_nested_block;
        let enclosing_method_id = if is_nested_block {
            self.frames.last().and_then(|f| f.enclosing_method_id)
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

    fn get_target_class_for_def(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<Gc<'gc, RefLock<Class<'gc>>>, String> {
        match receiver {
            Value::Class(c) => Ok(c),
            Value::ClassMeta(c) => Ok(c),
            Value::Object(obj) => {
                let class_ref = obj.borrow().class;
                if class_ref.borrow().name.starts_with('$') {
                    Ok(class_ref)
                } else {
                    let singleton_name = format!("${}", class_ref.borrow().name);
                    let s = gcl!(
                        mc,
                        Class {
                            name: singleton_name,
                            parent: Some(class_ref),
                            instance_vars: Vec::new(),
                            instance_methods: HashMap::new(),
                            class_methods: HashMap::new(),
                        }
                    );
                    obj.borrow_mut(mc).class = s;
                    Ok(s)
                }
            }
            Value::Bool(b) => {
                let name = if b { "TrueClass" } else { "FalseClass" };
                self.get_or_create_global_class(mc, name, "Boolean")
            }
            Value::Nil => self.get_or_create_global_class(mc, "Nil", "Object"),
            _ => {
                let name = receiver.type_name();
                self.get_or_create_global_class(mc, name, "Object")
            }
        }
    }

    fn get_or_create_global_class(
        &mut self,
        mc: &Mutation<'gc>,
        name: &str,
        default_parent: &str,
    ) -> Result<Gc<'gc, RefLock<Class<'gc>>>, String> {
        if let Some(Value::Class(c)) = self.globals.borrow().get(name).copied() {
            Ok(c)
        } else {
            let parent = self
                .globals
                .borrow()
                .get(default_parent)
                .copied()
                .and_then(|v| {
                    if let Value::Class(c) = v {
                        Some(c)
                    } else {
                        None
                    }
                });
            let c = gcl!(
                mc,
                Class {
                    name: name.to_string(),
                    parent,
                    instance_vars: Vec::new(),
                    instance_methods: HashMap::new(),
                    class_methods: HashMap::new(),
                }
            );
            self.globals
                .borrow_mut(mc)
                .insert(name.to_string(), Value::Class(c));
            Ok(c)
        }
    }

    pub fn step(&mut self, mc: &Mutation<'gc>) -> Result<VmStatus<'gc>, BBError> {
        if self.frames.is_empty() {
            let ret = self.pop().unwrap_or(Value::Nil);
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
                let ret_val = Value::Nil;
                self.frames.pop();
                self.push(ret_val);
                return Ok(VmStatus::Running);
            }
        };

        match inst {
            Instruction::LoadLocal(name) => {
                let frame = &self.frames[frame_idx];
                let val = EnvFrame::get(frame.env, &name).unwrap_or(Value::Nil);
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
                    .unwrap_or(Value::Nil);
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
                    Constant::Nil => Value::Nil,
                    Constant::Bool(b) => Value::Bool(b),
                    Constant::Int(i) => Value::Int(i),
                    Constant::Double(f) => Value::Double(f),
                    Constant::String(s) => Value::String(gc!(mc, s.clone())),
                    Constant::Block(sb) => {
                        let parent_env = self.frames.last().map(|f| f.env);
                        let block = gc!(
                            mc,
                            Block {
                                name: sb.name.clone(),
                                is_nested_block: sb.is_nested_block,
                                param_names: sb.param_names.clone(),
                                bytecode: sb.bytecode.clone(),
                                parent_env,
                            }
                        );
                        Value::Block(block)
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

                if let Value::Block(block) = receiver {
                    if selector == "value" || selector == "value:" || selector.starts_with("value:")
                    {
                        self.start_block(mc, block, args);
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
                    let vars = obj.borrow().class.borrow().instance_vars.clone();
                    for var in &vars {
                        if let Some(val) = env_borrow.vars.get(var) {
                            obj.borrow_mut(mc).fields.insert(var.clone(), *val);
                        }
                    }
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
                            let vars = obj.borrow().class.borrow().instance_vars.clone();
                            for var in &vars {
                                if let Some(val) = env_borrow.vars.get(var) {
                                    obj.borrow_mut(mc).fields.insert(var.clone(), *val);
                                }
                            }
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
                let list = gcl!(mc, elements);
                self.push(Value::List(list));
                self.frames[frame_idx].ip += 1;
            }
            Instruction::NewDict(n) => {
                let mut map = HashMap::new();
                for _ in 0..n {
                    let val = self.pop()?;
                    let key_val = self.pop()?;
                    if let Value::String(s) = key_val {
                        map.insert((*s).clone(), val);
                    } else {
                        return Err(BBError::TypeError {
                            expected: "String".to_string(),
                            got: key_val.type_name().to_string(),
                            msg: format!("Dict keys must be Strings, got: {:?}", key_val),
                        });
                    }
                }
                let dict = gcl!(mc, map);
                self.push(Value::Dict(dict));
                self.frames[frame_idx].ip += 1;
            }
            Instruction::NewRegex => {
                let pattern_val = self.pop()?;
                if let Value::String(s) = pattern_val {
                    let re =
                        regex::Regex::new(&**s).map_err(|e| format!("Invalid regex: {}", e))?;
                    let regex_val = gc!(mc, BBRegex(re));
                    self.push(Value::Regex(regex_val));
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
                    if name != "Object" {
                        if let Some(Value::Class(obj_class)) =
                            self.globals.borrow().get("Object").copied()
                        {
                            Some(obj_class)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                let class_obj = gcl!(
                    mc,
                    Class {
                        name: name.clone(),
                        parent,
                        instance_vars: instance_vars.clone(),
                        instance_methods: HashMap::new(),
                        class_methods: HashMap::new(),
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
                if let Value::Block(block) = block_val {
                    self.frames[frame_idx].ip += 1;
                    self.start_block_as_method(mc, block, self_val, Vec::new());
                    return Ok(VmStatus::Running);
                } else {
                    return Err(BBError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: format!("ExecuteBlockWithSelf expects a Block, got {:?}", block_val),
                    });
                }
            }
            Instruction::DefineMethod(selector) => {
                let block_val = self.pop()?;
                if let Value::Block(block) = block_val {
                    let self_val =
                        EnvFrame::get(self.frames[frame_idx].env, "self").unwrap_or(Value::Nil);
                    let target_class = self
                        .get_target_class_for_def(mc, self_val)
                        .map_err(|e| BBError::Other(e))?;

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
                            .insert(selector, Value::Block(block));
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
                            .insert(selector, Value::Block(block));
                    }
                    self.push(Value::Nil);
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
                if let Value::Block(block) = block_val {
                    let self_val =
                        EnvFrame::get(self.frames[frame_idx].env, "self").unwrap_or(Value::Nil);
                    let target_class = self
                        .get_target_class_for_def(mc, self_val)
                        .map_err(|e| BBError::Other(e))?;

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
                            .insert(selector, Value::Block(block));
                    } else {
                        target_class
                            .borrow_mut(mc)
                            .instance_methods
                            .insert(selector, Value::Block(block));
                    }
                    self.push(Value::Nil);
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
                let self_val = EnvFrame::get(frame.env, "self").unwrap_or(Value::Nil);
                let val = if let Value::Object(obj) = self_val {
                    obj.borrow().get_field_or_default(&name)
                } else {
                    Value::Nil
                };
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::StoreField(name) => {
                let val = self.pop()?;
                let frame = &self.frames[frame_idx];
                let self_val = EnvFrame::get(frame.env, "self").unwrap_or(Value::Nil);
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
    use crate::value::NativeClassBuilder;
    use gc_arena::{Arena, Rootable};

    fn native_add<'gc>(
        _vm: &mut VmState<'gc>,
        _mc: &Mutation<'gc>,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, BBError> {
        match (&args[0], &args[1]) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
            _ => Err(BBError::Other("Invalid types".to_string())),
        }
    }

    fn run_test_steps<F>(instructions: Vec<Instruction>, check_steps: F)
    where
        F: for<'gc> FnOnce(&mut VmState<'gc>, &Mutation<'gc>),
    {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc);

            // Register standard native functions we might need
            {
                let mut globals = vm.globals.borrow_mut(mc);
                globals.insert("+".to_string(), Value::Native(NativeFunc(native_add)));
            }

            // Register some basic native classes
            vm.register_native_class(mc, NativeClassBuilder::new("Object", None));

            let static_block = StaticBlock {
                name: Some("test_main".to_string()),
                is_nested_block: false,
                param_names: Vec::new(),
                bytecode: instructions,
            };
            let block = gc!(
                mc,
                Block {
                    name: static_block.name.clone(),
                    is_nested_block: static_block.is_nested_block,
                    param_names: static_block.param_names.clone(),
                    bytecode: static_block.bytecode.clone(),
                    parent_env: None,
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
                assert!(matches!(status, VmStatus::Running));
                assert_eq!(vm.stack, vec![Value::Int(10)]);

                // Step 2: Push(20)
                let status = vm.step(mc).unwrap();
                assert!(matches!(status, VmStatus::Running));
                assert_eq!(vm.stack, vec![Value::Int(10), Value::Int(20)]);

                // Step 3: Pop
                let status = vm.step(mc).unwrap();
                assert!(matches!(status, VmStatus::Running));
                assert_eq!(vm.stack, vec![Value::Int(10)]);

                // Step 4: Dup
                let status = vm.step(mc).unwrap();
                assert!(matches!(status, VmStatus::Running));
                assert_eq!(vm.stack, vec![Value::Int(10), Value::Int(10)]);

                // Step 5: Implicit return Nil
                let status = vm.step(mc).unwrap();
                assert!(matches!(status, VmStatus::Running));
                assert_eq!(vm.stack, vec![Value::Int(10), Value::Int(10), Value::Nil]);

                // Step 6: Finished
                let status = vm.step(mc).unwrap();
                assert!(matches!(status, VmStatus::Finished(Value::Nil)));
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
                assert_eq!(vm.stack, vec![Value::Int(42)]);

                // Step 2: DefineLocal("a") -> []
                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 0);

                // Step 3: LoadLocal("a") -> [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(42)]);

                // Step 4: Push(100) -> [Int(42), Int(100)]
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(42), Value::Int(100)]);

                // Step 5: StoreLocal("a") -> [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(42)]);

                // Step 6: LoadLocal("a") -> [Int(42), Int(100)]
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(42), Value::Int(100)]);
            },
        );
    }

    #[test]
    fn test_global_variables() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Int(77)),
                Instruction::StoreGlobal("g_var".to_string()),
                Instruction::LoadGlobal("g_var".to_string()),
            ],
            |vm, mc| {
                // Step 1: Push(77)
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(77)]);

                // Step 2: StoreGlobal("g_var")
                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 0);

                // Step 3: LoadGlobal("g_var")
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(77)]);
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
                assert_eq!(vm.stack, vec![Value::Nil]);

                // Bool
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Nil, Value::Bool(true)]);

                // Float
                vm.step(mc).unwrap();
                assert_eq!(
                    vm.stack,
                    vec![Value::Nil, Value::Bool(true), Value::Double(3.14)]
                );

                // String
                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 4);
                if let Value::String(s) = vm.stack[3] {
                    assert_eq!(**s, "hello".to_string());
                } else {
                    panic!("Expected string value");
                }
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
                assert_eq!(vm.stack, vec![Value::Bool(true)]);

                // IfJump(3) -> condition true -> jump to index 4 (Push 42). Stack becomes []
                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 0);
                assert_eq!(vm.frames[0].ip, 4);

                // Push 42 -> [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(42)]);

                // Push false -> [Int(42), Bool(false)]
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(42), Value::Bool(false)]);

                // ElseJump(3) -> condition false -> jump to index 9 (Push 55). Stack becomes [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(42)]);
                assert_eq!(vm.frames[0].ip, 9);

                // Push 55 -> [Int(42), Int(55)]
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(42), Value::Int(55)]);
            },
        );
    }

    #[test]
    fn test_list_dict_regex() {
        run_test_steps(
            vec![
                // List of 2 elements: Push 1, Push 2, NewList(2)
                Instruction::Push(Constant::Int(1)),
                Instruction::Push(Constant::Int(2)),
                Instruction::NewList(2),
                // Dict of 1 pair: Push key "a", Push val 10, NewDict(1)
                Instruction::Push(Constant::String("a".to_string())),
                Instruction::Push(Constant::Int(10)),
                Instruction::NewDict(1),
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
                if let Value::List(list) = vm.stack[0] {
                    assert_eq!(*list.borrow(), vec![Value::Int(1), Value::Int(2)]);
                } else {
                    panic!("Expected List");
                }

                // Dict creation
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                vm.step(mc).unwrap(); // NewDict(1)
                assert_eq!(vm.stack.len(), 2);
                if let Value::Dict(dict) = vm.stack[1] {
                    assert_eq!(dict.borrow().get("a").copied(), Some(Value::Int(10)));
                } else {
                    panic!("Expected Dict");
                }

                // Regex creation
                vm.step(mc).unwrap();
                vm.step(mc).unwrap(); // NewRegex
                assert_eq!(vm.stack.len(), 3);
                if let Value::Regex(re) = vm.stack[2] {
                    assert!(re.0.is_match("ab"));
                    assert!(!re.0.is_match("abc"));
                } else {
                    panic!("Expected Regex");
                }
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
                assert_eq!(vm.stack, vec![Value::Int(15)]);
            },
        );
    }

    #[test]
    fn test_block_execution_and_returns() {
        // We will push a block constant, then send "value" to it.
        // The block bytecode will load its parameter, add 1 to it, and return.
        let block_static = StaticBlock {
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
                assert_eq!(vm.stack, vec![Value::Int(41)]);

                // Inside block: Push(1) -> [41, 1]
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(41), Value::Int(1)]);

                // Inside block: Send("+", 1) -> [42]
                vm.step(mc).unwrap();
                assert_eq!(vm.stack, vec![Value::Int(42)]);

                // Inside block: Return -> pops block frame, leaves return value on stack -> [42]
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 1);
                assert_eq!(vm.stack, vec![Value::Int(42)]);
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
                assert!(matches!(status, VmStatus::Yeeted(Value::Int(500))));
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
        let block_nested = StaticBlock {
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
        let block_method = StaticBlock {
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
                assert_eq!(vm.stack, vec![Value::Int(999)]);
            },
        );
    }

    #[test]
    fn test_class_and_method_definition_vm() {
        let class_block = StaticBlock {
            name: Some("class_block".to_string()),
            is_nested_block: false,
            param_names: Vec::new(),
            bytecode: vec![
                // 1. Define inst method x
                Instruction::Push(Constant::Block(StaticBlock {
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
                Instruction::Push(Constant::Block(StaticBlock {
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
                    name: "Point".to_string(),
                    parent_name: None,
                    instance_vars: vec!["x".to_string(), "y".to_string()],
                },
                // Push class block
                Instruction::Push(Constant::Block(class_block)),
                // Execute block with Point as self
                Instruction::ExecuteBlockWithSelf,
                // Send "meta" to Point
                Instruction::LoadGlobal("Point".to_string()),
                Instruction::Send("meta".to_string(), 0),
            ],
            |vm, mc| {
                // Step DefineClass
                vm.step(mc).unwrap();
                let class_val = vm.peek().unwrap();
                if let Value::Class(c) = class_val {
                    assert_eq!(c.borrow().name, "Point");
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
                    assert_eq!(c.borrow().name, "Point");
                } else {
                    panic!("Expected ClassMeta, got {:?}", meta_val);
                }
            },
        );
    }
}
