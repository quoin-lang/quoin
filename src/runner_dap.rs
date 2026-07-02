//! The Debug Adapter Protocol (DAP) frontend for `qn debug --dap`: a `DriverFrontend` that speaks
//! DAP over a `crate::dap::Connection`, plus its request helpers. Split out of `runner.rs`.

use super::runner_driver::{DebugFlow, DriverFrontend};
use super::*;
/// Map a DAP wire I/O error into the driver's error type.
fn dap_io(e: std::io::Error) -> QuoinError {
    QuoinError::Other(format!("DAP I/O: {e}"))
}

/// Best-effort `stopped` reason from the paused debug state.
fn dap_stop_reason(vm: &VmState<'_>) -> &'static str {
    let Some(d) = vm.instrumentation.debug.as_ref() else {
        return "pause";
    };
    if d.at_throw {
        return "exception";
    }
    if let Some((file, line)) = vm.debug_current_pos()
        && d.breakpoints
            .get(&file)
            .is_some_and(|ls| ls.contains(&line))
    {
        return "breakpoint";
    }
    if d.step.is_some() {
        return "step";
    }
    "entry"
}

/// Apply a `setBreakpoints` request to the debug state and build its response body. Breakpoints
/// match by line at the step hook (no PC index), so every requested line is reported `verified`.
/// DAP gives the full set for a source per call, so existing lines for that file are replaced.
fn dap_set_breakpoints(arena: &mut ReplArena, args: &serde_json::Value) -> serde_json::Value {
    use serde_json::json;
    let path = args
        .get("source")
        .and_then(|s| s.get("path"))
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    let lines: Vec<usize> = args
        .get("breakpoints")
        .and_then(|b| b.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|bp| bp.get("line").and_then(|l| l.as_u64()).map(|l| l as usize))
                .collect()
        })
        .unwrap_or_default();
    arena.mutate_root(|_mc, vm| {
        if let Some(d) = vm.instrumentation.debug.as_mut() {
            let set = d.breakpoints.entry(path.clone()).or_default();
            set.clear();
            set.extend(lines.iter().copied());
        }
    });
    json!({
        "breakpoints": lines.iter().map(|&l| json!({ "verified": true, "line": l })).collect::<Vec<_>>(),
    })
}

/// One synthetic DAP thread — the VM is "pause the world", so there is exactly one.
fn dap_threads() -> serde_json::Value {
    serde_json::json!({ "threads": [{ "id": 1, "name": "main" }] })
}

/// The DAP `stackFrames` for the paused VM, innermost frame first. A frame's `id` is its stack
/// index, which `scopes`/`evaluate` use to target it.
fn dap_stack_frames(vm: &VmState<'_>) -> Vec<serde_json::Value> {
    use serde_json::json;
    (0..vm.frames.len())
        .rev()
        .map(|i| match vm.debug_frame_location(i) {
            Some((file, line, label)) => json!({
                "id": i,
                "name": label,
                "line": line,
                "column": 1,
                "source": { "path": file, "name": file },
            }),
            None => json!({ "id": i, "name": "<no source>", "line": 0, "column": 0 }),
        })
        .collect()
}

/// Program-loading context for a DAP session whose program path arrives in the `launch` request
/// (`qn debug --dap` with no file argument) rather than being installed eagerly from a CLI path.
pub(crate) struct PendingProgram {
    pub(crate) break_on_throw: Vec<String>,
    pub(crate) break_on_uncaught: Vec<String>,
}

/// A `variablesReference` target: a frame's `Locals` scope, or a specific value reached from a
/// frame's locals by a child-index `path`. The live value is re-fetched each `variables` request
/// (so no `Value` is held across the pause); handles are minted lazily as the client expands the
/// tree and cleared at each stop.
#[derive(Clone)]
enum VarRef {
    Scope { frame: usize },
    Value { frame: usize, path: Vec<usize> },
}

/// The DAP adapter frontend: translates the driver's debug touchpoints to/from the Debug Adapter
/// Protocol over its [`Connection`](crate::dap::Connection). `configure` runs the handshake +
/// breakpoint setup through `configurationDone`; `on_output` flushes program output as `output`
/// events; `on_pause` emits `stopped` and services requests until the client resumes/disconnects.
/// Generic over the streams so tests can drive it over in-memory buffers.
pub(crate) struct DapFrontend<R: std::io::BufRead, W: std::io::Write> {
    pub(crate) conn: crate::dap::Connection<R, W>,
    /// Per-pause `variablesReference` table: handle (1-based) -> what it expands (a frame scope or
    /// a nested value). Cleared at each pause (a DAP handle is valid only for the current stop).
    handles: Vec<VarRef>,
    /// `Some` until the program is installed from the `launch` request; `None` when the program was
    /// already installed eagerly (CLI path, or the test harness).
    pending: Option<PendingProgram>,
}

impl<R: std::io::BufRead, W: std::io::Write> DapFrontend<R, W> {
    /// The program is already installed; the `launch` request only carries `stopOnEntry`.
    pub(crate) fn new(conn: crate::dap::Connection<R, W>) -> Self {
        Self {
            conn,
            handles: Vec::new(),
            pending: None,
        }
    }

    /// The program path is supplied in the `launch` request and installed there.
    pub(crate) fn with_pending(
        conn: crate::dap::Connection<R, W>,
        pending: PendingProgram,
    ) -> Self {
        Self {
            conn,
            handles: Vec::new(),
            pending: Some(pending),
        }
    }
}

impl<R: std::io::BufRead, W: std::io::Write> DriverFrontend for DapFrontend<R, W> {
    fn configure(&mut self, arena: &mut ReplArena) -> Result<bool, QuoinError> {
        use serde_json::json;
        loop {
            let Some(req) = self.conn.read_request().map_err(dap_io)? else {
                return Ok(false); // client disconnected before running
            };
            match req.command.as_str() {
                "initialize" => {
                    self.conn
                        .ok(
                            &req,
                            Some(json!({ "supportsConfigurationDoneRequest": true })),
                        )
                        .map_err(dap_io)?;
                    self.conn.event("initialized", None).map_err(dap_io)?;
                }
                "launch" => {
                    // If the program wasn't installed eagerly from a CLI path, load it now from the
                    // launch request. The IDE/DAP client carries the path under `program` (we also
                    // accept `file`/`path`). Install/parse errors fail the launch with a message
                    // rather than killing the adapter, so the IDE shows what went wrong.
                    if let Some(pending) = self.pending.take() {
                        let prog = req
                            .arguments
                            .get("program")
                            .or_else(|| req.arguments.get("file"))
                            .or_else(|| req.arguments.get("path"))
                            .and_then(|v| v.as_str());
                        let Some(prog) = prog else {
                            self.conn
                                .fail(
                                    &req,
                                    "launch: no program path — expected `program` (or `file`/`path`) in the launch arguments".to_string(),
                                )
                                .map_err(dap_io)?;
                            return Ok(false);
                        };
                        if let Err(e) = install_dap_program(
                            arena,
                            prog,
                            &pending.break_on_throw,
                            &pending.break_on_uncaught,
                        ) {
                            self.conn
                                .fail(&req, format!("launch failed: {e}"))
                                .map_err(dap_io)?;
                            return Ok(false);
                        }
                    }
                    let stop_on_entry = req
                        .arguments
                        .get("stopOnEntry")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if stop_on_entry {
                        arena.mutate_root(|_mc, vm| {
                            if let Some(d) = vm.instrumentation.debug.as_mut() {
                                d.step = Some(crate::debug::StepMode::Into);
                            }
                        });
                    }
                    self.conn.ok(&req, None).map_err(dap_io)?;
                }
                "setBreakpoints" => {
                    let body = dap_set_breakpoints(arena, &req.arguments);
                    self.conn.ok(&req, Some(body)).map_err(dap_io)?;
                }
                "setExceptionBreakpoints" => {
                    self.conn.ok(&req, None).map_err(dap_io)?;
                }
                "threads" => {
                    self.conn.ok(&req, Some(dap_threads())).map_err(dap_io)?;
                }
                "configurationDone" => {
                    self.conn.ok(&req, None).map_err(dap_io)?;
                    return Ok(true); // begin running
                }
                "disconnect" | "terminate" => {
                    self.conn.ok(&req, None).map_err(dap_io)?;
                    return Ok(false);
                }
                other => {
                    self.conn
                        .fail(&req, format!("unsupported request: {other}"))
                        .map_err(dap_io)?;
                }
            }
        }
    }

    fn on_output(&mut self, arena: &mut ReplArena) -> Result<(), QuoinError> {
        use serde_json::json;
        let chunks = arena.mutate_root(|_mc, vm| vm.take_program_output());
        for chunk in chunks {
            let category = match chunk.stream {
                crate::vm::StdStream::Out => "stdout",
                crate::vm::StdStream::Err => "stderr",
            };
            let text = String::from_utf8_lossy(&chunk.bytes).into_owned();
            self.conn
                .event(
                    "output",
                    Some(json!({ "category": category, "output": text })),
                )
                .map_err(dap_io)?;
        }
        Ok(())
    }

    fn on_pause(&mut self, arena: &mut ReplArena) -> Result<DebugFlow, QuoinError> {
        use serde_json::json;
        self.handles.clear(); // variablesReference handles are valid only for this stop
        self.on_output(arena)?; // flush output before the stop, so console ordering is right
        let reason = arena.mutate_root(|_mc, vm| dap_stop_reason(vm));
        self.conn
            .event(
                "stopped",
                Some(json!({ "reason": reason, "threadId": 1, "allThreadsStopped": true })),
            )
            .map_err(dap_io)?;
        loop {
            let Some(req) = self.conn.read_request().map_err(dap_io)? else {
                return Ok(DebugFlow::Quit);
            };
            let action = match req.command.as_str() {
                "continue" => Some(crate::debug::DebugAction::Continue),
                "next" => Some(crate::debug::DebugAction::StepOver),
                "stepIn" => Some(crate::debug::DebugAction::StepInto),
                "stepOut" => Some(crate::debug::DebugAction::StepOut),
                _ => None,
            };
            if let Some(act) = action {
                arena.mutate_root(|_mc, vm| vm.apply_debug_action(act));
                let body =
                    (req.command == "continue").then(|| json!({ "allThreadsContinued": true }));
                self.conn.ok(&req, body).map_err(dap_io)?;
                return Ok(DebugFlow::Resume);
            }
            match req.command.as_str() {
                "threads" => self.conn.ok(&req, Some(dap_threads())).map_err(dap_io)?,
                "stackTrace" => {
                    let frames = arena.mutate_root(|_mc, vm| dap_stack_frames(vm));
                    let total = frames.len();
                    self.conn
                        .ok(
                            &req,
                            Some(json!({ "stackFrames": frames, "totalFrames": total })),
                        )
                        .map_err(dap_io)?;
                }
                "scopes" => {
                    let frame = req
                        .arguments
                        .get("frameId")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    self.handles.push(VarRef::Scope { frame });
                    let var_ref = self.handles.len(); // 1-based handle
                    self.conn
                        .ok(
                            &req,
                            Some(json!({
                                "scopes": [{
                                    "name": "Locals",
                                    "variablesReference": var_ref,
                                    "expensive": false,
                                }]
                            })),
                        )
                        .map_err(dap_io)?;
                }
                "variables" => {
                    let var_ref = req
                        .arguments
                        .get("variablesReference")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    let vars = match self.handles.get(var_ref.wrapping_sub(1)).cloned() {
                        Some(target) => {
                            let (frame, path) = match target {
                                VarRef::Scope { frame } => (frame, Vec::new()),
                                VarRef::Value { frame, path } => (frame, path),
                            };
                            let rows =
                                arena.mutate_root(|_mc, vm| vm.debug_variables(frame, &path));
                            // Mint a child handle for each expandable row so the client can expand
                            // it; `i` is the child index used to re-fetch it (path + [i]).
                            rows.into_iter()
                                .enumerate()
                                .map(|(i, (name, value, expandable))| {
                                    let child_ref = if expandable {
                                        let mut child_path = path.clone();
                                        child_path.push(i);
                                        self.handles.push(VarRef::Value {
                                            frame,
                                            path: child_path,
                                        });
                                        self.handles.len() // 1-based handle
                                    } else {
                                        0
                                    };
                                    json!({
                                        "name": name,
                                        "value": value,
                                        "variablesReference": child_ref,
                                    })
                                })
                                .collect::<Vec<_>>()
                        }
                        None => Vec::new(),
                    };
                    self.conn
                        .ok(&req, Some(json!({ "variables": vars })))
                        .map_err(dap_io)?;
                }
                "evaluate" => {
                    let expr = req
                        .arguments
                        .get("expression")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let frame = req
                        .arguments
                        .get("frameId")
                        .and_then(|v| v.as_u64())
                        .map(|f| f as usize);
                    let result = arena.mutate_root(|mc, vm| {
                        let idx = frame.unwrap_or_else(|| vm.frames.len().saturating_sub(1));
                        let self_val = vm.frames.get(idx).and_then(|f| f.receiver);
                        let bindings = vm.debug_frame_bindings(idx);
                        vm.debug_eval(mc, &expr, self_val, &bindings)
                            .map(|v| vm.debug_render(v))
                    });
                    match result {
                        Ok(rendered) => self
                            .conn
                            .ok(
                                &req,
                                Some(json!({ "result": rendered, "variablesReference": 0 })),
                            )
                            .map_err(dap_io)?,
                        Err(msg) => self.conn.fail(&req, msg).map_err(dap_io)?,
                    }
                }
                "disconnect" | "terminate" => {
                    self.conn.ok(&req, None).map_err(dap_io)?;
                    return Ok(DebugFlow::Quit);
                }
                other => self
                    .conn
                    .fail(&req, format!("unsupported request while paused: {other}"))
                    .map_err(dap_io)?,
            }
        }
    }

    fn on_finished(
        &mut self,
        arena: &mut ReplArena,
        err: Option<&QuoinError>,
    ) -> Result<(), QuoinError> {
        use serde_json::json;
        self.on_output(arena)?;
        if let Some(e) = err {
            self.conn
                .event(
                    "output",
                    Some(json!({ "category": "stderr", "output": format!("{e}\n") })),
                )
                .map_err(dap_io)?;
        }
        self.conn.event("terminated", None).map_err(dap_io)?;
        self.conn
            .event(
                "exited",
                Some(json!({ "exitCode": i32::from(err.is_some()) })),
            )
            .map_err(dap_io)?;
        Ok(())
    }
}
