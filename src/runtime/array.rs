//! `Array` — a typed, contiguous numeric array (Tier 1 data plane; see `docs/internal/FUTURE_EXT_ARCH.md`
//! §6/§7). Slice 6a is the **value type**: a homogeneous column of `Float64` or `Int64` held as a
//! single buffer behind a native-state handle, **distinct from `List`** — it is never exploded
//! into per-element `Value`s. The buffer is Apache Arrow's non-nullable primitive layout
//! (contiguous little-endian values, no validity bitmap), so a future Python/polars extension
//! reads the same bytes as numpy/pyarrow with zero conversion. Nulls/validity bitmaps, more
//! dtypes, the full Arrow C Data Interface structs, and the over-the-socket data plane (Slice 6b)
//! are separate steps.
//!
//! `Array` vs `Bytes`: both are contiguous GC buffers, but `Bytes` is untyped octets (an opaque
//! blob — image, gzip stream) with byte ops, while `Array` is a *typed* column (dtype + element
//! count) with numeric/columnar ops and an Arrow layout contract — the `bytes` vs `numpy.ndarray`
//! distinction (§6's blob vs bulk-tabular categories).

use std::any::Any;

use gc_arena::Mutation;
use gc_arena::collect::Trace;

use crate::arg;
use crate::error::QuoinError;
use crate::runtime::list::NativeListState;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;

/// The element type of an `Array`. Both are 8 bytes (Arrow `Float64` / `Int64`). Public so the
/// extension data plane (`runtime/extension.rs`) can bridge it to the wire `ArrowDType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArrayDType {
    Float64,
    Int64,
}

impl ArrayDType {
    const fn size(self) -> usize {
        8
    }
    const fn symbol(self) -> &'static str {
        match self {
            ArrayDType::Float64 => "float64",
            ArrayDType::Int64 => "int64",
        }
    }
}

/// Native state behind an `Array`: a dtype tag + the contiguous little-endian value buffer
/// (Arrow non-nullable primitive layout). Holds no `Gc` — trivially traced.
#[derive(Debug)]
pub struct NativeArray {
    dtype: ArrayDType,
    data: Vec<u8>,
}

impl NativeArray {
    /// Element count (byte length / element size).
    fn len(&self) -> usize {
        self.data.len() / self.dtype.size()
    }
}

impl AnyCollect for NativeArray {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

fn read_f64(data: &[u8], i: usize) -> f64 {
    f64::from_le_bytes(data[i * 8..i * 8 + 8].try_into().unwrap())
}

fn read_i64(data: &[u8], i: usize) -> i64 {
    i64::from_le_bytes(data[i * 8..i * 8 + 8].try_into().unwrap())
}

/// A `Value` as an `f64` (accepting `Double` or `Int`), or `None` if it isn't numeric.
fn as_f64(v: Value<'_>) -> Option<f64> {
    match v {
        Value::Double(d) => Some(d),
        Value::Int(i) => Some(i as f64),
        _ => None,
    }
}

/// A `Value` as an `i64` (`Int` only), or `None`.
fn as_i64(v: Value<'_>) -> Option<i64> {
    match v {
        Value::Int(i) => Some(i),
        _ => None,
    }
}

/// Extract the elements of a Quoin list `Value`.
fn list_elems<'gc>(value: Value<'gc>, who: &str) -> Result<Vec<Value<'gc>>, QuoinError> {
    value
        .with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
        .map_err(|_| QuoinError::TypeError {
            expected: "List".to_string(),
            got: value.type_name().to_string(),
            msg: format!("{who} expects a list"),
        })
}

/// Build an `Array` value from a dtype + packed buffer. Public so the extension data plane can
/// reconstruct an `Array` returned by an extension (`CallReturnArray`).
pub fn new_array<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    dtype: ArrayDType,
    data: Vec<u8>,
) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "Array");
    vm.new_native_state(mc, class, NativeArray { dtype, data })
}

/// If `value` is an `Array`, return its dtype + a copy of its buffer (for transfer to an
/// extension); otherwise `None`. Lets the data plane serialize an `Array` without touching
/// `NativeArray` internals.
pub fn array_parts(value: Value<'_>) -> Option<(ArrayDType, Vec<u8>)> {
    value
        .with_native_state::<NativeArray, _, _>(|a| (a.dtype, a.data.clone()))
        .ok()
}

pub fn build_array_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Array", Some("Object"))
        .construct_with("use Array.ofInts: / Array.ofFloats:")
        .class_doc(
            "A typed, contiguous numeric column: every element is the same dtype (`#float64` \
             or `#int64`), held in one packed buffer rather than exploded into per-element \
             values. The layout is Apache Arrow's non-nullable primitive format, so extensions \
             (numpy, databases) read the same bytes with zero conversion.\n\nBuild with \
             `Array.ofInts:` / `Array.ofFloats:`; leave the bulk world with `toList`. An \
             untyped blob of octets belongs in `Bytes` instead.",
        )
        // `Array ofFloats: #( 1.0 2.0 … )` — pack a list of numbers into a Float64 column.
        .class_method("ofFloats:", |vm, mc, _receiver, args| {
            let elems = list_elems(
                *args.first().ok_or_else(|| {
                    QuoinError::Other("Array.ofFloats: expects a list".to_string())
                })?,
                "Array.ofFloats:",
            )?;
            let mut data = Vec::with_capacity(elems.len() * 8);
            for (i, &v) in elems.iter().enumerate() {
                let x = as_f64(v).ok_or_else(|| QuoinError::TypeError {
                    expected: "Number".to_string(),
                    got: v.type_name().to_string(),
                    msg: format!("Array.ofFloats: element {i} is not a number"),
                })?;
                data.extend_from_slice(&x.to_le_bytes());
            }
            Ok(new_array(vm, mc, ArrayDType::Float64, data))
        })
        .doc(
            "A float64 Array packed from a list of numbers (integers are widened); a \
             non-number raises a TypeError naming the element.\n\n\
             ```\n\
             Array.ofFloats:#(1.5 2.5)     \"* -> Array[float64; 2]\n\
             ```",
        )
        // `Array ofInts: #( 1 2 … )` — pack a list of integers into an Int64 column.
        .class_method("ofInts:", |vm, mc, _receiver, args| {
            let elems = list_elems(
                *args
                    .first()
                    .ok_or_else(|| QuoinError::Other("Array.ofInts: expects a list".to_string()))?,
                "Array.ofInts:",
            )?;
            let mut data = Vec::with_capacity(elems.len() * 8);
            for (i, &v) in elems.iter().enumerate() {
                let x = as_i64(v).ok_or_else(|| QuoinError::TypeError {
                    expected: "Integer".to_string(),
                    got: v.type_name().to_string(),
                    msg: format!("Array.ofInts: element {i} is not an integer"),
                })?;
                data.extend_from_slice(&x.to_le_bytes());
            }
            Ok(new_array(vm, mc, ArrayDType::Int64, data))
        })
        .doc(
            "An int64 Array packed from a list of integers; a non-integer raises a TypeError \
             naming the element.\n\n\
             ```\n\
             Array.ofInts:#(1 2 3)     \"* -> Array[int64; 3]\n\
             ```",
        )
        // `array.length` — element count.
        .instance_method("length", |vm, mc, receiver, _args| {
            let n = receiver
                .with_native_state::<NativeArray, _, _>(|a| a.len())
                .map_err(QuoinError::Other)?;
            Ok(vm.new_int(mc, n as i64))
        })
        .doc(
            "The number of elements.\n\n\
             ```\n\
             (Array.ofInts:#(1 2 3)).length     \"* -> 3\n\
             ```",
        )
        // `array.dtype` — the element type as a symbol (`#float64` / `#int64`).
        .instance_method("dtype", |vm, mc, receiver, _args| {
            let sym = receiver
                .with_native_state::<NativeArray, _, _>(|a| a.dtype.symbol())
                .map_err(QuoinError::Other)?;
            Ok(vm.new_symbol(mc, sym.to_string()))
        })
        .doc(
            "The element type as a Symbol: `#float64` or `#int64`.\n\n\
             ```\n\
             (Array.ofInts:#(1 2 3)).dtype     \"* -> int64\n\
             ```",
        )
        // `array.sum` — reduce the whole buffer (never exploded). Float64 -> Double, Int64 -> Integer.
        .instance_method("sum", |vm, mc, receiver, _args| {
            enum Sum {
                F(f64),
                I(i64),
            }
            let sum = receiver
                .with_native_state::<NativeArray, _, _>(|a| match a.dtype {
                    ArrayDType::Float64 => Sum::F((0..a.len()).map(|i| read_f64(&a.data, i)).sum()),
                    ArrayDType::Int64 => Sum::I((0..a.len()).map(|i| read_i64(&a.data, i)).sum()),
                })
                .map_err(QuoinError::Other)?;
            Ok(match sum {
                Sum::F(f) => vm.new_double(mc, f),
                Sum::I(i) => vm.new_int(mc, i),
            })
        })
        .doc(
            "The sum of all elements, reduced over the packed buffer in one pass: a Double \
             from a float64 array, an Integer from an int64 one.\n\n\
             ```\n\
             (Array.ofInts:#(1 2 3)).sum     \"* -> 6\n\
             (Array.ofFloats:#(1.5 2.25)).sum     \"* -> 3.75\n\
             ```",
        )
        // `array scale: n` — multiply every element by `n`, returning a new `Array` of the same dtype.
        .instance_method("scale:", |vm, mc, receiver, args| {
            let factor = *args
                .first()
                .ok_or_else(|| QuoinError::Other("Array scale: expects a factor".to_string()))?;
            let dtype = receiver
                .with_native_state::<NativeArray, _, _>(|a| a.dtype)
                .map_err(QuoinError::Other)?;
            let data = match dtype {
                ArrayDType::Float64 => {
                    let f = as_f64(factor).ok_or_else(|| QuoinError::TypeError {
                        expected: "Number".to_string(),
                        got: factor.type_name().to_string(),
                        msg: "Array scale: factor must be a number".to_string(),
                    })?;
                    receiver
                        .with_native_state::<NativeArray, _, _>(|a| {
                            let mut d = Vec::with_capacity(a.data.len());
                            for i in 0..a.len() {
                                d.extend_from_slice(&(read_f64(&a.data, i) * f).to_le_bytes());
                            }
                            d
                        })
                        .map_err(QuoinError::Other)?
                }
                ArrayDType::Int64 => {
                    let f = as_i64(factor).ok_or_else(|| QuoinError::TypeError {
                        expected: "Integer".to_string(),
                        got: factor.type_name().to_string(),
                        msg: "Array scale: factor must be an integer".to_string(),
                    })?;
                    receiver
                        .with_native_state::<NativeArray, _, _>(|a| {
                            let mut d = Vec::with_capacity(a.data.len());
                            for i in 0..a.len() {
                                d.extend_from_slice(
                                    &read_i64(&a.data, i).wrapping_mul(f).to_le_bytes(),
                                );
                            }
                            d
                        })
                        .map_err(QuoinError::Other)?
                }
            };
            Ok(new_array(vm, mc, dtype, data))
        })
        .doc(
            "A new Array of the same dtype with every element multiplied by a factor — any \
             number for a float64 array, an integer (wrapping on overflow) for an int64 \
             one.\n\n\
             ```\n\
             ((Array.ofInts:#(1 2 3)).scale:10).toList     \"* -> #(10 20 30)\n\
             ```",
        )
        // `array at: i` — one element (Double/Integer); does not explode the array.
        .instance_method("at:", |vm, mc, receiver, args| {
            let idx = arg!(args, Int, 0);
            enum Elem {
                F(f64),
                I(i64),
                OutOfBounds(usize),
            }
            let elem = receiver
                .with_native_state::<NativeArray, _, _>(|a| {
                    let n = a.len();
                    if idx < 0 || idx as usize >= n {
                        Elem::OutOfBounds(n)
                    } else {
                        let i = idx as usize;
                        match a.dtype {
                            ArrayDType::Float64 => Elem::F(read_f64(&a.data, i)),
                            ArrayDType::Int64 => Elem::I(read_i64(&a.data, i)),
                        }
                    }
                })
                .map_err(QuoinError::Other)?;
            match elem {
                Elem::F(f) => Ok(vm.new_double(mc, f)),
                Elem::I(i) => Ok(vm.new_int(mc, i)),
                Elem::OutOfBounds(n) => Err(QuoinError::IndexError {
                    index: idx,
                    len: n as i64,
                    msg: format!("Array at: index {idx} out of bounds (length {n})"),
                }),
            }
        })
        .doc(
            "The element at a zero-based index — a Double or an Integer to match the dtype; \
             out of range raises a catchable IndexError.\n\n\
             ```\n\
             (Array.ofFloats:#(1.5 2.5)).at:1     \"* -> 2.5\n\
             ```",
        )
        // `array.toList` — explode the column into an ordinary Quoin `List` (leaving the bulk world).
        .instance_method("toList", |vm, mc, receiver, _args| {
            let dtype = receiver
                .with_native_state::<NativeArray, _, _>(|a| a.dtype)
                .map_err(QuoinError::Other)?;
            let values: Vec<Value> = match dtype {
                ArrayDType::Float64 => {
                    let xs = receiver
                        .with_native_state::<NativeArray, _, _>(|a| {
                            (0..a.len())
                                .map(|i| read_f64(&a.data, i))
                                .collect::<Vec<f64>>()
                        })
                        .map_err(QuoinError::Other)?;
                    xs.into_iter().map(|x| vm.new_double(mc, x)).collect()
                }
                ArrayDType::Int64 => {
                    let xs = receiver
                        .with_native_state::<NativeArray, _, _>(|a| {
                            (0..a.len())
                                .map(|i| read_i64(&a.data, i))
                                .collect::<Vec<i64>>()
                        })
                        .map_err(QuoinError::Other)?;
                    xs.into_iter().map(|x| vm.new_int(mc, x)).collect()
                }
            };
            Ok(vm.new_list(mc, values))
        })
        .doc(
            "The elements exploded into an ordinary List — leaving the packed, bulk world, so \
             reach for it at the edges rather than per element.\n\n\
             ```\n\
             (Array.ofInts:#(1 2 3)).toList     \"* -> #(1 2 3)\n\
             ```",
        )
        // `array.s` — a compact, structural description (not the elements).
        .instance_method("s", |vm, mc, receiver, _args| {
            let (sym, n) = receiver
                .with_native_state::<NativeArray, _, _>(|a| (a.dtype.symbol(), a.len()))
                .map_err(QuoinError::Other)?;
            Ok(vm.new_string(mc, format!("Array[{sym}; {n}]")))
        })
        .doc(
            "The display string: dtype and element count, not the elements.\n\n\
             ```\n\
             (Array.ofInts:#(1 2 3)).s     \"* -> Array[int64; 3]\n\
             ```",
        )
}

#[cfg(test)]
#[path = "array_tests.rs"]
mod array_tests;
