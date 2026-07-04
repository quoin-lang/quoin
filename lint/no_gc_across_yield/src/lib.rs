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
    Warn,
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
    calls: Vec<(rustc_hir::def_id::DefId, rustc_span::Span)>,
}

impl<'tcx, 'sym> GcYieldVisitor<'tcx, 'sym> {
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
                        if contains_gc_lifetime(ty) {
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
                if let rustc_hir::def::Res::Local(hir_id) = path.res {
                    self.usages.push((hir_id, expr.span));
                }
            }
            _ => {}
        }
        rustc_hir::intravisit::walk_expr(self, expr);
    }
}

/// A generic-position match (`Value<…>` / `Gc<…>`, incl. wrappers like `Vec<Value<…>>`),
/// not a bare substring one: plain-data types that merely *mention* the words — a
/// `ValueInfo` of Strings, a `GcMetrics` — carry nothing collectable and must not match.
fn contains_gc_lifetime(ty: rustc_middle::ty::Ty<'_>) -> bool {
    let ty_str = format!("{:?}", ty);
    ty_str.contains("Value<") || ty_str.contains("Gc<")
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
            calls: Vec::new(),
        };

        // Parameters are NOT registered (see the lint docs): their rooting is the
        // caller's contract — the hold is flagged where the value was created, and the
        // conventional callers here root what they pass (`active_native_args`, frames).

        rustc_hir::intravisit::Visitor::visit_expr(&mut visitor, body.value);

        let info = FnLintInfo {
            locals: visitor.locals,
            usages: visitor.usages,
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

            // 3. Analyze each function for live variables across yielding call sites
            for (&_caller, info) in infos.iter() {
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
                            // Check if there is any usage after this yield
                            if let Some((_, usage_span)) = info_tcx
                                .usages
                                .iter()
                                .find(|(id, span)| *id == hir_id && span.lo() > yield_span.hi())
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
