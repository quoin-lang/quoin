//! Runtime helpers called from compiled code (docs/AOT_ARCH.md v0.2).
//!
//! Compiled frames keep every GC value in a *slot window* on `vm.stack`
//! (rooted by construction — registers carry only scalars and slot indices),
//! so these helpers take absolute stack indices, never raw object pointers.
//! Values cross the ABI as `(kind, bits)` lanes: scalars by value, objects by
//! slot index. Errors go through `VmState::aot_pending_error` (`TAG_ERR`);
//! a thrown Quoin value travels exactly as it does across any native
//! boundary — `QuoinError::Thrown` with the value GC-rooted in
//! `exceptions.active`.
//!
//! Lifetime erasure: `vm`/`mc` cross the ABI as opaque pointers and are
//! reconstituted here, the same pattern as `VMContext` and the stored
//! yielder. No `'gc` value is held across a suspend inside these helpers
//! except via GC-visible homes (`vm.stack`, `active_native_args`).

use std::ffi::c_void;

use gc_arena::Mutation;

use crate::devirt_ops;
use crate::error::QuoinError;
use crate::runtime::elem_tag;
use crate::runtime::list::NativeListState;
use crate::symbol::Symbol;
use crate::value::NamespacedName;
use crate::value::{ObjectPayload, Value};
use crate::vm::VmState;
#[allow(unused_imports)]
use gc_arena::Gc;
#[allow(unused_imports)]
use gc_arena::lock::RefLock;

use super::{TAG_ERR, TAG_OK};

/// Value-lane kinds in the compiled ABI. Kept DISJOINT from the nonzero
/// status TAGs (`super::TAG_*`, 0x11+) that share the same integer ABI —
/// see the note at the TAG definitions.
pub const KIND_INT: i64 = 0;
pub const KIND_DOUBLE: i64 = 1;
pub const KIND_BOOL: i64 = 2;
pub const KIND_SLOT: i64 = 3;
pub const KIND_NIL: i64 = 4;

/// Reconstitute the erased `(vm, mc)` pair for one helper call.
///
/// # Safety
/// Both pointers must be the live pair passed into the compiled frame by
/// `invoke` for the current resume segment (established `VMContext` pattern).
unsafe fn vm_mc<'a>(
    vm: *mut c_void,
    mc: *const c_void,
) -> (&'a mut VmState<'static>, &'a Mutation<'static>) {
    unsafe {
        (
            &mut *(vm as *mut VmState<'static>),
            &*(mc as *const Mutation<'static>),
        )
    }
}

fn decode<'gc>(vm: &VmState<'gc>, kind: i64, bits: i64) -> Value<'gc> {
    match kind {
        KIND_INT => Value::Int(bits),
        KIND_DOUBLE => Value::Double(f64::from_bits(bits as u64)),
        KIND_BOOL => Value::Bool(bits != 0),
        KIND_SLOT => vm.stack[bits as usize],
        _ => Value::Nil,
    }
}

fn store_err(vm: &mut VmState<'_>, e: QuoinError) -> u8 {
    vm.aot_pending_error = Some(e);
    TAG_ERR
}

fn invariant(vm: &mut VmState<'_>, what: &str) -> u8 {
    store_err(
        vm,
        QuoinError::Other(format!("AOT invariant violated: {what} (please report)")),
    )
}

/// The ONE way a helper writes a result into the compiled frame's slot
/// window. Checked: a slot index past the stack top means the stack was
/// truncated under us (an unwind the calling protocol failed to surface —
/// the S5 absorb-at-baseline bug aborted the process exactly here), and a
/// catchable AOT error beats a panic that cannot unwind across the
/// Cranelift frames.
#[inline(always)]
fn slot_write<'gc>(vm: &mut VmState<'gc>, idx: i64, v: Value<'gc>) -> u8 {
    match vm.stack.get_mut(idx as usize) {
        Some(slot) => {
            *slot = v;
            TAG_OK
        }
        None => invariant(vm, "slot write past the stack top"),
    }
}

/// `vm.stack[idx] = decode(kind, bits)` — writes into the compiled frame's
/// slot window (or copies slot→slot when `kind == KIND_SLOT`).
pub(super) unsafe extern "C" fn slot_set(
    vm: *mut c_void,
    mc: *const c_void,
    idx: i64,
    kind: i64,
    bits: i64,
) -> u8 {
    let (vm, _mc) = unsafe { vm_mc(vm, mc) };
    let v = decode(vm, kind, bits);
    slot_write(vm, idx, v)
}

/// Read `vm.stack[idx]` as `(kind, bits)`: scalars by value; anything else
/// (objects) reports `KIND_SLOT` with the index itself, so the value stays
/// rooted where it is.
pub(super) unsafe extern "C" fn slot_peek(
    vm: *mut c_void,
    mc: *const c_void,
    idx: i64,
    out_bits: *mut i64,
) -> i64 {
    let (vm, _mc) = unsafe { vm_mc(vm, mc) };
    let (kind, bits) = match vm.stack[idx as usize] {
        Value::Int(i) => (KIND_INT, i),
        Value::Double(d) => (KIND_DOUBLE, d.to_bits() as i64),
        Value::Bool(b) => (KIND_BOOL, b as i64),
        Value::Nil => (KIND_NIL, 0),
        _ => (KIND_SLOT, idx),
    };
    unsafe { *out_bits = bits };
    kind
}

/// `#()` — a fresh empty list into `out_idx`.
pub(super) unsafe extern "C" fn list_new(vm: *mut c_void, mc: *const c_void, out_idx: i64) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let list = vm.new_list(mc, Vec::new());
    slot_write(vm, out_idx, list)
}

/// `#(a b …)` — a list built from `n` value lanes.
pub(super) unsafe extern "C" fn list_from(
    vm: *mut c_void,
    mc: *const c_void,
    out_idx: i64,
    n: i64,
    kinds: *const i64,
    bits: *const i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let mut elems = Vec::with_capacity(n as usize);
    for i in 0..n as usize {
        let (k, b) = unsafe { (*kinds.add(i), *bits.add(i)) };
        elems.push(decode(vm, k, b));
    }
    let list = vm.new_list(mc, elems);
    slot_write(vm, out_idx, list)
}

/// `list.add:value` — mirrors the interpreter's `ListPush` arm. The compiler
/// only emits `ListPush` where the receiver is proven `List`, so a non-list
/// here is an invariant violation, not a fallback.
pub(super) unsafe extern "C" fn list_push(
    vm: *mut c_void,
    mc: *const c_void,
    list_idx: i64,
    kind: i64,
    bits: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let value = decode(vm, kind, bits);
    let receiver = vm.stack[list_idx as usize];
    // Mirrors the interpreter's ListPush arm: untagged and scalar-tag checks
    // decide inside the one borrow; only Class tags escalate to the walk.
    let res = receiver.with_native_state_mut::<NativeListState, _, _>(mc, |l| match l.elem {
        None => {
            l.get_vec_mut().push(value);
            Some(Ok(()))
        }
        Some(t) => match t.matches_value(&value) {
            Some(true) => {
                l.get_vec_mut().push(value);
                Some(Ok(()))
            }
            Some(false) => Some(Err(elem_tag::elem_type_error("List", t, &value, None))),
            None => None,
        },
    });
    match res {
        Ok(Some(Ok(()))) => TAG_OK,
        Ok(Some(Err(e))) => store_err(vm, e),
        Ok(None) => match vm.tagged_list_push(mc, receiver, value) {
            Ok(()) => TAG_OK,
            Err(e) => store_err(vm, e),
        },
        Err(_) => invariant(vm, "ListPush on a non-list receiver"),
    }
}

/// `list.at:i` — `devirt_ops::list_get` semantics (nil out of bounds).
pub(super) unsafe extern "C" fn list_get(
    vm: *mut c_void,
    mc: *const c_void,
    list_idx: i64,
    index: i64,
    out_idx: i64,
) -> u8 {
    let (vm, _mc) = unsafe { vm_mc(vm, mc) };
    let receiver = vm.stack[list_idx as usize];
    let out = receiver.with_native_state::<NativeListState, _, _>(|l| {
        devirt_ops::list_get(l.get_vec(), index).unwrap_or(Value::Nil)
    });
    match out {
        Ok(v) => slot_write(vm, out_idx, v),
        Err(_) => invariant(vm, "ListGet on a non-list receiver"),
    }
}

/// `list.count` — the fused `each:` loop bound (B1). Reached only behind a
/// proven-List guard (`BranchIfNotList` in the translator), so the value at
/// `list_idx` is a native List by construction; the defensive 0 keeps a
/// violated invariant from reading garbage (an empty loop, never UB).
pub(super) unsafe extern "C" fn list_len(vm: *mut c_void, mc: *const c_void, list_idx: i64) -> i64 {
    let (vm, _mc) = unsafe { vm_mc(vm, mc) };
    let receiver = vm.stack[list_idx as usize];
    receiver
        .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len() as i64)
        .unwrap_or(0)
}

/// Read a compiled block template's FREE variable through its closure's real
/// `EnvFrame` chain (B3a). Slot `block_idx` holds the block object (rooted by
/// `invoke_block`). A missing name mirrors the interpreter's `LoadLocal`
/// exactly: nil, not an error.
pub(super) unsafe extern "C" fn env_get(
    vm: *mut c_void,
    mc: *const c_void,
    block_idx: i64,
    sym: *const crate::symbol::Symbol,
    out_idx: i64,
) -> u8 {
    let (vm, _mc) = unsafe { vm_mc(vm, mc) };
    let sym = unsafe { *sym };
    let bv = vm.stack[block_idx as usize];
    let Value::Object(obj) = bv else {
        return invariant(vm, "env_get on a non-block slot");
    };
    let ObjectPayload::Block(block) = &obj.borrow().payload else {
        return invariant(vm, "env_get on a non-block slot");
    };
    let val = block
        .parent_env
        .and_then(|env| crate::value::EnvFrame::get(env, sym))
        .unwrap_or(Value::Nil);
    slot_write(vm, out_idx, val)
}

/// Write a compiled block template's FREE variable through its closure's
/// `EnvFrame` chain (B3a) — the same shared cell the enclosing frame reads,
/// so `sum = sum + x` mutates the caller's binding exactly as interpreted.
/// The name is compile-time-scoped at the original site, so a missing
/// binding is a broken invariant, not a user error.
pub(super) unsafe extern "C" fn env_set(
    vm: *mut c_void,
    mc: *const c_void,
    block_idx: i64,
    sym: *const crate::symbol::Symbol,
    kind: i64,
    bits: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let sym = unsafe { *sym };
    let val = decode(vm, kind, bits);
    let bv = vm.stack[block_idx as usize];
    let Value::Object(obj) = bv else {
        return invariant(vm, "env_set on a non-block slot");
    };
    let ObjectPayload::Block(block) = &obj.borrow().payload else {
        return invariant(vm, "env_set on a non-block slot");
    };
    let Some(env) = block.parent_env else {
        return invariant(vm, "env_set: compiled block has no captured environment");
    };
    if crate::value::EnvFrame::set(env, mc, sym, val) {
        TAG_OK
    } else {
        invariant(vm, "env_set: captured variable has no binding")
    }
}

/// The per-element block-invocation seam (B3a): a `valueWithSelfOrArg:` send
/// from compiled code. Registry hit (a compiled block template) → direct
/// native call; miss → the interpreted block body; a non-block receiver →
/// the full send (a custom class may define `valueWithSelfOrArg:`).
// `recv`/`arg`/`block` are copies of values ROOTED in the calling compiled
// frame's slot window on `vm.stack` for the whole outcall (the AOT rooting
// convention, AOT_ARCH §9) — safe across the compiled/interpreted block
// invocations below, which the lint's span heuristic can't see.
#[allow(no_gc_across_yield)]
pub(super) unsafe extern "C" fn block_call(
    vm: *mut c_void,
    mc: *const c_void,
    tid: i64,
    ip: i64,
    bc_len: i64,
    recv_kind: i64,
    recv_bits: i64,
    arg_kind: i64,
    arg_bits: i64,
    out_idx: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let recv = decode(vm, recv_kind, recv_bits);
    let arg = decode(vm, arg_kind, arg_bits);
    let block = match recv {
        Value::Object(obj) => match &obj.borrow().payload {
            ObjectPayload::Block(b) => Some(*b),
            _ => None,
        },
        _ => None,
    };
    let Some(block) = block else {
        // Not a block: the ordinary (cached) send — dispatch decides: custom
        // classes, or MNU with the exact interpreter shape.
        return match vm.call_method_cached(
            mc,
            tid as u32,
            ip as usize,
            bc_len as usize,
            recv,
            crate::symbol::Symbol::intern("valueWithSelfOrArg:"),
            vec![arg],
            None,
        ) {
            Ok(v) => slot_write(vm, out_idx, v),
            Err(e) => store_err(vm, e),
        };
    };
    let self_val = super::self_or_arg_self(&block, arg);
    if vm.aot.outcall_nesting < super::spec::MAX_OUTCALL_NESTING
        && let Some(tid) = block.template.template_id
        && let Some(entry) = super::block_entry_for(vm, tid)
    {
        match super::invoke_block(vm, mc, entry, recv, arg, self_val) {
            super::AotOutcome::Value(v) => {
                return slot_write(vm, out_idx, v);
            }
            super::AotOutcome::Err(e) => return store_err(vm, e),
            super::AotOutcome::Bail => {}
        }
    }
    // Interpreted fallback: same self-or-arg answer (a parameterless block
    // gets the item as self; a parameterized one keeps lexical self).
    let self_opt = block.template.param_syms.is_empty().then_some(arg);
    match vm.execute_block(mc, block, vec![arg], self_opt) {
        Ok(v) => slot_write(vm, out_idx, v),
        Err(e) => store_err(vm, e),
    }
}

/// Fused-instantiation guard (M2, `BranchIfNotPlainNew`) for a compiled site,
/// sharing the interpreted site's `(template, ip)` cache cell. Returns 1 =
/// plain new (hot path), 0 = cold path. Never errors — every failure mode is
/// "cold", where the real send raises it.
pub(super) unsafe extern "C" fn plain_new_check(
    vm: *mut c_void,
    mc: *const c_void,
    tid: i64,
    ip: i64,
    bc_len: i64,
    recv_kind: i64,
    recv_bits: i64,
) -> i64 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let receiver = decode(vm, recv_kind, recv_bits);
    let cell =
        (tid as u32 != u32::MAX).then(|| (vm.ic_cell_by_id(mc, tid as u32), bc_len as usize));
    i64::from(vm.plain_new_check_cached(mc, cell, ip as usize, receiver))
}

/// Fused-instantiation body (M2, `NewWithFields`) for a compiled site: decode
/// the receiver class and the n field values, push `[class, v1..vn]` as a
/// rooted stack window (the A2d pattern — the init chain can park), and run
/// the interpreter's own core; the finished object lands in `out_idx`.
pub(super) unsafe extern "C" fn new_with_fields(
    vm: *mut c_void,
    mc: *const c_void,
    names: *const Symbol,
    n: i64,
    recv_kind: i64,
    recv_bits: i64,
    kinds: *const i64,
    bits: *const i64,
    out_idx: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let names = unsafe { std::slice::from_raw_parts(names, n as usize) };
    let recv_at = vm.stack.len();
    let receiver = decode(vm, recv_kind, recv_bits);
    vm.stack.push(receiver);
    for i in 0..n as usize {
        let (k, b) = unsafe { (*kinds.add(i), *bits.add(i)) };
        let v = decode(vm, k, b);
        vm.stack.push(v);
    }
    match vm.exec_new_with_fields(mc, recv_at + 1, names) {
        Ok(()) => {
            // The core replaced the window with the finished object.
            let v = vm.stack[recv_at];
            vm.stack.truncate(recv_at);
            slot_write(vm, out_idx, v)
        }
        Err(e) => {
            // The S1/finish_frame rule: a `^^` escaping an init has already
            // truncated to its target and delivered its value — never chop it.
            if !matches!(e, crate::error::QuoinError::NonLocalReturn) {
                vm.stack.truncate(recv_at.min(vm.stack.len()));
            }
            store_err(vm, e)
        }
    }
}

/// Materialize a closure from a compiled frame's cold path (B3b): a fresh
/// snapshot `EnvFrame` (populated by the `closure_bind` calls the translator
/// emits right after) + the leaked template + the registry-shared IC cell —
/// the same shape `block_from_template` builds for interpreted frames.
/// Read-only-capture semantics are the translator's gate; a `^^` in the nest
/// is fine (S5) — `want_home != 0` iff the nest contains one, and then the
/// closure's home is the invoking compiled frame, carried in
/// `vm.aot.home_frame_id` and addressable through `vm.aot.frame_marks`
/// (a `^^`-free nest never consults `enclosing_method_id`, and its invoking
/// frame skips the S5 bookkeeping entirely, so the field would be stale).
pub(super) unsafe extern "C" fn make_closure(
    vm: *mut c_void,
    mc: *const c_void,
    tmpl: *const std::sync::Arc<crate::instruction::StaticBlock>,
    out_idx: i64,
    want_home: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let tmpl = unsafe { &*tmpl };
    // Chain the snapshot to the invoking frame's enclosing environment, so a
    // nested materialized closure's free names resolve through the FULL
    // lexical chain, exactly as interpreted (the webapp `path` lesson).
    let env = crate::gcl!(mc, crate::value::EnvFrame::new(vm.aot.enclosing_env));
    let inline_cache = vm.ic_cell_for(mc, tmpl);
    let v = vm.new_block(
        mc,
        crate::value::Block {
            template: tmpl.clone(),
            parent_env: Some(env),
            enclosing_method_id: if want_home != 0 {
                vm.aot.home_frame_id
            } else {
                None
            },
            decl_block: None,
            inline_cache,
        },
    );
    slot_write(vm, out_idx, v)
}

/// Bind one captured value into a `make_closure`-built snapshot env. The
/// block sits rooted in its slot throughout construction.
pub(super) unsafe extern "C" fn closure_bind(
    vm: *mut c_void,
    mc: *const c_void,
    block_idx: i64,
    sym: *const crate::symbol::Symbol,
    kind: i64,
    bits: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let sym = unsafe { *sym };
    let val = decode(vm, kind, bits);
    let bv = vm.stack[block_idx as usize];
    let Value::Object(obj) = bv else {
        return invariant(vm, "closure_bind on a non-block slot");
    };
    let ObjectPayload::Block(block) = &obj.borrow().payload else {
        return invariant(vm, "closure_bind on a non-block slot");
    };
    let Some(env) = block.parent_env else {
        return invariant(vm, "closure_bind: snapshot env missing");
    };
    env.borrow_mut(mc).bind(sym, val);
    TAG_OK
}

/// `@name` read in a compiled frame (S3): the receiver is the frame's slot-0
/// value; the slot cache is the SHARED `(template_id, ip)` cell. Missing /
/// undeclared / non-object => nil, exactly as interpreted.
pub(super) unsafe extern "C" fn field_get(
    vm: *mut c_void,
    mc: *const c_void,
    tid: i64,
    ip: i64,
    bc_len: i64,
    self_idx: i64,
    name_ptr: *const u8,
    name_len: i64,
    out_idx: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let self_val = vm.stack[self_idx as usize];
    let v = vm.field_load_cached(mc, tid as u32, ip as usize, bc_len as usize, self_val, name);
    slot_write(vm, out_idx, v)
}

/// `@name = v` in a compiled frame (S3) — same shared cache; undeclared
/// fields raise the interpreter's exact errors.
pub(super) unsafe extern "C" fn field_set(
    vm: *mut c_void,
    mc: *const c_void,
    tid: i64,
    ip: i64,
    bc_len: i64,
    self_idx: i64,
    name_ptr: *const u8,
    name_len: i64,
    kind: i64,
    bits: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let self_val = vm.stack[self_idx as usize];
    let val = decode(vm, kind, bits);
    match vm.field_store_cached(
        mc,
        tid as u32,
        ip as usize,
        bc_len as usize,
        self_val,
        name,
        val,
    ) {
        Ok(()) => TAG_OK,
        Err(e) => store_err(vm, e),
    }
}

/// `list.at:i put:v` — `devirt_ops::list_set` semantics (IndexError OOB).
pub(super) unsafe extern "C" fn list_set(
    vm: *mut c_void,
    mc: *const c_void,
    list_idx: i64,
    index: i64,
    kind: i64,
    bits: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let value = decode(vm, kind, bits);
    let receiver = vm.stack[list_idx as usize];
    let res = receiver.with_native_state_mut::<NativeListState, _, _>(mc, |l| {
        if l.elem.is_none() {
            Some(devirt_ops::list_set(l.get_vec_mut(), index, value))
        } else {
            None
        }
    });
    match res {
        Ok(Some(Ok(()))) => TAG_OK,
        Ok(Some(Err(e))) => store_err(vm, e),
        Ok(None) => match vm.tagged_list_set(mc, receiver, index, value) {
            Ok(()) => TAG_OK,
            Err(e) => store_err(vm, e),
        },
        Err(_) => invariant(vm, "ListSet on a non-list receiver"),
    }
}

/// `TagCollection`: verify + stamp a FRESH collection literal in a slot
/// (annotation-driven tagged literals inside compiled code). Mirrors the
/// interpreter arm exactly (`vm.tag_fresh_collection`).
pub(super) unsafe extern "C" fn tag_collection(
    vm: *mut c_void,
    mc: *const c_void,
    slot_idx: i64,
    tag_code: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let Some(tag) = crate::runtime::elem_tag::ElemTag::from_code(tag_code) else {
        return invariant(vm, "TagCollection: bad tag code");
    };
    let v = vm.stack[slot_idx as usize];
    match vm.tag_fresh_collection(mc, v, tag) {
        Ok(()) => TAG_OK,
        Err(e) => store_err(vm, e),
    }
}

/// The provable cold path of an inlined conditional on a PROVEN
/// `Boolean`-or-nil value (a checked `List(Boolean)` element read): the only
/// runtime possibility here is nil, whose sealed class has no `if:` — raise
/// the exact MessageNotUnderstood the interpreter's real send would
/// (GENERICS_ARCH.md §7). Renders the actual value defensively, so even an
/// impossible tag-corruption failure names what arrived.
pub(super) unsafe extern "C" fn nil_mnu(
    vm: *mut c_void,
    mc: *const c_void,
    kind: i64,
    bits: i64,
    selector: *const Symbol,
    argc: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let _ = mc;
    let receiver = decode(vm, kind, bits);
    let selector = unsafe { &*selector };
    let candidates = vm
        .collect_method_candidates(receiver, *selector)
        .iter()
        .map(|&mv| vm.format_candidate_signature(mv, *selector))
        .collect();
    store_err(
        vm,
        crate::error::QuoinError::MessageNotUnderstood {
            receiver: receiver.class_name(),
            selector: selector.as_str().to_string(),
            args: vec!["Block".to_string(); argc as usize],
            candidates,
        },
    )
}

/// A string literal, materialized into `out_idx`.
pub(super) unsafe extern "C" fn string_const(
    vm: *mut c_void,
    mc: *const c_void,
    ptr: *const u8,
    len: i64,
    out_idx: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let s = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len as usize)) };
    // Same shared-buffer path as the interpreter's literal materialization.
    let buf = vm.literal_string_buffer(mc, s);
    let v = vm.new_string_shared(mc, buf);
    slot_write(vm, out_idx, v)
}

/// A dynamic send from compiled code: the general boundary out of the
/// compiled subset (cold `BranchIfNotBool` paths, calls to non-compiled
/// methods). Runs through `call_method` — the same nested-step-loop native
/// re-entry every native uses, with its depth guard, suspension safety, and
/// thrown-value transparency. The result lands in `out_idx`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe extern "C" fn outcall(
    vm: *mut c_void,
    mc: *const c_void,
    tid: i64,
    ip_site: i64,
    bc_len: i64,
    recv_kind: i64,
    recv_bits: i64,
    selector: *const Symbol,
    argc: i64,
    kinds: *const i64,
    bits: *const i64,
    out_idx: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    // The D2 site id rides the ip lane's high bits (bytecode ips are tiny),
    // keeping the pre-D2 12-arg signature — the 13th argument crossed
    // further into the ABI's stack-passing region and taxed every outcall.
    let ip = ip_site & 0xffff_ffff;
    let site = (ip_site >> 32) as u32;
    let receiver = decode(vm, recv_kind, recv_bits);
    let n = argc as usize;
    // D2 fast path (docs/OUTCALL_ARCH.md), receiver phase FIRST — a site
    // whose target is not compiled (native, polymorphic) pays a few loads
    // here and then takes the classic path untouched. On a receiver hit the
    // lanes decode once into a fixed window buffer (compiled sites cap at 8),
    // arg guards + S1 preconditions check per call, and the entry is invoked
    // directly — no registry hash, no IC borrow/probe, no Callable dispatch.
    if site != u32::MAX
        && n <= 8
        && vm.aot.outcall_nesting < crate::codegen::spec::MAX_OUTCALL_NESTING
        && let Some(cell) = vm.aot_site_peek(site as usize, receiver, n)
    {
        let mut argv = [Value::Nil; 8];
        for (i, slot) in argv.iter_mut().enumerate().take(n) {
            let (k, b) = unsafe { (*kinds.add(i), *bits.add(i)) };
            *slot = decode(vm, k, b);
        }
        let args = &argv[..n];
        let entry = cell.entry.expect("peeked cell has an entry");
        if VmState::aot_site_args_match(&cell, args)
            && entry
                .param_preconditions
                .iter()
                .zip(args.iter())
                .all(|(pre, a)| pre.is_none_or(|k| crate::codegen::scalar_matches(k, *a)))
        {
            let recv_start = vm.stack.len();
            vm.stack.push(receiver);
            for &a in args {
                vm.stack.push(a);
            }
            vm.aot.outcall_nesting += 1;
            let outcome = crate::codegen::invoke(
                vm,
                mc,
                entry,
                receiver,
                args,
                cell.parent_env,
                Some(recv_start),
            );
            vm.aot.outcall_nesting = vm.aot.outcall_nesting.saturating_sub(1);
            match outcome {
                crate::codegen::AotOutcome::Value(v) => return slot_write(vm, out_idx, v),
                crate::codegen::AotOutcome::Err(e) => {
                    // The S1/finish_frame rule: an escaping `^^` already
                    // delivered at (possibly) the window start — only non-NLR
                    // errors tear the window down here.
                    if !matches!(e, QuoinError::NonLocalReturn) {
                        vm.stack.truncate(recv_start.min(vm.stack.len()));
                    }
                    vm.exceptions.last_send_args = args.to_vec();
                    return store_err(vm, e);
                }
                crate::codegen::AotOutcome::Bail => {
                    // Bails fire before invoke's scratch pushes: only our own
                    // window is on the stack. The slow path re-resolves.
                    vm.stack.truncate(recv_start.min(vm.stack.len()));
                }
            }
        }
        // Arg-shape / precondition miss (or Bail): classic path, args ready.
        return outcall_classic(
            vm,
            mc,
            tid,
            ip,
            bc_len,
            receiver,
            selector,
            args.to_vec(),
            out_idx,
            site,
        );
    }
    let mut args = Vec::with_capacity(n);
    for i in 0..n {
        let (k, b) = unsafe { (*kinds.add(i), *bits.add(i)) };
        args.push(decode(vm, k, b));
    }
    outcall_classic(
        vm, mc, tid, ip, bc_len, receiver, selector, args, out_idx, site,
    )
}

/// The classic outcall path, out of line so the hot fast path above stays
/// small (interpreter-heavy programs are sensitive to the helper's code
/// footprint). `(tid, ip)` is the SAME call-site identity the interpreted
/// send at this instruction uses, so compiled and interpreted execution
/// share one warm inline cache — without it every compiled operator send
/// paid an uncached lookup and lost to the interpreted body it replaced.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn outcall_classic(
    vm: &mut VmState<'static>,
    mc: &gc_arena::Mutation<'static>,
    tid: i64,
    ip: i64,
    bc_len: i64,
    receiver: Value<'static>,
    selector: *const Symbol,
    args: Vec<Value<'static>>,
    out_idx: i64,
    site: u32,
) -> u8 {
    let selector = unsafe { *selector };
    match vm.call_method_cached(
        mc,
        tid as u32,
        ip as usize,
        bc_len as usize,
        receiver,
        selector,
        args,
        (site != u32::MAX).then_some(site),
    ) {
        Ok(v) => slot_write(vm, out_idx, v),
        Err(e) => store_err(vm, e),
    }
}

/// A checked narrow failed: a `Dyn` value flowed into a scalar-declared
/// position (return or direct-call argument) with the wrong runtime type.
/// Interpreted code would silently carry the mis-typed value until some later
/// op notices; compiled code cannot, so this is the one deliberate divergence
/// — a clear, catchable TypeError at the annotation that lied.
pub(super) unsafe extern "C" fn narrow_error(
    vm: *mut c_void,
    mc: *const c_void,
    expected: i64,
    got_kind: i64,
) -> u8 {
    let (vm, _mc) = unsafe { vm_mc(vm, mc) };
    let name = |k: i64| match k {
        KIND_INT => "Integer",
        KIND_DOUBLE => "Double",
        KIND_BOOL => "Boolean",
        KIND_NIL => "Nil",
        _ => "Object",
    };
    store_err(
        vm,
        QuoinError::TypeError {
            expected: name(expected).to_string(),
            got: name(got_kind).to_string(),
            msg: "AOT-compiled method: a value did not match its declared scalar type".to_string(),
        },
    )
}

/// `LoadGlobal` — a global (usually a class reference) into `out_idx`;
/// missing globals read as nil, exactly like the interpreter arm.
pub(super) unsafe extern "C" fn load_global(
    vm: *mut c_void,
    mc: *const c_void,
    name: *const NamespacedName,
    out_idx: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let name = unsafe { &*name };
    let v = vm.globals.borrow().get(name).copied().unwrap_or(Value::Nil);
    let _ = mc;
    slot_write(vm, out_idx, v)
}
