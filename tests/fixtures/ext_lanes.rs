//! An out-of-process extension that declares **two lanes** (see `tests/extension.rs`): the
//! host opens two connections and calls on both concurrently, so calls to *different* `Slot`
//! instances overlap while calls to one instance serialize on its per-object mailbox.
//!
//! - `Slot make: n` / `Slot slowMake: n` — constructors (the slow one proves class-side
//!   sends contend only on lanes: two `slowMake:`s run in parallel).
//! - `s tag` / `s slowTag` — instance reads (the slow one proves the overlap/serialize split).
//! - `s applyHeld: aBlock` — holds the instance briefly, then applies the host block to its
//!   tag: two tasks cross-calling each other's held slots through the blocks close a claim
//!   cycle, which the host must detect as a catchable deadlock (§5.1 rule 6).
//!
//! A test/example fixture, not a shipped feature.

use std::time::Duration;

use quoin_ext::{DataValue, Extension};

struct Slot {
    tag: i64,
}

/// Long enough that overlap vs. serial is unambiguous under test-suite load, short enough
/// to keep the suite fast.
const SLEEP: Duration = Duration::from_millis(150);

fn tag_arg(args: &[quoin_ext::Arg]) -> Result<i64, String> {
    match args.first().and_then(|a| a.data()) {
        Some(DataValue::Int(n)) => Ok(*n),
        _ => Err("expects an integer tag".to_string()),
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ext_lanes <socket-path>");
    let mut ext = Extension::new();
    ext.lanes(2);
    ext.class::<Slot>("Slot", |c| {
        c.constructor("make:", |_host, args| {
            Ok(Slot {
                tag: tag_arg(args)?,
            })
        });
        c.constructor("slowMake:", |_host, args| {
            let tag = tag_arg(args)?;
            std::thread::sleep(SLEEP);
            Ok(Slot { tag })
        });
        c.method("tag", |s, _host, _args| Ok(DataValue::Int(s.tag)));
        c.method("slowTag", |s, _host, _args| {
            std::thread::sleep(SLEEP);
            Ok(DataValue::Int(s.tag))
        });
        // Hold the instance long enough for the other task's call to be in flight, then run
        // the block — whose body (host-side, on the caller's fiber) calls the OTHER slot.
        c.method("applyHeld:", |s, host, args| {
            let block = args[0].handle().ok_or("applyHeld: expects a block")?;
            std::thread::sleep(Duration::from_millis(80));
            let out = host.apply_block(block, &[DataValue::Int(s.tag)])?;
            Ok(out.into_iter().next().unwrap_or(DataValue::Null))
        });
    });
    ext.serve(&path).expect("ext_lanes: serve failed");
}
