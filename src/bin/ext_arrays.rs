//! A tiny out-of-process Quoin extension that exercises the Tier-1 **Array data plane**
//! (Slice 6b; see `tests/extension.rs`). The host passes a bulk `Array` as a call argument; the
//! extension operates on the whole contiguous buffer (never per-element) and returns either a
//! scalar or a new `Array`.
//!
//! Ops (Float64 only, matching the skeleton):
//! - `sum`   — sum the array's elements, returning the total as a scalar string.
//! - `scale` — multiply every element by the factor in `arg`, returning a new `Array`.
//!
//! It is a test/example fixture, not a shipped feature.

use quoin_ext::{ArrowArray, ArrowDType, Reply};

/// Read a Float64 column's contiguous little-endian buffer into `f64`s.
fn read_f64s(a: &ArrowArray) -> Vec<f64> {
    a.data
        .chunks_exact(8)
        .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ext_arrays <socket-path>");

    quoin_ext::serve(&path, |host, op, arg| -> Reply {
        let array = host
            .arrays()
            .first()
            .cloned()
            .expect("ext_arrays expects one Array argument");
        let xs = read_f64s(&array);
        match op {
            "sum" => Reply::Scalar(xs.iter().sum::<f64>().to_string()),
            "scale" => {
                let factor: f64 = arg.parse().expect("scale factor must be a number");
                let data: Vec<u8> = xs.iter().flat_map(|x| (x * factor).to_le_bytes()).collect();
                Reply::Array(ArrowArray {
                    dtype: ArrowDType::Float64,
                    length: xs.len() as u64,
                    data,
                })
            }
            other => Reply::Scalar(format!("unknown op: {other}")),
        }
    })
    .expect("ext_arrays serve loop");
}
