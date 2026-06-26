use crate::arg;
use crate::error::QuoinError;
use crate::recv;
use crate::runtime::list::NativeListState;
use crate::value::{Block, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmState;

use gc_arena::{Gc, Mutation};

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
        // `{…}.catch:{|e| …}` — run the protected block; on a throw, the handler runs if its
        // declared exception type matches (an untyped `|e|` catches everything), else the error
        // re-raises to an enclosing `catch:`.
        .instance_method("catch:", |vm, mc, receiver, args| {
            let protected = recv!(receiver, Block);
            do_catch(vm, mc, protected, &[args[0]])
        })
        // Variadic multi-catch: `{…}.catch:{|e:A| …} catch:{|e:B| …}` folds the run of `catch:`
        // keywords into one List of handler blocks (see the variadic-selector machinery). Each is
        // tried in source order by its declared exception type; first match wins.
        .instance_method("catch+:", |vm, mc, receiver, args| {
            let protected = recv!(receiver, Block);
            let handlers =
                args[0].with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())?;
            do_catch(vm, mc, protected, &handlers)
        })
        .instance_method("catch:finally:", |vm, mc, receiver, args| {
            let protected = recv!(receiver, Block);
            let finally = arg!(args, Block, 1);
            do_catch_finally(vm, mc, protected, &[args[0]], finally)
        })
        .instance_method("catch+:finally:", |vm, mc, receiver, args| {
            let protected = recv!(receiver, Block);
            let handlers =
                args[0].with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())?;
            let finally = arg!(args, Block, 1);
            do_catch_finally(vm, mc, protected, &handlers, finally)
        })
}

/// Pop frames back to `initial` — unwind a partially-executed block after a throw.
fn unwind(vm: &mut VmState<'_>, initial: usize) {
    while vm.frames.len() > initial {
        vm.frames.pop();
    }
}

/// The value to hand a catch handler for error `e`: a user `throw` already parked it in
/// `active_exception`; a structured (Rust-raised) error is materialized to its typed `Error`
/// object. Does NOT clear `active_exception`, so a non-matching typed catch can re-raise it intact.
fn exception_value<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>, e: &QuoinError) -> Value<'gc> {
    match vm.active_exception {
        Some(v) => v,
        None => vm.quoinerror_to_value(mc, e),
    }
}

/// A catch handler's block and the declared type of its exception parameter (`|e:IoError|` ->
/// `Some("IoError")`); `Some("Object")` for an untyped `|e|` and `None` for a zero-param `{…}` —
/// both catch-all. Returns `None` when the value isn't a block.
fn handler_parts<'gc>(v: Value<'gc>) -> Option<(Gc<'gc, Block<'gc>>, Option<String>)> {
    if let Value::Object(o) = v
        && let ObjectPayload::Block(blk) = &o.borrow().payload
    {
        return Some((*blk, blk.param_types.first().cloned()));
    }
    None
}

/// Run the first handler (source order) whose declared type catches `exc`, clearing the in-flight
/// exception first; returns its result. `None` => no handler matched, so the caller re-raises
/// (leaving `active_exception` intact for an enclosing `catch:`).
///
/// Handlers are tried **first-match**, so they must be written most- to least-specific (an untyped
/// `|e|`/`Object` or zero-param handler is a catch-all — put it LAST). This is a deliberate
/// exception to Quoin's order-independent, specificity-scored multimethod dispatch: a handler's
/// type lives on a runtime block value, not a scored method chain, so there is no specificity
/// order to fall back on — a broad handler placed first silently shadows the narrower ones below.
fn run_first_matching<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    exc: Value<'gc>,
    handlers: &[Value<'gc>],
) -> Option<Result<Value<'gc>, QuoinError>> {
    for &handler in handlers {
        let Some((block, declared)) = handler_parts(handler) else {
            continue;
        };
        let catches = match declared.as_deref() {
            None | Some("Object") => true,
            Some(t) => vm.value_matches_type(exc, t),
        };
        if catches {
            vm.active_exception = None;
            return Some(vm.execute_block(mc, block, vec![exc], None));
        }
    }
    None
}

/// `{protected}.catch:…` / `catch+:` core: run `protected`; on a catchable throw, dispatch to the
/// first matching handler, else re-raise. Cancellation is never caught.
fn do_catch<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    protected: Gc<'gc, Block<'gc>>,
    handlers: &[Value<'gc>],
) -> Result<Value<'gc>, QuoinError> {
    let initial = vm.frames.len();
    match vm.execute_block(mc, protected, Vec::new(), None) {
        Ok(val) => Ok(val),
        Err(QuoinError::Cancelled) => {
            unwind(vm, initial);
            Err(QuoinError::Cancelled)
        }
        Err(e) => {
            // Break-on-throw: pause with the throw-site frames still live, before unwinding.
            if vm.has_break_on_throw() {
                vm.debug_check_throw(mc, &e);
            }
            unwind(vm, initial);
            let exc = exception_value(vm, mc, &e);
            let res = match run_first_matching(vm, mc, exc, handlers) {
                Some(r) => r,
                None => Err(e), // no handler's type matched -> re-raise
            };
            unwind(vm, initial);
            res
        }
    }
}

/// `catch:finally:` / `catch+:finally:` core: like [`do_catch`], but `finally` always runs (on
/// success, on a caught or re-raised throw, and on cancellation), and a `finally` error overrides
/// the result it runs after.
fn do_catch_finally<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    protected: Gc<'gc, Block<'gc>>,
    handlers: &[Value<'gc>],
    finally: Gc<'gc, Block<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    let initial = vm.frames.len();
    match vm.execute_block(mc, protected, Vec::new(), None) {
        Ok(val) => {
            vm.push(val);
            let finally_res = vm.execute_block(mc, finally, Vec::new(), None);
            let val = vm.pop()?;
            finally_res.map(|_| val)
        }
        // Cancellation runs `finally` but is not caught (cancellation wins over a finally error).
        Err(QuoinError::Cancelled) => {
            unwind(vm, initial);
            let _ = vm.execute_block(mc, finally, Vec::new(), None);
            unwind(vm, initial);
            Err(QuoinError::Cancelled)
        }
        Err(e) => {
            if vm.has_break_on_throw() {
                vm.debug_check_throw(mc, &e);
            }
            unwind(vm, initial);
            let exc = exception_value(vm, mc, &e);
            let catch_res = match run_first_matching(vm, mc, exc, handlers) {
                Some(r) => r,
                None => Err(e),
            };
            unwind(vm, initial);
            // Root an Ok result across the finally run (which may allocate / GC).
            let catch_ok = match catch_res {
                Ok(val) => {
                    vm.push(val);
                    Ok(())
                }
                Err(err) => Err(err),
            };
            let finally_res = vm.execute_block(mc, finally, Vec::new(), None);
            match catch_ok {
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
            }
        }
    }
}
