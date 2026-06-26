//! Tests for the extension handle table — most importantly the **root-set property**:
//! a retained handle keeps its host `Value` alive across a real collection.

use crate::value::{ObjectPayload, Value};
use crate::vm::{VmOptions, VmState};
use gc_arena::{Arena, Mutation, Rootable};

type TestArena = Arena<Rootable![VmState<'_>]>;

fn new_arena() -> TestArena {
    Arena::<Rootable![VmState<'_>]>::new(|mc| VmState::new(mc, VmOptions::default()))
}

/// Allocate a pile of unreachable strings so a forced collection has real work to sweep —
/// if a handle's `Value` weren't rooted, it would be swept right alongside this garbage.
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

#[test]
fn global_handle_survives_collection() {
    let mut arena = new_arena();

    // Mint a handle to a fresh String and retain it (promote to global).
    let handle = arena.mutate_root(|mc, vm| {
        let epoch = vm.handle_table.begin_call();
        let value = vm.new_string(mc, "kept".to_string());
        let h = vm.handle_table.mint_local(value, epoch);
        vm.handle_table.retain(h).expect("retain");
        vm.handle_table.end_call(epoch); // local sweep must NOT touch the retained handle
        h
    });

    // Force full collections with real garbage in between.
    arena.mutate_root(|mc, vm| make_garbage(vm, mc));
    arena.finish_cycle();
    arena.finish_cycle();

    // The retained handle still resolves to the exact original string.
    arena.mutate_root(|_mc, vm| {
        let value = vm
            .handle_table
            .get(handle)
            .expect("retained handle must survive collection");
        assert_eq!(
            string_of(value).as_deref(),
            Some("kept"),
            "the rooted Value must be intact after GC"
        );
        assert_eq!(vm.handle_table.live_count(), 1);
    });
}

#[test]
fn local_handle_auto_released_on_end_call() {
    let mut arena = new_arena();
    arena.mutate_root(|mc, vm| {
        let epoch = vm.handle_table.begin_call();
        let value = vm.new_string(mc, "tmp".to_string());
        let h = vm.handle_table.mint_local(value, epoch);
        assert_eq!(vm.handle_table.live_count(), 1);
        assert!(vm.handle_table.get(h).is_ok());

        vm.handle_table.end_call(epoch);

        assert_eq!(vm.handle_table.live_count(), 0, "call exit sweeps locals");
        assert!(
            vm.handle_table.get(h).is_err(),
            "a swept local no longer resolves"
        );
    });
}

#[test]
fn released_global_frees_slot_and_stale_handle_fails() {
    let mut arena = new_arena();
    arena.mutate_root(|mc, vm| {
        let epoch = vm.handle_table.begin_call();
        let old = vm
            .handle_table
            .mint_local(vm.new_string(mc, "a".to_string()), epoch);
        vm.handle_table.retain(old).expect("retain");
        vm.handle_table.release(&[old]);
        assert!(
            vm.handle_table.get(old).is_err(),
            "released handle must not resolve"
        );
        assert_eq!(vm.handle_table.live_count(), 0);

        // Re-mint: the freed slot index is reused but its generation advanced, so the old
        // handle stays stale (no ABA aliasing) while the new handle resolves.
        let new = vm
            .handle_table
            .mint_local(vm.new_string(mc, "b".to_string()), epoch);
        assert_ne!(old, new, "reused slot must carry a fresh generation");
        assert!(vm.handle_table.get(old).is_err(), "old handle still stale");
        assert_eq!(
            string_of(vm.handle_table.get(new).unwrap()).as_deref(),
            Some("b")
        );
    });
}
