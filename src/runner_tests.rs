use super::runner_dap::{DapFrontend, PendingProgram};
use super::runner_driver::{drive_main_task, drive_with_frontend, install_main_task};
use super::runner_repl::flow_names;
use super::*;
use crate::debug::{DebugAction, DebugState};
use std::collections::{HashMap, HashSet, VecDeque};

/// Run `source` to completion under the real scheduler with a debug session attached:
/// line `breakpoints` as `(file, line)`, and a `script` of actions applied at successive
/// pauses (one per pause; the run continues past any pause the script doesn't cover).
/// Returns the `pause_log`. Exercises the full mechanism end-to-end — the step-loop hook
/// fires, suspends via `DebugBreak`, the driver applies the action, and the task resumes
/// in place to completion.
fn run_debug(
    source: &str,
    filename: &str,
    breakpoints: &[(&str, usize)],
    script: &[DebugAction],
) -> Vec<(String, usize)> {
    run_debug_full(source, filename, breakpoints, &[], &[], script)
}

/// Like [`run_debug`], but also arms break-on-throw / break-on-uncaught for the given exception
/// types. Used to exercise the exception-breakpoint paths end-to-end. A non-empty
/// `break_on_uncaught` lets the task end with its (uncaught) error rather than asserting success.
fn run_debug_full(
    source: &str,
    filename: &str,
    breakpoints: &[(&str, usize)],
    break_on_throw: &[&str],
    break_on_uncaught: &[&str],
    script: &[DebugAction],
) -> Vec<(String, usize)> {
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        register_builtins(mc, &mut vm);
        vm
    });
    // Load the prelude so the fixture can use the stdlib (blocks, ranges, …).
    for ast in prelude_asts() {
        arena.mutate_root(|mc, vm| {
            if let NodeValue::Program(p) = &ast.value
                && let Ok(sb) = Compiler::new().compile_program(p)
            {
                let block = build_block(mc, &sb);
                let _ = vm.execute_block(mc, block, Vec::new(), None);
            }
        });
    }
    // Compile the fixture, attach the debug session, install it as task #0.
    let node = try_parse_quoin_string_named(source, filename).expect("fixture parses");
    arena.mutate_root(|mc, vm| {
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a Program node");
        };
        let sb = Compiler::new()
            .compile_program(p)
            .expect("fixture compiles");
        let block = build_block(mc, &sb);
        let mut bps: HashMap<String, HashSet<usize>> = HashMap::new();
        for (f, l) in breakpoints {
            bps.entry((*f).to_string()).or_default().insert(*l);
        }
        vm.instrumentation.debug = Some(DebugState {
            breakpoints: bps,
            break_on_throw: break_on_throw.iter().map(|s| s.to_string()).collect(),
            break_on_uncaught: break_on_uncaught.iter().map(|s| s.to_string()).collect(),
            script: VecDeque::from(script.to_vec()),
            ..Default::default()
        });
        vm.start_block(mc, block, Vec::new(), None, None);
        install_main_task(mc, vm);
    });
    let drive = drive_main_task(&mut arena);
    // A break-on-uncaught fixture deliberately lets the exception escape — the task ends with
    // that (uncaught) error, which is expected; we still want the pause log.
    if break_on_uncaught.is_empty() {
        drive.expect("fixture runs to completion");
    }
    arena.mutate_root(|_mc, vm| {
        vm.instrumentation
            .debug
            .as_ref()
            .map(|d| d.pause_log.clone())
            .unwrap_or_default()
    })
}

/// Like [`run_debug_full`] but arms break-on-*uncaught* (fixture filename `fixture.qn`).
fn run_debug_uncaught(
    source: &str,
    break_on_uncaught: &[&str],
    script: &[DebugAction],
) -> Vec<(String, usize)> {
    run_debug_full(source, "fixture.qn", &[], &[], break_on_uncaught, script)
}

/// Drive the DAP frontend over a scripted, in-memory request stream and return the raw
/// protocol output (framed responses + events). Mirrors `run_debug_full`'s setup, but speaks
/// DAP: each request's `seq`/`type` are filled in here, in order.
fn run_dap_script(
    source: &str,
    filename: &str,
    breakpoints: &[(&str, usize)],
    requests: &[serde_json::Value],
) -> String {
    let mut input = Vec::new();
    for (i, req) in requests.iter().enumerate() {
        let mut obj = req.clone();
        obj["seq"] = serde_json::json!(i + 1);
        obj["type"] = serde_json::json!("request");
        let body = serde_json::to_string(&obj).unwrap();
        input.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        input.extend_from_slice(body.as_bytes());
    }

    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        register_builtins(mc, &mut vm);
        vm
    });
    for ast in prelude_asts() {
        arena.mutate_root(|mc, vm| {
            if let NodeValue::Program(p) = &ast.value
                && let Ok(sb) = Compiler::new().compile_program(p)
            {
                let block = build_block(mc, &sb);
                let _ = vm.execute_block(mc, block, Vec::new(), None);
            }
        });
    }
    let node = try_parse_quoin_string_named(source, filename).expect("fixture parses");
    arena.mutate_root(|mc, vm| {
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a Program node");
        };
        let sb = Compiler::new()
            .compile_program(p)
            .expect("fixture compiles");
        let block = build_block(mc, &sb);
        let mut bps: HashMap<String, HashSet<usize>> = HashMap::new();
        for (f, l) in breakpoints {
            bps.entry((*f).to_string()).or_default().insert(*l);
        }
        vm.instrumentation.debug = Some(DebugState {
            breakpoints: bps,
            interactive: true, // bubble pauses to the DAP frontend
            ..Default::default()
        });
        vm.output.capture = true;
        vm.start_block(mc, block, Vec::new(), None, None);
        install_main_task(mc, vm);
    });

    let conn = crate::dap::Connection::new(std::io::Cursor::new(input), Vec::new());
    let mut frontend = DapFrontend::new(conn);
    let _ = drive_with_frontend(&mut arena, &mut frontend);
    String::from_utf8_lossy(&frontend.conn.into_writer()).into_owned()
}

/// The DAP spine round-trip: `initialize`/`launch`/`setBreakpoints`/`configurationDone` →
/// run → `stopped`(breakpoint) → `continue` → `terminated`.
#[test]
fn dap_spine_launch_breakpoint_continue_terminate() {
    use serde_json::json;
    // Breakpoint on line 2 (`n * 2`, the block body), which fires when `f` is invoked.
    let source = "f = { |n|\n    n * 2\n};\nf.value: 21\n";
    let out = run_dap_script(
        source,
        "fixture.qn",
        &[("fixture.qn", 2)],
        &[
            json!({ "command": "initialize", "arguments": { "adapterID": "quoin" } }),
            json!({ "command": "launch", "arguments": {} }),
            json!({ "command": "setBreakpoints", "arguments": {
                "source": { "path": "fixture.qn" },
                "breakpoints": [ { "line": 2 } ],
            }}),
            json!({ "command": "configurationDone", "arguments": {} }),
            json!({ "command": "continue", "arguments": { "threadId": 1 } }),
        ],
    );
    // Handshake: capabilities + the `initialized` event.
    assert!(out.contains(r#""command":"initialize""#), "{out}");
    assert!(
        out.contains(r#""supportsConfigurationDoneRequest":true"#),
        "{out}"
    );
    assert!(out.contains(r#""event":"initialized""#), "{out}");
    // Breakpoint hit -> stopped(breakpoint), then continue -> terminated, in that order.
    let stopped = out
        .find(r#""event":"stopped""#)
        .unwrap_or_else(|| panic!("no stopped event in:\n{out}"));
    assert!(out[stopped..].contains(r#""reason":"breakpoint""#), "{out}");
    let terminated = out
        .find(r#""event":"terminated""#)
        .expect("a terminated event");
    assert!(
        stopped < terminated,
        "stopped must precede terminated:\n{out}"
    );
}

/// `qn debug --dap` with no CLI file: the program path arrives in the `launch` request's
/// `program` field and the adapter loads it from there (via a `DapFrontend::with_pending`).
/// Proven by a breakpoint firing in the launch-supplied program, then terminating.
#[test]
fn dap_launch_loads_program_from_request() {
    use serde_json::json;
    use std::io::Write;

    // The launch handler reads the program by path, so the fixture must exist on disk.
    let source = "f = { |n|\n    n * 2\n};\nf.value: 21\n";
    let mut path = std::env::temp_dir();
    path.push("quoin_dap_launch_from_request.qn");
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(source.as_bytes()))
        .expect("write temp fixture");
    let path_str = path.to_string_lossy().into_owned();

    let requests = [
        json!({ "command": "initialize", "arguments": { "adapterID": "quoin" } }),
        json!({ "command": "launch", "arguments": { "program": path_str } }),
        json!({ "command": "setBreakpoints", "arguments": {
            "source": { "path": path_str },
            "breakpoints": [ { "line": 2 } ],
        }}),
        json!({ "command": "configurationDone", "arguments": {} }),
        json!({ "command": "continue", "arguments": { "threadId": 1 } }),
    ];
    let mut input = Vec::new();
    for (i, req) in requests.iter().enumerate() {
        let mut obj = req.clone();
        obj["seq"] = json!(i + 1);
        obj["type"] = json!("request");
        let body = serde_json::to_string(&obj).unwrap();
        input.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        input.extend_from_slice(body.as_bytes());
    }

    // Build the arena + prelude only — NO program install; the launch handler installs it.
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        register_builtins(mc, &mut vm);
        vm
    });
    for ast in prelude_asts() {
        arena.mutate_root(|mc, vm| {
            if let NodeValue::Program(p) = &ast.value
                && let Ok(sb) = Compiler::new().compile_program(p)
            {
                let block = build_block(mc, &sb);
                let _ = vm.execute_block(mc, block, Vec::new(), None);
            }
        });
    }

    let conn = crate::dap::Connection::new(std::io::Cursor::new(input), Vec::new());
    let mut frontend = DapFrontend::with_pending(
        conn,
        PendingProgram {
            break_on_throw: Vec::new(),
            break_on_uncaught: Vec::new(),
        },
    );
    let _ = drive_with_frontend(&mut arena, &mut frontend);
    let out = String::from_utf8_lossy(&frontend.conn.into_writer()).into_owned();
    let _ = std::fs::remove_file(&path);

    // The launch-supplied program ran, hit the breakpoint, and terminated.
    let stopped = out
        .find(r#""event":"stopped""#)
        .unwrap_or_else(|| panic!("no stopped event in:\n{out}"));
    assert!(out[stopped..].contains(r#""reason":"breakpoint""#), "{out}");
    assert!(out.contains(r#""event":"terminated""#), "{out}");
}

/// At a breakpoint inside an invoked block, `stackTrace`/`scopes`/`variables`/`evaluate`
/// return real frame state: the call stack, the block parameter `n`, and an expression
/// evaluated in that frame.
#[test]
fn dap_inspection_stack_variables_and_evaluate() {
    use serde_json::json;
    // Breakpoint on line 2 (`n * 2`); the innermost frame at the stop is the invoked block.
    let source = "double = { |n|\n    n * 2\n}\nresult = double.value: 21\nresult.print\n";
    let out = run_dap_script(
        source,
        "fixture.qn",
        &[("fixture.qn", 2)],
        &[
            json!({ "command": "initialize", "arguments": {} }),
            json!({ "command": "launch", "arguments": {} }),
            json!({ "command": "setBreakpoints", "arguments": {
                "source": { "path": "fixture.qn" },
                "breakpoints": [ { "line": 2 } ],
            }}),
            json!({ "command": "configurationDone", "arguments": {} }),
            json!({ "command": "stackTrace", "arguments": { "threadId": 1 } }),
            json!({ "command": "scopes", "arguments": { "frameId": 1 } }),
            json!({ "command": "variables", "arguments": { "variablesReference": 1 } }),
            json!({ "command": "evaluate", "arguments": { "expression": "n * 2", "frameId": 1 } }),
            json!({ "command": "continue", "arguments": { "threadId": 1 } }),
        ],
    );
    // stackTrace: two frames, the invoked block at line 2 innermost.
    assert!(out.contains(r#""totalFrames":2"#), "{out}");
    assert!(out.contains(r#""line":2"#), "{out}");
    // scopes -> a Locals scope with a handle; variables -> the block parameter n = 21.
    assert!(out.contains(r#""name":"Locals""#), "{out}");
    assert!(
        out.contains(r#"{"name":"n","value":"21","variablesReference":0}"#),
        "{out}"
    );
    // evaluate `n * 2` in that frame -> 42.
    assert!(out.contains(r#""result":"42""#), "{out}");
}

/// `variables` returns an expandable tree: a collection local reports a non-zero
/// `variablesReference`, and requesting it returns the element children — recursively (a nested
/// list mints its own child handle). Handles are lazy: scope -> handle 1, the first expandable
/// row -> handle 2, its first expandable child -> handle 3.
#[test]
fn dap_variables_expand_nested_collection() {
    use serde_json::json;
    // At line 2, the block local `xs` is `#(#(1 2) 30)` — a list whose first element is a sub-list.
    let source = "f = { |xs|\n    xs.size\n};\nf.value: #(#(1 2) 30)\n";
    let out = run_dap_script(
        source,
        "fixture.qn",
        &[("fixture.qn", 2)],
        &[
            json!({ "command": "initialize", "arguments": {} }),
            json!({ "command": "launch", "arguments": {} }),
            json!({ "command": "setBreakpoints", "arguments": {
                "source": { "path": "fixture.qn" },
                "breakpoints": [ { "line": 2 } ],
            }}),
            json!({ "command": "configurationDone", "arguments": {} }),
            json!({ "command": "scopes", "arguments": { "frameId": 1 } }),               // -> handle 1
            json!({ "command": "variables", "arguments": { "variablesReference": 1 } }),  // Locals: xs -> handle 2
            json!({ "command": "variables", "arguments": { "variablesReference": 2 } }),  // xs: [0] sub-list -> handle 3
            json!({ "command": "variables", "arguments": { "variablesReference": 3 } }),  // expand the sub-list
            json!({ "command": "continue", "arguments": { "threadId": 1 } }),
        ],
    );
    // `xs` is expandable (it got child handle 2).
    assert!(out.contains(r#""name":"xs""#), "{out}");
    assert!(out.contains(r#""variablesReference":2"#), "{out}");
    // Its first element is itself an expandable sub-list (handle 3); the second is a leaf.
    assert!(
        out.contains(r##"{"name":"[0]","value":"#(1 2)","variablesReference":3}"##),
        "{out}"
    );
    assert!(
        out.contains(r#"{"name":"[1]","value":"30","variablesReference":0}"#),
        "{out}"
    );
    // Expanding the sub-list (handle 3) yields its leaf elements.
    assert!(
        out.contains(r#"{"name":"[0]","value":"1","variablesReference":0}"#),
        "{out}"
    );
    assert!(
        out.contains(r#"{"name":"[1]","value":"2","variablesReference":0}"#),
        "{out}"
    );
}

#[test]
fn breakpoint_pauses_per_arrival_and_resumes_to_completion() {
    // `dbl`'s body is on line 2 alone; it is invoked twice, so the line-2 breakpoint
    // must fire once per invocation (a new frame each time) — proving the hook fires,
    // suspends, and resumes, and that re-entry in a fresh frame re-triggers.
    let source = "\
dbl = { |n|
    n * 2
};
a = dbl.value: 3;
b = dbl.value: 4;
a + b
";
    let log = run_debug(source, "fixture.qn", &[("fixture.qn", 2)], &[]);
    assert_eq!(
        log,
        vec![("fixture.qn".to_string(), 2), ("fixture.qn".to_string(), 2),],
    );
}

#[test]
fn no_breakpoints_never_pauses() {
    let log = run_debug("x = 1;\ny = 2;\nx + y\n", "fixture.qn", &[], &[]);
    assert!(log.is_empty());
}

/// A fixture with a helper called from the top level — exercises step-into / over / out.
/// Lines: 1 `f = { |n|`, 2 `a = n + 1;`, 3 `a * 2`, 4 `};`, 5 `x = 5;`,
/// 6 `y = f.value: x;`, 7 `y + 1`.
const STEP_FIXTURE: &str = "\
f = { |n|
    a = n + 1;
    a * 2
};
x = 5;
y = f.value: x;
y + 1
";

fn lines(log: &[(String, usize)]) -> Vec<usize> {
    log.iter().map(|(_, l)| *l).collect()
}

#[test]
fn step_over_skips_the_call_and_advances_line_by_line() {
    // Break at line 5; step-over to 6, then step-over the call on 6 (must NOT descend
    // into f's body on lines 2-3) landing on 7; then continue.
    let log = run_debug(
        STEP_FIXTURE,
        "fixture.qn",
        &[("fixture.qn", 5)],
        &[
            DebugAction::StepOver,
            DebugAction::StepOver,
            DebugAction::Continue,
        ],
    );
    assert_eq!(lines(&log), vec![5, 6, 7]);
}

#[test]
fn step_into_descends_into_the_called_block() {
    // Break at the call site (line 6); step-into must stop at f's first body line (2).
    let log = run_debug(
        STEP_FIXTURE,
        "fixture.qn",
        &[("fixture.qn", 6)],
        &[DebugAction::StepInto, DebugAction::Continue],
    );
    assert_eq!(lines(&log), vec![6, 2]);
}

#[test]
fn step_out_runs_to_the_caller() {
    // Break inside f (line 2); step-out runs f to its return and stops back in the
    // caller (the still-executing call site, line 6).
    let log = run_debug(
        STEP_FIXTURE,
        "fixture.qn",
        &[("fixture.qn", 2)],
        &[DebugAction::StepOut, DebugAction::Continue],
    );
    assert_eq!(lines(&log), vec![2, 6]);
}

/// Break-on-throw pauses at the throw site with the throwing frame's stack still live.
/// The fixture throws `MessageNotUnderstood` on line 2 (inside a block run by `.catch:`),
/// caught on line 3. The pause must be logged at line 2 — the *failing* instruction — not
/// the block-epilogue line the throwing frame's already-advanced `ip` points at (the
/// `frame_display_ip` `ip - 1` adjustment). Hierarchy matching: `Error` matches the MNU
/// subclass. After `$continue`, the catch handler runs and the program completes.
#[test]
fn break_on_throw_pauses_at_the_throw_site() {
    let source = "\
r = {
    nil.bogusMethod
}.catch:{ |e| 0 };
r
";
    let log = run_debug_full(
        source,
        "fixture.qn",
        &[],
        &["Error"],
        &[],
        &[DebugAction::Continue],
    );
    assert_eq!(log, vec![("fixture.qn".to_string(), 2)]);
}

/// A break-on-throw type that doesn't match the thrown exception's class (nor any ancestor)
/// never pauses — the throw propagates to its `catch:` untouched.
#[test]
fn break_on_throw_ignores_a_non_matching_type() {
    let source = "\
r = {
    nil.bogusMethod
}.catch:{ |e| 0 };
r
";
    let log = run_debug_full(source, "fixture.qn", &[], &["TypeError"], &[], &[]);
    assert!(log.is_empty());
}

#[test]
fn parse_debug_break_on_throw_flag() {
    let args: Vec<String> = [
        "qn",
        "debug",
        "--break-on-throw=TypeError, Error",
        "file.qn",
        "extra",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let opts = VmRunnerOptions::parse(&args);
    assert!(matches!(opts.mode, VmRunnerMode::Debug));
    assert_eq!(opts.target_path.as_deref(), Some("file.qn"));
    // Comma-separated, whitespace-trimmed; the flag is consumed (not treated as the file).
    assert_eq!(
        opts.break_on_throw,
        vec!["TypeError".to_string(), "Error".to_string()]
    );
    assert_eq!(opts.vm_options.arguments, vec!["extra".to_string()]);
}

#[test]
fn parse_debug_without_break_flag_has_no_break_on_throw() {
    let args: Vec<String> = ["qn", "debug", "file.qn"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let opts = VmRunnerOptions::parse(&args);
    assert!(matches!(opts.mode, VmRunnerMode::Debug));
    assert_eq!(opts.target_path.as_deref(), Some("file.qn"));
    assert!(opts.break_on_throw.is_empty());
}

/// `--break-on-uncaught` pauses when a matching exception will NOT be caught: here the typed
/// `catch:{ |e:ValueError| … }` doesn't match the thrown `MessageNotUnderstood`, so it re-raises.
#[test]
fn break_on_uncaught_pauses_when_no_handler_matches() {
    let source = "\
r = {
    nil.bogusMethod
}.catch:{ |e:ValueError| 0 };
r
";
    let log = run_debug_uncaught(source, &["MessageNotUnderstood"], &[DebugAction::Continue]);
    assert_eq!(log, vec![("fixture.qn".to_string(), 2)]);
}

/// A matching enclosing handler means the exception is caught, so `--break-on-uncaught` stays
/// silent — `catch:{ |e:Error| … }` catches the `MessageNotUnderstood`.
#[test]
fn break_on_uncaught_skips_a_caught_exception() {
    let source = "\
r = {
    nil.bogusMethod
}.catch:{ |e:Error| 0 };
r
";
    let log = run_debug_uncaught(source, &["MessageNotUnderstood"], &[DebugAction::Continue]);
    assert!(log.is_empty(), "caught exception must not pause: {log:?}");
}

/// An uncaught exception that bubbles through several non-matching `catch:`es pauses exactly
/// once — at the innermost throw site — not again at each catch it re-raises past.
#[test]
fn break_on_uncaught_fires_once_for_a_bubbling_throw() {
    let source = "\
r = {
    { nil.bogusMethod }.catch:{ |e:ValueError| 1 }
}.catch:{ |e:ArgumentError| 2 };
r
";
    let log = run_debug_uncaught(source, &["MessageNotUnderstood"], &[DebugAction::Continue]);
    assert_eq!(log, vec![("fixture.qn".to_string(), 2)]);
}

/// The uncaught search spans the whole enclosing-catch stack: an outer `catch:{ |e:Error| … }`
/// catches the `MessageNotUnderstood` the inner `ValueError` handler missed, so no pause.
#[test]
fn break_on_uncaught_sees_an_outer_matching_handler() {
    let source = "\
r = {
    { nil.bogusMethod }.catch:{ |e:ValueError| 1 }
}.catch:{ |e:Error| 2 };
r
";
    let log = run_debug_uncaught(source, &["MessageNotUnderstood"], &[]);
    assert!(log.is_empty(), "outer handler catches it: {log:?}");
}

#[test]
fn parse_debug_break_on_uncaught_flag() {
    let args: Vec<String> = [
        "qn",
        "debug",
        "--break-on-uncaught=TypeError, ValueError",
        "f.qn",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let opts = VmRunnerOptions::parse(&args);
    assert!(matches!(opts.mode, VmRunnerMode::Debug));
    assert_eq!(
        opts.break_on_uncaught,
        vec!["TypeError".to_string(), "ValueError".to_string()]
    );
    assert!(opts.break_on_throw.is_empty());
}

#[test]
fn flow_names_wraps_to_width() {
    let names = vec!["aaa", "bbb", "ccc", "ddd"];
    // Width 12: "  aaa  bbb" is 10 cols; adding "  ccc" (→15) overflows, so it wraps.
    assert_eq!(flow_names(&names, 12), "  aaa  bbb\n  ccc  ddd\n");
    // A wide width keeps everything on one line.
    assert_eq!(flow_names(&names, 80), "  aaa  bbb  ccc  ddd\n");
    // No line carries trailing whitespace, at any width.
    for w in [8, 12, 40, 80] {
        assert!(flow_names(&names, w).lines().all(|l| l == l.trim_end()));
    }
}
