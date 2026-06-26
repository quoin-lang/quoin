//! A tiny out-of-process Quoin extension that exercises the Tier-1 **handle table**
//! (`docs/FUTURE_EXT_ARCH.md` §2; see `tests/extension.rs`). The VM spawns it with a socket
//! path as argv[1]. It serves three ops that use the host-callback client to hold a host
//! value across calls:
//!
//! - `stash` — make a host String from the arg, **retain** its handle (promote to global),
//!   and remember it for later. Returns `"ok"`.
//! - `fetch` — read the previously stashed handle back into a string. Returns that string,
//!   proving the host kept the `Value` alive (rooted by the handle) across the two calls.
//! - `release` — release the stashed handle.
//!
//! It is a test/example fixture, not a shipped feature.

use std::cell::RefCell;

use quoin_ext::Handle;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ext_handles <socket-path>");

    // A handle the extension retains across calls. `RefCell` because `serve`'s handler is
    // `FnMut` and we mutate this from inside it.
    let stashed: RefCell<Option<Handle>> = RefCell::new(None);

    quoin_ext::serve(&path, |host, op, arg| match op {
        "stash" => {
            let handle = host.make_string(arg).expect("make_string");
            host.retain(handle).expect("retain");
            *stashed.borrow_mut() = Some(handle);
            "ok".to_string()
        }
        "fetch" => {
            let handle = stashed.borrow().expect("nothing stashed yet");
            host.handle_to_string(handle).expect("handle_to_string")
        }
        "release" => {
            if let Some(handle) = stashed.borrow_mut().take() {
                host.release(&[handle]).expect("release");
            }
            "ok".to_string()
        }
        other => format!("unknown op: {other}"),
    })
    .expect("ext_handles serve loop");
}
