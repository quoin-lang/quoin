# Custom Linter Design: Enforcing "Flush before Yield" in Quoin

*Status (verified 2026-07-09 at `dbe188d`): **SHIPPED.** `lint/no_gc_across_yield/` is a real
Dylint `LateLintPass`, and a companion this document does not mention — `lint/no_borrow_across_yield/`
— ships alongside it. Both are workspace members registered under `[workspace.metadata.dylint]`,
run by `cargo lint` and by CI's `lint` job (both deny-level since 2026-07-12 — warn-level let
four real findings accumulate unseen behind a green job; audited `#[allow]`s are the escape
hatch). They need the nightly `rustc-private` toolchain they pin themselves, which is why the
main `cargo build --workspace` excludes them. Read "Proposed Lint" below as the design that got
built.*

This document designs a custom static analysis tool (linter) to automatically verify that native methods in the Quoin VM follow the safety rules for stackful fibers. Specifically, it ensures that no garbage-collected references (`Value<'gc>` or `Gc<'gc, T>`) are held in local variables across a fiber yield point (`yielder.suspend(...)`).

---

## 1. Why the Rust Compiler Alone Cannot Enforce This

In the `gc_arena` model, the lifetime `'gc` is invariant and represents the lifetime of the entire garbage-collected arena. Any value stored on the heap or stack has the type `Value<'gc>` or `Gc<'gc, T>`.

Because the `'gc` lifetime is valid for the entire duration of the arena's existence:
1. The Rust borrow checker views `'gc` as a persistent lifetime that does not expire during execution.
2. When a stackful fiber yields, its stack is suspended, but the arena itself remains alive on the host thread.
3. Therefore, the borrow checker assumes that holding a `Value<'gc>` across `yielder.suspend()` is completely safe.
4. However, the garbage collector **cannot scan the fiber's native stack** during tracing. If the GC sweeps an object because the only reference to it is on a suspended fiber's stack, the pointer becomes dangling, leading to Undefined Behavior (UB) when the fiber resumes.

Since the type system cannot verify this boundary, we must use **static analysis** via a custom linter.

---

## 2. Lint Architecture: `cargo-dylint`

Rather than forking Clippy (which is hard to maintain and update), we use **Dylint**. Dylint allows compiling custom compiler lints as dynamic libraries (`.dylib` / `.so`) and running them alongside Clippy.

### Proposed Lint: `no_gc_across_yield`
* **Pass Type**: `LateLintPass` (runs after type checking, giving us access to full type information).
* **Goal**: Detect if any variable containing the `'gc` lifetime is live across a call to `yielder.suspend(...)`.

---

## 3. High-Level Lint Logic

To detect violations, the linter performs the following checks for every function or closure:

1. **Find Yield Points**: Search the AST/HIR of the function for expressions calling the `.suspend(...)` method on `Yielder`.
2. **Find `'gc` Variables**: Identify all local variables (`let` bindings) in the function whose types contain the `'gc` lifetime (e.g. `Value<'gc>`, `Gc<'gc, T>`, `Object<'gc>`).
3. **Liveness Analysis**: Verify if any `'gc` variable is defined *before* a yield point and used *after* that yield point.
4. **Trigger Warning**: If a `'gc` variable is live across a yield point, emit a compile-time lint warning/error.

```
       let val: Value<'gc> = ...; // 1. Variable containing 'gc defined
       ...
       yielder.suspend(...);      // 2. Yield point called
       ...
       do_something(val);         // 3. Variable used after yield -> LINT TRIGGERED!
```

---

## 4. Custom Lint Implementation Skeleton

Below is the Rust code skeleton for the Dylint library. It utilizes `rustc_hir`, `rustc_middle`, and `clippy_utils`.

```rust
// my_lints/no_gc_across_yield/src/lib.rs
#![feature(rustc_private)]

extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::ty::is_type_diagnostic_item;
use rustc_hir::def_id::DefId;
use rustc_hir::intravisit::{walk_expr, Visitor};
use rustc_hir::{Expr, ExprKind, Local, PatKind};
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty::{self, Ty};
use rustc_session::{declare_tool_lint, impl_lint_pass};
use rustc_span::Symbol;

declare_tool_lint! {
    pub clippy::NO_GC_ACROSS_YIELD,
    Warn,
    "checks for GC-managed values held in local variables across a fiber yield point",
}

pub struct NoGcAcrossYield;

impl_lint_pass!(NoGcAcrossYield => [NO_GC_ACROSS_YIELD]);

/// Visitor to find yield points and track expression ordering
struct YieldVisitor<'tcx> {
    yield_spans: Vec<rustc_span::Span>,
    usages_after_yield: Vec<(rustc_span::Span, rustc_hir::HirId)>,
    target_yield_span: Option<rustc_span::Span>,
}

impl<'tcx> Visitor<'tcx> for YieldVisitor<'tcx> {
    fn visit_expr(&mut self, expr: &'tcx Expr<'tcx>) {
        if let ExprKind::MethodCall(path, _, _, _) = expr.kind {
            if path.ident.name == Symbol::intern("suspend") {
                self.yield_spans.push(expr.span);
                self.target_yield_span = Some(expr.span);
            }
        }

        // If we have passed a yield point, record all variable usages (Path expressions)
        if let Some(yield_span) = self.target_yield_span {
            if let ExprKind::Path(rustc_hir::QPath::Resolved(_, path)) = expr.kind {
                if let rustc_hir::def::Res::Local(hir_id) = path.res {
                    self.usages_after_yield.push((expr.span, hir_id));
                }
            }
        }

        walk_expr(self, expr);
    }
}

impl<'tcx> LateLintPass<'tcx> for NoGcAcrossYield {
    fn check_fn(
        &mut self,
        cx: &LateContext<'tcx>,
        _: rustc_hir::intravisit::FnKind<'tcx>,
        _: &'tcx rustc_hir::FnDecl<'tcx>,
        body: &'tcx rustc_hir::Body<'tcx>,
        _: rustc_span::Span,
        _: rustc_hir::def_id::LocalDefId,
    ) {
        let mut visitor = YieldVisitor {
            yield_spans: Vec::new(),
            usages_after_yield: Vec::new(),
            target_yield_span: None,
        };
        visitor.visit_expr(body.value);

        if visitor.yield_spans.is_empty() {
            return;
        }

        // Analyze local variables in the function
        let typeck_results = cx.typeck_results();
        
        // Find local variables that contain the 'gc lifetime
        struct LocalGcFinder<'tcx, 'a> {
            cx: &'a LateContext<'tcx>,
            gc_locals: Vec<(rustc_hir::HirId, rustc_span::Span, Ty<'tcx>)>,
        }
        
        impl<'tcx, 'a> Visitor<'tcx> for LocalGcFinder<'tcx, 'a> {
            fn visit_local(&mut self, local: &'tcx Local<'tcx>) {
                if let PatKind::Binding(_, hir_id, _, _) = local.pat.kind {
                    let ty = self.cx.typeck_results().node_type(hir_id);
                    if contains_gc_lifetime(ty) {
                        self.gc_locals.push((hir_id, local.span, ty));
                    }
                }
            }
        }
        
        let mut finder = LocalGcFinder { cx, gc_locals: Vec::new() };
        finder.visit_expr(body.value);

        // Check if any GC-local is used after a yield point
        for (hir_id, local_span, ty) in finder.gc_locals {
            if let Some(&(usage_span, _)) = visitor.usages_after_yield.iter().find(|&&(_, id)| id == hir_id) {
                span_lint_and_help(
                    cx,
                    NO_GC_ACROSS_YIELD,
                    usage_span,
                    "GC value held and used across a fiber yield point",
                    Some(local_span),
                    "consider flushing this value back to the GC-tracked stack or receiver before yielding, and re-reading it after",
                );
            }
        }
    }
}

/// Helper function to check if a type contains the 'gc lifetime or references a GC-managed type
fn contains_gc_lifetime(ty: Ty<'_>) -> bool {
    // 1. Check if the type name or path contains 'gc
    // 2. Or check if the type matches known GC-managed classes (Value, Gc, Object)
    let ty_str = format!("{:?}", ty);
    ty_str.contains("'gc") || ty_str.contains("Value") || ty_str.contains("Gc")
}

#[no_mangle]
pub fn register_lints(_sess: &rustc_session::Session, lint_store: &mut rustc_lint::LintStore) {
    lint_store.register_lints(&[NO_GC_ACROSS_YIELD]);
    lint_store.register_late_pass(|_| Box::new(NoGcAcrossYield));
}

#[no_mangle]
pub fn dylint_version() -> &'static str {
    "0.1.0"
}
```

---

## 5. Integrating the Linter into the Build Workflow

To run the custom linter automatically during development:

1. **Install Dylint Tooling**:
   ```bash
   cargo install cargo-dylint dylint-link
   ```

2. **Add a `.dylint` workspace configuration**:
   Add a subdirectory `dylint-rules` in the workspace root, containing the lint crate above.

3. **Run Lint Checks**:
   Instead of running standard clippy, run:
   ```bash
   cargo dylint --all
   ```

4. **CI Integration**:
   Configure the CI pipeline (e.g. GitHub Actions) to run `cargo dylint --all` on pull requests. Any code that tries to keep a `Value<'gc>` across a yield point will fail the build, acting as a hard gate preventing garbage-collection safety regressions.
