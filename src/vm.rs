use crate::instruction::{Constant, Instruction};
use crate::value::{BBRegex, Block, Class, EnvFrame, NativeClass, Value};

use gc_arena::{lock::RefLock, Collect, Gc, Mutation};
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

impl<'gc> VmState<'gc> {
    pub fn new(mc: &Mutation<'gc>) -> Self {
        Self {
            stack: Vec::new(),
            frames: Vec::new(),
            globals: Gc::new(mc, RefLock::new(HashMap::new())),
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

        let class_obj = Gc::new(
            mc,
            RefLock::new(Class {
                name: native_class.name().to_string(),
                parent: parent_class,
                instance_methods: inst_methods,
                class_methods: cls_methods,
            }),
        );

        self.globals
            .borrow_mut(mc)
            .insert(native_class.name().to_string(), Value::Class(class_obj));
    }

    pub fn lookup_method(&self, receiver: Value<'gc>, selector: &str) -> Option<Value<'gc>> {
        match receiver {
            Value::Object(obj) => {
                let class_ref = obj.borrow().class;
                if let Some(m) = self.lookup_in_class_hierarchy(class_ref, selector, false) {
                    Some(m)
                } else {
                    self.globals.borrow().get(selector).copied()
                }
            }
            Value::Class(class_obj) => self.lookup_in_class_hierarchy(class_obj, selector, true),
            _ => {
                let type_name = receiver.type_name();
                if let Some(Value::Class(class_obj)) = self.globals.borrow().get(type_name).copied()
                {
                    if let Some(m) = self.lookup_in_class_hierarchy(class_obj, selector, false) {
                        return Some(m);
                    }
                }
                self.globals.borrow().get(selector).copied()
            }
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
        let env_ref = Gc::new(mc, RefLock::new(env_frame));

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
        });
    }

    pub fn step(&mut self, mc: &Mutation<'gc>) -> Result<VmStatus<'gc>, String> {
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
                    Constant::Float(f) => Value::Float(f),
                    Constant::String(s) => Value::String(Gc::new(mc, s.clone())),
                    Constant::Block(sb) => {
                        let parent_env = self.frames.last().map(|f| f.env);
                        let block = Gc::new(
                            mc,
                            Block {
                                name: sb.name.clone(),
                                is_nested_block: sb.is_nested_block,
                                param_names: sb.param_names.clone(),
                                bytecode: sb.bytecode.clone(),
                                parent_env,
                            },
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
            Instruction::Call(num_args) => {
                let mut args = Vec::new();
                for _ in 0..num_args {
                    args.push(self.pop()?);
                }
                args.reverse();

                let callable = self.pop()?;
                self.frames[frame_idx].ip += 1; // Advance caller frame IP

                match callable {
                    Value::Block(block) => {
                        self.start_block(mc, block, args);
                    }
                    Value::Method(method) => {
                        let mut method_args = vec![method.receiver];
                        method_args.extend(args);
                        self.start_block(mc, method.block, method_args);
                    }
                    Value::Native(native_fn) => {
                        let ret = native_fn.0(self, mc, args)?;
                        self.push(ret);
                    }
                    _ => return Err(format!("Value is not callable: {:?}", callable)),
                }
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
                if let Some(method_val) = method_opt {
                    match method_val {
                        Value::Native(native_fn) => {
                            let mut all_args = vec![receiver];
                            all_args.extend(args);
                            let ret = native_fn.0(self, mc, all_args)?;
                            self.push(ret);
                        }
                        Value::Block(block) => {
                            let mut all_args = vec![receiver];
                            all_args.extend(args);
                            self.start_block(mc, block, all_args);
                        }
                        _ => {
                            return Err(format!(
                                "Selector '{}' resolved to non-callable value: {:?}",
                                selector, method_val
                            ));
                        }
                    }
                } else {
                    return Err(format!(
                        "Message not understood: receiver={:?}, selector='{}', args={:?}",
                        receiver, selector, args
                    ));
                }
            }
            Instruction::Return | Instruction::BlockReturn => {
                let ret_val = self.pop()?;
                self.frames.pop();
                self.push(ret_val);
            }
            Instruction::MethodReturn => {
                let ret_val = self.pop()?;
                let enclosing_id = self.frames[frame_idx].enclosing_method_id;
                if let Some(target_id) = enclosing_id {
                    while let Some(f) = self.frames.pop() {
                        if f.id == target_id {
                            break;
                        }
                    }
                    self.push(ret_val);
                } else {
                    return Err("MethodReturn executed outside of a method context".to_string());
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
                let list = Gc::new(mc, RefLock::new(elements));
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
                        return Err(format!("Dict keys must be Strings, got: {:?}", key_val));
                    }
                }
                let dict = Gc::new(mc, RefLock::new(map));
                self.push(Value::Dict(dict));
                self.frames[frame_idx].ip += 1;
            }
            Instruction::NewRegex => {
                let pattern_val = self.pop()?;
                if let Value::String(s) = pattern_val {
                    let re =
                        regex::Regex::new(&**s).map_err(|e| format!("Invalid regex: {}", e))?;
                    let regex_val = Gc::new(mc, BBRegex(re));
                    self.push(Value::Regex(regex_val));
                } else {
                    return Err(format!(
                        "Regex pattern must be a String, got: {:?}",
                        pattern_val
                    ));
                }
                self.frames[frame_idx].ip += 1;
            }
        }

        Ok(VmStatus::Running)
    }
}
