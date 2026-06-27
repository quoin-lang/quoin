//! An out-of-process Quoin extension that *provides* the classes `Vector` and `Matrix` (Phase 3,
//! extension-backed classes; see `tests/extension.rs`). The SDK owns the instances, so writing each
//! class is just writing a Rust type and registering its selectors:
//!
//! - `Vector ofFloats: aList` (constructor); `v sum` / `v length` / `v scale: f`.
//! - `v dot: other` — an **ext-instance argument** (another `Vector`); returns a scalar.
//! - `v map: aBlock` — a **host-block argument**; applies the block to each element -> a new `Vector`.
//! - `Matrix ofRows: aListOfLists` (constructor); `m rowCount`; `m row: i` -> a **`Vector`** (a
//!   *cross-class* return).
//!
//! A test/example fixture, not a shipped feature.

use quoin_ext::{Arg, DataValue, Extension};

/// A plain Rust type. The SDK keeps instances in its object table keyed by an opaque id (the
/// resource id the host holds); method sends arrive already routed to the right instance.
struct Vector {
    data: Vec<f64>,
}

impl Vector {
    fn sum(&self) -> f64 {
        self.data.iter().sum()
    }

    fn scaled(&self, factor: f64) -> Vector {
        Vector {
            data: self.data.iter().map(|x| x * factor).collect(),
        }
    }

    fn dot(&self, other: &Vector) -> f64 {
        self.data.iter().zip(&other.data).map(|(a, b)| a * b).sum()
    }
}

/// A second class — `row:` returns a `Vector`, exercising cross-class returns.
struct Matrix {
    rows: Vec<Vec<f64>>,
}

impl Matrix {
    fn row(&self, i: usize) -> Vector {
        Vector {
            data: self.rows.get(i).cloned().unwrap_or_default(),
        }
    }
}

/// Read a numeric `DataValue` as an `f64` (a Quoin Double or Integer).
fn as_f64(d: &DataValue) -> f64 {
    match d {
        DataValue::Float(f) => *f,
        DataValue::Int(i) => *i as f64,
        _ => 0.0,
    }
}

/// Read a data argument as an `f64`.
fn arg_f64(a: &Arg) -> f64 {
    a.data().map(as_f64).unwrap_or(0.0)
}

/// Read the constructor's list argument (`args[0]` is a `DvList` of numbers) into a `Vec<f64>`.
fn float_list(args: &[Arg]) -> Vec<f64> {
    if let Some(DataValue::List(items)) = args.first().and_then(|a| a.data()) {
        items.iter().map(as_f64).collect()
    } else {
        Vec::new()
    }
}

/// Read the constructor's list-of-lists argument (`args[0]` is a `DvList` of `DvList`s) into rows.
fn float_rows(args: &[Arg]) -> Vec<Vec<f64>> {
    match args.first().and_then(|a| a.data()) {
        Some(DataValue::List(rows)) => rows
            .iter()
            .map(|r| match r {
                DataValue::List(items) => items.iter().map(as_f64).collect(),
                _ => Vec::new(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ext_vector <socket-path>");

    let mut ext = Extension::new();
    ext.class::<Vector>("Vector", |c| {
        c.constructor("ofFloats:", |_host, args| Vector {
            data: float_list(args),
        });
        c.method("sum", |v, _host, _args| DataValue::Float(v.sum()));
        c.method("length", |v, _host, _args| {
            DataValue::Int(v.data.len() as i64)
        });
        c.makes("scale:", |v, _host, args| v.scaled(arg_f64(&args[0])));
        // An ext-instance argument: the other `Vector` is resolved to a live instance.
        c.method("dot:", |v, _host, args| {
            let other = args[0].object::<Vector>().expect("dot: expects a Vector");
            DataValue::Float(v.dot(other))
        });
        // A host-block argument: apply the block to each element, building a new `Vector`.
        c.makes("map:", |v, host, args| {
            let block = args[0].handle().expect("map: expects a block");
            let inputs: Vec<DataValue> = v.data.iter().map(|x| DataValue::Float(*x)).collect();
            let results = host.apply_block(block, &inputs).expect("apply block");
            Vector {
                data: results.iter().map(as_f64).collect(),
            }
        });
    });
    ext.class::<Matrix>("Matrix", |c| {
        c.constructor("ofRows:", |_host, args| Matrix {
            rows: float_rows(args),
        });
        c.method("rowCount", |m, _host, _args| {
            DataValue::Int(m.rows.len() as i64)
        });
        // Returns a `Vector` — a different registered class (cross-class return).
        c.makes("row:", |m, _host, args| m.row(arg_f64(&args[0]) as usize));
    });
    ext.serve(&path).expect("ext_vector serve loop");
}
