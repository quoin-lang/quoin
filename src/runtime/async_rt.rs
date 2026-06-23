use crate::arg;
use crate::error::QuoinError;
use crate::runtime::list::NativeListState;
use crate::value::{Block, NativeClassBuilder, ObjectPayload, Value};

use gc_arena::Gc;

/// The `Async` class — Stage 2a structured concurrency. `gather:` is the only
/// primitive for now: it runs a list of zero-arg blocks as concurrent tasks whose
/// I/O overlaps, and returns their results as a list once all complete. The
/// underlying machinery is distinct from guest `Fiber`s (asymmetric coroutines);
/// the surfaced API will be revisited as it grows. See `docs/ASYNC_ARCH.md`.
pub fn build_async_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Async", Some("Object"))
        // `Async.gather:[ {…}, {…} ] -> list` — spawn one task per block, overlap
        // their I/O, and return the results in order. Propagates the first error.
        .class_method("gather:", |vm, _mc, _receiver, args| {
            let list_val = *args
                .first()
                .ok_or_else(|| QuoinError::Other("Async.gather: expects a list".to_string()))?;
            let elems = list_val
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
                .map_err(|_| QuoinError::TypeError {
                    expected: "List".to_string(),
                    got: list_val.type_name().to_string(),
                    msg: "Async.gather: expects a list of blocks".to_string(),
                })?;
            let mut blocks: Vec<Gc<Block>> = Vec::with_capacity(elems.len());
            for el in &elems {
                match el {
                    Value::Object(obj) => match &obj.borrow().payload {
                        ObjectPayload::Block(b) => blocks.push(*b),
                        _ => return Err(gather_elem_type_error(el)),
                    },
                    _ => return Err(gather_elem_type_error(el)),
                }
            }
            vm.await_gather(blocks)
        })
        // `Async.timeout:ms do:{block}` — run `block` with a deadline of `ms` ms. Returns
        // its value if it finishes in time; throws a catchable `'timeout'` if the deadline
        // fires first (the block is cancelled, its `finally` runs, in-flight I/O aborts).
        .class_method("timeout:do:", |vm, mc, _receiver, args| {
            let ms = arg!(args, Int, 0).max(0) as u64;
            let block = arg!(args, Block, 1);
            vm.await_timeout(mc, block, ms, None)
        })
        // `Async.timeout:ms do:{block} onCancel:{handler}` — as above, but on the deadline
        // run `handler` and return *its* value instead of throwing (`onCancel:{ nil }` is
        // the non-throwing form). `onCancel:` catches only *this* deadline; an outer
        // cancellation still propagates and the handler does not run.
        .class_method("timeout:do:onCancel:", |vm, mc, _receiver, args| {
            let ms = arg!(args, Int, 0).max(0) as u64;
            let block = arg!(args, Block, 1);
            let on_cancel = arg!(args, Block, 2);
            vm.await_timeout(mc, block, ms, Some(on_cancel))
        })
}

fn gather_elem_type_error(got: &Value) -> QuoinError {
    QuoinError::TypeError {
        expected: "Block".to_string(),
        got: got.type_name().to_string(),
        msg: "Async.gather: every list element must be a block".to_string(),
    }
}
