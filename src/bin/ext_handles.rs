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
//! - `compute` — exercise `call_method` (Slice 3b): build two host Strings, concatenate them
//!   via `+:` (passing the second as a *handle argument*), uppercase the result via `upper`,
//!   and read it back. For `arg = "ab"` it returns `"AB!"` (`("ab" +: "!").upper`).
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
        "compute" => {
            let base = host.make_string(arg).expect("make_string base");
            let suffix = host.make_string("!").expect("make_string suffix");
            // ("<arg>" +: "!") — `+:` takes the suffix handle as its argument.
            let joined = host.call_method(base, "+:", &[suffix]).expect("concat");
            // .upper — a unary send (no arguments).
            let upper = host.call_method(joined, "upper", &[]).expect("upper");
            host.handle_to_string(upper).expect("read result")
        }
        other => format!("unknown op: {other}"),
    })
    .expect("ext_handles serve loop");
}
