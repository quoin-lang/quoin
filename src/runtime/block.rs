use crate::arg;
use crate::error::QuoinError;
use crate::recv;
use crate::runtime::list::NativeListState;
use crate::value::{NativeClassBuilder, Value};

pub fn build_block_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Block", Some("Object"))
        .instance_method("arity", |vm, mc, receiver, _args| {
            let block = recv!(receiver, Block);
            Ok(vm.new_int(mc, block.param_syms.len() as i64))
        })
        .instance_method("args", |vm, mc, receiver, _args| {
            let block = recv!(receiver, Block);
            Ok(vm.new_list(
                mc,
                block
                    .param_syms
                    .iter()
                    .map(|s| vm.new_string(mc, s.as_str().to_string()))
                    .collect(),
            ))
        })
        .instance_method("name", |vm, mc, receiver, _args| {
            let block = recv!(receiver, Block);
            if let Some(name) = &block.name {
                Ok(vm.new_symbol(mc, name.clone()))
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        .instance_method("code", |vm, mc, receiver, _args| {
            let block = recv!(receiver, Block);
            if let Some(source_info) = &block.source_info
                && let Some(text) = &source_info.source_text
            {
                Ok(vm.new_string(mc, text.clone()))
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        // source -> #( filenameStr lineInt columnInt ) for where this block was
        // defined, or nil if the block carries no source info. `line` is 1-indexed and
        // `column` is 0-indexed (the raw `SourceInfo` convention). Used by the test
        // reporter to point a failed assertion at its source location.
        .instance_method("source", |vm, mc, receiver, _args| {
            let block = recv!(receiver, Block);
            if let Some(si) = &block.source_info {
                let file = vm.new_string(mc, si.filename.clone());
                let line = vm.new_int(mc, si.line as i64);
                let column = vm.new_int(mc, si.column as i64);
                Ok(vm.new_list(mc, vec![file, line, column]))
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        // .instance_method("value", |vm, mc, receiver, args| {
        //     let block = recv!(receiver, Block);
        //     vm.execute_block(mc, block, Vec::new(), None)
        // })
        // .instance_method("value:", |vm, mc, receiver, args| {
        //     let block = recv!(receiver, Block);
        //     let val = args[0];
        //     vm.execute_block(mc, block, vec![val], None)
        // })
        .instance_method("valueWithArgs:", |vm, mc, receiver, args| {
            let block = recv!(receiver, Block);
            let block_args =
                args[0].with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())?;
            vm.execute_block(mc, block, block_args, None)
        })
        .instance_method("valueWithSelf:", |vm, mc, receiver, args| {
            let block = recv!(receiver, Block);
            let self_val = args[0];
            vm.execute_block(mc, block, Vec::new(), Some(self_val))
        })
        .instance_method("value:withSelf:", |vm, mc, receiver, args| {
            let block = recv!(receiver, Block);
            let arg_val = args[0];
            let self_val = args[1];
            let block_args = match arg_val
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
            {
                Ok(vec) => vec,
                Err(_) => vec![arg_val],
            };
            vm.execute_block(mc, block, block_args, Some(self_val))
        })
        .instance_method("valueWithSelfOrArg:", |vm, mc, receiver, args| {
            let block = recv!(receiver, Block);
            let arg_val = args[0];
            vm.execute_block(mc, block, vec![arg_val], Some(arg_val))
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            Ok(vm.new_bool(mc, receiver == args[0]))
        })
        .instance_method("catch:", |vm, mc, receiver, _args| {
            let receiver_block = recv!(receiver, Block);

            let initial_frame_count = vm.frames.len();
            let res = vm.execute_block(mc, receiver_block, Vec::new(), None);
            match res {
                Ok(val) => Ok(val),
                // Cancellation is not catchable: unwind frames and re-propagate so the
                // task still cancels (a `catch:` cannot swallow it).
                Err(QuoinError::Cancelled) => {
                    while vm.frames.len() > initial_frame_count {
                        vm.frames.pop();
                    }
                    Err(QuoinError::Cancelled)
                }
                Err(e) => {
                    // Break-on-throw: pause here (throw-site frames still live) if a debug
                    // session is watching for this error's type, before the frames are torn down.
                    if vm.has_break_on_throw() {
                        vm.debug_check_throw(mc, &e);
                    }
                    let active_args = &vm.active_native_args.last().unwrap().args;
                    let catch_block = arg!(active_args, Block, 0);
                    while vm.frames.len() > initial_frame_count {
                        vm.frames.pop();
                    }
                    let exception_val = if let Some(val) = vm.active_exception.take() {
                        val
                    } else {
                        vm.quoinerror_to_value(mc, &e)
                    };

                    vm.execute_block(mc, catch_block, vec![exception_val], None)
                }
            }
        })
        .instance_method("catch:finally:", |vm, mc, receiver, _args| {
            let receiver_block = recv!(receiver, Block);

            let initial_frame_count = vm.frames.len();
            let res = vm.execute_block(mc, receiver_block, Vec::new(), None);
            match res {
                Ok(val) => {
                    vm.push(val);
                    let active_args = &vm.active_native_args.last().unwrap().args;
                    let finally_block = arg!(active_args, Block, 1);
                    let finally_res = vm.execute_block(mc, finally_block, Vec::new(), None);
                    let val = vm.pop()?;
                    finally_res.map(|_| val)
                }
                // Cancellation runs `finally` (its always-runs guarantee holds) but is
                // not caught — re-propagate so the task still cancels. Cancellation wins
                // over any error the `finally` block itself raises.
                Err(QuoinError::Cancelled) => {
                    while vm.frames.len() > initial_frame_count {
                        vm.frames.pop();
                    }
                    let active_args = &vm.active_native_args.last().unwrap().args;
                    let finally_block = arg!(active_args, Block, 1);
                    let _ = vm.execute_block(mc, finally_block, Vec::new(), None);
                    while vm.frames.len() > initial_frame_count {
                        vm.frames.pop();
                    }
                    Err(QuoinError::Cancelled)
                }
                Err(e) => {
                    // Break-on-throw before the throw-site frames are torn down (see `catch:`).
                    if vm.has_break_on_throw() {
                        vm.debug_check_throw(mc, &e);
                    }
                    while vm.frames.len() > initial_frame_count {
                        vm.frames.pop();
                    }
                    let exception_val = if let Some(val) = vm.active_exception.take() {
                        val
                    } else {
                        vm.quoinerror_to_value(mc, &e)
                    };

                    let active_args = &vm.active_native_args.last().unwrap().args;
                    let catch_block = arg!(active_args, Block, 0);
                    let catch_res = vm.execute_block(mc, catch_block, vec![exception_val], None);
                    while vm.frames.len() > initial_frame_count {
                        vm.frames.pop();
                    }

                    // Root catch_res if Ok using stack
                    let catch_res_err = match catch_res {
                        Ok(val) => {
                            vm.push(val);
                            Ok(())
                        }
                        Err(err) => Err(err),
                    };

                    let active_args = &vm.active_native_args.last().unwrap().args;
                    let finally_block = arg!(active_args, Block, 1);
                    let finally_res = vm.execute_block(mc, finally_block, Vec::new(), None);

                    let catch_res = match catch_res_err {
                        Ok(()) => {
                            let val = vm.pop()?;
                            if finally_res.is_err() {
                                finally_res.map(|_| val)
                            } else {
                                Ok(val)
                            }
                        }
                        Err(err) => {
                            if finally_res.is_err() {
                                finally_res.map(|_| vm.new_nil(mc))
                            } else {
                                Err(err)
                            }
                        }
                    };
                    catch_res
                }
            }
        })
}
