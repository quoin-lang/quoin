//! A tiny out-of-process Quoin extension that exercises Tier-1 **ext-resource handles**
//! (Slice 5b; see `tests/extension.rs`). A "resource" here is a mutable counter that lives in
//! the extension process; the host holds an opaque `ExtResource` token for it and passes it back
//! into later calls. The extension owns the registry (a `HashMap` keyed by an id it assigns),
//! mirroring how a real DB extension owns its connections.
//!
//! Ops:
//! - `new`  — create a counter (starting at 0), return its resource id (`Reply::Resource`).
//! - `inc`  — `resources()[0]` names the counter; increment it, return the new value as a string.
//! - `live` — return how many counters are still registered (proves drop-reap freed them).
//!
//! Each call first frees any `releases()` (resources the host has dropped) from the registry.
//! It is a test/example fixture, not a shipped feature.

use std::cell::RefCell;
use std::collections::HashMap;

use quoin_ext::Reply;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ext_resources <socket-path>");

    // The extension's own resource registry: id -> counter value. `next_id` assigns ids.
    let counters: RefCell<HashMap<u64, i64>> = RefCell::new(HashMap::new());
    let next_id: RefCell<u64> = RefCell::new(1);

    quoin_ext::serve(&path, |host, op, _arg| -> Reply {
        // Free any resources the host has dropped (batched onto this call as `releases`).
        for &id in host.releases() {
            counters.borrow_mut().remove(&id);
        }

        match op {
            "new" => {
                let id = {
                    let mut n = next_id.borrow_mut();
                    let id = *n;
                    *n += 1;
                    id
                };
                counters.borrow_mut().insert(id, 0);
                Reply::Resource(id)
            }
            "inc" => {
                let id = *host
                    .resources()
                    .first()
                    .expect("inc expects a resource arg");
                let mut map = counters.borrow_mut();
                let value = map.get_mut(&id).expect("unknown resource");
                *value += 1;
                Reply::Scalar(value.to_string())
            }
            "live" => Reply::Scalar(counters.borrow().len().to_string()),
            other => Reply::Scalar(format!("unknown op: {other}")),
        }
    })
    .expect("ext_resources serve loop");
}
