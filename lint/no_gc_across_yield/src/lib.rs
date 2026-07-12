#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_span;

use rustc_lint::{LateContext, LateLintPass};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

dylint_linting::declare_late_lint! {
    /// ### What it does
    /// Checks for GC-managed values held in `let`-bound locals across a fiber yield point.
    ///
    /// ### Why is this bad?
    /// The garbage collector cannot scan the native stacks of suspended fibers. If a GC-managed
    /// value is held on a suspended fiber stack, the GC may collect it, leading to dangling pointers.
    ///
    /// Function/closure **parameters are deliberately not flagged**: a parameter is the
    /// caller's value, so an unrooted hold is reported at the caller where the value was
    /// created — and in this codebase the callers root them by construction (a native
    /// method's `(receiver, args)` are pinned in `active_native_args` for the whole call,
    /// and anything passed *into* `execute_block`/`call_method` is reachable through the
    /// callee frame for exactly the duration of its yields).
    ///
    /// ### Example
    /// ```rust
    /// let val: Value<'gc> = ...;
    /// yielder.suspend(...);
    /// use(val);
    /// ```
    pub NO_GC_ACROSS_YIELD,
    Deny,
    "GC-managed values held in local variables across a fiber yield point"
}

struct FnLintInfo<'tcx> {
    // Local declarations: (HirId, Span, Ty, Option<init_span>)
    locals: Vec<(
        rustc_hir::HirId,
        rustc_span::Span,
        rustc_middle::ty::Ty<'tcx>,
        Option<rustc_span::Span>,
    )>,
    // Usages: (HirId, Span)
    usages: Vec<(rustc_hir::HirId, rustc_span::Span)>,
    // Whole-local writes (assignment LHS): a write between a yield and a use
    // means the used value was created AFTER the yield — not a hold.
    writes: Vec<(rustc_hir::HirId, rustc_span::Span)>,
    // Method/Function calls inside this function: (Callee DefId, Span of call)
    calls: Vec<(rustc_hir::def_id::DefId, rustc_span::Span)>,
    // Direct yielder suspend spans in this function
    suspend_spans: Vec<rustc_span::Span>,
}

thread_local! {
    static FN_INFOS: RefCell<HashMap<rustc_hir::def_id::LocalDefId, FnLintInfo<'static>>> = RefCell::new(HashMap::new());
}

struct GcYieldVisitor<'tcx, 'sym> {
    cx: &'sym LateContext<'tcx>,
    locals: Vec<(
        rustc_hir::HirId,
        rustc_span::Span,
        rustc_middle::ty::Ty<'tcx>,
        Option<rustc_span::Span>,
    )>,
    registered_locals: HashSet<rustc_hir::HirId>,
    suspend_spans: Vec<rustc_span::Span>,
    usages: Vec<(rustc_hir::HirId, rustc_span::Span)>,
    writes: Vec<(rustc_hir::HirId, rustc_span::Span)>,
    calls: Vec<(rustc_hir::def_id::DefId, rustc_span::Span)>,
    /// Expr ids that are the LHS of an assignment: a WRITE kills the old
    /// value rather than using it, so `ret_val = self.pop()?` right after a
    /// yield must not count as holding `ret_val` across it.
    assign_lhs: HashSet<rustc_hir::HirId>,
    /// The function's parameter binding ids: locals initialized from a bare
    /// param path (`let c = receiver;`, `if let Value::Class(c) = receiver`)
    /// are the caller's rooted value under another name and inherit the
    /// param exemption.
    param_ids: HashSet<rustc_hir::HirId>,
}

impl<'tcx, 'sym> GcYieldVisitor<'tcx, 'sym> {
    /// Is `expr` a bare path to one of this function's parameters?
    fn is_param_path(&self, expr: &rustc_hir::Expr<'_>) -> bool {
        if let rustc_hir::ExprKind::Path(rustc_hir::QPath::Resolved(_, path)) = &expr.kind
            && let rustc_hir::def::Res::Local(hir_id) = path.res
        {
            return self.param_ids.contains(&hir_id);
        }
        false
    }

    /// Register `pat`'s bindings as exempt (never GC-flagged) — used for
    /// patterns destructuring a parameter.
    fn exempt_pat_bindings(&mut self, pat: &'tcx rustc_hir::Pat<'tcx>) {
        pat.walk(|p| {
            if let rustc_hir::PatKind::Binding(_, hir_id, _, _) = p.kind {
                self.registered_locals.insert(hir_id);
            }
            true
        });
    }

    fn register_pat_bindings(
        &mut self,
        pat: &'tcx rustc_hir::Pat<'tcx>,
        init_span: Option<rustc_span::Span>,
    ) {
        struct BindingFinder<'a, 'tcx, 'sym> {
            visitor: &'a mut GcYieldVisitor<'tcx, 'sym>,
            init_span: Option<rustc_span::Span>,
        }
        impl<'a, 'tcx, 'sym> rustc_hir::intravisit::Visitor<'tcx> for BindingFinder<'a, 'tcx, 'sym> {
            fn visit_pat(&mut self, pat: &'tcx rustc_hir::Pat<'tcx>) {
                if let rustc_hir::PatKind::Binding(_, hir_id, _ident, _) = pat.kind {
                    if !self.visitor.registered_locals.contains(&hir_id) {
                        let ty = self.visitor.cx.typeck_results().node_type(hir_id);
                        if carries_gc(self.visitor.cx.tcx, ty) {
                            self.visitor
                                .locals
                                .push((hir_id, pat.span, ty, self.init_span));
                            self.visitor.registered_locals.insert(hir_id);
                        }
                    }
                }
                rustc_hir::intravisit::walk_pat(self, pat);
            }
        }
        let mut finder = BindingFinder {
            visitor: self,
            init_span,
        };
        rustc_hir::intravisit::Visitor::visit_pat(&mut finder, pat);
    }
}

impl<'tcx, 'sym> rustc_hir::intravisit::Visitor<'tcx> for GcYieldVisitor<'tcx, 'sym> {
    fn visit_local(&mut self, local: &'tcx rustc_hir::LetStmt<'tcx>) {
        if let Some(init) = local.init
            && self.is_param_path(init)
        {
            self.exempt_pat_bindings(local.pat);
        }
        let init_span = local.init.map(|i| i.span);
        self.register_pat_bindings(local.pat, init_span);
        rustc_hir::intravisit::walk_local(self, local);
    }

    fn visit_pat(&mut self, pat: &'tcx rustc_hir::Pat<'tcx>) {
        self.register_pat_bindings(pat, None);
        rustc_hir::intravisit::walk_pat(self, pat);
    }

    fn visit_expr(&mut self, expr: &'tcx rustc_hir::Expr<'tcx>) {
        match expr.kind {
            // `if let PAT = SCRUT` / `while let` / `let-else`: the bindings
            // are BORN FROM the scrutinee — a yield inside it must not flag
            // them (same rule as a LetStmt initializer), and destructuring a
            // parameter inherits the param exemption.
            rustc_hir::ExprKind::Let(let_expr) => {
                if self.is_param_path(let_expr.init) {
                    self.exempt_pat_bindings(let_expr.pat);
                }
                self.register_pat_bindings(let_expr.pat, Some(let_expr.init.span));
            }
            // `match SCRUT { PAT => … }`: same — arm bindings are born from
            // the scrutinee.
            rustc_hir::ExprKind::Match(scrut, arms, _) => {
                let from_param = self.is_param_path(scrut);
                for arm in arms {
                    if from_param {
                        self.exempt_pat_bindings(arm.pat);
                    }
                    self.register_pat_bindings(arm.pat, Some(scrut.span));
                }
            }
            rustc_hir::ExprKind::Assign(lhs, _, _) => {
                if let rustc_hir::ExprKind::Path(rustc_hir::QPath::Resolved(_, path)) = &lhs.kind {
                    if let rustc_hir::def::Res::Local(hir_id) = path.res {
                        self.assign_lhs.insert(lhs.hir_id);
                        self.writes.push((hir_id, expr.span));
                    }
                }
            }
            rustc_hir::ExprKind::MethodCall(path, _, _, _) => {
                if path.ident.name.as_str() == "suspend" {
                    self.suspend_spans.push(expr.span);
                }
                if let Some(def_id) = self.cx.typeck_results().type_dependent_def_id(expr.hir_id) {
                    self.calls.push((def_id, expr.span));
                }
            }
            rustc_hir::ExprKind::Call(callee, _) => {
                if let rustc_hir::ExprKind::Path(qpath) = &callee.kind {
                    let res = self.cx.typeck_results().qpath_res(qpath, callee.hir_id);
                    if let Some(def_id) = res.opt_def_id() {
                        self.calls.push((def_id, expr.span));
                    }
                }
            }
            rustc_hir::ExprKind::Path(rustc_hir::QPath::Resolved(_, path)) => {
                if let rustc_hir::def::Res::Local(hir_id) = path.res
                    && !self.assign_lhs.contains(&expr.hir_id)
                {
                    self.usages.push((hir_id, expr.span));
                }
            }
            _ => {}
        }
        rustc_hir::intravisit::walk_expr(self, expr);
    }
}

/// Does `ty` transitively OWN `Gc`/`GcWeak` data? A real type walk — through
/// ADT fields (all variants), ADT generic arguments (so containers like
/// `Vec<Value>` match without descending into their raw-pointer internals),
/// tuples, arrays and slices — terminating at the gc_arena smart pointers
/// themselves (matched by ADT name at the def level).
///
/// This replaced a name-substring test on the PRINTED type, which missed
/// wrapper structs: `Frame<'gc>` prints without its field types, so a popped
/// frame held across a yield sailed through unregistered — the S0 segfault.
/// Plain-data types that merely mention the words (`ValueInfo` of Strings, a
/// `GcMetrics`) still don't match: they own no Gc anywhere.
///
/// References and raw pointers are deliberately NOT traversed: a `&Frame`
/// borrowed from rooted storage stays valid across a yield (rustc's borrowck
/// covers the Rust side), and the OWNED local behind any dangerous reference
/// is flagged on its own.
fn carries_gc<'tcx>(tcx: rustc_middle::ty::TyCtxt<'tcx>, ty: rustc_middle::ty::Ty<'tcx>) -> bool {
    fn go<'tcx>(
        tcx: rustc_middle::ty::TyCtxt<'tcx>,
        ty: rustc_middle::ty::Ty<'tcx>,
        seen: &mut HashSet<rustc_middle::ty::Ty<'tcx>>,
        depth: u32,
    ) -> bool {
        use rustc_middle::ty::TyKind;
        if depth > 32 || !seen.insert(ty) {
            return false;
        }
        match ty.kind() {
            TyKind::Adt(def, args) => {
                let name = tcx.item_name(def.did());
                if name.as_str() == "Gc" || name.as_str() == "GcWeak" {
                    return true;
                }
                def.all_fields()
                    .any(|f| go(tcx, f.ty(tcx, args), seen, depth + 1))
                    || args.types().any(|t| go(tcx, t, seen, depth + 1))
            }
            TyKind::Array(t, _) | TyKind::Slice(t) => go(tcx, *t, seen, depth + 1),
            TyKind::Tuple(ts) => ts.iter().any(|t| go(tcx, t, seen, depth + 1)),
            _ => false,
        }
    }
    go(tcx, ty, &mut HashSet::new(), 0)
}

impl<'tcx> LateLintPass<'tcx> for NoGcAcrossYield {
    fn check_fn(
        &mut self,
        cx: &LateContext<'tcx>,
        _: rustc_hir::intravisit::FnKind<'tcx>,
        _: &'tcx rustc_hir::FnDecl<'tcx>,
        body: &'tcx rustc_hir::Body<'tcx>,
        _: rustc_span::Span,
        def_id: rustc_hir::def_id::LocalDefId,
    ) {
        let mut visitor = GcYieldVisitor {
            cx,
            locals: Vec::new(),
            registered_locals: HashSet::new(),
            suspend_spans: Vec::new(),
            usages: Vec::new(),
            writes: Vec::new(),
            calls: Vec::new(),
            assign_lhs: HashSet::new(),
            param_ids: HashSet::new(),
        };
        for param in body.params {
            param.pat.walk(|p| {
                if let rustc_hir::PatKind::Binding(_, hir_id, _, _) = p.kind {
                    visitor.param_ids.insert(hir_id);
                }
                true
            });
        }

        // Parameters are NOT registered (see the lint docs): their rooting is the
        // caller's contract — the hold is flagged where the value was created, and the
        // conventional callers here root what they pass (`active_native_args`, frames).

        rustc_hir::intravisit::Visitor::visit_expr(&mut visitor, body.value);

        let info = FnLintInfo {
            locals: visitor.locals,
            usages: visitor.usages,
            writes: visitor.writes,
            calls: visitor.calls,
            suspend_spans: visitor.suspend_spans,
        };
        let info_static: FnLintInfo<'static> = unsafe { std::mem::transmute(info) };
        FN_INFOS.with(|infos| {
            infos.borrow_mut().insert(def_id, info_static);
        });
    }

    fn check_crate_post(&mut self, cx: &LateContext<'tcx>) {
        FN_INFOS.with(|infos| {
            let mut infos = infos.borrow_mut();

            // 1. Build the call graph: Caller -> Set of local Callees
            let mut call_graph: HashMap<rustc_hir::def_id::LocalDefId, HashSet<rustc_hir::def_id::LocalDefId>> = HashMap::new();
            let mut yielding_fns: HashSet<rustc_hir::def_id::LocalDefId> = HashSet::new();

            for (&caller, info) in infos.iter() {
                if !info.suspend_spans.is_empty() {
                    yielding_fns.insert(caller);
                }
                for &(callee, _) in &info.calls {
                    if let Some(local_callee) = callee.as_local() {
                        call_graph.entry(caller).or_default().insert(local_callee);
                    }
                }
            }

            // 2. Compute transitive closure of yielding functions
            let mut changed = true;
            while changed {
                changed = false;
                for (&caller, callees) in &call_graph {
                    if !yielding_fns.contains(&caller) {
                        for callee in callees {
                            if yielding_fns.contains(callee) {
                                yielding_fns.insert(caller);
                                changed = true;
                                break;
                            }
                        }
                    }
                }
            }

            // 3. Analyze each function for live variables across yielding call
            // sites — in source order (HashMap iteration would make the
            // diagnostic order, and therefore the UI test, nondeterministic).
            let mut ordered: Vec<_> = infos.iter().collect();
            ordered.sort_by_key(|(_, info)| {
                info.locals.first().map(|&(_, span, _, _)| span.lo())
            });
            for (&_caller, info) in ordered {
                let info_tcx: &FnLintInfo<'tcx> = unsafe { std::mem::transmute(info) };

                // Find all yield points inside this function
                let mut yield_spans = info_tcx.suspend_spans.clone();
                for &(callee, span) in &info_tcx.calls {
                    let yields = if let Some(local_callee) = callee.as_local() {
                        yielding_fns.contains(&local_callee)
                    } else {
                        false
                    };
                    if yields {
                        yield_spans.push(span);
                    }
                }

                if yield_spans.is_empty() {
                    continue;
                }

                for &(hir_id, decl_span, ty, init_span) in &info_tcx.locals {
                    for &yield_span in &yield_spans {
                        // Skip if the yield point is inside the variable's own initializer
                        if let Some(init) = init_span {
                            if yield_span.lo() >= init.lo() && yield_span.hi() <= init.hi() {
                                continue;
                            }
                        }

                        // Check if variable is declared before the yield
                        if decl_span.lo() < yield_span.lo() {
                            // A use after this yield is a hold ONLY if no
                            // whole-local write lands in between (a write
                            // means the used value was created post-yield —
                            // `ret_val = self.pop()?` after finalize). Flow-
                            // insensitive: a write on a sibling branch can
                            // over-suppress; the lint is a net, not a prover.
                            // A local passed as an ARGUMENT to the yielding
                            // call is rooted through it (the crate convention
                            // the param exemption already assumes: VM entry
                            // points bind what they're given into frames/envs
                            // for the duration of their yields) — so uses
                            // after THAT call are safe; a later yield still
                            // flags via its own pair.
                            let arg_of_yield = info_tcx.usages.iter().any(|(id, span)| {
                                *id == hir_id
                                    && span.lo() >= yield_span.lo()
                                    && span.hi() <= yield_span.hi()
                            });
                            if arg_of_yield {
                                continue;
                            }
                            if let Some((_, usage_span)) = info_tcx
                                .usages
                                .iter()
                                .find(|(id, span)| {
                                    *id == hir_id
                                        && span.lo() > yield_span.hi()
                                        && !info_tcx.writes.iter().any(|(wid, wspan)| {
                                            *wid == hir_id
                                                && wspan.lo() > yield_span.hi()
                                                && wspan.hi() < span.lo()
                                        })
                                })
                            {
                                clippy_utils::diagnostics::span_lint_hir_and_then(
                                    cx,
                                    NO_GC_ACROSS_YIELD,
                                    hir_id,
                                    *usage_span,
                                    "GC value held and used across a fiber yield point",
                                    |diag| {
                                        diag.span_help(
                                            decl_span,
                                            format!(
                                                "The variable of type `{}` is held across a yield point. Consider flushing it or moving its use.",
                                                ty
                                            ),
                                        );
                                    },
                                );
                                break;
                            }
                        }
                    }
                }
            }

            // Reset the thread-local state to avoid memory leaks
            infos.clear();
        });
    }
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}
