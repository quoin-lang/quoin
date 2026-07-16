//! Tests for the pin table — the root-set property (a pinned `Value` survives
//! a real collection) plus the slot/owner bookkeeping.

use crate::pin_table::PinOwner;
use crate::value::{ObjectPayload, Value};
use crate::vm::{VmOptions, VmState};
use gc_arena::{Arena, Mutation, Rootable};

type TestArena = Arena<Rootable![VmState<'_>]>;

fn new_arena() -> TestArena {
    Arena::<Rootable![VmState<'_>]>::new(|mc| VmState::new(mc, VmOptions::default()))
}

fn make_garbage<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>) {
    for i in 0..512 {
        let _garbage = vm.new_string(mc, format!("garbage-{i}"));
    }
}

fn string_of(value: Value<'_>) -> Option<String> {
    match value {
        Value::Object(obj) => match &obj.borrow().payload {
            ObjectPayload::String(s) => Some(s.as_str().to_string()),
            _ => None,
        },
        _ => None,
    }
}

const OWNER: PinOwner = PinOwner {
    kind: "test",
    id: 7,
};

#[test]
fn pinned_value_survives_collection() {
    let mut arena = new_arena();
    let pin = arena.mutate_root(|mc, vm| {
        let value = vm.new_string(mc, "kept".to_string());
        vm.pins.pin(OWNER, value)
    });
    arena.mutate_root(|mc, vm| make_garbage(vm, mc));
    arena.finish_cycle();
    arena.finish_cycle();
    arena.mutate_root(|_mc, vm| {
        let value = vm.pins.get(pin).expect("pin must survive collection");
        assert_eq!(string_of(value).as_deref(), Some("kept"));
    });
}

#[test]
fn unpin_releases_and_reuses_the_slot() {
    let mut arena = new_arena();
    arena.mutate_root(|mc, vm| {
        let a = vm.new_string(mc, "a".to_string());
        let b = vm.new_string(mc, "b".to_string());
        let pin_a = vm.pins.pin(OWNER, a);
        assert_eq!(
            string_of(vm.pins.unpin(pin_a).unwrap()).as_deref(),
            Some("a")
        );
        assert!(
            vm.pins.get(pin_a).is_none(),
            "released pin must not resolve"
        );
        assert!(vm.pins.unpin(pin_a).is_none(), "double unpin is a no-op");
        // The freed slot is reused rather than growing the table.
        let pin_b = vm.pins.pin(OWNER, b);
        assert_eq!(pin_b, pin_a, "free slot must be reused");
        assert_eq!(vm.pins.live(), 1);
    });
}

#[test]
fn pin_or_find_dedupes_by_identity_and_index_ops_check_the_owner() {
    // The hosted-object contract over pins: same object -> same ticket (and
    // so the same wire id); a raw index resolves/releases ONLY through the
    // matching owner kind, so a wire-derived id can never touch a foreign pin.
    let mut arena = new_arena();
    arena.mutate_root(|mc, vm| {
        let obj = vm.new_string(mc, "obj".to_string());
        let first = vm.pins.pin_or_find(OWNER, obj);
        let again = vm.pins.pin_or_find(OWNER, obj);
        assert_eq!(first, again, "same object must answer the same pin");
        assert_eq!(vm.pins.live(), 1);

        let idx = crate::pin_table::PinTable::index(first);
        assert_eq!(
            string_of(vm.pins.get_at(idx, "test").unwrap()).as_deref(),
            Some("obj")
        );
        assert!(
            vm.pins.get_at(idx, "other-kind").is_none(),
            "a foreign kind must not resolve the slot"
        );
        vm.pins.unpin_at(idx, "other-kind");
        assert_eq!(vm.pins.live(), 1, "foreign unpin_at must be a no-op");
        vm.pins.unpin_at(idx, "test");
        assert_eq!(vm.pins.live(), 0);
        vm.pins.unpin_at(idx, "test"); // idempotent on a freed slot
    });
}

#[test]
fn unpin_owned_releases_exactly_that_owner() {
    let mut arena = new_arena();
    arena.mutate_root(|mc, vm| {
        let other = PinOwner {
            kind: "other",
            id: 1,
        };
        let v1 = vm.new_string(mc, "1".to_string());
        let v2 = vm.new_string(mc, "2".to_string());
        let v3 = vm.new_string(mc, "3".to_string());
        vm.pins.pin(OWNER, v1);
        vm.pins.pin(OWNER, v2);
        let keep = vm.pins.pin(other, v3);
        assert_eq!(vm.pins.unpin_owned(OWNER), 2);
        assert_eq!(vm.pins.live(), 1);
        assert_eq!(string_of(vm.pins.get(keep).unwrap()).as_deref(), Some("3"));
        assert_eq!(vm.pins.counts_by_kind(), vec![("other", 1)]);
    });
}
