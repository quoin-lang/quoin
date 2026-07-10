use crate::arg;
use crate::error::QuoinError;
use crate::io_backend::IoRequest;
use crate::runtime::duration::duration_to_millis;
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
        .abstract_class()
        .class_doc(
            "Structured concurrency over the task scheduler: run blocks as concurrent tasks \
             whose I/O overlaps (`gather:`), park without blocking other tasks (`sleep:`), \
             and put a deadline on anything (`timeout:do:`). See docs/ASYNC_ARCH.md.",
        )
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
        .doc(
            "Run a List of zero-parameter blocks as concurrent tasks -- their I/O overlaps \
             -- and answer their results as a List in input order once all complete. \
             Propagates the first error.\n\n\
             ```\n\
             Async.gather:#( { Async.sleep:2; 'a' } { 'b' } )    \"* -> #(a b)\n\
             ```",
        )
        // `Async.sleep:` — park the running fiber via the async IoBackend, without blocking other
        // fibers (Stage 1 — see docs/ASYNC_ARCH.md). Accepts a bare ms count or a Duration; nil.
        .typed_class_method("sleep:", &["Integer"], |vm, mc, _receiver, args| {
            vm.await_io(IoRequest::Sleep {
                ms: arg!(args, Int, 0).max(0) as u64,
            })?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Park the running task for the given milliseconds (a Duration is also accepted) \
             without blocking other tasks; answers nil.",
        )
        .typed_class_method("sleep:", &["Duration"], |vm, mc, _receiver, args| {
            let ms = duration_to_millis(args[0], "sleep:")? as u64;
            vm.await_io(IoRequest::Sleep { ms })?;
            Ok(vm.new_nil(mc))
        })
        // `Async.timeout:ms do:{block}` — run `block` with a deadline of `ms` ms (or a Duration).
        // Returns its value if it finishes in time; throws a catchable `'timeout'` if the deadline
        // fires first (the block is cancelled, its `finally` runs, in-flight I/O aborts).
        .typed_class_method(
            "timeout:do:",
            &["Integer", "Block"],
            |vm, mc, _receiver, args| {
                let ms = arg!(args, Int, 0).max(0) as u64;
                let block = arg!(args, Block, 1);
                vm.await_timeout(mc, block, ms, None)
            },
        )
        .doc(
            "Run the block with a deadline of the given milliseconds (or a Duration): its \
             value if it finishes in time; if the deadline fires first, the block is \
             cancelled (its `finally` runs, in-flight I/O aborts) and a catchable timeout \
             error raises.\n\n\
             ```\n\
             Async.timeout:50 do:{ 42 }    \"* -> 42\n\
             ```",
        )
        .typed_class_method(
            "timeout:do:",
            &["Duration", "Block"],
            |vm, mc, _receiver, args| {
                let ms = duration_to_millis(args[0], "timeout:do:")? as u64;
                let block = arg!(args, Block, 1);
                vm.await_timeout(mc, block, ms, None)
            },
        )
        // `Async.timeout:ms do:{block} onCancel:{handler}` — as above, but on the deadline
        // run `handler` and return *its* value instead of throwing (`onCancel:{ nil }` is
        // the non-throwing form). `onCancel:` catches only *this* deadline; an outer
        // cancellation still propagates and the handler does not run.
        .typed_class_method(
            "timeout:do:onCancel:",
            &["Integer", "Block", "Block"],
            |vm, mc, _receiver, args| {
                let ms = arg!(args, Int, 0).max(0) as u64;
                let block = arg!(args, Block, 1);
                let on_cancel = arg!(args, Block, 2);
                vm.await_timeout(mc, block, ms, Some(on_cancel))
            },
        )
        .doc(
            "As `timeout:do:`, but when the deadline fires run the handler and answer ITS \
             value instead of throwing (`onCancel:{ nil }` is the non-throwing form). The \
             handler covers only this deadline: an outer cancellation still propagates and \
             the handler does not run.\n\n\
             ```\n\
             Async.timeout:5 do:{ Async.sleep:200; 1 } onCancel:{ 'late' }    \"* -> late\n\
             ```",
        )
        .typed_class_method(
            "timeout:do:onCancel:",
            &["Duration", "Block", "Block"],
            |vm, mc, _receiver, args| {
                let ms = duration_to_millis(args[0], "timeout:do:onCancel:")? as u64;
                let block = arg!(args, Block, 1);
                let on_cancel = arg!(args, Block, 2);
                vm.await_timeout(mc, block, ms, Some(on_cancel))
            },
        )
}

fn gather_elem_type_error(got: &Value) -> QuoinError {
    QuoinError::TypeError {
        expected: "Block".to_string(),
        got: got.type_name().to_string(),
        msg: "Async.gather: every list element must be a block".to_string(),
    }
}
