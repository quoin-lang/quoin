//! An out-of-process Quoin extension that *provides* the classes `Vector` and `Matrix` (Phase 3,
//! extension-backed classes; see `tests/extension.rs`). The SDK owns the instances, so writing each
//! class is just writing a Rust type and registering its selectors:
//!
//! - `Vector ofFloats: aList` (constructor) -> a new `Vector`; `v sum` / `v length` / `v scale: f`.
//! - `Matrix ofRows: aListOfLists` (constructor); `m rowCount`; `m row: i` -> a **`Vector`** — a
//!   *cross-class* return (a method on `Matrix` returning an instance of a different class).
//!
//! A test/example fixture, not a shipped feature.

use quoin_ext::{DataValue, Extension};

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

/// Read a numeric `DataValue` as a `usize` index.
fn as_usize(d: &DataValue) -> usize {
    match d {
        DataValue::Int(i) => *i as usize,
        DataValue::Float(f) => *f as usize,
        _ => 0,
    }
}

/// Read the constructor's list argument (`args[0]` is a `DvList` of numbers) into a `Vec<f64>`.
fn float_list(args: &[DataValue]) -> Vec<f64> {
    match args.first() {
        Some(DataValue::List(items)) => items.iter().map(as_f64).collect(),
        _ => Vec::new(),
    }
}

/// Read the constructor's list-of-lists argument (`args[0]` is a `DvList` of `DvList`s) into rows.
fn float_rows(args: &[DataValue]) -> Vec<Vec<f64>> {
    match args.first() {
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
        c.makes("scale:", |v, _host, args| v.scaled(as_f64(&args[0])));
    });
    ext.class::<Matrix>("Matrix", |c| {
        c.constructor("ofRows:", |_host, args| Matrix {
            rows: float_rows(args),
        });
        c.method("rowCount", |m, _host, _args| {
            DataValue::Int(m.rows.len() as i64)
        });
        // Returns a `Vector` — a different registered class (cross-class return).
        c.makes("row:", |m, _host, args| m.row(as_usize(&args[0])));
    });
    ext.serve(&path).expect("ext_vector serve loop");
}
