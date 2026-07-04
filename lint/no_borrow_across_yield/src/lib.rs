#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_hir;
extern crate rustc_span;

use rustc_lint::{LateContext, LateLintPass};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

dylint_linting::declare_late_lint! {
    /// ### What it does
    /// Flags `RefCell`/`RefLock` borrow guards that are still alive when execution can
    /// cooperatively yield (a `yielder.suspend(...)` call, or any call that transitively
    /// reaches one).
    ///
    /// ### Why is this bad?
    /// A yield suspends the coroutine with the guard alive on its Rust stack. Any other
    /// borrow of the same cell — from another task, or re-entrantly from the Quoin code
    /// the suspended call was running — then panics "RefCell already borrowed". Unlike
    /// the GC hazard (`no_gc_across_yield`), the guard doesn't have to be *used* after
    /// the yield: being not-yet-dropped is the whole bug, so this lint reasons about
    /// drop scopes, not uses. Three shapes are covered:
    ///
    /// 1. a named guard local whose scope (or explicit `drop`) ends after a yield point;
    /// 2. a guard temporary in an `if let`/`while let`/`match` scrutinee — such
    ///    temporaries live through the success branch / all arms (edition 2024's rescope
    ///    only shortened the `else` path), which is how the VM's
    ///    `finalize_instantiation` held a class borrow across `init` execution;
    /// 3. a guard temporary nested inside a yield-capable call's receiver or arguments.
    ///
    /// ### Example
    /// ```rust,ignore
    /// let g = class.borrow();               // guard...
    /// vm.call_method(mc, v, "s", vec![])?;  // ...alive across a yield-capable call
    /// use_it(&g);
    /// ```
    /// Use instead: copy what you need out of the borrow and drop it first —
    /// ```rust,ignore
    /// let method = class.borrow().instance_methods.get("init").copied(); // temp dies here
    /// vm.call_method(mc, v, "s", vec![])?;
    /// ```
    ///
    /// ### Known problems
    /// Span-based, per-crate: closures are analyzed as separate functions but calls
    /// *through* function pointers/closures are not edges in the yield graph, and a
    /// guard smuggled through a non-`drop` move (e.g. into a struct) ends liveness
    /// tracking silently.
    pub NO_BORROW_ACROSS_YIELD,
    Deny,
    "RefCell/RefLock borrow guards held across a fiber yield point"
}

/// Everything recorded about one function body. Plain data (spans, ids, strings),
/// so the cross-function pass needs no lifetime tricks.
#[derive(Default)]
struct FnLintInfo {
    /// Named guard locals: (binding, decl span, type string, enclosing drop scope,
    /// initializer span — yields inside the initializer predate the guard).
    guard_locals: Vec<(
        rustc_hir::HirId,
        rustc_span::Span,
        String,
        rustc_span::Span,
        Option<rustc_span::Span>,
    )>,
    /// Explicit `drop(x)` calls: (dropped local, span of the drop call).
    drops: Vec<(rustc_hir::HirId, rustc_span::Span)>,
    /// Guard-producing calls (`.borrow()`/`.borrow_mut()`/`try_` variants on a
    /// RefLock/RefCell receiver): (expr id for allow-resolution, span).
    guard_calls: Vec<(rustc_hir::HirId, rustc_span::Span)>,
    /// Extended-temporary windows: a guard call inside an `if let`/`match` scrutinee
    /// keeps its guard alive through `window` (the success branch / the arms).
    windows: Vec<(rustc_hir::HirId, rustc_span::Span, rustc_span::Span)>,
    /// Calls out of this function: (callee, call span).
    calls: Vec<(rustc_hir::def_id::DefId, rustc_span::Span)>,
    /// Direct `yielder.suspend(...)` spans.
    suspend_spans: Vec<rustc_span::Span>,
}

thread_local! {
    static FN_INFOS: RefCell<HashMap<rustc_hir::def_id::LocalDefId, FnLintInfo>> =
        RefCell::new(HashMap::new());
}

/// Does this type (as a debug string) look like a live borrow guard?
/// `Ref<…>`/`RefMut<…>` from `std::cell` or `gc_arena::lock`, possibly wrapped
/// (`Result<Ref<…>, …>` from the `try_` variants). Note `RefLock<`/`RefCell<`
/// themselves do NOT match — holding the *cell* (or a `Gc` to one) is fine.
fn is_guard_ty(ty_str: &str) -> bool {
    ty_str.contains("Ref<") || ty_str.contains("RefMut<")
}

/// Is this the receiver of a real cell borrow (rather than some unrelated
/// `borrow()` method)?
fn is_cell_receiver(ty_str: &str) -> bool {
    ty_str.contains("RefLock") || ty_str.contains("RefCell")
}

struct BorrowYieldVisitor<'tcx, 'sym> {
    cx: &'sym LateContext<'tcx>,
    /// Innermost enclosing block spans; a local's guard drops at the end of the
    /// block it was declared in.
    block_stack: Vec<rustc_span::Span>,
    registered_locals: HashSet<rustc_hir::HirId>,
    info: FnLintInfo,
}

impl<'tcx, 'sym> BorrowYieldVisitor<'tcx, 'sym> {
    fn current_scope(&self) -> rustc_span::Span {
        *self
            .block_stack
            .last()
            .expect("visiting inside a function body")
    }

    fn register_pat_bindings(
        &mut self,
        pat: &'tcx rustc_hir::Pat<'tcx>,
        init_span: Option<rustc_span::Span>,
    ) {
        struct BindingFinder<'a, 'tcx, 'sym> {
            visitor: &'a mut BorrowYieldVisitor<'tcx, 'sym>,
            init_span: Option<rustc_span::Span>,
        }
        impl<'a, 'tcx, 'sym> rustc_hir::intravisit::Visitor<'tcx> for BindingFinder<'a, 'tcx, 'sym> {
            fn visit_pat(&mut self, pat: &'tcx rustc_hir::Pat<'tcx>) {
                if let rustc_hir::PatKind::Binding(_, hir_id, _ident, _) = pat.kind
                    && !self.visitor.registered_locals.contains(&hir_id)
                {
                    let ty = self.visitor.cx.typeck_results().node_type(hir_id);
                    let ty_str = format!("{:?}", ty);
                    if is_guard_ty(&ty_str) {
                        let scope = self.visitor.current_scope();
                        self.visitor.info.guard_locals.push((
                            hir_id,
                            pat.span,
                            ty_str,
                            scope,
                            self.init_span,
                        ));
                        self.visitor.registered_locals.insert(hir_id);
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

    /// Collect guard-producing calls within `expr` (an `if let`/`match` scrutinee).
    fn guard_calls_within(
        &self,
        expr: &'tcx rustc_hir::Expr<'tcx>,
    ) -> Vec<(rustc_hir::HirId, rustc_span::Span)> {
        struct GuardFinder<'a, 'tcx, 'sym> {
            visitor: &'a BorrowYieldVisitor<'tcx, 'sym>,
            found: Vec<(rustc_hir::HirId, rustc_span::Span)>,
        }
        impl<'a, 'tcx, 'sym> rustc_hir::intravisit::Visitor<'tcx> for GuardFinder<'a, 'tcx, 'sym> {
            fn visit_expr(&mut self, expr: &'tcx rustc_hir::Expr<'tcx>) {
                if self.visitor.is_guard_call(expr) {
                    self.found.push((expr.hir_id, expr.span));
                }
                rustc_hir::intravisit::walk_expr(self, expr);
            }
        }
        let mut finder = GuardFinder {
            visitor: self,
            found: Vec::new(),
        };
        rustc_hir::intravisit::Visitor::visit_expr(&mut finder, expr);
        finder.found
    }

    fn is_guard_call(&self, expr: &'tcx rustc_hir::Expr<'tcx>) -> bool {
        if let rustc_hir::ExprKind::MethodCall(path, receiver, _, _) = expr.kind {
            let name = path.ident.name.as_str();
            if matches!(
                name,
                "borrow" | "borrow_mut" | "try_borrow" | "try_borrow_mut"
            ) {
                let recv_ty = self.cx.typeck_results().expr_ty_adjusted(receiver);
                return is_cell_receiver(&format!("{:?}", recv_ty));
            }
        }
        false
    }

    /// Does `expr` contain a `let` sub-expression (an `if let` / let-chain condition)?
    /// A plain-bool `if` condition is wrapped in `DropTemps` instead, and its
    /// temporaries die before the branch runs — no extension.
    fn contains_let(expr: &'tcx rustc_hir::Expr<'tcx>) -> bool {
        struct LetFinder(bool);
        impl<'tcx> rustc_hir::intravisit::Visitor<'tcx> for LetFinder {
            fn visit_expr(&mut self, expr: &'tcx rustc_hir::Expr<'tcx>) {
                if matches!(expr.kind, rustc_hir::ExprKind::Let(_)) {
                    self.0 = true;
                }
                if !self.0 {
                    rustc_hir::intravisit::walk_expr(self, expr);
                }
            }
        }
        let mut finder = LetFinder(false);
        rustc_hir::intravisit::Visitor::visit_expr(&mut finder, expr);
        finder.0
    }
}

impl<'tcx, 'sym> rustc_hir::intravisit::Visitor<'tcx> for BorrowYieldVisitor<'tcx, 'sym> {
    fn visit_block(&mut self, block: &'tcx rustc_hir::Block<'tcx>) {
        self.block_stack.push(block.span);
        rustc_hir::intravisit::walk_block(self, block);
        self.block_stack.pop();
    }

    fn visit_local(&mut self, local: &'tcx rustc_hir::LetStmt<'tcx>) {
        self.register_pat_bindings(local.pat, local.init.map(|i| i.span));
        rustc_hir::intravisit::walk_local(self, local);
    }

    fn visit_pat(&mut self, pat: &'tcx rustc_hir::Pat<'tcx>) {
        // Match-arm and other non-`let` bindings of guard values.
        self.register_pat_bindings(pat, None);
        rustc_hir::intravisit::walk_pat(self, pat);
    }

    fn visit_expr(&mut self, expr: &'tcx rustc_hir::Expr<'tcx>) {
        match expr.kind {
            rustc_hir::ExprKind::MethodCall(path, _, _, _) => {
                if path.ident.name.as_str() == "suspend" {
                    self.info.suspend_spans.push(expr.span);
                }
                if let Some(def_id) = self.cx.typeck_results().type_dependent_def_id(expr.hir_id) {
                    self.info.calls.push((def_id, expr.span));
                }
                if self.is_guard_call(expr) {
                    self.info.guard_calls.push((expr.hir_id, expr.span));
                }
            }
            rustc_hir::ExprKind::Call(callee, args) => {
                if let rustc_hir::ExprKind::Path(qpath) = &callee.kind {
                    let res = self.cx.typeck_results().qpath_res(qpath, callee.hir_id);
                    if let Some(def_id) = res.opt_def_id() {
                        self.info.calls.push((def_id, expr.span));
                    }
                    // `drop(x)` / `mem::drop(x)` on a local ends its guard's life.
                    if let rustc_hir::QPath::Resolved(_, path) = qpath
                        && let Some(seg) = path.segments.last()
                        && seg.ident.name.as_str() == "drop"
                        && args.len() == 1
                        && let rustc_hir::ExprKind::Path(rustc_hir::QPath::Resolved(_, arg_path)) =
                            &args[0].kind
                        && let rustc_hir::def::Res::Local(hir_id) = arg_path.res
                    {
                        self.info.drops.push((hir_id, expr.span));
                    }
                }
            }
            // `if let …` / let-chains: scrutinee temporaries live through the success
            // branch. (A plain-bool condition arrives wrapped in `DropTemps` — its
            // temporaries die before the branch — and `contains_let` is false.)
            rustc_hir::ExprKind::If(cond, then, _) => {
                if Self::contains_let(cond) {
                    for (hir_id, guard_span) in self.guard_calls_within(cond) {
                        self.info.windows.push((hir_id, guard_span, then.span));
                    }
                }
            }
            // `match` (and the `for`/`while let` desugarings): scrutinee temporaries
            // live through every arm.
            rustc_hir::ExprKind::Match(scrutinee, _, _) => {
                for (hir_id, guard_span) in self.guard_calls_within(scrutinee) {
                    self.info.windows.push((hir_id, guard_span, expr.span));
                }
            }
            _ => {}
        }
        rustc_hir::intravisit::walk_expr(self, expr);
    }
}

impl<'tcx> LateLintPass<'tcx> for NoBorrowAcrossYield {
    fn check_fn(
        &mut self,
        cx: &LateContext<'tcx>,
        _: rustc_hir::intravisit::FnKind<'tcx>,
        _: &'tcx rustc_hir::FnDecl<'tcx>,
        body: &'tcx rustc_hir::Body<'tcx>,
        _: rustc_span::Span,
        def_id: rustc_hir::def_id::LocalDefId,
    ) {
        let mut visitor = BorrowYieldVisitor {
            cx,
            block_stack: vec![body.value.span],
            registered_locals: HashSet::new(),
            info: FnLintInfo::default(),
        };

        for param in body.params {
            visitor.register_pat_bindings(param.pat, None);
        }
        rustc_hir::intravisit::Visitor::visit_expr(&mut visitor, body.value);

        let info = visitor.info;
        FN_INFOS.with(|infos| {
            infos.borrow_mut().insert(def_id, info);
        });
    }

    fn check_crate_post(&mut self, cx: &LateContext<'tcx>) {
        FN_INFOS.with(|infos| {
            let infos = infos.borrow();

            // 1. Which local functions can (transitively) suspend?
            let mut call_graph: HashMap<
                rustc_hir::def_id::LocalDefId,
                HashSet<rustc_hir::def_id::LocalDefId>,
            > = HashMap::new();
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
            let mut changed = true;
            while changed {
                changed = false;
                for (&caller, callees) in &call_graph {
                    if !yielding_fns.contains(&caller)
                        && callees.iter().any(|c| yielding_fns.contains(c))
                    {
                        yielding_fns.insert(caller);
                        changed = true;
                    }
                }
            }

            // 2. Per function: every yield point is a direct suspend or a call to a
            //    (transitively) yielding local function. Findings are collected and
            //    emitted in source order (the map's iteration order is arbitrary).
            struct Finding {
                hir_id: rustc_hir::HirId,
                primary: rustc_span::Span,
                msg: &'static str,
                help_span: rustc_span::Span,
                help: String,
            }
            let mut findings: Vec<Finding> = Vec::new();
            for info in infos.values() {
                let mut yield_spans = info.suspend_spans.clone();
                for &(callee, span) in &info.calls {
                    if callee
                        .as_local()
                        .is_some_and(|local| yielding_fns.contains(&local))
                    {
                        yield_spans.push(span);
                    }
                }
                if yield_spans.is_empty() {
                    continue;
                }

                // Shape 1: a named guard local alive (scope not ended, not dropped)
                // at a yield point.
                for &(hir_id, decl_span, ref ty_str, scope, init_span) in &info.guard_locals {
                    let live_hi = info
                        .drops
                        .iter()
                        .filter(|(id, span)| *id == hir_id && span.lo() > decl_span.hi())
                        .map(|(_, span)| span.lo())
                        .min()
                        .unwrap_or_else(|| scope.hi());
                    if let Some(&y) = yield_spans.iter().find(|y| {
                        // a yield inside the initializer predates the guard
                        let in_init = init_span
                            .is_some_and(|init| y.lo() >= init.lo() && y.hi() <= init.hi());
                        !in_init && y.lo() > decl_span.hi() && y.hi() <= live_hi
                    }) {
                        findings.push(Finding {
                            hir_id,
                            primary: y,
                            msg: "borrow guard is still alive at a yield point: a suspended \
                                  task holding it panics any other borrow of the same cell",
                            help_span: decl_span,
                            help: format!(
                                "guard of type `{}` declared here — drop it (or copy what \
                                 you need out of it) before anything that can yield",
                                ty_str
                            ),
                        });
                    }
                }

                // Shape 2: a guard temporary in an `if let`/`match` scrutinee, with a
                // yield inside the branch/arms the temporary lives through.
                for &(hir_id, guard_span, window) in &info.windows {
                    if let Some(&y) = yield_spans
                        .iter()
                        .find(|y| y.lo() > guard_span.hi() && y.hi() <= window.hi())
                    {
                        findings.push(Finding {
                            hir_id,
                            primary: y,
                            msg: "a scrutinee's temporary borrow guard is still alive at \
                                  this yield point (scrutinee temporaries live through the \
                                  branch)",
                            help_span: guard_span,
                            help: "the guard is created here; hoist this borrow into a `let` \
                                   above the `if let`/`match` so it drops at the statement's \
                                   end"
                            .to_string(),
                        });
                    }
                }

                // Shape 3: a guard temporary nested inside a yield-capable call —
                // alive for the whole call by construction.
                for &(hir_id, guard_span) in &info.guard_calls {
                    if let Some(&y) = yield_spans
                        .iter()
                        .find(|y| guard_span.lo() >= y.lo() && guard_span.hi() < y.hi())
                    {
                        findings.push(Finding {
                            hir_id,
                            primary: guard_span,
                            msg: "temporary borrow guard is alive for the duration of a \
                                  yield-capable call",
                            help_span: y,
                            help: "this call can suspend while the guard is live; bind the \
                                   borrowed data to a local first so the guard drops before \
                                   the call"
                                .to_string(),
                        });
                    }
                }
            }

            findings.sort_by_key(|f| (f.primary.lo(), f.help_span.lo()));
            for f in findings {
                clippy_utils::diagnostics::span_lint_hir_and_then(
                    cx,
                    NO_BORROW_ACROSS_YIELD,
                    f.hir_id,
                    f.primary,
                    f.msg,
                    |diag| {
                        diag.span_help(f.help_span, f.help);
                    },
                );
            }
        });
        FN_INFOS.with(|infos| infos.borrow_mut().clear());
    }
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}
