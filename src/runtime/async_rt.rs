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
}

fn gather_elem_type_error(got: &Value) -> QuoinError {
    QuoinError::TypeError {
        expected: "Block".to_string(),
        got: got.type_name().to_string(),
        msg: "Async.gather: every list element must be a block".to_string(),
    }
}
