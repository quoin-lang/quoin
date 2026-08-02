//! Always-on compile-time decision counters — the `'compile'` section of
//! `VM.stats` (docs/internal/AOT_ARCH.md §9.7, issue #70).
//!
//! These count what the **bytecode compiler** decided, which is a different tier
//! from the `'aot'` section's native translator: every one of these fires under
//! `QN_AOT=0` too, because inlining and typed devirtualization are unconditional
//! front-end lowerings, not part of the AOT tier.
//!
//! Process-global atomics rather than per-VM fields, matching `codegen`'s
//! `TOTAL_COMPILED` / `TOTAL_REFUSED` idiom (`src/codegen/refusal.rs`): the
//! compiler runs from wherever a unit is loaded, with no `VmState` in hand at
//! the decision site. Monotonic for the process lifetime — a fresh subprocess is
//! how a test gets a zeroed baseline.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Self-sends and explicit-receiver sends whose callee body was spliced into the
/// caller instead of dispatched (`compiler/inlining.rs`, Phase 5).
pub(crate) static TOTAL_INLINED: AtomicUsize = AtomicUsize::new(0);

/// Sends lowered to a typed opcode because both operand types were statically
/// known — the scalar `Int*`/`Double*` arithmetic and comparison ops, and the
/// `List`/`Map` accessors (`compiler/lowering.rs`, `compiler/devirt.rs`).
pub(crate) static TOTAL_DEVIRT_OPS: AtomicUsize = AtomicUsize::new(0);

/// Control-flow and iteration protocols fused into native jumps/loops rather
/// than block sends: `if:`/`if:else:`, `whileTrue:`, `each:`, and fused
/// instantiation (`compiler/devirt.rs`). Same family of decision as the two
/// above — a send that stopped being a send — but counted apart because it
/// removes a *block allocation*, not just a dispatch.
pub(crate) static TOTAL_CONTROL_FLOW_INLINED: AtomicUsize = AtomicUsize::new(0);

#[inline]
pub(crate) fn note_inlined() {
    TOTAL_INLINED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub(crate) fn note_devirt_op() {
    TOTAL_DEVIRT_OPS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub(crate) fn note_control_flow_inlined() {
    TOTAL_CONTROL_FLOW_INLINED.fetch_add(1, Ordering::Relaxed);
}

/// `(inlined, devirt_ops, control_flow_inlined)` across the process so far.
pub fn totals() -> (usize, usize, usize) {
    (
        TOTAL_INLINED.load(Ordering::Relaxed),
        TOTAL_DEVIRT_OPS.load(Ordering::Relaxed),
        TOTAL_CONTROL_FLOW_INLINED.load(Ordering::Relaxed),
    )
}
