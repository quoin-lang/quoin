//! The extension handle table — the GC-boundary crux of the out-of-process extension
//! tier (Tier 1; `docs/FUTURE_EXT_ARCH.md` §2).
//!
//! An extension lives in another process and can never touch the host heap directly: a
//! `Value<'gc>` carries a lifetime brand that cannot cross a dynamic boundary, and a bare
//! `Gc` held across a yield is unsound (`no_gc_across_yield`). So every host value the
//! extension holds is represented as an opaque `u64` **handle** indexing this table, and
//! the extension calls back into the host to do anything GC-related.
//!
//! The table **is a GC root set**: it is an inline field of [`VmState`](crate::vm::VmState)
//! (itself the arena root), so `#[derive(Collect)]` traces every live slot — a handle the
//! extension holds keeps its host `Value` alive across collections, exactly like JNI
//! local/global refs or a Ruby `RTypedData` mark function.
//!
//! Lifetime is JNI-style:
//! - **Local** — the default. Minted during one extension call and auto-released when that
//!   call returns ([`begin_call`](HandleTable::begin_call) / [`end_call`](HandleTable::end_call)),
//!   so a call's transient handles generate no release traffic.
//! - **Global** — an extension [`retain`](HandleTable::retain)s a local handle to hold it
//!   across calls, then [`release`](HandleTable::release)s it explicitly (batched).
//!
//! Slots are generation-tagged: a handle is `(generation << 32) | index`, and a stale
//! handle (its slot freed and reused) fails to resolve instead of aliasing a new value.
//!
//! No method here needs the `&Mutation` write-barrier token: the table is owned by the
//! arena *root*, which is fully traced every cycle, so mutating root-owned fields is barrier
//! -free (the same reason `VmState.stack`/`sched.tasks` are pushed without `mc`). Minting the
//! `Value` to store still needs `mc`, but inserting it here does not.

use gc_arena::Collect;

use crate::value::Value;

/// The reserved null handle. No real handle is ever this value (slot generations start at 1
/// and never wrap back to 0), so the protocol can use `0` to mean "no handle" — e.g. the
/// optional block handle on a `Call`. This is JNI's null `jobject` / Lua's `LUA_NOREF`.
pub const NULL_HANDLE: u64 = 0;

/// Pack a slot index + generation into the opaque `u64` handle the extension holds.
fn handle_of(index: u32, generation: u32) -> u64 {
    ((generation as u64) << 32) | (index as u64)
}

/// Unpack a handle into `(index, generation)`.
fn split(handle: u64) -> (u32, u32) {
    (handle as u32, (handle >> 32) as u32)
}

/// The lifetime scope of an occupied slot (or `Free` for an empty one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HandleScope {
    /// Empty slot, awaiting reuse via the free list.
    Free,
    /// Call-scoped: auto-released when the call with this epoch ends.
    Local(u32),
    /// Retained across calls; released only by an explicit `release`.
    Global,
}

#[derive(Collect)]
#[collect(no_drop)]
struct HandleSlot<'gc> {
    /// The rooted host value, or `None` when the slot is free.
    value: Option<Value<'gc>>,
    #[collect(require_static)]
    scope: HandleScope,
    /// Bumped on every free, so a stale handle to this slot no longer resolves.
    #[collect(require_static)]
    generation: u32,
    /// The extension that minted this handle, so a dead/dropped peer's handles can be
    /// bulk-released (`release_for_ext`). Meaningful only while occupied.
    #[collect(require_static)]
    ext_id: u64,
}

/// The extension handle table. An inline, GC-traced field of [`VmState`](crate::vm::VmState).
#[derive(Collect, Default)]
#[collect(no_drop)]
pub struct HandleTable<'gc> {
    slots: Vec<HandleSlot<'gc>>,
    /// Indices of free slots, for reuse before growing `slots`.
    #[collect(require_static)]
    free: Vec<u32>,
    /// Monotonic call-epoch counter; each `begin_call` mints a fresh epoch id.
    #[collect(require_static)]
    epoch: u32,
}

impl<'gc> HandleTable<'gc> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a (possibly re-entrant) extension call, returning its unique epoch id.
    /// Handles minted under this epoch via [`mint_local`](Self::mint_local) are swept by the
    /// matching [`end_call`](Self::end_call).
    pub fn begin_call(&mut self) -> u32 {
        self.epoch = self.epoch.wrapping_add(1);
        self.epoch
    }

    /// Sweep every still-local handle minted under `epoch` (auto-release on call exit).
    /// Handles promoted to global via [`retain`](Self::retain) are left alone.
    pub fn end_call(&mut self, epoch: u32) {
        for i in 0..self.slots.len() {
            if self.slots[i].scope == HandleScope::Local(epoch) {
                self.free_index(i as u32);
            }
        }
    }

    /// Mint a call-local handle for `value` under the given call `epoch`.
    pub fn mint_local(&mut self, value: Value<'gc>, epoch: u32, ext_id: u64) -> u64 {
        self.alloc(value, HandleScope::Local(epoch), ext_id)
    }

    /// Release every handle (local or global) owned by `ext_id` — for a dead or dropped
    /// extension, so the host Values it held drop their GC roots instead of leaking until VM exit.
    pub fn release_for_ext(&mut self, ext_id: u64) {
        for i in 0..self.slots.len() {
            if self.slots[i].value.is_some() && self.slots[i].ext_id == ext_id {
                self.free_index(i as u32);
            }
        }
    }

    /// Resolve a handle to its host value, or `Err` if it is invalid, stale, or released.
    pub fn get(&self, handle: u64) -> Result<Value<'gc>, String> {
        let (index, generation) = split(handle);
        let slot = self
            .slots
            .get(index as usize)
            .filter(|s| s.generation == generation)
            .ok_or_else(|| format!("extension handle {handle} is invalid or stale"))?;
        slot.value
            .ok_or_else(|| format!("extension handle {handle} has been released"))
    }

    /// Promote a call-local handle to retained (global), so it survives the originating call.
    pub fn retain(&mut self, handle: u64) -> Result<(), String> {
        let (index, generation) = split(handle);
        let slot = self
            .slots
            .get_mut(index as usize)
            .filter(|s| s.generation == generation && s.value.is_some())
            .ok_or_else(|| format!("retain: extension handle {handle} is invalid or released"))?;
        slot.scope = HandleScope::Global;
        Ok(())
    }

    /// Release handles (batched). Unknown/stale handles are ignored — release is idempotent.
    pub fn release(&mut self, handles: &[u64]) {
        for &handle in handles {
            let (index, generation) = split(handle);
            let still_live = self
                .slots
                .get(index as usize)
                .is_some_and(|s| s.generation == generation && s.value.is_some());
            if still_live {
                self.free_index(index);
            }
        }
    }

    /// Number of currently-occupied slots (test/inspection helper).
    pub fn live_count(&self) -> usize {
        self.slots.iter().filter(|s| s.value.is_some()).count()
    }

    fn alloc(&mut self, value: Value<'gc>, scope: HandleScope, ext_id: u64) -> u64 {
        let index = if let Some(i) = self.free.pop() {
            let slot = &mut self.slots[i as usize];
            slot.value = Some(value);
            slot.scope = scope;
            slot.ext_id = ext_id;
            i
        } else {
            let i = self.slots.len() as u32;
            // Generations start at 1, so slot 0's first handle isn't 0 (the null sentinel).
            self.slots.push(HandleSlot {
                value: Some(value),
                scope,
                generation: 1,
                ext_id,
            });
            i
        };
        handle_of(index, self.slots[index as usize].generation)
    }

    fn free_index(&mut self, index: u32) {
        let slot = &mut self.slots[index as usize];
        slot.value = None;
        slot.scope = HandleScope::Free;
        // Skip generation 0 on wrap so a recycled slot 0 never yields the null handle.
        slot.generation = match slot.generation.wrapping_add(1) {
            0 => 1,
            g => g,
        };
        self.free.push(index);
    }
}

#[cfg(test)]
#[path = "handle_table_tests.rs"]
mod handle_table_tests;
