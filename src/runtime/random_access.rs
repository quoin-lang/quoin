//! `RandomAccessFile` — positioned reads over a seekable file, pread-style: no
//! cursor, every `readAt:count:` names its own offset, so calls are independent
//! and there is no hidden position state. The substrate the zip reader stands
//! on (the central directory lives at the END of a zip, so reading one is
//! seeking, not streaming). Constructed via `[IO]File#randomAccess`; read-only
//! (positioned WRITE is future work — nothing needs it yet).

use crate::arg;
use crate::error::QuoinError;
use crate::io_backend::{IoRequest, IoResult, StreamId};
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
use gc_arena::collect::Trace;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub struct NativeRandomAccess {
    id: StreamId,
    /// The open-time stat size — also the read bound (see `readAt:count:`).
    size: u64,
    reap: Rc<RefCell<Vec<StreamId>>>,
    closed: bool,
}

impl std::fmt::Debug for NativeRandomAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RandomAccessFile{{id:{} size:{} closed:{}}}",
            self.id.0, self.size, self.closed
        )
    }
}

impl AnyCollect for NativeRandomAccess {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {} // no Gc fields
}

impl Drop for NativeRandomAccess {
    fn drop(&mut self) {
        // The reap backstop, exactly as for streams: a handle collected without
        // an explicit close reaps its fd. A read-only file has nothing to
        // finish, so the ordinary drop-close is a complete close.
        if !self.closed {
            self.reap.borrow_mut().push(self.id);
        }
    }
}

/// Build a `RandomAccessFile` handle over a freshly opened seekable `id`.
pub fn make_random_access<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    id: StreamId,
    size: u64,
) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "RandomAccessFile");
    vm.new_native_state(
        mc,
        class,
        NativeRandomAccess {
            id,
            size,
            reap: vm.io.socket_reap.clone(),
            closed: false,
        },
    )
}

pub fn build_random_access_class() -> NativeClassBuilder {
    NativeClassBuilder::new("RandomAccessFile", Some("Object"))
        .construct_with("use [IO]File#randomAccess")
        .class_doc(
            "Positioned reads over a file — pread-style, no cursor: every \
             `readAt:count:` names its own offset, so reads are independent and \
             there is no hidden position state. This is the substrate for \
             random-access FORMATS (zip's central directory lives at the end of \
             the file), where a sequential ByteStream is the wrong shape. \
             Read-only. Together with `size`, this is the informal random-access \
             read protocol — Bytes speaks it too (core/17-zip.qn), so code \
             written against it reads a file or a byte buffer alike.\n\n\
             ```\n\
             var out = [IO]File.create:'/tmp/ra-doc.bin'\n\
             out.writeAll:'0123456789'.asBytes\n\
             out.close\n\
             var f = ([IO]File.open:'/tmp/ra-doc.bin').randomAccess\n\
             (f.readAt:3 count:4).asString     \"* -> 3456\n\
             f.size                            \"* -> 10\n\
             f.close\n\
             [IO]File.delete:'/tmp/ra-doc.bin'\n\
             ```",
        )
        .typed_instance_method(
            "readAt:count:",
            &["Integer", "Integer"],
            |vm, mc, receiver, args| {
                let offset = arg!(args, Int, 0);
                let count = arg!(args, Int, 1);
                if offset < 0 || count < 0 {
                    return Err(QuoinError::ValueError(
                        "readAt:count:: offset and count must be non-negative".to_string(),
                    ));
                }
                let (id, size, closed) = receiver
                    .with_native_state::<NativeRandomAccess, _, _>(|s| (s.id, s.size, s.closed))
                    .map_err(QuoinError::Other)?;
                if closed {
                    return Err(QuoinError::io_closed("readAt:count:: the file is closed"));
                }
                // Clamp to the open-time size: EOF answers short/empty without an op,
                // and a mistaken huge count can't become a huge allocation.
                let remaining = size.saturating_sub(offset as u64);
                let max = (count as u64).min(remaining) as usize;
                if max == 0 {
                    return Ok(vm.new_bytes(mc, Vec::new()));
                }
                match vm.await_io(IoRequest::ReadAt {
                    id,
                    offset: offset as u64,
                    max,
                })? {
                    IoResult::Read(bytes) => Ok(vm.new_bytes(mc, bytes)),
                    IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
                    other => Err(QuoinError::Other(format!(
                        "readAt:count:: unexpected I/O result {other:?}"
                    ))),
                }
            },
        )
        .doc(
            "Up to `count` bytes starting at byte `offset` — short only when the file \
             ends first (an offset at or past the end answers empty Bytes). Reads park \
             the task, not the scheduler; one read at a time per handle.",
        )
        .instance_method("size", |vm, mc, receiver, _args| {
            receiver
                .with_native_state::<NativeRandomAccess, _, _>(|s| s.size)
                .map_err(QuoinError::Other)
                .map(|n| vm.new_int(mc, n as i64))
        })
        .doc("The file's length in bytes, from the stat taken when the handle opened.")
        .instance_method("close", |vm, mc, receiver, _args| {
            let to_reap = receiver
                .with_native_state_mut::<NativeRandomAccess, _, _>(mc, |s| {
                    if s.closed {
                        None
                    } else {
                        s.closed = true;
                        Some(s.id)
                    }
                })
                .map_err(QuoinError::Other)?;
            if let Some(id) = to_reap {
                vm.io.socket_reap.borrow_mut().push(id);
            }
            Ok(vm.new_nil(mc))
        })
        .doc("Close the file (idempotent). Further reads throw. Returns nil.")
        .instance_method("closed?", |vm, mc, receiver, _args| {
            receiver
                .with_native_state::<NativeRandomAccess, _, _>(|s| s.closed)
                .map_err(QuoinError::Other)
                .map(|b| vm.new_bool(mc, b))
        })
        .doc("Whether the handle has been closed.")
        .instance_method("s", |vm, mc, receiver, _args| {
            let (size, closed) = receiver
                .with_native_state::<NativeRandomAccess, _, _>(|s| (s.size, s.closed))
                .map_err(QuoinError::Other)?;
            Ok(vm.new_string(
                mc,
                format!(
                    "RandomAccessFile({size} bytes{})",
                    if closed { ", closed" } else { "" }
                ),
            ))
        })
        .doc("A short description: size and closed-ness.")
}
