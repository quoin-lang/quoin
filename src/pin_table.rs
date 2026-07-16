//! The pin table — ONE traced side table for every native feature that must
//! retain a `Value` long-term.
//!
//! Native state (`NativeExtension`, `NativeServiceState`, worker handles) is
//! deliberately untraced (`NEEDS_TRACE = false` behind `AnyCollect`), so a
//! feature that needs to keep a `Value` alive used to add its own traced
//! `VmState` field — `recipe_chans`, `life_channels`, ... — each reinventing
//! the same slot vector, allocator, and release path. This table is the
//! consolidation: features **pin** a value and store the plain (`Copy`,
//! `Send`-friendly) [`PinId`] wherever they like; the table is an inline
//! field of [`VmState`](crate::vm::VmState) (the arena root), so
//! `#[derive(Collect)]` traces every live pin.
//!
//! Every pin carries a [`PinOwner`] — a `(kind, id)` tag — so a feature can
//! bulk-release everything it owns ([`unpin_owned`](PinTable::unpin_owned))
//! and `VM.stats` can report live pins per kind (the leak-accounting view).
//!
//! Like the handle table, no method here needs the `&Mutation` write-barrier
//! token: the table is owned by the arena root, which is fully traced every
//! cycle, so mutating root-owned fields is barrier-free.

use gc_arena::Collect;

use crate::value::Value;

/// A pinned value's ticket: plain data, safe to stash in untraced native
/// state. Only meaningful against the `VmState` that minted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub struct PinId(usize);

/// Who a pin belongs to: a feature kind plus that feature's own id, so a
/// teardown can release everything it pinned in one call and the stats view
/// can aggregate by kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub struct PinOwner {
    pub kind: &'static str,
    pub id: u64,
}

#[derive(Debug, Default, Collect)]
#[collect(no_drop)]
pub struct PinTable<'gc> {
    slots: Vec<Option<(PinOwner, Value<'gc>)>>,
    free: Vec<usize>,
}

impl<'gc> PinTable<'gc> {
    /// Root `value` and return its ticket.
    pub fn pin(&mut self, owner: PinOwner, value: Value<'gc>) -> PinId {
        match self.free.pop() {
            Some(i) => {
                self.slots[i] = Some((owner, value));
                PinId(i)
            }
            None => {
                self.slots.push(Some((owner, value)));
                PinId(self.slots.len() - 1)
            }
        }
    }

    /// The pinned value, `None` if the pin was released.
    pub fn get(&self, id: PinId) -> Option<Value<'gc>> {
        self.slots.get(id.0).and_then(|s| s.as_ref()).map(|s| s.1)
    }

    /// Release one pin, answering the value it held.
    pub fn unpin(&mut self, id: PinId) -> Option<Value<'gc>> {
        let slot = self.slots.get_mut(id.0)?.take()?;
        self.free.push(id.0);
        Some(slot.1)
    }

    /// Release every pin `owner` holds; answers how many were live.
    pub fn unpin_owned(&mut self, owner: PinOwner) -> usize {
        let mut released = 0;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if matches!(slot, Some((o, _)) if *o == owner) {
                *slot = None;
                self.free.push(i);
                released += 1;
            }
        }
        released
    }

    /// How many pins are live.
    pub fn live(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// `(kind, live count)` pairs, sorted by kind — the `VM.stats` view.
    pub fn counts_by_kind(&self) -> Vec<(&'static str, usize)> {
        let mut counts: Vec<(&'static str, usize)> = Vec::new();
        for (owner, _) in self.slots.iter().flatten() {
            match counts.iter_mut().find(|(k, _)| *k == owner.kind) {
                Some((_, n)) => *n += 1,
                None => counts.push((owner.kind, 1)),
            }
        }
        counts.sort_by_key(|(k, _)| *k);
        counts
    }
}

#[cfg(test)]
#[path = "pin_table_tests.rs"]
mod tests;
