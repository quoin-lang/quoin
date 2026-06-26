//! A tiny out-of-process Quoin extension exercising **structured values** (Phase 1; see
//! `tests/extension.rs`). It receives/returns `DataValue` trees that materialize as nested Quoin
//! Values:
//!
//! - `echoData` — return the structured `data:` payload unchanged (round-trips Quoin <-> DataValue).
//! - `mkList`   — return a structured `List` built extension-side (`#(1 2 3)`).
//! - `buildArray` — Phase 2 (host reach): reach the host `Array` class (`get_global`), build a float
//!   list arg (`make_value`), call `ofFloats:` on the class, and return the live `Array`.
//! - `inspect`  — Phase 2: `read_handle` the value passed via `args:` back into structured data.
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
            "buildArray" => {
                let array_class = host.get_global("Array").expect("get_global Array");
                let list = host
                    .make_value(DataValue::List(vec![
                        DataValue::Float(1.0),
                        DataValue::Float(2.0),
                        DataValue::Float(3.0),
                    ]))
                    .expect("make_value");
                let arr = host
                    .call_method(array_class, "ofFloats:", &[list])
                    .expect("ofFloats:");
                Reply::Handle(arr)
            }
            "inspect" => {
                let h = host.handles()[0];
                Reply::Data(host.read_handle(h).expect("read_handle"))
            }
            other => Reply::Scalar(format!("unknown op: {other}")),
        }
    })
    .expect("ext_data serve loop");
}
