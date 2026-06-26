//! Low-level layout tests for `Array`: the buffer must be Apache Arrow's non-nullable primitive
//! layout (contiguous little-endian values), so an extension reads the same bytes as numpy/pyarrow
//! with zero conversion. The Quoin suite (`qnlib/tests/42-array.qn`) covers behavior; this pins
//! the wire-compatible byte layout the Slice-6b data plane relies on.

use super::*;

#[test]
fn float64_layout_is_arrow_little_endian() {
    let data: Vec<u8> = [1.0f64, 2.5, -3.0]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    let arr = NativeArray {
        dtype: ArrayDType::Float64,
        data,
    };
    assert_eq!(arr.len(), 3);
    assert_eq!(read_f64(&arr.data, 0), 1.0);
    assert_eq!(read_f64(&arr.data, 1), 2.5);
    assert_eq!(read_f64(&arr.data, 2), -3.0);
    // The first element's bytes are exactly numpy/Arrow's little-endian f64 layout.
    assert_eq!(&arr.data[0..8], &1.0f64.to_le_bytes());
}

#[test]
fn int64_layout_is_arrow_little_endian() {
    let data: Vec<u8> = [7i64, -8].iter().flat_map(|x| x.to_le_bytes()).collect();
    let arr = NativeArray {
        dtype: ArrayDType::Int64,
        data,
    };
    assert_eq!(arr.len(), 2);
    assert_eq!(read_i64(&arr.data, 0), 7);
    assert_eq!(read_i64(&arr.data, 1), -8);
    assert_eq!(&arr.data[0..8], &7i64.to_le_bytes());
}

#[test]
fn empty_array_has_zero_length() {
    let arr = NativeArray {
        dtype: ArrayDType::Float64,
        data: Vec::new(),
    };
    assert_eq!(arr.len(), 0);
}
