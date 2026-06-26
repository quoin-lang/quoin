//! A tiny out-of-process Quoin extension exercising **structured values** (Phase 1; see
//! `tests/extension.rs`). It receives/returns `DataValue` trees that materialize as nested Quoin
//! Values:
//!
//! - `echoData` — return the structured `data:` payload unchanged (round-trips Quoin <-> DataValue).
//! - `mkList`   — return a structured `List` built extension-side (`#(1 2 3)`).
//!
//! A test/example fixture, not a shipped feature.

use quoin_ext::{DataValue, Reply};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ext_data <socket-path>");
    quoin_ext::serve(&path, |host, op, _arg| -> Reply {
        match op {
            "echoData" => Reply::Data(host.data().cloned().unwrap_or(DataValue::Null)),
            "mkList" => Reply::Data(DataValue::List(vec![
                DataValue::Int(1),
                DataValue::Int(2),
                DataValue::Int(3),
            ])),
            other => Reply::Scalar(format!("unknown op: {other}")),
        }
    })
    .expect("ext_data serve loop");
}
