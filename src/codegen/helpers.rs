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

/// Value-lane kinds in the compiled ABI.
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
    vm.stack[idx as usize] = v;
    TAG_OK
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
    vm.stack[out_idx as usize] = list;
    TAG_OK
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
    vm.stack[out_idx as usize] = list;
    TAG_OK
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
        Ok(v) => {
            vm.stack[out_idx as usize] = v;
            TAG_OK
        }
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
    vm.stack[out_idx as usize] = val;
    TAG_OK
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
        ) {
            Ok(v) => {
                vm.stack[out_idx as usize] = v;
                TAG_OK
            }
            Err(e) => store_err(vm, e),
        };
    };
    if let Some(tid) = block.template.template_id
        && let Some(entry) = super::block_entry_for(vm, tid)
    {
        match super::invoke_block(vm, mc, entry, recv, arg) {
            super::AotOutcome::Value(v) => {
                vm.stack[out_idx as usize] = v;
                return TAG_OK;
            }
            super::AotOutcome::Err(e) => return store_err(vm, e),
            super::AotOutcome::Bail => {}
        }
    }
    match vm.execute_block(mc, block, vec![arg], Some(arg)) {
        Ok(v) => {
            vm.stack[out_idx as usize] = v;
            TAG_OK
        }
        Err(e) => store_err(vm, e),
    }
}

/// Materialize a closure from a compiled frame's cold path (B3b): a fresh
/// snapshot `EnvFrame` (populated by the `closure_bind` calls the translator
/// emits right after) + the leaked template + the registry-shared IC cell —
/// the same shape `block_from_template` builds for interpreted frames.
/// Read-only-capture and no-`^^` semantics are the translator's gates.
pub(super) unsafe extern "C" fn make_closure(
    vm: *mut c_void,
    mc: *const c_void,
    tmpl: *const std::rc::Rc<crate::instruction::StaticBlock>,
    out_idx: i64,
) -> u8 {
    let (vm, mc) = unsafe { vm_mc(vm, mc) };
    let tmpl = unsafe { &*tmpl };
    // Chain the snapshot to the invoking frame's enclosing environment, so a
    // nested materialized closure's free names resolve through the FULL
    // lexical chain, exactly as interpreted (the webapp `path` lesson).
    let env = crate::gcl!(mc, crate::value::EnvFrame::new(vm.aot_enclosing_env));
    let inline_cache = vm.ic_cell_for(mc, tmpl);
    let v = vm.new_block(
        mc,
        crate::value::Block {
            template: tmpl.clone(),
            parent_env: Some(env),
            enclosing_method_id: None,
            decl_block: None,
            inline_cache,
        },
    );
    vm.stack[out_idx as usize] = v;
    TAG_OK
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
    let v = vm.new_string(mc, s.to_string());
    vm.stack[out_idx as usize] = v;
    TAG_OK
}

/// A dynamic send from compiled code: the general boundary out of the
/// compiled subset (cold `BranchIfNotBool` paths, calls to non-compiled
/// methods). Runs through `call_method` — the same nested-step-loop native
/// re-entry every native uses, with its depth guard, suspension safety, and
/// thrown-value transparency. The result lands in `out_idx`.
pub(super) unsafe extern "C" fn outcall(
    vm: *mut c_void,
    mc: *const c_void,
    tid: i64,
    ip: i64,
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
    let receiver = decode(vm, recv_kind, recv_bits);
    let mut args = Vec::with_capacity(argc as usize);
    for i in 0..argc as usize {
        let (k, b) = unsafe { (*kinds.add(i), *bits.add(i)) };
        args.push(decode(vm, k, b));
    }
    let selector = unsafe { *selector };
    // `(tid, ip)` is the SAME call-site identity the interpreted send at this
    // instruction uses, so compiled and interpreted execution share one warm
    // inline cache — without it every compiled operator send paid an uncached
    // lookup and lost to the interpreted body it replaced.
    match vm.call_method_cached(
        mc,
        tid as u32,
        ip as usize,
        bc_len as usize,
        receiver,
        selector,
        args,
    ) {
        Ok(v) => {
            vm.stack[out_idx as usize] = v;
            TAG_OK
        }
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
    vm.stack[out_idx as usize] = v;
    TAG_OK
}

/// The symbol table registered with every JIT module.
pub(super) fn symbols() -> Vec<(&'static str, *const u8)> {
    vec![
        ("qn_aot_checkpoint", super::checkpoint_addr()),
        ("qn_aot_slot_set", slot_set as *const u8),
        ("qn_aot_slot_peek", slot_peek as *const u8),
        ("qn_aot_list_new", list_new as *const u8),
        ("qn_aot_list_from", list_from as *const u8),
        ("qn_aot_list_push", list_push as *const u8),
        ("qn_aot_list_get", list_get as *const u8),
        ("qn_aot_list_len", list_len as *const u8),
        ("qn_aot_env_get", env_get as *const u8),
        ("qn_aot_env_set", env_set as *const u8),
        ("qn_aot_block_call", block_call as *const u8),
        ("qn_aot_make_closure", make_closure as *const u8),
        ("qn_aot_closure_bind", closure_bind as *const u8),
        ("qn_aot_list_set", list_set as *const u8),
        ("qn_aot_string_const", string_const as *const u8),
        ("qn_aot_outcall", outcall as *const u8),
        ("qn_aot_load_global", load_global as *const u8),
        ("qn_aot_narrow_error", narrow_error as *const u8),
        ("qn_aot_tag_collection", tag_collection as *const u8),
        ("qn_aot_nil_mnu", nil_mnu as *const u8),
    ]
}
