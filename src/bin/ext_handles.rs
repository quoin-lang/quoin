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
//! - `mapUpper` — exercise `invoke_block` (Slice 4): invoke the host block passed via
//!   `call:with:args:` (the first handle arg) over the inputs `["a","b","c"]` in one batched
//!   round-trip, then join the results. With the block `{ |s| s.upper }` it returns `"A,B,C"`.
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
        "mapUpper" => {
            // Invoke the host block passed via `call:with:args:` (the first handle arg) over three
            // host Strings in a single batched round-trip, then join the results. Block is
            // `{ |s| s.upper }`.
            let block = *host
                .handles()
                .first()
                .expect("mapUpper expects a block handle");
            let batches: Vec<Vec<Handle>> = ["a", "b", "c"]
                .iter()
                .map(|s| vec![host.make_string(s).expect("make_string input")])
                .collect();
            let results = host.invoke_block(block, &batches).expect("invoke_block");
            let parts: Vec<String> = results
                .iter()
                .map(|&h| host.handle_to_string(h).expect("read result"))
                .collect();
            parts.join(",")
        }
        other => format!("unknown op: {other}"),
    })
    .expect("ext_handles serve loop");
}
