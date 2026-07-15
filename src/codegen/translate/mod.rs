//! Bytecode → Cranelift translation (docs/internal/AOT_ARCH.md §4.2, v0.2).
//!
//! One JIT module per candidate *group* (one class body / `.meta` extension).
//! Members that fail translation are refused individually (a retry loop
//! rebuilds the module without them); anything not provably translatable
//! refuses — never guards, never silently diverges.
//!
//! Value model (v0.2): scalars live in SSA registers; every GC value lives in
//! the frame's *slot window* on `vm.stack` (rooted by construction) and is
//! carried as an absolute slot index — registers never hold object pointers,
//! so fuel-checkpoint suspends still need no rooting. Dynamic values
//! (`AV::Dyn`) are slot-resident; `BranchIfNotBool` narrows them to scalars
//! on the hot path. Sends leave the compiled world through the `outcall`
//! helper (`call_method` native re-entry: depth-guarded, suspension-safe,
//! thrown-value-transparent); only *scalar-pure* siblings (all-scalar
//! signatures whose bodies touch no slots, transitively) keep the direct
//! native-call fast path — fib-shaped recursion.
//!
//! Semantics are pinned to `devirt_ops`: wrapping i64 add/sub/mul, `/`/`%`
//! raising only on a zero divisor (`i64::MIN / -1` wraps — Cranelift's `sdiv`
//! would trap, hence the explicit −1 path), f64 ops that never raise, and
//! f64 `%` via an imported helper (Cranelift has no `frem`).

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::Arc;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    AbiParam, Block as CBlock, BlockArg, InstBuilder, MemFlagsData, Signature, StackSlotData,
    StackSlotKind, Type, Value as CVal, types,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};

use crate::instruction::{Constant, Instruction, IntBinKind, StaticBlock};
use crate::runtime::elem_tag::ElemTag;
use crate::symbol::{Symbol, self_symbol};
use crate::value::NamespacedName;

use super::helpers::{self, KIND_BOOL, KIND_DOUBLE, KIND_INT, KIND_NIL, KIND_SLOT};
use super::{
    AOT_MAX_CALL_DEPTH, AotCandidate, AotEntry, AotKind, AotParam, AotRawFn, AotRet, AotRole,
    Refusal, RefusalKind, TAG_DEPTH, TAG_DIV_ZERO, TAG_INT_OVERFLOW,
};

mod abi;
mod driver;
mod emit;
mod model;
mod walk;

pub(super) use driver::compile_all;
// Siblings reach each other through these module-private globs (children see them
// via their own `use super::*`).
use abi::*;
use driver::*;
use model::*;

/// `%` on doubles: Rust's truncated remainder (what `devirt_ops::double_bin`
/// computes); Cranelift has no `frem`, so compiled code imports this.
unsafe extern "C" fn aot_fmod(a: f64, b: f64) -> f64 {
    a % b
}

/// Outcall arity cap: lane buffers are fixed-size native stack slots.
const MAX_OUTCALL_ARGS: usize = 8;
