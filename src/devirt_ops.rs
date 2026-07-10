//! Single source of truth for the semantics of every devirtualized operation.
//!
//! Each op is implemented once here, as an `#[inline]` free function on raw operands, and is
//! called by BOTH sides that would otherwise re-implement it:
//! - the devirtualized VM instructions (`IntAdd`/`DoubleDiv`/`ListGet`/… in `vm.rs`), and
//! - the native methods they shadow (`Integer#+:`, `List#at:`, … in `runtime/*.rs`).
//!
//! So the fast (devirt) path and the dispatched (native) path can't drift: the wrapping differs
//! (the VM builds a `Value`, the native builds via `Host::new_*`) but the *semantics* — integer
//! div-by-zero, f64 inf/NaN, list bounds, set membership — live in exactly one place.

use crate::error::QuoinError;
use crate::instruction::IntBinKind;
use crate::value::Value;

/// Result of an Integer binary op: an `Int` for arithmetic, a `Bool` for a comparison.
pub enum IntBinOut {
    Int(i64),
    Bool(bool),
}

/// Result of a Double binary op: a `Double` for arithmetic, a `Bool` for a comparison.
pub enum DoubleBinOut {
    Double(f64),
    Bool(bool),
}

fn division_by_zero() -> QuoinError {
    QuoinError::ArithmeticError("Division by zero".to_string())
}

fn integer_overflow() -> QuoinError {
    QuoinError::ArithmeticError("Integer overflow".to_string())
}

/// Integer binary op. Arithmetic that overflows `i64` raises a catchable "Integer overflow"
/// (it used to wrap in release and PANIC in debug — RELEASE_PREP Tier 4b); `/`/`%` raise
/// "Division by zero" on a zero divisor; comparisons yield a `Bool`. The AOT codegen's
/// `emit_int_bin` (translate.rs) mirrors these semantics instruction for instruction —
/// `codegen/tests.rs` sweeps the edges to hold the two together.
#[inline]
pub fn int_bin(kind: IntBinKind, a: i64, b: i64) -> Result<IntBinOut, QuoinError> {
    use IntBinKind::*;
    use IntBinOut::{Bool, Int};
    Ok(match kind {
        Add => Int(a.checked_add(b).ok_or_else(integer_overflow)?),
        Sub => Int(a.checked_sub(b).ok_or_else(integer_overflow)?),
        Mul => Int(a.checked_mul(b).ok_or_else(integer_overflow)?),
        // Only a zero *divisor* is a division error. The one overflowing quotient,
        // `i64::MIN / -1` (which PANICS in plain Rust `/` in every build — LLVM UB
        // otherwise, BUGS.md Finding 2), raises "Integer overflow" like the other ops.
        Div => {
            if b == 0 {
                return Err(division_by_zero());
            }
            if b == -1 {
                Int(a.checked_neg().ok_or_else(integer_overflow)?)
            } else {
                Int(a / b)
            }
        }
        Mod => {
            if b == 0 {
                return Err(division_by_zero());
            }
            if b == -1 { Int(0) } else { Int(a % b) }
        }
        Lt => Bool(a < b),
        Le => Bool(a <= b),
        Gt => Bool(a > b),
        Ge => Bool(a >= b),
        Eq => Bool(a == b),
        Ne => Bool(a != b),
    })
}

/// Double binary op — plain IEEE-754 f64: `/`/`%` yield inf/NaN on a zero divisor (never raise,
/// unlike `int_bin`); `==` is f64 equality (`NaN != NaN`); comparisons yield a `Bool`.
#[inline]
pub fn double_bin(kind: IntBinKind, a: f64, b: f64) -> DoubleBinOut {
    use DoubleBinOut::{Bool, Double};
    use IntBinKind::*;
    match kind {
        Add => Double(a + b),
        Sub => Double(a - b),
        Mul => Double(a * b),
        Div => Double(a / b),
        Mod => Double(a % b),
        Lt => Bool(a < b),
        Le => Bool(a <= b),
        Gt => Bool(a > b),
        Ge => Bool(a >= b),
        Eq => Bool(a == b),
        Ne => Bool(a != b),
    }
}

/// `List#at:` — the element at `i`, or `None` (→ `nil`) if out of bounds. Negative indices are
/// out of bounds (no from-the-end indexing).
#[inline]
pub fn list_get<'gc>(vec: &[Value<'gc>], i: i64) -> Option<Value<'gc>> {
    usize::try_from(i).ok().and_then(|i| vec.get(i).copied())
}

/// `List#at:put:` — set the element at `i`, or an `IndexError` if out of bounds.
#[inline]
pub fn list_set<'gc>(vec: &mut [Value<'gc>], i: i64, value: Value<'gc>) -> Result<(), QuoinError> {
    match usize::try_from(i).ok().and_then(|i| vec.get_mut(i)) {
        Some(slot) => {
            *slot = value;
            Ok(())
        }
        None => Err(QuoinError::IndexError {
            index: i,
            len: vec.len() as i64,
            msg: format!(
                "Index out of bounds: index is {}, but length is {}",
                i,
                vec.len()
            ),
        }),
    }
}

// Note: Set has no verb here. Native `Set#contains?:`/`add:` dispatch `==:` per element (so
// membership is structural for List/Map elements and custom for user classes) — a raw
// `Value ==` verb would silently diverge, so Set is intentionally not devirtualized.
