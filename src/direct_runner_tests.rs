//! Native coverage for the coroutine-less driver — the exact execution path the wasm
//! playground runs, minus the browser. Each test boots the real stdlib prelude.

use super::{DirectConfig, DirectOutcome, run_source};
use crate::vm::StdStream;

fn run_with(source: &str, cfg: DirectConfig) -> (DirectOutcome, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let outcome = run_source(
        "playground.qn",
        source,
        cfg,
        &mut |stream, bytes| match stream {
            StdStream::Out => out.extend_from_slice(bytes),
            StdStream::Err => err.extend_from_slice(bytes),
        },
    );
    (
        outcome,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

fn run(source: &str) -> (DirectOutcome, String, String) {
    run_with(source, DirectConfig::default())
}

#[test]
fn hello_world_prints_through_the_capture_seam() {
    let (outcome, stdout, _stderr) = run("'hello from the playground'.print");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.exit_code, 0);
    assert!(stdout.contains("hello from the playground"), "{stdout:?}");
}

#[test]
fn final_expression_value_is_rendered() {
    let (outcome, _stdout, _stderr) = run("6 * 7");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.result.as_deref(), Some("42"));
}

#[test]
fn stdlib_collections_work() {
    let (outcome, _stdout, _stderr) = run("#( 1 2 3 ).collect:{ |x| x * x }");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.result.as_deref(), Some("#(1 4 9)"));
}

#[test]
fn uncaught_errors_surface_in_the_outcome() {
    let (outcome, _stdout, _stderr) = run("nil.frobnicate");
    let error = outcome.error.expect("an uncaught error");
    assert!(error.contains("frobnicate"), "{error:?}");
}

#[test]
fn compile_errors_report_through_the_stderr_sink() {
    // Assignment to an undeclared name is the canonical strict-`var` compile error.
    let (outcome, _stdout, stderr) = run("undeclared = 1");
    assert_eq!(outcome.error.as_deref(), Some("compile error"));
    assert!(stderr.contains("error"), "{stderr:?}");
}

#[test]
fn async_primitives_raise_catchable_errors_without_a_scheduler() {
    let (outcome, _stdout, _stderr) = run("var got = 'not caught';\n\
         { Async.sleep:1 }.catch:{ |e| got = 'caught' };\n\
         got");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.result.as_deref(), Some("'caught'"));
}

#[test]
fn runtime_exit_carries_its_status() {
    let (outcome, _stdout, _stderr) = run("Runtime.exit:3");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.exit_code, 3);
    assert_eq!(outcome.result, None);
}

#[test]
fn the_batch_budget_stops_a_runaway_loop() {
    let (outcome, _stdout, _stderr) = run_with(
        "{ true }.whileDo:{ 1 }",
        DirectConfig {
            max_batches: Some(10),
            ..DirectConfig::default()
        },
    );
    let error = outcome.error.expect("budget exhaustion");
    assert!(error.contains("instruction budget"), "{error:?}");
}
