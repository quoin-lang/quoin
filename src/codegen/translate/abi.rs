//! The Cranelift ABI layer: scalar-kind -> IR types, helper-import signatures
//! (`ClAbi`/`HelperSig` and the `aot_helpers!` table wiring `crate::codegen::helpers`), the
//! inner/trampoline signatures, and the extern trampoline builder.

use super::*;

pub(super) fn kind_type(k: AotKind) -> Type {
    match k {
        AotKind::Int => types::I64,
        AotKind::Double => types::F64,
        AotKind::Bool => types::I8,
    }
}

pub(super) fn param_type(p: AotParam) -> Type {
    match p {
        AotParam::Scalar(k) => kind_type(k),
        AotParam::Obj => types::I64, // absolute slot index
    }
}

pub(super) fn ret_type(r: AotRet) -> Type {
    match r {
        AotRet::Scalar(k) => kind_type(k),
        AotRet::Obj => types::I64, // absolute slot index
    }
}

/// Rust ABI type -> Cranelift type, for deriving helper import signatures
/// from the helpers' own `extern "C"` fn types.
pub(super) trait ClAbi {
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
pub(super) trait HelperSig {
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
impl_helper_sig!(A B C D E F G H I J K L M);

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
        pub(super) struct Helpers {
            $(pub(super) $field: FuncId,)+
        }

        pub(super) fn declare_helpers(module: &mut JITModule, ptr: Type) -> Result<Helpers, Refusal> {
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
        pub(super) fn helper_symbols() -> Vec<(&'static str, *const u8)> {
            vec![$((concat!("qn_aot_", stringify!($field)), $f as *const u8),)+]
        }
    };
}

aot_helpers! {
    checkpoint: crate::codegen::aot_checkpoint as fn(*mut c_void, *mut i64) -> u8,
    fmod: aot_fmod as fn(f64, f64) -> f64,
    slot_set: helpers::slot_set as fn(*mut c_void, *const c_void, i64, i64, i64) -> u8,
    guard_recv: helpers::guard_recv as fn(*mut c_void, *const c_void, i64, i64, i64, i64) -> u8,
    require_bool: helpers::require_bool as fn(*mut c_void, *const c_void, i64) -> u8,
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
        *mut c_void, *const c_void, *const Arc<StaticBlock>, i64, i64,
    ) -> u8,
    plain_new_check: helpers::plain_new_check as fn(
        *mut c_void, *const c_void, i64, i64, i64, i64, i64,
    ) -> i64,
    new_with_fields: helpers::new_with_fields as fn(
        *mut c_void, *const c_void, *const Symbol, i64, i64, i64, *const i64, *const i64, i64,
    ) -> u8,
    closure_bind: helpers::closure_bind as fn(*mut c_void, *const c_void, i64, *const Symbol, i64, i64) -> u8,
    field_get: helpers::field_get as fn(
        *mut c_void, *const c_void, i64, i64, i64, i64, *const u8, i64, i64,
    ) -> u8,
    field_set: helpers::field_set as fn(
        *mut c_void, *const c_void, i64, i64, i64, i64, *const u8, i64, i64, i64,
    ) -> u8,
}

pub(super) fn inner_sig(
    module: &mut JITModule,
    ptr: Type,
    m: &AotCandidate,
    eff: AotRet,
) -> Signature {
    let mut sig = module.make_signature();
    for _ in 0..6 {
        sig.params.push(AbiParam::new(ptr)); // vm, mc, fuel, depth, epoch, slots
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

pub(super) fn tramp_sig(module: &mut JITModule, ptr: Type) -> Signature {
    let mut sig = module.make_signature();
    for _ in 0..6 {
        sig.params.push(AbiParam::new(ptr)); // vm, mc, fuel, depth, epoch, slots
    }
    sig.params.push(AbiParam::new(types::I64)); // slot_base
    sig.params.push(AbiParam::new(ptr)); // args
    sig.params.push(AbiParam::new(ptr)); // ret
    sig.returns.push(AbiParam::new(types::I8));
    sig
}

pub(super) fn build_trampoline(
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
    let (vm, mc, fuel, depth, epoch, slots, slot_base, args, ret) =
        (p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8]);
    let mut call_args = vec![vm, mc, fuel, depth, epoch, slots, slot_base];
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
