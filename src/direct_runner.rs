//! Coroutine-less program execution: parse → compile → step `run_dispatch` directly
//! on the caller's stack.
//!
//! This is the wasm32 playground's engine — no corosensei, no scheduler, no reactor —
//! but it is target-clean and compiled everywhere, so the native test suite exercises
//! the exact path the browser runs (`direct_runner_tests.rs`). With no scheduler,
//! `vm.sched.yielder` stays `None` for the whole run: async/fiber/channel primitives
//! raise their existing catchable "outside the VM scheduler" errors instead of
//! parking, and OS-bound classes bottom out in the backend's `Unsupported`. Program
//! stdout/stderr (and compile diagnostics — same sink) arrive through
//! `VmState::write_std`'s capture seam, drained to the caller's sink between batches.

use crate::error::QuoinError;
use crate::parser::try_parse_quoin_string_named;
use crate::registry::register_builtins;
use crate::runner_core::compile_and_start;
use crate::runtime::pretty;
use crate::vm::{StdStream, VmOptions, VmState, VmStatus};
use gc_arena::{Arena, Rootable};

/// How a [`run_source`] call ended. At most one of `result`/`error` is set: a program
/// that runs to completion gets its final expression pretty-rendered into `result`; a
/// parse error, uncaught guest error, or exhausted budget lands in `error` (compile
/// errors are reported through the captured stderr sink instead, like the CLI).
#[derive(Debug)]
pub struct DirectOutcome {
    pub result: Option<String>,
    pub error: Option<String>,
    /// `Runtime.exit:`'s status when the guest called it, else 0 — orthogonal to
    /// `error`, exactly like a process exit code.
    pub exit_code: i32,
}

pub struct DirectConfig {
    pub vm_options: VmOptions,
    /// Hard cap on the *user unit's* dispatch batches (each
    /// `crate::tuning::step_batch()` instructions); the trusted stdlib prelude is not
    /// metered. `None` runs to completion. The playground's belt-and-suspenders
    /// against a runaway loop when the hosting worker isn't simply terminated.
    pub max_batches: Option<u64>,
    /// Columns for pretty-rendering the final value.
    pub render_width: usize,
}

impl Default for DirectConfig {
    fn default() -> Self {
        DirectConfig {
            vm_options: VmOptions::default(),
            max_batches: None,
            render_width: 100,
        }
    }
}

/// What one unit's drive loop concluded; `Finished` carries the rendered final value.
enum UnitEnd {
    Finished(String),
    Error(String),
    Exit(i32),
    BudgetExhausted,
}

/// One `run_dispatch` batch: still going (budget spent, GC debt to pay), or done.
enum StepOutcome {
    Running,
    Done(UnitEnd),
}

/// Parse and run `source` (named `name` in diagnostics) against the embedded stdlib
/// prelude, streaming captured stdout/stderr chunks to `on_output` between batches.
pub fn run_source(
    name: &str,
    source: &str,
    cfg: DirectConfig,
    on_output: &mut dyn FnMut(StdStream, &[u8]),
) -> DirectOutcome {
    let fail = |error: String| DirectOutcome {
        result: None,
        error: Some(error),
        exit_code: 0,
    };

    // Parse everything up front: nothing runs when the user unit doesn't parse.
    let Some(prelude_source) = crate::packages::read_stdlib_unit("prelude") else {
        return fail("cannot load the stdlib prelude".to_string());
    };
    let prelude_ast = match try_parse_quoin_string_named(&prelude_source, "prelude.qn") {
        Ok(node) => node,
        Err(e) => {
            // A parse failure in the shipped stdlib is our bug, not the user's.
            return fail(format!(
                "parse error in the stdlib prelude at line {}, col {}: {}",
                e.line,
                e.column + 1,
                e.message
            ));
        }
    };
    let user_ast = match try_parse_quoin_string_named(source, name) {
        Ok(node) => node,
        Err(e) => {
            return fail(format!(
                "{name}:{}:{}: parse error: {}",
                e.line,
                e.column + 1,
                e.message
            ));
        }
    };

    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, cfg.vm_options.clone());
        register_builtins(mc, &mut vm);
        // Route guest output (and compile diagnostics) into `output.chunks` instead
        // of the process fds; the drive loop drains them to `on_output`.
        vm.output.capture = true;
        vm
    });
    arena.metrics().set_pacing(crate::vm::gc_pacing());

    let batch = crate::tuning::step_batch();
    let mut batches_left = cfg.max_batches;

    for (is_user, ast) in [(false, &prelude_ast), (true, &user_ast)] {
        let compiled = arena.mutate_root(|mc, vm| compile_and_start(mc, vm, ast));
        if compiled.is_err() {
            drain_output(&mut arena, on_output);
            // The details (file:line:col, caret line) went to the captured stderr sink.
            return fail("compile error".to_string());
        }

        let end = loop {
            if is_user && let Some(left) = batches_left.as_mut() {
                if *left == 0 {
                    break UnitEnd::BudgetExhausted;
                }
                *left -= 1;
            }
            let step = arena.mutate_root(|mc, vm| match vm.run_dispatch(mc, batch) {
                Ok(VmStatus::Running) => StepOutcome::Running,
                Ok(VmStatus::Finished(val)) => StepOutcome::Done(UnitEnd::Finished(
                    pretty::render(val, cfg.render_width, vm.options.supports_color),
                )),
                Ok(VmStatus::Yeeted(val)) => {
                    StepOutcome::Done(UnitEnd::Error(format!("Uncaught exception: {}", val)))
                }
                Err(QuoinError::ExitRequested(code)) => StepOutcome::Done(UnitEnd::Exit(code)),
                Err(e) => StepOutcome::Done(UnitEnd::Error(e.to_string())),
            });
            drain_output(&mut arena, on_output);
            match step {
                StepOutcome::Running => arena.collect_debt(),
                StepOutcome::Done(end) => break end,
            }
        };

        match end {
            UnitEnd::Finished(rendered) => {
                if is_user {
                    return DirectOutcome {
                        result: Some(rendered),
                        error: None,
                        exit_code: 0,
                    };
                }
            }
            UnitEnd::Error(msg) => return fail(msg),
            UnitEnd::Exit(code) => {
                return DirectOutcome {
                    result: None,
                    error: None,
                    exit_code: code,
                };
            }
            UnitEnd::BudgetExhausted => {
                return fail(format!(
                    "instruction budget exhausted ({} batches of {} instructions)",
                    cfg.max_batches.unwrap_or(0),
                    batch
                ));
            }
        }
    }
    unreachable!("the user unit always returns above");
}

fn drain_output(
    arena: &mut Arena<Rootable![VmState<'_>]>,
    on_output: &mut dyn FnMut(StdStream, &[u8]),
) {
    let chunks = arena.mutate_root(|_mc, vm| vm.take_program_output());
    for chunk in chunks {
        on_output(chunk.stream, &chunk.bytes);
    }
}

#[cfg(test)]
#[path = "direct_runner_tests.rs"]
mod tests;
