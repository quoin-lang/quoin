//! Bytecode → Cranelift translation (docs/AOT_ARCH.md §4.2, v0.2).
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
use std::rc::Rc;

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
    TAG_DEPTH, TAG_DIV_ZERO,
};

/// `%` on doubles: Rust's truncated remainder (what `devirt_ops::double_bin`
/// computes); Cranelift has no `frem`, so compiled code imports this.
unsafe extern "C" fn aot_fmod(a: f64, b: f64) -> f64 {
    a % b
}

/// Outcall arity cap: lane buffers are fixed-size native stack slots.
const MAX_OUTCALL_ARGS: usize = 8;

type SiblingMap = HashMap<(u32, String), (Vec<AotParam>, AotRet, u32)>;

/// Why a member's translation attempt stopped — a TYPED protocol between
/// the translator and `compile_all`'s retry loop. The demote variants are
/// RETRY instructions, never user-facing refusals; they used to ride
/// in-band as magic strings in the same `Err(String)` as refusal reasons,
/// matched by `==`/prefix-parse, where one context-adding `.map_err` on the
/// propagation path silently downgraded a retry into a permanent refusal.
/// They now travel out-of-band (`Translator::pending_abort`, set at the
/// same moment the aborting `Err` is returned), so the refusal strings stay
/// free-form messages.
enum TranslateAbort {
    /// A real refusal: the member runs interpreted; the string is the
    /// human-readable reason (surfaced under `QN_AOT_VERBOSE`).
    Refuse(String),
    /// The member used the slot window while marked scalar-pure — demote it
    /// from the direct-call set and retry.
    PurityDemote,
    /// A SPECULATED scalar return hit a return path the translator can't
    /// prove scalar — retry with the ret demoted to Obj (S2). Never used
    /// for annotated returns, whose checked-narrow divergence is deliberate.
    RetDemote,
    /// A merge point first planned SCALAR sees a Dyn predecessor (S3) —
    /// retry with the merge at this ip FORCED to Dyn from the start
    /// (scalars box on entry).
    MergeDemote(usize),
}

/// Compile every group; members are refused individually. Returns the
/// registered entries and refusals `(selector, reason)`.
pub(super) fn compile_all(
    cands: &[AotCandidate],
    siblings: &SiblingMap,
) -> (Vec<(u32, AotEntry)>, Vec<(String, String)>) {
    let mut groups: HashMap<u32, Vec<&AotCandidate>> = HashMap::new();
    for c in cands {
        groups.entry(c.group_id).or_default().push(c);
    }
    let mut compiled = Vec::new();
    let mut refused = Vec::new();
    for (_, mut active) in groups {
        // Per-member refusal: rebuild the module without the failed member and
        // retry (sends to it become outcalls). A purity violation demotes the
        // member from the direct-call set instead of refusing it. Groups are
        // small; worst-case quadratic compile cost is trivial — as are the
        // per-attempt `Box::leak`ed selector/name/template constants of a
        // FAILED attempt (bounded by the monotone retry sets; a successful
        // attempt's leaks are load-bearing, referenced by the compiled code
        // for the process lifetime).
        let mut demoted: HashSet<u32> = HashSet::new();
        let mut ret_demoted: HashSet<u32> = HashSet::new();
        let mut dyn_merges: HashMap<u32, HashSet<usize>> = HashMap::new();
        loop {
            if active.is_empty() {
                break;
            }
            match compile_group(&active, siblings, &demoted, &ret_demoted, &dyn_merges) {
                Ok(mut entries) => {
                    compiled.append(&mut entries);
                    break;
                }
                Err((failed_tid, TranslateAbort::PurityDemote)) => {
                    demoted.insert(failed_tid);
                }
                Err((failed_tid, TranslateAbort::RetDemote)) => {
                    ret_demoted.insert(failed_tid);
                }
                Err((failed_tid, TranslateAbort::MergeDemote(ip))) => {
                    dyn_merges.entry(failed_tid).or_default().insert(ip);
                }
                Err((failed_tid, TranslateAbort::Refuse(reason))) => {
                    if let Some(i) = active
                        .iter()
                        .position(|c| c.block.template_id == Some(failed_tid))
                    {
                        refused.push((active[i].selector.clone(), reason));
                        active.remove(i);
                    } else {
                        // A failure not attributable to one member (a
                        // candidate without a template id, or a module-level
                        // error blamed on a placeholder tid): refuse the
                        // whole group instead of panicking on a member
                        // lookup that cannot succeed.
                        for c in active.drain(..) {
                            refused.push((c.selector.clone(), reason.clone()));
                        }
                    }
                }
            }
        }
    }
    (compiled, refused)
}

fn kind_type(k: AotKind) -> Type {
    match k {
        AotKind::Int => types::I64,
        AotKind::Double => types::F64,
        AotKind::Bool => types::I8,
    }
}

fn param_type(p: AotParam) -> Type {
    match p {
        AotParam::Scalar(k) => kind_type(k),
        AotParam::Obj => types::I64, // absolute slot index
    }
}

fn ret_type(r: AotRet) -> Type {
    match r {
        AotRet::Scalar(k) => kind_type(k),
        AotRet::Obj => types::I64, // absolute slot index
    }
}

/// The scalar-pure subset of a group: all-scalar signatures whose bodies stay
/// in the scalar instruction set and send only to other scalar-pure siblings.
/// These keep the direct native-call path; everything else outcalls.
fn scalar_pure_set(
    members: &[&AotCandidate],
    siblings: &SiblingMap,
    ret_demoted: &HashSet<u32>,
) -> HashSet<u32> {
    let mut pure: HashSet<u32> = members
        .iter()
        .filter(|c| {
            c.params.iter().all(|p| matches!(p, AotParam::Scalar(_)))
                && matches!(eff_ret(c, ret_demoted), AotRet::Scalar(_))
        })
        .filter_map(|c| c.block.template_id)
        .collect();
    loop {
        let mut changed = false;
        for c in members {
            let Some(tid) = c.block.template_id else {
                continue;
            };
            if !pure.contains(&tid) {
                continue;
            }
            let ok = c.block.bytecode.0.iter().all(|inst| match inst {
                Instruction::Push(
                    Constant::Int(_) | Constant::Double(_) | Constant::Bool(_) | Constant::Nil,
                )
                | Instruction::LoadLocal(_)
                | Instruction::DefineLocal(_)
                | Instruction::StoreLocal(_)
                | Instruction::DefineLocalKeep(_)
                | Instruction::StoreLocalKeep(_)
                | Instruction::Dup
                | Instruction::Pop
                | Instruction::IntAdd
                | Instruction::IntSub
                | Instruction::IntMul
                | Instruction::IntDiv
                | Instruction::IntMod
                | Instruction::IntLt
                | Instruction::IntLe
                | Instruction::IntGt
                | Instruction::IntGe
                | Instruction::IntEq
                | Instruction::IntNe
                | Instruction::DoubleAdd
                | Instruction::DoubleSub
                | Instruction::DoubleMul
                | Instruction::DoubleDiv
                | Instruction::DoubleMod
                | Instruction::DoubleLt
                | Instruction::DoubleLe
                | Instruction::DoubleGt
                | Instruction::DoubleGe
                | Instruction::DoubleEq
                | Instruction::DoubleNe
                | Instruction::IntBinLL(..)
                | Instruction::IntBinLC(..)
                | Instruction::DoubleBinLL(..)
                | Instruction::DoubleBinLC(..)
                | Instruction::Jump(_)
                | Instruction::IfJump(_)
                | Instruction::ElseJump(_)
                | Instruction::Return
                | Instruction::BlockReturn
                | Instruction::MethodReturn => true,
                inst => match inst.send_parts() {
                    // A sealed scalar operator devirtualizes at translation
                    // (S2) when its operands prove scalar — optimistically
                    // pure here; a member that still needs an outcall trips
                    // the translation purity check and demotes. Via the
                    // exhaustive `send_parts` — but `SendField` stays OUT of
                    // the pure set deliberately: its field read needs a slot,
                    // so admission would only demote-retry back out, paying
                    // a wasted compile attempt per such member.
                    Some((sel, _, _)) if !matches!(inst, Instruction::SendField(..)) => {
                        IntBinKind::from_selector(sel.as_str()).is_some()
                            || siblings
                                .get(&(c.group_id, sel.as_str().to_string()))
                                .is_some_and(|(_, _, callee)| pure.contains(callee))
                    }
                    _ => false,
                },
            });
            if !ok {
                pure.remove(&tid);
                changed = true;
            }
        }
        if !changed {
            return pure;
        }
    }
}

/// Rust ABI type -> Cranelift type, for deriving helper import signatures
/// from the helpers' own `extern "C"` fn types.
trait ClAbi {
    fn cl(ptr: Type) -> Type;
}
impl ClAbi for i64 {
    fn cl(_: Type) -> Type {
        types::I64
    }
}
impl ClAbi for u8 {
    fn cl(_: Type) -> Type {
        types::I8
    }
}
impl ClAbi for f64 {
    fn cl(_: Type) -> Type {
        types::F64
    }
}
impl<T> ClAbi for *const T {
    fn cl(ptr: Type) -> Type {
        ptr
    }
}
impl<T> ClAbi for *mut T {
    fn cl(ptr: Type) -> Type {
        ptr
    }
}

/// Fn-pointer types whose Cranelift import signature derives from the Rust
/// type itself (one impl per arity, below).
trait HelperSig {
    fn cl_sig(self, module: &JITModule, ptr: Type) -> Signature;
}

macro_rules! impl_helper_sig {
    ($($a:ident)*) => {
        impl<$($a: ClAbi,)* R: ClAbi> HelperSig for unsafe extern "C" fn($($a),*) -> R {
            fn cl_sig(self, module: &JITModule, ptr: Type) -> Signature {
                let mut s = module.make_signature();
                $(s.params.push(AbiParam::new(<$a>::cl(ptr)));)*
                s.returns.push(AbiParam::new(<R>::cl(ptr)));
                s
            }
        }
    };
}
impl_helper_sig!(A B);
impl_helper_sig!(A B C);
impl_helper_sig!(A B C D);
impl_helper_sig!(A B C D E);
impl_helper_sig!(A B C D E F);
impl_helper_sig!(A B C D E F G);
impl_helper_sig!(A B C D E F G H);
impl_helper_sig!(A B C D E F G H I);
impl_helper_sig!(A B C D E F G H I J);
impl_helper_sig!(A B C D E F G H I J K);
impl_helper_sig!(A B C D E F G H I J K L);

/// One row per helper: `field: path as fn(params) -> ret`. Generates the `Helpers`
/// struct, `declare_helpers`, and `helper_symbols` (the JIT symbol table);
/// the symbol name derives as `qn_aot_<field>`. Each row's fn type is checked
/// against the helper's definition by a `let` coercion, and the Cranelift
/// import signature is derived from that type (`HelperSig`) — so a helper
/// whose signature drifts from its declaration is a compile error, not a
/// silent ABI mismatch at runtime.
macro_rules! aot_helpers {
    ($($field:ident: $f:path as fn($($p:ty),* $(,)?) -> $r:ty),+ $(,)?) => {
        /// Imported helper function ids for one module (see `aot_helpers!`).
        struct Helpers {
            $($field: FuncId,)+
        }

        fn declare_helpers(module: &mut JITModule, ptr: Type) -> Result<Helpers, String> {
            Ok(Helpers {
                $($field: {
                    // The coercion checks the table row against the definition.
                    let f: unsafe extern "C" fn($($p),*) -> $r = $f;
                    let sig = f.cl_sig(module, ptr);
                    module
                        .declare_function(
                            concat!("qn_aot_", stringify!($field)),
                            Linkage::Import,
                            &sig,
                        )
                        .map_err(|e| e.to_string())?
                },)+
            })
        }

        /// The symbol table registered with every JIT module.
        fn helper_symbols() -> Vec<(&'static str, *const u8)> {
            vec![$((concat!("qn_aot_", stringify!($field)), $f as *const u8),)+]
        }
    };
}

aot_helpers! {
    checkpoint: super::aot_checkpoint as fn(*mut c_void, *mut i64) -> u8,
    fmod: aot_fmod as fn(f64, f64) -> f64,
    slot_set: helpers::slot_set as fn(*mut c_void, *const c_void, i64, i64, i64) -> u8,
    slot_peek: helpers::slot_peek as fn(*mut c_void, *const c_void, i64, *mut i64) -> i64,
    list_new: helpers::list_new as fn(*mut c_void, *const c_void, i64) -> u8,
    list_from: helpers::list_from as fn(*mut c_void, *const c_void, i64, i64, *const i64, *const i64) -> u8,
    list_push: helpers::list_push as fn(*mut c_void, *const c_void, i64, i64, i64) -> u8,
    list_get: helpers::list_get as fn(*mut c_void, *const c_void, i64, i64, i64) -> u8,
    list_len: helpers::list_len as fn(*mut c_void, *const c_void, i64) -> i64,
    list_set: helpers::list_set as fn(*mut c_void, *const c_void, i64, i64, i64, i64) -> u8,
    string_const: helpers::string_const as fn(*mut c_void, *const c_void, *const u8, i64, i64) -> u8,
    outcall: helpers::outcall as fn(
        *mut c_void, *const c_void, i64, i64, i64, i64, i64, *const Symbol, i64,
        *const i64, *const i64, i64,
    ) -> u8,
    narrow_error: helpers::narrow_error as fn(*mut c_void, *const c_void, i64, i64) -> u8,
    load_global: helpers::load_global as fn(*mut c_void, *const c_void, *const NamespacedName, i64) -> u8,
    tag_collection: helpers::tag_collection as fn(*mut c_void, *const c_void, i64, i64) -> u8,
    nil_mnu: helpers::nil_mnu as fn(*mut c_void, *const c_void, i64, i64, *const Symbol, i64) -> u8,
    env_get: helpers::env_get as fn(*mut c_void, *const c_void, i64, *const Symbol, i64) -> u8,
    env_set: helpers::env_set as fn(*mut c_void, *const c_void, i64, *const Symbol, i64, i64) -> u8,
    block_call: helpers::block_call as fn(
        *mut c_void, *const c_void, i64, i64, i64, i64, i64, i64, i64, i64,
    ) -> u8,
    make_closure: helpers::make_closure as fn(
        *mut c_void, *const c_void, *const Rc<StaticBlock>, i64, i64,
    ) -> u8,
    closure_bind: helpers::closure_bind as fn(*mut c_void, *const c_void, i64, *const Symbol, i64, i64) -> u8,
    field_get: helpers::field_get as fn(
        *mut c_void, *const c_void, i64, i64, i64, i64, *const u8, i64, i64,
    ) -> u8,
    field_set: helpers::field_set as fn(
        *mut c_void, *const c_void, i64, i64, i64, i64, *const u8, i64, i64, i64,
    ) -> u8,
}

/// Compile one attempt at a group. `Err((template_id, reason))` names the
/// member to refuse before retrying.
fn eff_ret(c: &AotCandidate, ret_demoted: &HashSet<u32>) -> AotRet {
    match c.block.template_id {
        Some(tid) if ret_demoted.contains(&tid) => AotRet::Obj,
        _ => c.ret,
    }
}

fn compile_group(
    members: &[&AotCandidate],
    siblings: &SiblingMap,
    demoted: &HashSet<u32>,
    ret_demoted: &HashSet<u32>,
    dyn_merges: &HashMap<u32, HashSet<usize>>,
) -> Result<Vec<(u32, AotEntry)>, (u32, TranslateAbort)> {
    let fail = |tid: u32, e: String| (tid, TranslateAbort::Refuse(e));
    let any_tid = members[0].block.template_id.unwrap_or(0);

    let mut flags = settings::builder();
    flags
        .set("opt_level", "speed")
        .map_err(|e| fail(any_tid, e.to_string()))?;
    let isa = cranelift_native::builder()
        .map_err(|e| fail(any_tid, e.to_string()))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| fail(any_tid, e.to_string()))?;
    let mut jb = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    for (name, addr) in helper_symbols() {
        jb.symbol(name, addr);
    }
    let mut module = JITModule::new(jb);
    let ptr = module.target_config().pointer_type();
    let helpers = declare_helpers(&mut module, ptr).map_err(|e| fail(any_tid, e))?;
    let mut pure = scalar_pure_set(members, siblings, ret_demoted);
    for d in demoted {
        pure.remove(d);
    }

    // Declare every member's inner fn first (mutual recursion among the pure
    // set), then define bodies and trampolines.
    let mut inner_ids: HashMap<u32, FuncId> = HashMap::new();
    for m in members {
        let tid = m
            .block
            .template_id
            .ok_or_else(|| fail(any_tid, "candidate without template id".into()))?;
        let sig = inner_sig(&mut module, ptr, m, eff_ret(m, ret_demoted));
        let fid = module
            .declare_function(&format!("t{tid}"), Linkage::Local, &sig)
            .map_err(|e| fail(tid, e.to_string()))?;
        inner_ids.insert(tid, fid);
    }

    let mut fb_ctx = FunctionBuilderContext::new();
    let mut tramp_ids: Vec<(u32, FuncId, &AotCandidate, u32, bool, bool, bool)> = Vec::new();

    for m in members {
        let tid = m.block.template_id.unwrap();
        if std::env::var("QN_AOT_DUMP").is_ok_and(|v| v == m.selector) {
            eprintln!(
                "=== bytecode {} (tid {tid}; pure={}, ret={:?}, spec_ret={}, open={}) ===",
                m.selector,
                pure.contains(&tid),
                eff_ret(m, ret_demoted),
                m.spec_ret,
                m.open_owner
            );
            for (i, inst) in m.block.bytecode.0.iter().enumerate() {
                eprintln!("  {i:3}: {inst:?}");
            }
        }
        let mut ctx = module.make_context();
        ctx.func.signature = inner_sig(&mut module, ptr, m, eff_ret(m, ret_demoted));
        let n_scratch;
        let needs_list_self;
        let direct_self;
        let materializes_nlr;
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            static EMPTY_MERGES: std::sync::OnceLock<HashSet<usize>> = std::sync::OnceLock::new();
            let mut tr = Translator {
                module: &mut module,
                cand: m,
                eff_ret: eff_ret(m, ret_demoted),
                used_direct_self: false,
                dyn_merges: dyn_merges
                    .get(&tid)
                    .unwrap_or_else(|| EMPTY_MERGES.get_or_init(HashSet::new)),
                siblings,
                inner_ids: &inner_ids,
                pure: &pure,
                helpers: &helpers,
                is_pure: pure.contains(&tid),
                next_scratch: 0,
                proofs: HashMap::new(),
                needs_list_self: false,
                nil_deferred: HashSet::new(),

                pending_writebacks: HashMap::new(),
                materialized: HashSet::new(),
                materialized_nlr: HashSet::new(),
                pending_abort: None,
            };
            if let Err(e) = tr.build_inner(&mut b) {
                // A demote signal set alongside the aborting Err travels
                // out-of-band — the message string is free-form and may be
                // wrapped without breaking the retry protocol.
                let abort = tr.pending_abort.take().unwrap_or(TranslateAbort::Refuse(e));
                return Err((tid, abort));
            }
            n_scratch = tr.next_scratch;
            needs_list_self = tr.needs_list_self;
            direct_self = tr.used_direct_self;
            materializes_nlr = !tr.materialized_nlr.is_empty();
            b.seal_all_blocks();
            b.finalize();
        }
        let fid = inner_ids[&tid];
        module
            .define_function(fid, &mut ctx)
            .map_err(|e| fail(tid, format!("{e:?}\nIR:\n{}", ctx.func.display())))?;
        if std::env::var("QN_AOT_DUMP").is_ok_and(|v| v == m.selector || v == "1") {
            eprintln!("=== {} (tid {tid}) ===\n{}", m.selector, ctx.func.display());
        }

        let mut tctx = module.make_context();
        tctx.func.signature = tramp_sig(&mut module, ptr);
        let tramp_id = module
            .declare_function(
                &format!("t{tid}_tramp"),
                Linkage::Local,
                &tctx.func.signature,
            )
            .map_err(|e| fail(tid, e.to_string()))?;
        {
            let mut b = FunctionBuilder::new(&mut tctx.func, &mut fb_ctx);
            build_trampoline(&mut module, &mut b, m, fid, eff_ret(m, ret_demoted));
            b.seal_all_blocks();
            b.finalize();
        }
        module
            .define_function(tramp_id, &mut tctx)
            .map_err(|e| fail(tid, e.to_string()))?;
        tramp_ids.push((
            tid,
            tramp_id,
            m,
            n_scratch,
            needs_list_self,
            direct_self,
            materializes_nlr,
        ));
    }

    module
        .finalize_definitions()
        .map_err(|e| fail(any_tid, e.to_string()))?;
    let mut out = Vec::new();
    for (tid, tramp_id, m, n_scratch, needs_list_self, direct_self, materializes_nlr) in tramp_ids {
        let addr = module.get_finalized_function(tramp_id);
        let raw: AotRawFn = unsafe { std::mem::transmute(addr) };
        out.push((
            tid,
            AotEntry {
                raw,
                params: m.params.clone().into_boxed_slice(),
                ret: eff_ret(m, ret_demoted),
                n_scratch,
                needs_list_self,
                role: m.role,
                template_id: tid,
                param_preconditions: m.spec_preconditions.clone().into_boxed_slice(),
                spec_bails: std::sync::atomic::AtomicU32::new(0),
                direct_self,
                compile_epoch: super::redef_epoch(),
                materializes_nlr,
            },
        ));
    }
    // The code must live for the process (fn pointers are registered
    // globally): leak the module, same append-only lifetime as the interner.
    std::mem::forget(module);
    Ok(out)
}

fn inner_sig(module: &mut JITModule, ptr: Type, m: &AotCandidate, eff: AotRet) -> Signature {
    let mut sig = module.make_signature();
    for _ in 0..4 {
        sig.params.push(AbiParam::new(ptr)); // vm, mc, fuel, depth
    }
    sig.params.push(AbiParam::new(types::I64)); // slot_base
    for &p in &m.params {
        sig.params.push(AbiParam::new(param_type(p)));
    }
    sig.returns.push(AbiParam::new(types::I8)); // tag
    let _ = m;
    sig.returns.push(AbiParam::new(ret_type(eff)));
    sig
}

fn tramp_sig(module: &mut JITModule, ptr: Type) -> Signature {
    let mut sig = module.make_signature();
    for _ in 0..4 {
        sig.params.push(AbiParam::new(ptr)); // vm, mc, fuel, depth
    }
    sig.params.push(AbiParam::new(types::I64)); // slot_base
    sig.params.push(AbiParam::new(ptr)); // args
    sig.params.push(AbiParam::new(ptr)); // ret
    sig.returns.push(AbiParam::new(types::I8));
    sig
}

fn build_trampoline(
    module: &mut JITModule,
    b: &mut FunctionBuilder,
    m: &AotCandidate,
    inner: FuncId,
    eff: AotRet,
) {
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    let p = b.block_params(entry).to_vec();
    let (vm, mc, fuel, depth, slot_base, args, ret) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6]);
    let mut call_args = vec![vm, mc, fuel, depth, slot_base];
    for (i, &k) in m.params.iter().enumerate() {
        let off = (i * 8) as i32;
        let v = match k {
            AotParam::Scalar(AotKind::Int) | AotParam::Obj => {
                b.ins().load(types::I64, MemFlagsData::trusted(), args, off)
            }
            AotParam::Scalar(AotKind::Double) => {
                b.ins().load(types::F64, MemFlagsData::trusted(), args, off)
            }
            AotParam::Scalar(AotKind::Bool) => {
                let w = b.ins().load(types::I64, MemFlagsData::trusted(), args, off);
                b.ins().ireduce(types::I8, w)
            }
        };
        call_args.push(v);
    }
    let callee = module.declare_func_in_func(inner, b.func);
    let call = b.ins().call(callee, &call_args);
    let results = b.inst_results(call).to_vec();
    let (tag, val) = (results[0], results[1]);
    // The EFFECTIVE ret (a speculated scalar may have demoted to Obj on
    // retry) — using the candidate's would type-mismatch the inner call.
    match eff {
        AotRet::Scalar(AotKind::Bool) => {
            let w = b.ins().uextend(types::I64, val);
            b.ins().store(MemFlagsData::trusted(), w, ret, 0);
        }
        _ => {
            b.ins().store(MemFlagsData::trusted(), val, ret, 0);
        }
    }
    b.ins().return_(&[tag]);
}

/// An abstract stack slot.
#[derive(Clone, Copy)]
enum AV {
    /// A scalar in SSA.
    C(CVal, AotKind),
    /// A slot-resident dynamic value: the SSA value is its *absolute index*
    /// into `vm.stack` (i64).
    Dyn(CVal),
    /// The method receiver (`self`): encoded as slot 0 when a value is needed.
    SelfRef,
    /// The `nil` a local-declaration prologue pushes; also a plain nil value
    /// (boxed into a slot when it must travel).
    Nil,
}

/// Block-boundary shape of one stack slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BKind {
    S(AotKind),
    Dyn,
}

fn bkind_type(k: BKind) -> Type {
    match k {
        BKind::S(k) => kind_type(k),
        BKind::Dyn => types::I64,
    }
}

/// A named local's storage.
#[derive(Clone, Copy)]
enum VarSlot {
    Scalar(Variable, AotKind),
    /// Scratch-slot number in the frame window, plus what the translator can
    /// PROVE about the value it holds (tag-backed; docs/GENERICS_ARCH.md §8).
    Obj(u32, Option<DynProof>),
}

/// A guarantee the translator carries for a slot-resident dynamic value —
/// only from sources the runtime enforces (a `TagCollection` it emitted, or
/// an element read from such a collection). Never from checker beliefs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DynProof {
    /// A collection whose element tag is enforced. In compiled code this is
    /// always a native List: `TagCollection` is only reachable after a list
    /// literal (`NewMap`/`NewSet` don't translate), and the only param source
    /// is a `List`-hinted Obj param (B1 seeding below).
    CollectionOf(ElemTag),
    /// An element read from such a collection: proven tag-or-nil.
    ElemOrNil(ElemTag),
    /// A native List with no (or unknown) element tag — a bare `List`-hinted
    /// Obj param (dispatch guarantees the class) or a fresh list literal.
    /// Enough for the fused `each:` guard (B1); mints no element proofs.
    NativeList,
}

struct Translator<'a> {
    module: &'a mut JITModule,
    cand: &'a AotCandidate,
    /// The ret this member ACTUALLY compiles with: the candidate's, or Obj
    /// after a speculated-scalar demotion retry (S2).
    eff_ret: AotRet,
    /// A direct self-recursion call was emitted (S2): the entry records the
    /// redefinition epoch and `invoke` Bails when it goes stale.
    used_direct_self: bool,
    siblings: &'a SiblingMap,
    inner_ids: &'a HashMap<u32, FuncId>,
    pure: &'a HashSet<u32>,
    helpers: &'a Helpers,
    /// This member is in the scalar-pure set (direct-callable): it must not
    /// touch the slot window, because direct callers pass their own base.
    is_pure: bool,
    next_scratch: u32,
    /// Proofs for in-flight `AV::Dyn` values, keyed by their SSA index value.
    /// Values that cross a control-flow join (block params) drop their proofs
    /// — a sound degradation; the load-bearing flows (element read → inlined
    /// conditional) stay within one block, and locals carry proofs in
    /// `VarSlot::Obj` across blocks.
    proofs: HashMap<CVal, DynProof>,
    /// Set when a fused-`each:` guard on `self` compiled hot-path-only (B2):
    /// becomes the entry's `needs_list_self` precondition.
    needs_list_self: bool,
    /// Merge ips FORCED to all-Dyn shapes (S3 retry): scalars box on entry,
    /// so predecessors with mixed shapes unify.
    dyn_merges: &'a HashSet<usize>,
    /// `var x = nil` declarations whose slot type is still DEFERRED to the
    /// first store. A closure materialization forces these into Obj slots
    /// first — see the DefineLocal arm.
    nil_deferred: HashSet<Symbol>,
    /// Frame locals a materialized closure WRITES (through its snapshot env),
    /// keyed by the closure's slot-index SSA value: after the consuming send
    /// returns, each is read back from the snapshot into the frame local, so
    /// `count:`-style `{ n = n + 1 }` cold arms stay exact (B3b).
    pending_writebacks: HashMap<CVal, Vec<(Symbol, VarSlot)>>,
    /// Every materialized closure's slot value: a send consuming TWO OR MORE
    /// of these where any writes a capture must refuse — sibling snapshots
    /// are INDEPENDENT envs, but interpreted siblings share one cell (the
    /// unfused-`whileDo:` bug: the body's `i` advanced while the condition's
    /// stayed frozen).
    materialized: HashSet<CVal>,
    /// Out-of-band demote signal (see [`TranslateAbort`]): set at the same
    /// moment the aborting `Err` is returned, consumed by `compile_group`.
    pending_abort: Option<TranslateAbort>,
    /// The materialized closures whose bodies contain a `^^` (S5). A
    /// `catch`-family send consuming one must refuse: interpreted, a
    /// catch-all can catch the `^^` crossing it — a compiled home cannot
    /// reproduce that (the runtime treats an in-flight compiled-target `^^`
    /// as uncatchable), so the method stays interpreted.
    materialized_nlr: HashSet<CVal>,
}

/// What a whole materialized NEST (a cold-path block plus every literal
/// nested inside it, transitively — S5b) does to the enclosing compiled
/// frame. The nest runs INTERPRETED, so nested execution needs no compiled
/// support; the translator only needs these facts for its gates.
#[derive(Default)]
struct NestScan {
    /// Symbols written that are free through the WHOLE nest — they resolve
    /// to the snapshot env, so the consuming send must flush them back.
    written_frees: Vec<Symbol>,
    /// A `^^` anywhere in the nest (profitability + catch-parity gates).
    has_nlr: bool,
    /// A `catch`-family send anywhere in the nest.
    has_catch_send: bool,
    /// A send of the enclosing candidate's own selector anywhere in the
    /// nest (the `^^s.whileDo:block` trampoline signature).
    sends_own_selector: bool,
}

/// Recursive gate scan for [`Translator::materialize_closure`]: each level's
/// params + `DefineLocal`s shadow the levels above, so only writes free
/// through EVERY level reach the snapshot.
fn scan_materialized_nest(
    rc: &StaticBlock,
    inherited: &HashSet<Symbol>,
    own_selector: &str,
    out: &mut NestScan,
) -> Result<(), String> {
    if rc.decl_block.is_some() {
        return Err("guarded block in a materialized nest".to_string());
    }
    let mut defined = inherited.clone();
    defined.extend(rc.param_syms.iter().copied());
    for inst in rc.bytecode.0.iter() {
        if let Instruction::DefineLocal(s) | Instruction::DefineLocalKeep(s) = inst {
            defined.insert(*s);
        }
    }
    for inst in rc.bytecode.0.iter() {
        match inst {
            Instruction::MethodReturn => out.has_nlr = true,
            Instruction::Push(Constant::Block(nb)) => {
                scan_materialized_nest(nb, &defined, own_selector, out)?;
            }
            Instruction::StoreLocal(s) | Instruction::StoreLocalKeep(s) => {
                if !defined.contains(s) {
                    out.written_frees.push(*s);
                }
            }
            _ => {}
        }
    }
    for inst in rc.bytecode.0.iter() {
        let Some((sel, _, fused_const)) = inst.send_parts() else {
            continue;
        };
        if sel.as_str() == own_selector {
            out.sends_own_selector = true;
        }
        if crate::runtime::block::is_catch_family(sel.as_str()) {
            out.has_catch_send = true;
        }
        if let Some(Constant::Block(nb)) = fused_const {
            scan_materialized_nest(nb, &defined, own_selector, out)?;
        }
    }
    Ok(())
}

struct FnCtx {
    vm: CVal,
    mc: CVal,
    fuel: CVal,
    depth: CVal,
    slot_base: CVal,
    exit: CBlock,
    ret: AotRet,
    /// Native-stack lane buffers for helper calls (kinds, bits) and the
    /// peek out-parameter.
    kinds_buf: cranelift_codegen::ir::StackSlot,
    bits_buf: cranelift_codegen::ir::StackSlot,
    peek_out: cranelift_codegen::ir::StackSlot,
}

impl<'a> Translator<'a> {
    fn alloc_scratch(&mut self) -> Result<u32, String> {
        if self.is_pure {
            // Translation-verified purity: the syntactic pure-set scan missed a
            // slot use (e.g. a sibling-selector send on a non-self receiver).
            // The caller demotes this member from the pure set and retries.
            self.pending_abort = Some(TranslateAbort::PurityDemote);
            return Err("scalar-pure member touched the slot window".to_string());
        }
        let fixed = match self.cand.role {
            // 0 = receiver, then obj params.
            AotRole::Method => {
                1 + self
                    .cand
                    .params
                    .iter()
                    .filter(|p| matches!(p, AotParam::Obj))
                    .count() as u32
            }
            // 0 = self (the vWSOA arg), 1 = the param's own cell, 2 = the
            // block object (env access) — see `invoke_block`.
            AotRole::BlockTemplate => 3,
        };
        let k = fixed + self.next_scratch;
        self.next_scratch += 1;
        Ok(k)
    }

    fn abs_slot(&self, b: &mut FunctionBuilder, fx: &FnCtx, window_idx: u32) -> CVal {
        b.ins().iadd_imm(fx.slot_base, i64::from(window_idx))
    }

    /// Encode an AV as `(kind, bits)` lanes for a helper call. May allocate a
    /// scratch slot (boxing `Nil` never needs one; scalars pass by value).
    fn encode(&mut self, b: &mut FunctionBuilder, fx: &FnCtx, v: AV) -> (CVal, CVal) {
        match v {
            AV::C(cv, AotKind::Int) => (b.ins().iconst(types::I64, KIND_INT), cv),
            AV::C(cv, AotKind::Double) => {
                let bits = b.ins().bitcast(types::I64, MemFlagsData::new(), cv);
                (b.ins().iconst(types::I64, KIND_DOUBLE), bits)
            }
            AV::C(cv, AotKind::Bool) => {
                let w = b.ins().uextend(types::I64, cv);
                (b.ins().iconst(types::I64, KIND_BOOL), w)
            }
            AV::Dyn(idx) => (b.ins().iconst(types::I64, KIND_SLOT), idx),
            AV::SelfRef => {
                let idx = self.abs_slot(b, fx, 0);
                (b.ins().iconst(types::I64, KIND_SLOT), idx)
            }
            AV::Nil => {
                let z = b.ins().iconst(types::I64, 0);
                (b.ins().iconst(types::I64, KIND_NIL), z)
            }
        }
    }

    /// Call a helper whose return is a status tag; branch to exit on non-zero.
    fn tag_check(&mut self, b: &mut FunctionBuilder, fx: &FnCtx, tag: CVal) {
        let bad = b.ins().icmp_imm(IntCC::NotEqual, tag, 0);
        let bad_bl = b.create_block();
        let ok_bl = b.create_block();
        b.ins().brif(bad, bad_bl, &[], ok_bl, &[]);
        b.switch_to_block(bad_bl);
        let zero = self.zero_of(b, fx.ret);
        b.ins().jump(fx.exit, &[tag.into(), zero.into()]);
        b.switch_to_block(ok_bl);
    }

    fn build_inner(&mut self, b: &mut FunctionBuilder) -> Result<(), String> {
        let insts = &self.cand.block.bytecode.0.clone();

        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let p = b.block_params(entry).to_vec();
        let (vm, mc, fuel, depth, slot_base) = (p[0], p[1], p[2], p[3], p[4]);

        // Named locals: params first. Object params occupy window slots 1..;
        // their SSA param value already carries the absolute index.
        let mut vars: HashMap<Symbol, VarSlot> = HashMap::new();
        let mut obj_param_avs: HashMap<Symbol, CVal> = HashMap::new();
        for (i, (&sym, &pk)) in self
            .cand
            .block
            .param_syms
            .iter()
            .zip(self.cand.params.iter())
            .enumerate()
        {
            match pk {
                AotParam::Scalar(k) => {
                    let var = b.declare_var(kind_type(k));
                    b.def_var(var, p[5 + i]);
                    vars.insert(sym, VarSlot::Scalar(var, k));
                }
                AotParam::Obj => {
                    obj_param_avs.insert(sym, p[5 + i]);
                    // B1: a `List`-hinted param is a dispatch-GUARANTEED native
                    // List (List is sealed; the hint only matches the native
                    // class) — and a tag-required param is guaranteed tagged,
                    // since tag requirements gate dispatch too (G1). These
                    // proofs are what let a fused `each:` guard fall away.
                    // METHOD role only: a block's annotations are beliefs
                    // (`value:` checks nothing) — never proofs.
                    if self.cand.role == AotRole::Method
                        && self.cand.block.param_types.get(i).map(String::as_str) == Some("List")
                    {
                        let proof = match self.cand.block.param_elem_tags.get(i).copied().flatten()
                        {
                            Some(tag) => DynProof::CollectionOf(tag),
                            None => DynProof::NativeList,
                        };
                        self.proofs.insert(p[5 + i], proof);
                    }
                }
            }
        }

        let exit = b.create_block();
        b.append_block_param(exit, types::I8);
        b.append_block_param(exit, ret_type(self.eff_ret));
        let kinds_buf = b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            (MAX_OUTCALL_ARGS * 8) as u32,
            3,
        ));
        let bits_buf = b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            (MAX_OUTCALL_ARGS * 8) as u32,
            3,
        ));
        let peek_out =
            b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
        let fx = FnCtx {
            vm,
            mc,
            fuel,
            depth,
            slot_base,
            exit,
            ret: self.eff_ret,
            kinds_buf,
            bits_buf,
            peek_out,
        };

        // Prologue: depth guard, then fuel tick (checkpoint on exhaustion).
        let d0 = b.ins().load(types::I64, MemFlagsData::trusted(), depth, 0);
        let d1 = b.ins().iadd_imm(d0, 1);
        b.ins().store(MemFlagsData::trusted(), d1, depth, 0);
        let too_deep = b
            .ins()
            .icmp_imm(IntCC::SignedGreaterThan, d1, AOT_MAX_CALL_DEPTH);
        let deep_bl = b.create_block();
        let cont1 = b.create_block();
        b.ins().brif(too_deep, deep_bl, &[], cont1, &[]);
        b.switch_to_block(deep_bl);
        self.bail(b, &fx, TAG_DEPTH);
        b.switch_to_block(cont1);
        self.emit_fuel_tick_empty(b, &fx);

        // Fill the exit block now (parameters flow straight to the return).
        {
            let saved = b.current_block().unwrap();
            b.switch_to_block(exit);
            let ep = b.block_params(exit).to_vec();
            let d = b.ins().load(types::I64, MemFlagsData::trusted(), depth, 0);
            let d2 = b.ins().iadd_imm(d, -1);
            b.ins().store(MemFlagsData::trusted(), d2, depth, 0);
            b.ins().return_(&[ep[0], ep[1]]);
            b.switch_to_block(saved);
        }

        // Basic-block map over the bytecode: leaders = jump targets +
        // conditional fallthroughs.
        let mut leaders: Vec<usize> = Vec::new();
        for (ip, inst) in insts.iter().enumerate() {
            let off = match inst {
                Instruction::Jump(o)
                | Instruction::IfJump(o)
                | Instruction::ElseJump(o)
                | Instruction::BranchIfNotBool(o)
                | Instruction::BranchIfNotList(o) => *o,
                _ => continue,
            };
            let target = ip as isize + off;
            if target < 0 || target as usize >= insts.len() {
                return Err(format!("jump out of range at ip {ip}"));
            }
            leaders.push(target as usize);
            if !matches!(inst, Instruction::Jump(_)) {
                leaders.push(ip + 1);
            }
        }
        leaders.sort_unstable();
        leaders.dedup();

        let mut blocks: HashMap<usize, (CBlock, Vec<BKind>)> = HashMap::new();
        let mut done: HashSet<usize> = HashSet::new();
        let mut work: Vec<usize> = Vec::new();

        let mut cursor = Some((0usize, Vec::<AV>::new()));
        loop {
            let (start_ip, mut stack) = match cursor.take() {
                Some(s) => s,
                None => match work.pop() {
                    Some(ip) => {
                        if done.contains(&ip) {
                            continue;
                        }
                        done.insert(ip);
                        let (bl, kinds) = blocks[&ip].clone();
                        b.switch_to_block(bl);
                        let params = b.block_params(bl).to_vec();
                        let stack = params
                            .iter()
                            .zip(kinds.iter())
                            .map(|(&v, &k)| match k {
                                BKind::S(sk) => AV::C(v, sk),
                                BKind::Dyn => AV::Dyn(v),
                            })
                            .collect();
                        (ip, stack)
                    }
                    None => break,
                },
            };

            let mut ip = start_ip;
            'block: loop {
                if ip >= insts.len() {
                    return Err("fell off the end of bytecode".to_string());
                }
                if ip != start_ip && leaders.binary_search(&ip).is_ok() {
                    let mut nstack = self.norm_stack(b, &fx, &stack)?;
                    let (bl, _) =
                        self.block_for(b, &fx, &mut blocks, &mut work, ip, &mut nstack)?;
                    let args = Self::stack_args(&nstack)?;
                    b.ins().jump(bl, &args);
                    break 'block;
                }
                match &insts[ip] {
                    Instruction::Push(c) => {
                        let av = self.const_av(b, &fx, &mut vars, &obj_param_avs, c, ip)?;
                        stack.push(av);
                    }
                    Instruction::LoadLocal(sym) => {
                        let av = self.local_av(b, &fx, &vars, &obj_param_avs, *sym, ip)?;
                        stack.push(av);
                    }
                    Instruction::LoadGlobal(name) => {
                        let leaked: &'static crate::value::NamespacedName =
                            Box::leak(Box::new(name.clone()));
                        let out = self.alloc_scratch()?;
                        let out_idx = self.abs_slot(b, &fx, out);
                        let np = b.ins().iconst(types::I64, leaked as *const _ as i64);
                        let f = self.func_ref(b, self.helpers.load_global);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, np, out_idx]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, &fx, tag);
                        stack.push(AV::Dyn(out_idx));
                    }
                    Instruction::DefineLocal(sym) | Instruction::StoreLocal(sym) => {
                        let v = stack.pop().ok_or("stack underflow")?;
                        self.refuse_tracked_escape(v, ip, "a local")?;
                        if matches!((v, &insts[ip]), (AV::Nil, Instruction::DefineLocal(_)))
                            && !vars.contains_key(sym)
                            && !obj_param_avs.contains_key(sym)
                        {
                            // declaration prologue: type decided at first store —
                            // TRACKED, because a closure materialization must
                            // force still-deferred vars into real slots (a
                            // write-captured block stores them OUT-OF-BAND
                            // through its snapshot env, invisible to "first
                            // store"; the S1 recordResult nil-capture bug).
                            self.nil_deferred.insert(*sym);
                        } else if self.free_in_block(&vars, &obj_param_avs, *sym)
                            && matches!(&insts[ip], Instruction::StoreLocal(_))
                        {
                            // B3a: a captured-variable write goes through the
                            // closure's real EnvFrame cell — exact shared-cell
                            // semantics (`sum = sum + x` mutates the caller's
                            // binding, as interpreted).
                            self.emit_env_set(b, &fx, *sym, v)?;
                        } else {
                            self.store_local(b, &fx, &mut vars, &obj_param_avs, *sym, v)?;
                        }
                    }
                    Instruction::DefineLocalKeep(sym) | Instruction::StoreLocalKeep(sym) => {
                        let v = *stack.last().ok_or("stack underflow")?;
                        self.refuse_tracked_escape(v, ip, "a local")?;
                        if self.free_in_block(&vars, &obj_param_avs, *sym)
                            && matches!(&insts[ip], Instruction::StoreLocalKeep(_))
                        {
                            self.emit_env_set(b, &fx, *sym, v)?;
                        } else {
                            self.store_local(b, &fx, &mut vars, &obj_param_avs, *sym, v)?;
                        }
                    }
                    Instruction::Dup => {
                        let v = *stack.last().ok_or("stack underflow")?;
                        stack.push(v);
                    }
                    Instruction::Pop => {
                        stack.pop().ok_or("stack underflow")?;
                    }
                    Instruction::IntAdd
                    | Instruction::IntSub
                    | Instruction::IntMul
                    | Instruction::IntDiv
                    | Instruction::IntMod
                    | Instruction::IntLt
                    | Instruction::IntLe
                    | Instruction::IntGt
                    | Instruction::IntGe
                    | Instruction::IntEq
                    | Instruction::IntNe => {
                        let kind = int_inst_kind(&insts[ip]);
                        let rb = Self::pop_kind(&mut stack, AotKind::Int)?;
                        let ra = Self::pop_kind(&mut stack, AotKind::Int)?;
                        let out = self.emit_int_bin(b, &fx, kind, ra, rb)?;
                        stack.push(out);
                    }
                    Instruction::IntBinLL(a, bb, kind) => {
                        let ra =
                            self.local_scalar(b, &fx, &vars, &obj_param_avs, *a, AotKind::Int, ip)?;
                        let rb = self.local_scalar(
                            b,
                            &fx,
                            &vars,
                            &obj_param_avs,
                            *bb,
                            AotKind::Int,
                            ip,
                        )?;
                        let out = self.emit_int_bin(b, &fx, *kind, ra, rb)?;
                        stack.push(out);
                    }
                    Instruction::IntBinLC(a, c, kind) => {
                        let ra =
                            self.local_scalar(b, &fx, &vars, &obj_param_avs, *a, AotKind::Int, ip)?;
                        let ci = c.as_int().ok_or("IntBinLC without int constant")?;
                        let rb = b.ins().iconst(types::I64, ci);
                        let out = self.emit_int_bin(b, &fx, *kind, ra, rb)?;
                        stack.push(out);
                    }
                    Instruction::DoubleAdd
                    | Instruction::DoubleSub
                    | Instruction::DoubleMul
                    | Instruction::DoubleDiv
                    | Instruction::DoubleMod
                    | Instruction::DoubleLt
                    | Instruction::DoubleLe
                    | Instruction::DoubleGt
                    | Instruction::DoubleGe
                    | Instruction::DoubleEq
                    | Instruction::DoubleNe => {
                        let kind = double_inst_kind(&insts[ip]);
                        let rb = Self::pop_kind(&mut stack, AotKind::Double)?;
                        let ra = Self::pop_kind(&mut stack, AotKind::Double)?;
                        let out = self.emit_double_bin(b, kind, ra, rb);
                        stack.push(out);
                    }
                    Instruction::DoubleBinLL(a, bb, kind) => {
                        let ra = self.local_scalar(
                            b,
                            &fx,
                            &vars,
                            &obj_param_avs,
                            *a,
                            AotKind::Double,
                            ip,
                        )?;
                        let rb = self.local_scalar(
                            b,
                            &fx,
                            &vars,
                            &obj_param_avs,
                            *bb,
                            AotKind::Double,
                            ip,
                        )?;
                        let out = self.emit_double_bin(b, *kind, ra, rb);
                        stack.push(out);
                    }
                    Instruction::DoubleBinLC(a, c, kind) => {
                        let ra = self.local_scalar(
                            b,
                            &fx,
                            &vars,
                            &obj_param_avs,
                            *a,
                            AotKind::Double,
                            ip,
                        )?;
                        let cd = match c {
                            Constant::Double(d) => *d,
                            Constant::Int(i) => *i as f64,
                            _ => return Err("DoubleBinLC without numeric constant".into()),
                        };
                        let rb = b.ins().f64const(cd);
                        let out = self.emit_double_bin(b, *kind, ra, rb);
                        stack.push(out);
                    }
                    Instruction::NewList(n) => {
                        let n = *n;
                        let out = self.alloc_scratch()?;
                        let out_idx = self.abs_slot(b, &fx, out);
                        if n == 0 {
                            let f = self.func_ref(b, self.helpers.list_new);
                            let call = b.ins().call(f, &[fx.vm, fx.mc, out_idx]);
                            let tag = b.inst_results(call)[0];
                            self.tag_check(b, &fx, tag);
                        } else {
                            if n > MAX_OUTCALL_ARGS {
                                return Err("list literal too long for v0.2".into());
                            }
                            let elems: Vec<AV> =
                                stack.split_off(stack.len().checked_sub(n).ok_or("underflow")?);
                            for e in &elems {
                                self.refuse_tracked_escape(*e, ip, "a list literal")?;
                            }
                            self.fill_lanes(b, &fx, &elems)?;
                            let ka = b.ins().stack_addr(types::I64, fx.kinds_buf, 0);
                            let ba = b.ins().stack_addr(types::I64, fx.bits_buf, 0);
                            let nn = b.ins().iconst(types::I64, n as i64);
                            let f = self.func_ref(b, self.helpers.list_from);
                            let call = b.ins().call(f, &[fx.vm, fx.mc, out_idx, nn, ka, ba]);
                            let tag = b.inst_results(call)[0];
                            self.tag_check(b, &fx, tag);
                        }
                        stack.push(AV::Dyn(out_idx));
                    }
                    Instruction::ListPush => {
                        let val = stack.pop().ok_or("stack underflow")?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        self.refuse_tracked_escape(val, ip, "a list")?;
                        let recv_idx = self.obj_index(b, &fx, recv, "ListPush receiver")?;
                        let (k, bits) = self.encode(b, &fx, val);
                        let f = self.func_ref(b, self.helpers.list_push);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, recv_idx, k, bits]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, &fx, tag);
                        stack.push(AV::Dyn(recv_idx));
                    }
                    Instruction::BranchIfNotList(_) => {
                        // The fused-`each:` guard (B1, docs/BLOCK_AOT_ARCH.md §3). A
                        // PROVEN native-List receiver takes the hot path
                        // unconditionally — no branch is emitted, so nothing ever
                        // jumps to the cold path (the literal re-materialization +
                        // real send) and it is never translated: the same
                        // reachability discipline that deleted the sieve refusal
                        // (G3). An unproven receiver refuses the member — the
                        // interpreter's guarded loop still runs it.
                        let proven = match stack.last() {
                            Some(AV::Dyn(cv)) => matches!(
                                self.proofs.get(cv),
                                Some(DynProof::NativeList) | Some(DynProof::CollectionOf(_))
                            ),
                            // A guard on `self` (an open-owner combinator body, B2)
                            // becomes the ENTRY's precondition: `invoke` Bails to
                            // the interpreted body for non-List receivers, so the
                            // hot path is proven-by-entry.
                            Some(AV::SelfRef) => {
                                self.needs_list_self = true;
                                true
                            }
                            _ => false,
                        };
                        if !proven {
                            return Err(format!(
                                "fused each: on an unproven receiver at ip {ip} — a \
                                 `List`-annotated param, a fresh/checked list, or `self` \
                                 (entry-gated) compiles"
                            ));
                        }
                    }
                    Instruction::ListLen => {
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let recv_idx = self.obj_index(b, &fx, recv, "ListLen receiver")?;
                        let f = self.func_ref(b, self.helpers.list_len);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, recv_idx]);
                        let len = b.inst_results(call)[0];
                        stack.push(AV::C(len, AotKind::Int));
                    }
                    Instruction::ListGet => {
                        let idx = Self::pop_kind(&mut stack, AotKind::Int)?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let recv_proof = match &recv {
                            AV::Dyn(ri) => self.proofs.get(ri).copied(),
                            _ => None,
                        };
                        let recv_idx = self.obj_index(b, &fx, recv, "ListGet receiver")?;
                        let out = self.alloc_scratch()?;
                        let out_idx = self.abs_slot(b, &fx, out);
                        let f = self.func_ref(b, self.helpers.list_get);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, recv_idx, idx, out_idx]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, &fx, tag);
                        if let Some(DynProof::CollectionOf(t)) = recv_proof {
                            // Tag-enforced source: the element is PROVEN t-or-nil.
                            self.proofs.insert(out_idx, DynProof::ElemOrNil(t));
                        }
                        stack.push(AV::Dyn(out_idx));
                    }
                    Instruction::TagCollection(tag) => {
                        // Verify + stamp the fresh literal on top of the stack
                        // (same helper contract as the interpreter arm), and
                        // record the PROOF — this is a tag the runtime enforces.
                        let AV::Dyn(idx) = *stack.last().ok_or("stack underflow")? else {
                            return Err("TagCollection on a non-slot value".into());
                        };
                        let Some(code) = tag.code() else {
                            return Err("user-class element tags in compiled literals are not \
                                 supported yet"
                                .into());
                        };
                        let code_v = b.ins().iconst(types::I64, code);
                        let f = self.func_ref(b, self.helpers.tag_collection);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, idx, code_v]);
                        let t = b.inst_results(call)[0];
                        self.tag_check(b, &fx, t);
                        self.proofs.insert(idx, DynProof::CollectionOf(*tag));
                    }
                    Instruction::ListSet => {
                        let val = stack.pop().ok_or("stack underflow")?;
                        let idx = Self::pop_kind(&mut stack, AotKind::Int)?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        self.refuse_tracked_escape(val, ip, "a list")?;
                        let recv_idx = self.obj_index(b, &fx, recv, "ListSet receiver")?;
                        let (k, bits) = self.encode(b, &fx, val);
                        let f = self.func_ref(b, self.helpers.list_set);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, recv_idx, idx, k, bits]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, &fx, tag);
                        stack.push(AV::Dyn(recv_idx));
                    }
                    // Map devirt ops reissue as outcalls: the native methods do
                    // the same lookup, so behavior is identical, and it keeps
                    // string keys out of the compiled ABI.
                    Instruction::MapGet => {
                        let key = stack.pop().ok_or("stack underflow")?;
                        self.refuse_tracked_escape(key, ip, "a map key")?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let out = self.emit_outcall(b, &fx, recv, "at:", &[key], ip)?;
                        stack.push(out);
                    }
                    Instruction::MapSet => {
                        let val = stack.pop().ok_or("stack underflow")?;
                        let key = stack.pop().ok_or("stack underflow")?;
                        // These reissue as outcalls WITHOUT the send head's
                        // post-send write-back flush — a tracked closure here
                        // would orphan its obligations.
                        self.refuse_tracked_escape(val, ip, "a map")?;
                        self.refuse_tracked_escape(key, ip, "a map key")?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let out = self.emit_outcall(b, &fx, recv, "at:put:", &[key, val], ip)?;
                        stack.push(out);
                    }
                    Instruction::Jump(off) => {
                        let target = (ip as isize + off) as usize;
                        let mut nstack = self.norm_stack(b, &fx, &stack)?;
                        if target <= ip {
                            nstack = self.emit_fuel_tick(b, &fx, &nstack)?;
                        }
                        let (bl, _) =
                            self.block_for(b, &fx, &mut blocks, &mut work, target, &mut nstack)?;
                        let args = Self::stack_args(&nstack)?;
                        b.ins().jump(bl, &args);
                        break 'block;
                    }
                    Instruction::IfJump(off) | Instruction::ElseJump(off) => {
                        // `is_truthy` lowering: everything but `false` and `nil`
                        // is truthy (the interpreter's exact contract). Statically
                        // known scalars fold; dynamic values peek their kind.
                        let cond = match stack.pop().ok_or("stack underflow")? {
                            AV::C(v, AotKind::Bool) => v,
                            AV::C(..) | AV::SelfRef => b.ins().iconst(types::I8, 1),
                            AV::Nil => b.ins().iconst(types::I8, 0),
                            AV::Dyn(idx) => {
                                let f = self.func_ref(b, self.helpers.slot_peek);
                                let oa = b.ins().stack_addr(types::I64, fx.peek_out, 0);
                                let call = b.ins().call(f, &[fx.vm, fx.mc, idx, oa]);
                                let kind = b.inst_results(call)[0];
                                let bits = b.ins().stack_load(types::I64, fx.peek_out, 0);
                                let is_nil = b.ins().icmp_imm(IntCC::Equal, kind, KIND_NIL);
                                let is_bool = b.ins().icmp_imm(IntCC::Equal, kind, KIND_BOOL);
                                let is_zero = b.ins().icmp_imm(IntCC::Equal, bits, 0);
                                let is_false = b.ins().band(is_bool, is_zero);
                                let not_truthy = b.ins().bor(is_nil, is_false);
                                b.ins().icmp_imm(IntCC::Equal, not_truthy, 0)
                            }
                        };
                        let target = (ip as isize + off) as usize;
                        let mut nstack = self.norm_stack(b, &fx, &stack)?;
                        if target <= ip {
                            // Conditional back-edge: tick before the branch
                            // (loops must stay preemptible and cancellable).
                            nstack = self.emit_fuel_tick(b, &fx, &nstack)?;
                        }
                        let (tbl, _) =
                            self.block_for(b, &fx, &mut blocks, &mut work, target, &mut nstack)?;
                        let (fbl, _) =
                            self.block_for(b, &fx, &mut blocks, &mut work, ip + 1, &mut nstack)?;
                        let args = Self::stack_args(&nstack)?;
                        if matches!(insts[ip], Instruction::IfJump(_)) {
                            b.ins().brif(cond, tbl, &args, fbl, &args);
                        } else {
                            b.ins().brif(cond, fbl, &args, tbl, &args);
                        }
                        break 'block;
                    }
                    Instruction::BranchIfNotBool(off) => {
                        let target = (ip as isize + off) as usize;
                        match *stack.last().ok_or("stack underflow")? {
                            AV::C(_, AotKind::Bool) => {} // statically Bool: fall through
                            AV::C(..) | AV::Nil | AV::SelfRef => {
                                // Statically not a Bool: always the cold path.
                                let mut nstack = self.norm_stack(b, &fx, &stack)?;
                                let (bl, _) = self.block_for(
                                    b,
                                    &fx,
                                    &mut blocks,
                                    &mut work,
                                    target,
                                    &mut nstack,
                                )?;
                                let args = Self::stack_args(&nstack)?;
                                b.ins().jump(bl, &args);
                                break 'block;
                            }
                            AV::Dyn(idx) => {
                                let f = self.func_ref(b, self.helpers.slot_peek);
                                let oa = b.ins().stack_addr(types::I64, fx.peek_out, 0);
                                let call = b.ins().call(f, &[fx.vm, fx.mc, idx, oa]);
                                let kind = b.inst_results(call)[0];
                                let is_bool = b.ins().icmp_imm(IntCC::Equal, kind, KIND_BOOL);
                                let bits = b.ins().stack_load(types::I64, fx.peek_out, 0);
                                let as_bool = b.ins().icmp_imm(IntCC::NotEqual, bits, 0);
                                let mut hot_stack = self.norm_stack(b, &fx, &stack)?;
                                hot_stack.pop();
                                hot_stack.push(AV::C(as_bool, AotKind::Bool));
                                let (hot, _) = self.block_for(
                                    b,
                                    &fx,
                                    &mut blocks,
                                    &mut work,
                                    ip + 1,
                                    &mut hot_stack,
                                )?;
                                let hot_args = Self::stack_args(&hot_stack)?;
                                if self.proofs.get(&idx).copied()
                                    == Some(DynProof::ElemOrNil(ElemTag::Bool))
                                {
                                    // PROVEN Boolean-or-nil (a checked List(Boolean)
                                    // element read): the only non-Bool is nil, whose
                                    // sealed class has no `if:` — raise the exact
                                    // interpreter MNU instead of jumping to the cold
                                    // path. Nothing jumps there, so its capturing
                                    // block re-materialization is never translated:
                                    // this deletes the sieve refusal
                                    // (GENERICS_ARCH.md §7, AOT_ARCH.md §9).
                                    let (sel, argc) = Self::cold_send(insts, target)?;
                                    let mnu_bl = b.create_block();
                                    b.ins().brif(is_bool, hot, &hot_args, mnu_bl, &[]);
                                    b.switch_to_block(mnu_bl);
                                    let sel_ptr = b.ins().iconst(
                                        types::I64,
                                        Box::leak(Box::new(sel)) as *const Symbol as i64,
                                    );
                                    let argc_v = b.ins().iconst(types::I64, argc);
                                    let nf = self.func_ref(b, self.helpers.nil_mnu);
                                    let call = b
                                        .ins()
                                        .call(nf, &[fx.vm, fx.mc, kind, bits, sel_ptr, argc_v]);
                                    let etag = b.inst_results(call)[0];
                                    let zero = self.zero_of(b, fx.ret);
                                    b.ins().jump(fx.exit, &[etag.into(), zero.into()]);
                                    break 'block;
                                }
                                // Unproven: Bool → hot; anything else → the cold
                                // path's real send.
                                let mut nstack = self.norm_stack(b, &fx, &stack)?;
                                let (cold, _) = self.block_for(
                                    b,
                                    &fx,
                                    &mut blocks,
                                    &mut work,
                                    target,
                                    &mut nstack,
                                )?;
                                let cold_args = Self::stack_args(&nstack)?;
                                b.ins().brif(is_bool, hot, &hot_args, cold, &cold_args);
                                break 'block;
                            }
                        }
                    }
                    // Fused sends push their folded operand(s), then share the
                    // generic path: `exec_send` semantics pop n args, then the
                    // receiver (the fused operand is the receiver only for
                    // 0-arg sends — for n≥1 it is the LAST ARGUMENT).
                    Instruction::Send(sel, n)
                    | Instruction::SendLocal(_, sel, n)
                    | Instruction::SendConst(_, sel, n)
                    | Instruction::SendField(_, sel, n)
                    | Instruction::SendLocalLocal(_, _, sel, n)
                    | Instruction::SendLocalConst(_, _, sel, n) => {
                        let (sel, n) = (*sel, *n);
                        match &insts[ip] {
                            Instruction::SendLocal(a, ..) => {
                                let v = self.local_av(b, &fx, &vars, &obj_param_avs, *a, ip)?;
                                stack.push(v);
                            }
                            Instruction::SendField(field, ..) => {
                                // Interpreter parity: `SendField` loads the
                                // field UNCACHED (its single ip belongs to the
                                // send IC), then shares the send tail.
                                let v = self.emit_field_get_uncached(b, &fx, field)?;
                                stack.push(v);
                            }
                            Instruction::SendConst(c, ..) => {
                                let v = self.const_av(b, &fx, &mut vars, &obj_param_avs, c, ip)?;
                                stack.push(v);
                            }
                            Instruction::SendLocalLocal(a, bb, ..) => {
                                let v = self.local_av(b, &fx, &vars, &obj_param_avs, *a, ip)?;
                                stack.push(v);
                                let v = self.local_av(b, &fx, &vars, &obj_param_avs, *bb, ip)?;
                                stack.push(v);
                            }
                            Instruction::SendLocalConst(a, c, ..) => {
                                let v = self.local_av(b, &fx, &vars, &obj_param_avs, *a, ip)?;
                                stack.push(v);
                                let v = self.const_av(b, &fx, &mut vars, &obj_param_avs, c, ip)?;
                                stack.push(v);
                            }
                            _ => {}
                        }
                        let args: Vec<AV> =
                            stack.split_off(stack.len().checked_sub(n).ok_or("underflow")?);
                        let recv = stack.pop().ok_or("stack underflow")?;
                        // B3a: the combinator seam — `valueWithSelfOrArg:` routes
                        // through the block-call helper, invoking a COMPILED block
                        // template directly on a registry hit (else the interpreted
                        // body, else the full send for non-block receivers).
                        // Sibling-closure interference: 2+ materialized
                        // closures consumed together, any of which WRITES a
                        // capture, cannot keep exact shared-cell semantics
                        // across independent snapshots — refuse (unfused
                        // `whileDo:`-shaped methods run interpreted).
                        let closure_args: Vec<CVal> = std::iter::once(&recv)
                            .chain(args.iter())
                            .filter_map(|v| match v {
                                AV::Dyn(idx) if self.materialized.contains(idx) => Some(*idx),
                                _ => None,
                            })
                            .collect();
                        if closure_args.len() >= 2
                            && closure_args
                                .iter()
                                .any(|idx| self.pending_writebacks.contains_key(idx))
                        {
                            return Err(format!(
                                "sibling closures share written captures at ip {ip}"
                            ));
                        }
                        // A `catch`-family send consuming a `^^`-carrying
                        // closure: interpreted, a catch-all can CATCH the
                        // `^^` crossing it; a compiled home cannot reproduce
                        // that (in-flight compiled-target `^^` is
                        // uncatchable) — the method runs interpreted. The
                        // predicate lives next to the runtime registrations
                        // it must mirror.
                        if crate::runtime::block::is_catch_family(sel.as_str())
                            && closure_args
                                .iter()
                                .any(|idx| self.materialized_nlr.contains(idx))
                        {
                            return Err(format!("non-local return (^^) under a catch at ip {ip}"));
                        }
                        let out = if sel.as_str() == "valueWithSelfOrArg:" && args.len() == 1 {
                            self.emit_block_call(b, &fx, recv, args[0], ip)?
                        } else {
                            self.emit_send(b, &fx, recv, sel, &args, ip)?
                        };
                        let mut consumed = args.clone();
                        consumed.push(recv);
                        self.flush_writebacks(b, &fx, &consumed)?;
                        stack.push(out);
                    }
                    // Within one method's bytecode, `MethodReturn` (`^^`) always
                    // targets THIS method's frame — a real nested block is a separate
                    // `StaticBlock` never translated inline, and a fused-`each:` body
                    // (B1) is spliced into this very frame. So all three return forms
                    // are the compiled function's return.
                    // In a BLOCK TEMPLATE (B3a) a `^^` must unwind interpreter
                    // frames the compiled world doesn't have — refused.
                    Instruction::MethodReturn if self.cand.role == AotRole::BlockTemplate => {
                        return Err(format!(
                            "non-local return (^^) from a compiled block at ip {ip}"
                        ));
                    }
                    Instruction::LoadField(name) => {
                        let out = self.emit_field_get(b, &fx, name, ip)?;
                        stack.push(out);
                    }
                    Instruction::StoreField(name) => {
                        let v = stack.pop().ok_or("stack underflow")?;
                        self.refuse_tracked_escape(v, ip, "a field")?;
                        self.emit_field_set(b, &fx, name, v, ip)?;
                    }
                    Instruction::StoreFieldKeep(name) => {
                        let v = *stack.last().ok_or("stack underflow")?;
                        self.refuse_tracked_escape(v, ip, "a field")?;
                        self.emit_field_set(b, &fx, name, v, ip)?;
                    }
                    Instruction::Return | Instruction::BlockReturn | Instruction::MethodReturn => {
                        let v = stack.pop().ok_or("stack underflow")?;
                        self.emit_return(b, &fx, v)?;
                        break 'block;
                    }
                    other => {
                        return Err(format!("unsupported instruction at ip {ip}: {other:?}"));
                    }
                }
                ip += 1;
            }
        }
        Ok(())
    }

    fn func_ref(&mut self, b: &mut FunctionBuilder, id: FuncId) -> cranelift_codegen::ir::FuncRef {
        self.module.declare_func_in_func(id, b.func)
    }

    /// The absolute slot index of an object-shaped AV (helpers take indices).
    fn obj_index(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        v: AV,
        what: &str,
    ) -> Result<CVal, String> {
        match v {
            AV::Dyn(idx) => Ok(idx),
            AV::SelfRef => Ok(self.abs_slot(b, fx, 0)),
            _ => Err(format!("{what} is not slot-resident")),
        }
    }

    /// Is `sym` a FREE variable of a block template — not a param, not a
    /// block-own local, not `self`? (Method role: never — unknown names there
    /// are compile errors, as before.)
    fn free_in_block(
        &self,
        vars: &HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        sym: Symbol,
    ) -> bool {
        self.cand.role == AotRole::BlockTemplate
            && sym != self_symbol()
            && !vars.contains_key(&sym)
            && !obj_params.contains_key(&sym)
    }

    /// Read a captured variable through the closure's EnvFrame chain (B3a).
    fn emit_env_get(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        sym: Symbol,
    ) -> Result<AV, String> {
        let block_idx = self.abs_slot(b, fx, 2);
        let leaked: &'static Symbol = Box::leak(Box::new(sym));
        let sym_ptr = b.ins().iconst(types::I64, leaked as *const Symbol as i64);
        let out = self.alloc_scratch()?;
        let out_idx = self.abs_slot(b, fx, out);
        let f = self.func_ref(b, self.helpers.env_get);
        let call = b
            .ins()
            .call(f, &[fx.vm, fx.mc, block_idx, sym_ptr, out_idx]);
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(AV::Dyn(out_idx))
    }

    /// Write a captured variable through the closure's EnvFrame chain (B3a) —
    /// the same shared cell the enclosing frame reads.
    fn emit_env_set(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        sym: Symbol,
        v: AV,
    ) -> Result<(), String> {
        let block_idx = self.abs_slot(b, fx, 2);
        let leaked: &'static Symbol = Box::leak(Box::new(sym));
        let sym_ptr = b.ins().iconst(types::I64, leaked as *const Symbol as i64);
        let (k, bits) = self.encode(b, fx, v);
        let f = self.func_ref(b, self.helpers.env_set);
        let call = b
            .ins()
            .call(f, &[fx.vm, fx.mc, block_idx, sym_ptr, k, bits]);
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(())
    }

    /// Per-element block invocation (B3a): registry hit → the compiled block
    /// template directly; miss → the interpreted body; non-block receiver →
    /// the full `valueWithSelfOrArg:` send. One helper call either way.
    fn emit_block_call(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        recv: AV,
        arg: AV,
        ip: usize,
    ) -> Result<AV, String> {
        let (rk, rbits) = self.encode(b, fx, recv);
        let (ak, abits) = self.encode(b, fx, arg);
        let out = self.alloc_scratch()?;
        let out_idx = self.abs_slot(b, fx, out);
        let (tid_v, ip_v, len_v) = self.site_consts(b, ip);
        let f = self.func_ref(b, self.helpers.block_call);
        let call = b.ins().call(
            f,
            &[
                fx.vm, fx.mc, tid_v, ip_v, len_v, rk, rbits, ak, abits, out_idx,
            ],
        );
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(AV::Dyn(out_idx))
    }

    /// `@name` read (S3): the receiver is this frame's slot-0 value and the
    /// slot cache is the shared `(template_id, ip)` cell — compiled and
    /// interpreted execution warm ONE field cache.
    /// `SendField`'s field read: the interpreter passes `cache_ip: None`
    /// there (the ip's cache slot belongs to the SEND), mirrored here by the
    /// out-of-range ip sentinel — probe and fill both miss past `bc_len`.
    fn emit_field_get_uncached(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        name: &str,
    ) -> Result<AV, String> {
        let sentinel = self.cand.block.bytecode.0.len();
        self.emit_field_get(b, fx, name, sentinel)
    }

    fn emit_field_get(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        name: &str,
        ip: usize,
    ) -> Result<AV, String> {
        let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
        let name_ptr = b.ins().iconst(types::I64, leaked.as_ptr() as i64);
        let name_len = b.ins().iconst(types::I64, leaked.len() as i64);
        let self_idx = self.abs_slot(b, fx, 0);
        let out = self.alloc_scratch()?;
        let out_idx = self.abs_slot(b, fx, out);
        let (tid_v, ip_v, len_v) = self.site_consts(b, ip);
        let f = self.func_ref(b, self.helpers.field_get);
        let call = b.ins().call(
            f,
            &[
                fx.vm, fx.mc, tid_v, ip_v, len_v, self_idx, name_ptr, name_len, out_idx,
            ],
        );
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(AV::Dyn(out_idx))
    }

    /// `@name = v` (S3) — same shared cache; undeclared fields raise the
    /// interpreter's exact errors through the tag channel.
    fn emit_field_set(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        name: &str,
        v: AV,
        ip: usize,
    ) -> Result<(), String> {
        let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
        let name_ptr = b.ins().iconst(types::I64, leaked.as_ptr() as i64);
        let name_len = b.ins().iconst(types::I64, leaked.len() as i64);
        let self_idx = self.abs_slot(b, fx, 0);
        let (k, bits) = self.encode(b, fx, v);
        let (tid_v, ip_v, len_v) = self.site_consts(b, ip);
        let f = self.func_ref(b, self.helpers.field_set);
        let call = b.ins().call(
            f,
            &[
                fx.vm, fx.mc, tid_v, ip_v, len_v, self_idx, name_ptr, name_len, k, bits,
            ],
        );
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(())
    }

    /// B3b: materialize a closure at a compiled cold-path `Push(Block)` site.
    /// The snapshot env carries EVERY frame binding (scalars, slots, obj
    /// params, `self`) — exactly the visibility the interpreter's live env
    /// chain would give — and the gates guarantee the whole NEST only READS
    /// its frame captures (a captured-var write would mutate the snapshot,
    /// invisible to the compiled frame) or writes ones flushed back after the
    /// consuming send. Known accepted edge (documented): a closure that
    /// ESCAPES its consuming send (a custom `if:` storing it) sees the
    /// snapshot, not later frame writes.
    fn materialize_closure(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &mut HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        rc: &Rc<StaticBlock>,
        ip: usize,
    ) -> Result<AV, String> {
        // Force still-deferred `var x = nil` locals into REAL slots before
        // snapshotting: the closure may read or write them through its
        // snapshot env, which "type decided at first store" cannot see. The
        // slot init to nil is exact — any earlier store would have typed the
        // var already.
        let deferred: Vec<Symbol> = self.nil_deferred.drain().collect();
        for sym in deferred {
            let w = self.alloc_scratch()?;
            let idx = self.abs_slot(b, fx, w);
            let (k, bits) = self.encode(b, fx, AV::Nil);
            let f = self.func_ref(b, self.helpers.slot_set);
            let call = b.ins().call(f, &[fx.vm, fx.mc, idx, k, bits]);
            let tag = b.inst_results(call)[0];
            self.tag_check(b, fx, tag);
            vars.insert(sym, VarSlot::Obj(w, None));
        }

        // Gates: scan the template's bytecode — TRANSITIVELY through nested
        // literals (S5b). The materialized closure runs INTERPRETED, so
        // nested blocks execute naturally (their env chain threads through
        // the closure's frame into the snapshot, and their `^^` home is
        // inherited from the closure's frame — the S5a machinery); the gate
        // only needs whole-nest knowledge of free WRITES (for writebacks),
        // `^^` presence, guarded blocks, and the trampoline signature.
        let mut scan = NestScan::default();
        scan_materialized_nest(rc, &HashSet::new(), self.cand.selector.as_str(), &mut scan)
            .map_err(|e| format!("{e} at ip {ip}"))?;
        let written_frees = scan.written_frees;
        let has_nlr = scan.has_nlr;
        // A `^^` with a catch-family send anywhere in the same nest: the
        // interpreted method would let a catch-all CATCH the `^^` crossing
        // it, which a compiled home cannot reproduce — refuse (mirrors the
        // send-head gate for method-level catch consumers).
        if has_nlr && scan.has_catch_send {
            return Err(format!(
                "non-local return (^^) with a catch in a materialized nest at ip {ip}"
            ));
        }
        // PROFITABILITY (S5a, empirical): a `^^` cold arm pays a snapshot
        // materialization (fresh EnvFrame + a bind per frame binding) each
        // time its site executes, so it is only worth compiling where it
        // runs AT MOST ONCE per invocation. Two shapes make it per-ITERATION
        // and pessimize the whole method — found by A/B: qnlib's `whileDo:`
        // trampoline made sieve 5.8x slower, `any?:` cost combinators 60%:
        // - the site sits inside a fused-loop span (a backward jump crosses
        //   it): one arm snapshot per element;
        // - the arm re-sends the candidate's OWN selector (the
        //   `^^s.whileDo:block` tail-recursive trampoline): one recursive
        //   call — and one snapshot — per iteration.
        // Straight-line early exits (richards' task bodies) pass both.
        if has_nlr {
            let insts = &self.cand.block.bytecode.0;
            // Every offset-carrying form counts (the guard branches only go
            // forward TODAY, but a future backward fused form must not
            // silently reopen the per-iteration hole).
            let in_loop = insts.iter().enumerate().any(|(j, inst)| match inst {
                Instruction::Jump(o)
                | Instruction::IfJump(o)
                | Instruction::ElseJump(o)
                | Instruction::BranchIfNotBool(o)
                | Instruction::BranchIfNotList(o) => {
                    *o < 0 && {
                        let target = (j as isize + *o) as usize;
                        target <= ip && ip <= j
                    }
                }
                _ => false,
            });
            if in_loop {
                return Err(format!(
                    "per-iteration ^^ materialization (fused loop) at ip {ip}"
                ));
            }
            if scan.sends_own_selector {
                return Err(format!(
                    "per-iteration ^^ materialization (recursive trampoline) at ip {ip}"
                ));
            }
        }
        // A write to a FRAME local mutates the snapshot — read back after the
        // consuming send. A write that resolves DEEPER than this frame walks
        // past the snapshot into the real env cells (exact as-is). A write to
        // a param/self has no writable home — refuse.
        let mut writebacks: Vec<(Symbol, VarSlot)> = Vec::new();
        for s in written_frees {
            if writebacks.iter().any(|(w, _)| *w == s) {
                continue;
            }
            if let Some(&slot) = vars.get(&s) {
                writebacks.push((s, slot));
            } else if obj_params.contains_key(&s) || s == self_symbol() {
                return Err(format!(
                    "materialized block writes parameter/self '{}' at ip {ip}",
                    s.as_str()
                ));
            }
        }
        // Build the closure in a scratch slot (rooted throughout), then bind
        // the whole frame environment into its snapshot env.
        let tmpl: &'static Rc<StaticBlock> = Box::leak(Box::new(rc.clone()));
        let tmpl_ptr = b
            .ins()
            .iconst(types::I64, tmpl as *const Rc<StaticBlock> as i64);
        let out = self.alloc_scratch()?;
        let out_idx = self.abs_slot(b, fx, out);
        let want_home = b.ins().iconst(types::I64, i64::from(has_nlr));
        let f = self.func_ref(b, self.helpers.make_closure);
        let call = b
            .ins()
            .call(f, &[fx.vm, fx.mc, tmpl_ptr, out_idx, want_home]);
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        let bind = |tr: &mut Self, b: &mut FunctionBuilder, sym: Symbol, v: AV| {
            let leaked: &'static Symbol = Box::leak(Box::new(sym));
            let sym_ptr = b.ins().iconst(types::I64, leaked as *const Symbol as i64);
            let (k, bits) = tr.encode(b, fx, v);
            let f = tr.func_ref(b, tr.helpers.closure_bind);
            let call = b.ins().call(f, &[fx.vm, fx.mc, out_idx, sym_ptr, k, bits]);
            let tag = b.inst_results(call)[0];
            tr.tag_check(b, fx, tag);
        };
        bind(self, b, self_symbol(), AV::SelfRef);
        for (&sym, &cv) in obj_params.iter() {
            bind(self, b, sym, AV::Dyn(cv));
        }
        let entries: Vec<(Symbol, VarSlot)> = vars.iter().map(|(&s, &v)| (s, v)).collect();
        for (sym, slot) in entries {
            let av = match slot {
                VarSlot::Scalar(var, k) => AV::C(b.use_var(var), k),
                VarSlot::Obj(w, _) => {
                    let idx = self.abs_slot(b, fx, w);
                    AV::Dyn(idx)
                }
            };
            bind(self, b, sym, av);
        }
        if !writebacks.is_empty() {
            self.pending_writebacks.insert(out_idx, writebacks);
        }
        self.materialized.insert(out_idx);
        if has_nlr {
            self.materialized_nlr.insert(out_idx);
        }
        Ok(AV::Dyn(out_idx))
    }

    /// Flush a consumed closure's captured-write read-backs (B3b): each
    /// written frame local is read from the snapshot env and stored back into
    /// its SSA/slot home. Called right after the consuming send returns.
    fn flush_writebacks(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        consumed: &[AV],
    ) -> Result<(), String> {
        for v in consumed {
            let AV::Dyn(idx) = v else { continue };
            let Some(wbs) = self.pending_writebacks.remove(idx) else {
                continue;
            };
            for (sym, slot) in wbs {
                let leaked: &'static Symbol = Box::leak(Box::new(sym));
                let sym_ptr = b.ins().iconst(types::I64, leaked as *const Symbol as i64);
                let tmp = self.alloc_scratch()?;
                let tmp_idx = self.abs_slot(b, fx, tmp);
                let f = self.func_ref(b, self.helpers.env_get);
                let call = b.ins().call(f, &[fx.vm, fx.mc, *idx, sym_ptr, tmp_idx]);
                let tag = b.inst_results(call)[0];
                self.tag_check(b, fx, tag);
                match slot {
                    VarSlot::Scalar(var, k) => {
                        let val = self.narrow_to_scalar(b, fx, tmp_idx, k);
                        b.def_var(var, val);
                    }
                    VarSlot::Obj(w, _) => {
                        let dst = self.abs_slot(b, fx, w);
                        let kind = b.ins().iconst(types::I64, KIND_SLOT);
                        let f = self.func_ref(b, self.helpers.slot_set);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, dst, kind, tmp_idx]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, fx, tag);
                    }
                }
            }
        }
        Ok(())
    }

    /// A materialized closure with OBLIGATIONS — pending write-backs, or a
    /// `^^` whose catch-parity gate must see its consumer — is tracked by
    /// the SSA value of its slot, and that bookkeeping does NOT survive a
    /// store/load round-trip through a local, field, or collection: the
    /// reloaded value is a fresh SSA id, so the obligations silently orphan.
    /// (Found live: `var blk = { total = total + 1 }; .run:blk` compiled to
    /// a method whose write-backs never flushed — `total` stayed 0.) Such
    /// closures must flow DIRECTLY from materialization to their consuming
    /// send; any escape refuses. Obligation-free closures may escape (their
    /// only divergence is the documented snapshot-vs-live-env edge).
    fn refuse_tracked_escape(&self, v: AV, ip: usize, what: &str) -> Result<(), String> {
        if let AV::Dyn(idx) = v {
            if self.pending_writebacks.contains_key(&idx) {
                return Err(format!(
                    "write-capturing closure escapes to {what} at ip {ip}"
                ));
            }
            if self.materialized_nlr.contains(&idx) {
                return Err(format!(
                    "non-local-return closure escapes to {what} at ip {ip}"
                ));
            }
        }
        Ok(())
    }

    fn const_av(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &mut HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        c: &Constant,
        ip: usize,
    ) -> Result<AV, String> {
        Ok(match c {
            Constant::Int(i) => AV::C(b.ins().iconst(types::I64, *i), AotKind::Int),
            Constant::Double(d) => AV::C(b.ins().f64const(*d), AotKind::Double),
            Constant::Bool(x) => AV::C(b.ins().iconst(types::I8, *x as i64), AotKind::Bool),
            Constant::Nil => AV::Nil,
            Constant::String(s) => {
                // Leak once per site; the code referencing it is process-lived.
                let leaked: &'static str = Box::leak(s.clone().into_boxed_str());
                let out = self.alloc_scratch()?;
                let out_idx = self.abs_slot(b, fx, out);
                let ptr = b.ins().iconst(types::I64, leaked.as_ptr() as i64);
                let len = b.ins().iconst(types::I64, leaked.len() as i64);
                let f = self.func_ref(b, self.helpers.string_const);
                let call = b.ins().call(f, &[fx.vm, fx.mc, ptr, len, out_idx]);
                let tag = b.inst_results(call)[0];
                self.tag_check(b, fx, tag);
                AV::Dyn(out_idx)
            }
            Constant::Block(rc) => {
                // B3b: materialize a real closure over a SNAPSHOT of the whole
                // frame environment (docs/BLOCK_AOT_ARCH.md §3). Gated to
                // read-only captures, no `^^`, no nested literals, no guard
                // block — anything else still refuses.
                let rc = rc.clone();
                return self.materialize_closure(b, fx, vars, obj_params, &rc, ip);
            }
            _ => return Err(format!("unsupported constant at ip {ip}")),
        })
    }

    fn local_av(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        sym: Symbol,
        ip: usize,
    ) -> Result<AV, String> {
        if sym == self_symbol() {
            return Ok(AV::SelfRef);
        }
        if let Some(&idx) = obj_params.get(&sym) {
            return Ok(AV::Dyn(idx));
        }
        match vars.get(&sym) {
            Some(&VarSlot::Scalar(var, k)) => Ok(AV::C(b.use_var(var), k)),
            Some(&VarSlot::Obj(slot, proof)) => {
                let idx = self.abs_slot(b, fx, slot);
                if let Some(pr) = proof {
                    self.proofs.insert(idx, pr);
                }
                Ok(AV::Dyn(idx))
            }
            None if self.cand.role == AotRole::BlockTemplate => {
                // B3a: a free variable — read the closure's real EnvFrame cell.
                self.emit_env_get(b, fx, sym)
            }
            None => Err(format!(
                "read of unknown/uninitialized local '{}' at ip {ip}",
                sym.as_str()
            )),
        }
    }

    fn local_scalar(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        sym: Symbol,
        want: AotKind,
        ip: usize,
    ) -> Result<CVal, String> {
        match self.local_av(b, fx, vars, obj_params, sym, ip)? {
            AV::C(v, k) if k == want => Ok(v),
            _ => Err(format!(
                "local '{}' is not a {want:?} at ip {ip}",
                sym.as_str()
            )),
        }
    }

    fn store_local(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &mut HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        sym: Symbol,
        v: AV,
    ) -> Result<(), String> {
        if obj_params.contains_key(&sym) || sym == self_symbol() {
            return Err(format!("store to parameter/self '{}'", sym.as_str()));
        }
        self.nil_deferred.remove(&sym);
        match v {
            AV::C(cv, k) => match vars.get(&sym) {
                Some(&VarSlot::Scalar(var, vk)) => {
                    if vk != k {
                        return Err(format!("local '{}' changes kind", sym.as_str()));
                    }
                    b.def_var(var, cv);
                    Ok(())
                }
                Some(VarSlot::Obj(..)) => Err(format!("local '{}' changes kind", sym.as_str())),
                None => {
                    let var = b.declare_var(kind_type(k));
                    b.def_var(var, cv);
                    vars.insert(sym, VarSlot::Scalar(var, k));
                    Ok(())
                }
            },
            AV::Dyn(idx) if matches!(vars.get(&sym), Some(VarSlot::Scalar(..))) => {
                // Accumulator pattern: `total = total + (dynamic)` — narrow the
                // dynamic value back into the scalar local, checked.
                let Some(&VarSlot::Scalar(var, k)) = vars.get(&sym) else {
                    unreachable!()
                };
                let val = self.narrow_to_scalar(b, fx, idx, k);
                b.def_var(var, val);
                return Ok(());
            }
            AV::Dyn(_) | AV::Nil | AV::SelfRef => {
                let vproof = match &v {
                    AV::Dyn(idx) => self.proofs.get(idx).copied(),
                    _ => None,
                };
                let slot = match vars.get(&sym) {
                    Some(&VarSlot::Obj(slot, _)) => {
                        // Reassignment: the slot's proof becomes the new value's.
                        vars.insert(sym, VarSlot::Obj(slot, vproof));
                        slot
                    }
                    Some(VarSlot::Scalar(..)) => {
                        return Err(format!("local '{}' changes kind", sym.as_str()));
                    }
                    None => {
                        let slot = self.alloc_scratch()?;
                        vars.insert(sym, VarSlot::Obj(slot, vproof));
                        slot
                    }
                };
                let dst = self.abs_slot(b, fx, slot);
                let (k, bits) = self.encode(b, fx, v);
                let f = self.func_ref(b, self.helpers.slot_set);
                let call = b.ins().call(f, &[fx.vm, fx.mc, dst, k, bits]);
                let tag = b.inst_results(call)[0];
                self.tag_check(b, fx, tag);
                Ok(())
            }
        }
    }

    /// The selector + block-arg count of the cold path's re-materialized send
    /// (the real `if:`/`if:else:` an inlined conditional falls back to) — what
    /// the proven-nil MNU stub must name to match the interpreter exactly.
    fn cold_send(insts: &[Instruction], target: usize) -> Result<(Symbol, i64), String> {
        for inst in insts.iter().skip(target).take(8) {
            if let Some((sel, n, _)) = inst.send_parts() {
                return Ok((*sel, n as i64));
            }
        }
        // The proven-nil MNU stub must name the interpreter's EXACT selector
        // and arity; an unclassifiable cold path used to silently default to
        // ("if:", 1) — a wrong error message waiting for the first cold
        // shape this scan doesn't recognize. Refuse instead (the member runs
        // interpreted, which raises the real MNU).
        Err(format!(
            "cold-path send at ip {target} not identifiable for the nil-MNU stub"
        ))
    }

    /// Fill the lane buffers with encoded AVs.
    fn fill_lanes(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vals: &[AV],
    ) -> Result<(), String> {
        if vals.len() > MAX_OUTCALL_ARGS {
            return Err("too many arguments for the compiled ABI".into());
        }
        for (i, &v) in vals.iter().enumerate() {
            let (k, bits) = self.encode(b, fx, v);
            let off = (i * 8) as i32;
            b.ins().stack_store(k, fx.kinds_buf, off);
            b.ins().stack_store(bits, fx.bits_buf, off);
        }
        Ok(())
    }

    /// A dynamic send: the general exit from the compiled world.
    fn emit_outcall(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        recv: AV,
        selector: &str,
        args: &[AV],
        ip: usize,
    ) -> Result<AV, String> {
        let (rk, rb) = self.encode(b, fx, recv);
        self.fill_lanes(b, fx, args)?;
        let sym: &'static Symbol = Box::leak(Box::new(Symbol::intern(selector)));
        let sel = b.ins().iconst(types::I64, sym as *const Symbol as i64);
        let out = self.alloc_scratch()?;
        let out_idx = self.abs_slot(b, fx, out);
        let ka = b.ins().stack_addr(types::I64, fx.kinds_buf, 0);
        let ba = b.ins().stack_addr(types::I64, fx.bits_buf, 0);
        let argc = b.ins().iconst(types::I64, args.len() as i64);
        let (tid_v, ip_v, len_v) = self.site_consts(b, ip);
        let f = self.func_ref(b, self.helpers.outcall);
        let call = b.ins().call(
            f,
            &[
                fx.vm, fx.mc, tid_v, ip_v, len_v, rk, rb, sel, argc, ka, ba, out_idx,
            ],
        );
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(AV::Dyn(out_idx))
    }

    /// The `(template_id, ip, bytecode-len)` constants identifying this send
    /// site — the interpreted send's own inline-cache identity, shared with it.
    fn site_consts(&mut self, b: &mut FunctionBuilder, ip: usize) -> (CVal, CVal, CVal) {
        let tid = self.cand.block.template_id.unwrap_or(u32::MAX);
        (
            b.ins().iconst(types::I64, i64::from(tid)),
            b.ins().iconst(types::I64, ip as i64),
            b.ins()
                .iconst(types::I64, self.cand.block.bytecode.0.len() as i64),
        )
    }

    /// A send site: direct native call when the callee is a scalar-pure
    /// sibling and the receiver is `self`; otherwise an outcall.
    fn emit_send(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        recv: AV,
        sel: Symbol,
        args: &[AV],
        ip: usize,
    ) -> Result<AV, String> {
        // Sealed scalar operators devirtualize when both operands PROVE
        // scalar (S2): Integer/Double are startup-sealed, so `Int +: Int` is
        // frozen semantics — the same guarantee the compiler's typed devirt
        // uses. Anything unproven falls through to the outcall.
        if args.len() == 1
            && let Some(kind) = IntBinKind::from_selector(sel.as_str())
        {
            match (recv, args[0]) {
                (AV::C(a, AotKind::Int), AV::C(c, AotKind::Int)) => {
                    return self.emit_int_bin(b, fx, kind, a, c);
                }
                (AV::C(a, AotKind::Double), AV::C(c, AotKind::Double)) => {
                    return Ok(self.emit_double_bin(b, kind, a, c));
                }
                _ => {}
            }
        }
        let key = (self.cand.group_id, sel.as_str().to_string());
        // An OPEN owner (B2) never emits direct calls — EXCEPT to itself
        // (S2): the entry records the redefinition epoch and `invoke` Bails
        // the whole method to the interpreter once ANY method table mutates,
        // so a stale direct recursion is never entered. Non-self sends keep
        // the outcall (dispatch-equivalent) seam.
        let own_tid = self.cand.block.template_id;
        if matches!(recv, AV::SelfRef)
            && let Some((psig, pret, callee_tid)) = self.siblings.get(&key)
            && (!self.cand.open_owner || Some(*callee_tid) == own_tid)
            && self.pure.contains(callee_tid)
            && psig.len() == args.len()
        {
            // Direct call. Scalar-pure callee: args must be exact scalars.
            let mut ok = true;
            let mut call_args = vec![fx.vm, fx.mc, fx.fuel, fx.depth, fx.slot_base];
            for (a, pk) in args.iter().zip(psig.iter()) {
                match (a, pk) {
                    (AV::C(v, k), AotParam::Scalar(want)) if k == want => call_args.push(*v),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                if self.cand.open_owner {
                    self.used_direct_self = true;
                }
                // A self-call's ret may have been DEMOTED this retry; the
                // sibling map still holds the pre-demotion signature.
                let effective = if Some(*callee_tid) == own_tid {
                    self.eff_ret
                } else {
                    *pret
                };
                let callee_fid = self.inner_ids[callee_tid];
                let callee = self.func_ref(b, callee_fid);
                let call = b.ins().call(callee, &call_args);
                let res = b.inst_results(call).to_vec();
                let (tag, val) = (res[0], res[1]);
                self.tag_check(b, fx, tag);
                let AotRet::Scalar(rk) = effective else {
                    return Err(format!("pure sibling with non-scalar ret at ip {ip}"));
                };
                return Ok(AV::C(val, rk));
            }
        }
        self.emit_outcall(b, fx, recv, sel.as_str(), args, ip)
    }

    /// Checked narrow of a slot-resident value to a scalar kind: peek the
    /// kind, extract on match, raise a clear TypeError otherwise (the one
    /// deliberate divergence — a wrong dynamic type surfaces at the annotation
    /// or accumulator that expected the scalar, instead of corrupting later).
    fn narrow_to_scalar(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        idx: CVal,
        want: AotKind,
    ) -> CVal {
        let f = self.func_ref(b, self.helpers.slot_peek);
        let oa = b.ins().stack_addr(types::I64, fx.peek_out, 0);
        let call = b.ins().call(f, &[fx.vm, fx.mc, idx, oa]);
        let kind = b.inst_results(call)[0];
        let want_code = match want {
            AotKind::Int => KIND_INT,
            AotKind::Double => KIND_DOUBLE,
            AotKind::Bool => KIND_BOOL,
        };
        let is_ok = b.ins().icmp_imm(IntCC::Equal, kind, want_code);
        let ok_bl = b.create_block();
        let err_bl = b.create_block();
        b.ins().brif(is_ok, ok_bl, &[], err_bl, &[]);
        b.switch_to_block(err_bl);
        let wc = b.ins().iconst(types::I64, want_code);
        let nf = self.func_ref(b, self.helpers.narrow_error);
        let ecall = b.ins().call(nf, &[fx.vm, fx.mc, wc, kind]);
        let etag = b.inst_results(ecall)[0];
        let zero = self.zero_of(b, fx.ret);
        b.ins().jump(fx.exit, &[etag.into(), zero.into()]);
        b.switch_to_block(ok_bl);
        let bits = b.ins().stack_load(types::I64, fx.peek_out, 0);
        match want {
            AotKind::Int => bits,
            AotKind::Double => b.ins().bitcast(types::F64, MemFlagsData::new(), bits),
            AotKind::Bool => b.ins().icmp_imm(IntCC::NotEqual, bits, 0),
        }
    }

    /// Return: narrow to the declared shape. A `Dyn` flowing into a scalar
    /// return is runtime-checked (the one deliberate divergence: a lying
    /// annotation raises a clear TypeError instead of corrupting callers).
    fn emit_return(&mut self, b: &mut FunctionBuilder, fx: &FnCtx, v: AV) -> Result<(), String> {
        let tag0 = b.ins().iconst(types::I8, 0);
        match (fx.ret, v) {
            (AotRet::Scalar(want), AV::C(cv, k)) if k == want => {
                b.ins().jump(fx.exit, &[tag0.into(), cv.into()]);
            }
            // A SPECULATED scalar ret must be statically provable on every
            // return path — no runtime narrowing whose failure would raise an
            // error the interpreter wouldn't. Demote to Obj and retry.
            (AotRet::Scalar(_), _) if self.cand.spec_ret => {
                self.pending_abort = Some(TranslateAbort::RetDemote);
                return Err("speculated scalar return not provable on this path".to_string());
            }
            (AotRet::Scalar(want), AV::Dyn(idx)) => {
                let val = self.narrow_to_scalar(b, fx, idx, want);
                b.ins().jump(fx.exit, &[tag0.into(), val.into()]);
            }
            (AotRet::Scalar(_), _) => {
                return Err("return value does not match the declared scalar type".into());
            }
            (AotRet::Obj, v) => {
                let idx = match v {
                    AV::Dyn(idx) => idx,
                    AV::SelfRef => self.abs_slot(b, fx, 0),
                    other => {
                        // Box a scalar/nil into a scratch slot (a lying `^List`
                        // etc. returns the honest value, as the interpreter's
                        // trusted-return contract does).
                        let out = self.alloc_scratch()?;
                        let dst = self.abs_slot(b, fx, out);
                        let (k, bits) = self.encode(b, fx, other);
                        let f = self.func_ref(b, self.helpers.slot_set);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, dst, k, bits]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, fx, tag);
                        dst
                    }
                };
                b.ins().jump(fx.exit, &[tag0.into(), idx.into()]);
            }
        }
        Ok(())
    }

    fn pop_kind(stack: &mut Vec<AV>, want: AotKind) -> Result<CVal, String> {
        match stack.pop() {
            Some(AV::C(v, k)) if k == want => Ok(v),
            Some(AV::C(_, k)) => Err(format!("operand kind {k:?}, wanted {want:?}")),
            Some(_) => Err("non-scalar operand where a scalar was proven".to_string()),
            None => Err("stack underflow".to_string()),
        }
    }

    /// Box `Nil`/`SelfRef` stack entries into slots so they can cross a block
    /// boundary as jump arguments (a statement-position inlined `if:` joins an
    /// arm value with the nil of the not-taken path). Scalars and slot values
    /// pass through untouched.
    fn assert_no_pending_writebacks(&self, stack: &[AV], where_: &str) -> Result<(), String> {
        for v in stack {
            if let AV::Dyn(idx) = v
                && self.pending_writebacks.contains_key(idx)
            {
                return Err(format!(
                    "write-captured closure crosses a block boundary ({where_})"
                ));
            }
        }
        Ok(())
    }

    fn norm_stack(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        stack: &[AV],
    ) -> Result<Vec<AV>, String> {
        self.assert_no_pending_writebacks(stack, "norm_stack")?;
        let mut out = Vec::with_capacity(stack.len());
        for v in stack {
            match v {
                AV::C(..) | AV::Dyn(_) => out.push(*v),
                AV::SelfRef => out.push(AV::Dyn(self.abs_slot(b, fx, 0))),
                AV::Nil => {
                    let slot = self.alloc_scratch()?;
                    let dst = self.abs_slot(b, fx, slot);
                    let (k, bits) = self.encode(b, fx, AV::Nil);
                    let f = self.func_ref(b, self.helpers.slot_set);
                    let call = b.ins().call(f, &[fx.vm, fx.mc, dst, k, bits]);
                    let tag = b.inst_results(call)[0];
                    self.tag_check(b, fx, tag);
                    out.push(AV::Dyn(dst));
                }
            }
        }
        Ok(out)
    }

    fn stack_args(stack: &[AV]) -> Result<Vec<BlockArg>, String> {
        stack
            .iter()
            .map(|v| match v {
                AV::C(cv, _) => Ok((*cv).into()),
                AV::Dyn(idx) => Ok((*idx).into()),
                _ => Err("self/nil live at block boundary".to_string()),
            })
            .collect()
    }

    fn stack_bkinds(stack: &[AV]) -> Result<Vec<BKind>, String> {
        stack
            .iter()
            .map(|v| match v {
                AV::C(_, k) => Ok(BKind::S(*k)),
                AV::Dyn(_) => Ok(BKind::Dyn),
                _ => Err("self/nil live at block boundary".to_string()),
            })
            .collect()
    }

    fn block_for(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        blocks: &mut HashMap<usize, (CBlock, Vec<BKind>)>,
        work: &mut Vec<usize>,
        ip: usize,
        stack: &mut Vec<AV>,
    ) -> Result<(CBlock, Vec<BKind>), String> {
        // A merge FORCED to all-Dyn by an earlier retry (mixed scalar/Dyn
        // predecessors, S3): box scalars before shape computation, so every
        // predecessor unifies. The interpreted value world is uniform —
        // boxing is exact, only the abstraction loses precision.
        if self.dyn_merges.contains(&ip) {
            for i in 0..stack.len() {
                if let AV::C(..) = stack[i] {
                    stack[i] = self.box_av(b, fx, stack[i])?;
                }
            }
        }
        let kinds = Self::stack_bkinds(stack)?;
        if let Some((bl, expect)) = blocks.get(&ip) {
            let (bl, expect) = (*bl, expect.clone());
            if expect != kinds {
                if expect.len() != kinds.len() {
                    return Err(format!(
                        "stack shape mismatch at merge ip {ip}: {expect:?} vs {kinds:?}"
                    ));
                }
                for i in 0..kinds.len() {
                    match (&expect[i], &kinds[i]) {
                        (a, bk) if a == bk => {}
                        // Box toward an existing Dyn expectation, in place.
                        (BKind::Dyn, BKind::S(_)) => {
                            stack[i] = self.box_av(b, fx, stack[i])?;
                        }
                        // The merge was first planned scalar; a Dyn (or a
                        // different scalar) predecessor needs it re-planned
                        // all-Dyn — signal the retry.
                        _ => {
                            self.pending_abort = Some(TranslateAbort::MergeDemote(ip));
                            return Err(format!("merge at ip {ip} re-planned as Dyn"));
                        }
                    }
                }
            }
            return Ok((bl, expect));
        }
        let bl = b.create_block();
        for &k in &kinds {
            b.append_block_param(bl, bkind_type(k));
        }
        blocks.insert(ip, (bl, kinds.clone()));
        work.push(ip);
        Ok((bl, kinds))
    }

    /// Box any AV into a fresh scratch slot (merge-shape unification).
    fn box_av(&mut self, b: &mut FunctionBuilder, fx: &FnCtx, v: AV) -> Result<AV, String> {
        let slot = self.alloc_scratch()?;
        let dst = self.abs_slot(b, fx, slot);
        let (k, bits) = self.encode(b, fx, v);
        let f = self.func_ref(b, self.helpers.slot_set);
        let call = b.ins().call(f, &[fx.vm, fx.mc, dst, k, bits]);
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(AV::Dyn(dst))
    }

    fn zero_of(&self, b: &mut FunctionBuilder, r: AotRet) -> CVal {
        match r {
            AotRet::Scalar(AotKind::Int) | AotRet::Obj => b.ins().iconst(types::I64, 0),
            AotRet::Scalar(AotKind::Double) => b.ins().f64const(0.0),
            AotRet::Scalar(AotKind::Bool) => b.ins().iconst(types::I8, 0),
        }
    }

    fn bail(&self, b: &mut FunctionBuilder, fx: &FnCtx, tag: u8) {
        let t = b.ins().iconst(types::I8, tag as i64);
        let zero = self.zero_of(b, fx.ret);
        b.ins().jump(fx.exit, &[t.into(), zero.into()]);
    }

    /// Fuel decrement + (rarely) checkpoint, carrying the live abstract stack
    /// through the checkpoint's control flow as block params. Emitted in
    /// every prologue (covers recursion) and at loop back-edges (covers
    /// loops) — the two shapes that must stay preemptible and cancellable.
    /// `Dyn` values pass through as slot indices (their contents are rooted
    /// in the window, so suspending needs no extra work).
    fn emit_fuel_tick(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        stack: &[AV],
    ) -> Result<Vec<AV>, String> {
        let keep = Self::stack_args(stack)?;
        let kinds = Self::stack_bkinds(stack)?;
        let f0 = b
            .ins()
            .load(types::I64, MemFlagsData::trusted(), fx.fuel, 0);
        let f1 = b.ins().iadd_imm(f0, -1);
        b.ins().store(MemFlagsData::trusted(), f1, fx.fuel, 0);
        let spent = b.ins().icmp_imm(IntCC::SignedLessThanOrEqual, f1, 0);
        let cp_bl = b.create_block();
        let cont = b.create_block();
        for &k in &kinds {
            b.append_block_param(cont, bkind_type(k));
        }
        b.ins().brif(spent, cp_bl, &[], cont, &keep);
        b.switch_to_block(cp_bl);
        let cp = self.func_ref(b, self.helpers.checkpoint);
        let call = b.ins().call(cp, &[fx.vm, fx.fuel]);
        let tag = b.inst_results(call)[0];
        let bad = b.ins().icmp_imm(IntCC::NotEqual, tag, 0);
        let cp_bad = b.create_block();
        b.ins().brif(bad, cp_bad, &[], cont, &keep);
        b.switch_to_block(cp_bad);
        let zero = self.zero_of(b, fx.ret);
        b.ins().jump(fx.exit, &[tag.into(), zero.into()]);
        b.switch_to_block(cont);
        let params = b.block_params(cont).to_vec();
        Ok(params
            .iter()
            .zip(kinds.iter())
            .map(|(&v, &k)| match k {
                BKind::S(sk) => AV::C(v, sk),
                BKind::Dyn => AV::Dyn(v),
            })
            .collect())
    }

    fn emit_fuel_tick_empty(&mut self, b: &mut FunctionBuilder, fx: &FnCtx) {
        self.emit_fuel_tick(b, fx, &[])
            .expect("empty-stack tick cannot fail");
    }

    /// Integer ops with `devirt_ops::int_bin` semantics.
    fn emit_int_bin(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        kind: IntBinKind,
        a: CVal,
        rb: CVal,
    ) -> Result<AV, String> {
        use IntBinKind::*;
        let out = match kind {
            Add => AV::C(b.ins().iadd(a, rb), AotKind::Int),
            Sub => AV::C(b.ins().isub(a, rb), AotKind::Int),
            Mul => AV::C(b.ins().imul(a, rb), AotKind::Int),
            Div | Mod => {
                let is_zero = b.ins().icmp_imm(IntCC::Equal, rb, 0);
                let zero_bl = b.create_block();
                let cont = b.create_block();
                b.ins().brif(is_zero, zero_bl, &[], cont, &[]);
                b.switch_to_block(zero_bl);
                self.bail(b, fx, TAG_DIV_ZERO);
                b.switch_to_block(cont);
                let is_m1 = b.ins().icmp_imm(IntCC::Equal, rb, -1);
                let m1_bl = b.create_block();
                let norm_bl = b.create_block();
                let join = b.create_block();
                b.append_block_param(join, types::I64);
                b.ins().brif(is_m1, m1_bl, &[], norm_bl, &[]);
                b.switch_to_block(m1_bl);
                let m1v = if matches!(kind, Div) {
                    b.ins().ineg(a)
                } else {
                    b.ins().iconst(types::I64, 0)
                };
                b.ins().jump(join, &[m1v.into()]);
                b.switch_to_block(norm_bl);
                let nv = if matches!(kind, Div) {
                    b.ins().sdiv(a, rb)
                } else {
                    b.ins().srem(a, rb)
                };
                b.ins().jump(join, &[nv.into()]);
                b.switch_to_block(join);
                AV::C(b.block_params(join)[0], AotKind::Int)
            }
            Lt => AV::C(b.ins().icmp(IntCC::SignedLessThan, a, rb), AotKind::Bool),
            Le => AV::C(
                b.ins().icmp(IntCC::SignedLessThanOrEqual, a, rb),
                AotKind::Bool,
            ),
            Gt => AV::C(b.ins().icmp(IntCC::SignedGreaterThan, a, rb), AotKind::Bool),
            Ge => AV::C(
                b.ins().icmp(IntCC::SignedGreaterThanOrEqual, a, rb),
                AotKind::Bool,
            ),
            Eq => AV::C(b.ins().icmp(IntCC::Equal, a, rb), AotKind::Bool),
            Ne => AV::C(b.ins().icmp(IntCC::NotEqual, a, rb), AotKind::Bool),
        };
        Ok(out)
    }

    /// f64 ops with `devirt_ops::double_bin` semantics.
    fn emit_double_bin(
        &mut self,
        b: &mut FunctionBuilder,
        kind: IntBinKind,
        a: CVal,
        rb: CVal,
    ) -> AV {
        use IntBinKind::*;
        match kind {
            Add => AV::C(b.ins().fadd(a, rb), AotKind::Double),
            Sub => AV::C(b.ins().fsub(a, rb), AotKind::Double),
            Mul => AV::C(b.ins().fmul(a, rb), AotKind::Double),
            Div => AV::C(b.ins().fdiv(a, rb), AotKind::Double),
            Mod => {
                let f = self.func_ref(b, self.helpers.fmod);
                let call = b.ins().call(f, &[a, rb]);
                AV::C(b.inst_results(call)[0], AotKind::Double)
            }
            Lt => AV::C(b.ins().fcmp(FloatCC::LessThan, a, rb), AotKind::Bool),
            Le => AV::C(b.ins().fcmp(FloatCC::LessThanOrEqual, a, rb), AotKind::Bool),
            Gt => AV::C(b.ins().fcmp(FloatCC::GreaterThan, a, rb), AotKind::Bool),
            Ge => AV::C(
                b.ins().fcmp(FloatCC::GreaterThanOrEqual, a, rb),
                AotKind::Bool,
            ),
            Eq => AV::C(b.ins().fcmp(FloatCC::Equal, a, rb), AotKind::Bool),
            Ne => AV::C(b.ins().fcmp(FloatCC::NotEqual, a, rb), AotKind::Bool),
        }
    }
}

fn int_inst_kind(i: &Instruction) -> IntBinKind {
    match i {
        Instruction::IntAdd => IntBinKind::Add,
        Instruction::IntSub => IntBinKind::Sub,
        Instruction::IntMul => IntBinKind::Mul,
        Instruction::IntDiv => IntBinKind::Div,
        Instruction::IntMod => IntBinKind::Mod,
        Instruction::IntLt => IntBinKind::Lt,
        Instruction::IntLe => IntBinKind::Le,
        Instruction::IntGt => IntBinKind::Gt,
        Instruction::IntGe => IntBinKind::Ge,
        Instruction::IntEq => IntBinKind::Eq,
        Instruction::IntNe => IntBinKind::Ne,
        _ => unreachable!(),
    }
}

fn double_inst_kind(i: &Instruction) -> IntBinKind {
    match i {
        Instruction::DoubleAdd => IntBinKind::Add,
        Instruction::DoubleSub => IntBinKind::Sub,
        Instruction::DoubleMul => IntBinKind::Mul,
        Instruction::DoubleDiv => IntBinKind::Div,
        Instruction::DoubleMod => IntBinKind::Mod,
        Instruction::DoubleLt => IntBinKind::Lt,
        Instruction::DoubleLe => IntBinKind::Le,
        Instruction::DoubleGt => IntBinKind::Gt,
        Instruction::DoubleGe => IntBinKind::Ge,
        Instruction::DoubleEq => IntBinKind::Eq,
        Instruction::DoubleNe => IntBinKind::Ne,
        _ => unreachable!(),
    }
}
