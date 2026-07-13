//! `[OS]Process` — subprocesses on the scheduler. A one-shot `run` parks the calling
//! task (other tasks keep running) for the child's whole lifecycle; `start` spawns for
//! streaming and answers a handle whose pipes read/write like sockets.
//!
//! The ergonomic selector family (`run:`, `run:input:`, `run:env:dir:`, …) and the
//! ProcessResult wrapper live in Quoin (`qnlib/core/12-os.qn`); the two `prim…` class
//! methods here are the nil-tolerant kitchen sinks they delegate to.
//!
//! Lifecycle: a `run` child dies with its op (kill-on-drop — an `Async.timeout:`
//! cancelling the park kills the child, never leaks it). A `start` child is owned by
//! its handle: collected undetached → killed via the child-reap queue; VM teardown
//! kills undetached survivors; `detach` opts out of all of it.

use crate::error::QuoinError;
use crate::io_backend::{IoBackend, IoRequest, IoResult, StreamId};
use crate::runtime::pretty::{PpChild, PpRole, PpShape, PrettyPrint};
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
use gc_arena::collect::Trace;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::rc::Rc;

// POSIX-fixed signal numbers (the two the surface exposes).
const SIGKILL: i32 = 9;
const SIGTERM: i32 = 15;

/// Native backing state for a `[OS]Process` handle. Holds only ids (the `Child` and
/// the pipe fds live in the backend registry) plus clones of the reap queues. No `Gc`
/// fields. Collection of an undetached handle kills the child (the backstop); the
/// held pipe ends are reaped either way, exactly like a socket's fd.
pub struct NativeProcess {
    child_id: u64,
    pid: u32,
    /// `None` once closed (EOF delivered to the child).
    stdin_id: Cell<Option<StreamId>>,
    /// `None` once taken by a stream mint — a pipe reads through ONE stream.
    stdout_id: Cell<Option<StreamId>>,
    stderr_id: Cell<Option<StreamId>>,
    /// `(code, signal)` once a `wait` observed the exit.
    exit: Cell<Option<(Option<i32>, Option<i32>)>>,
    detached: Cell<bool>,
    child_reap: Rc<RefCell<Vec<u64>>>,
    socket_reap: Rc<RefCell<Vec<StreamId>>>,
}

impl Drop for NativeProcess {
    fn drop(&mut self) {
        // The pipe ends this handle still holds close like any socket fd — a
        // detached child that keeps writing sees EPIPE, which is the honest
        // consequence of detaching without draining (redirect or drain first).
        let mut sockets = self.socket_reap.borrow_mut();
        for id in [
            self.stdin_id.take(),
            self.stdout_id.take(),
            self.stderr_id.take(),
        ]
        .into_iter()
        .flatten()
        {
            sockets.push(id);
        }
        drop(sockets);
        if !self.detached.get() {
            self.child_reap.borrow_mut().push(self.child_id);
        }
    }
}

impl std::fmt::Debug for NativeProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NativeProcess{{pid:{}}}", self.pid)
    }
}

impl AnyCollect for NativeProcess {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {} // no Gc fields
}

impl PrettyPrint for NativeProcess {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        let mut fields = vec![("pid".to_string(), PpChild::Val(Value::Int(self.pid as i64)))];
        if let Some((code, signal)) = self.exit.get() {
            match (code, signal) {
                (Some(c), _) => {
                    fields.push(("exitCode".to_string(), PpChild::Val(Value::Int(c as i64))))
                }
                (None, Some(s)) => {
                    fields.push(("signal".to_string(), PpChild::Val(Value::Int(s as i64))))
                }
                _ => {}
            }
        } else if self.detached.get() {
            fields.push((
                "detached".to_string(),
                PpChild::Text("true".to_string(), PpRole::Str),
            ));
        }
        PpShape::Record {
            name: "Process",
            fields,
        }
    }
}

/// The argv a `run`/`start` first argument denotes: a List of Strings (program +
/// args), or a bare String (program alone — argument SPLITTING is shell territory,
/// deliberately absent).
fn argv_of(v: Value, who: &str) -> Result<(OsString, Vec<OsString>), QuoinError> {
    let type_err = |got: &str| QuoinError::TypeError {
        expected: "String or List(String)".to_string(),
        got: got.to_string(),
        msg: format!(
            "{who} takes the command as a List (program + arguments) or a bare String (program \
             alone — there is no shell, so nothing splits)"
        ),
    };
    if let Value::Object(o) = v
        && let ObjectPayload::String(s) = &o.borrow().payload
    {
        return Ok((OsString::from(&**s), Vec::new()));
    }
    let strings = v
        .with_native_state::<crate::runtime::list::NativeListState, _, _>(|l| {
            l.get_vec()
                .iter()
                .map(|e| {
                    if let Value::Object(o) = e
                        && let ObjectPayload::String(s) = &o.borrow().payload
                    {
                        return Some(OsString::from(&**s));
                    }
                    None
                })
                .collect::<Option<Vec<OsString>>>()
        })
        .map_err(|_| type_err(v.type_name()))?
        .ok_or_else(|| type_err("a List with a non-String element"))?;
    let mut it = strings.into_iter();
    match it.next() {
        Some(program) => Ok((program, it.collect())),
        None => Err(QuoinError::ValueError(format!(
            "{who}: the command list is empty (no program to run)"
        ))),
    }
}

/// The env option: nil (inherit unchanged) or a Map of String → String set ON TOP of
/// the inherited environment.
fn env_of(v: Value, who: &str) -> Result<Option<Vec<(OsString, OsString)>>, QuoinError> {
    if v.is_nil() {
        return Ok(None);
    }
    let pairs = v
        .with_native_state::<crate::runtime::map::NativeMapState, _, _>(|m| {
            m.entries()
                .iter()
                .map(|(_, k, val)| {
                    let k = string_of(*k)?;
                    let val = string_of(*val)?;
                    Some((OsString::from(k), OsString::from(val)))
                })
                .collect::<Option<Vec<_>>>()
        })
        .map_err(|_| QuoinError::TypeError {
            expected: "Map".to_string(),
            got: v.type_name().to_string(),
            msg: format!("{who} env: takes a Map of String → String (or nil to inherit)"),
        })?;
    match pairs {
        Some(p) => Ok(Some(p)),
        None => Err(QuoinError::TypeError {
            expected: "String keys and values".to_string(),
            got: "a non-String entry".to_string(),
            msg: format!("{who} env: takes a Map of String → String"),
        }),
    }
}

fn string_of(v: Value) -> Option<String> {
    if let Value::Object(o) = v
        && let ObjectPayload::String(s) = &o.borrow().payload
    {
        return Some(s.to_string());
    }
    None
}

/// The dir option: nil or a String path.
fn dir_of(v: Value, who: &str) -> Result<Option<OsString>, QuoinError> {
    if v.is_nil() {
        return Ok(None);
    }
    string_of(v)
        .map(|s| Some(OsString::from(s)))
        .ok_or_else(|| QuoinError::TypeError {
            expected: "String".to_string(),
            got: v.type_name().to_string(),
            msg: format!("{who} dir: takes a String path (or nil)"),
        })
}

/// The input option: nil, a String (UTF-8 bytes), or Bytes.
fn input_of(v: Value, who: &str) -> Result<Option<Vec<u8>>, QuoinError> {
    if v.is_nil() {
        return Ok(None);
    }
    if let Value::Object(o) = v {
        match &o.borrow().payload {
            ObjectPayload::String(s) => return Ok(Some(s.as_bytes().to_vec())),
            ObjectPayload::Bytes(b) => return Ok(Some(b.to_vec())),
            _ => {}
        }
    }
    Err(QuoinError::TypeError {
        expected: "String or Bytes".to_string(),
        got: v.type_name().to_string(),
        msg: format!("{who} input: takes a String or Bytes (or nil)"),
    })
}

fn make_process<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    child: u64,
    pid: u32,
    stdin: StreamId,
    stdout: StreamId,
    stderr: StreamId,
) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "[OS]Process");
    vm.new_native_state_boxed(
        mc,
        class,
        Box::new(NativeProcess {
            child_id: child,
            pid,
            stdin_id: Cell::new(Some(stdin)),
            stdout_id: Cell::new(Some(stdout)),
            stderr_id: Cell::new(Some(stderr)),
            exit: Cell::new(None),
            detached: Cell::new(false),
            child_reap: Rc::clone(&vm.io.child_reap),
            socket_reap: Rc::clone(&vm.io.socket_reap),
        }),
    )
}

/// Read a field of the handle's state.
fn with_proc<'gc, R>(
    v: Value<'gc>,
    who: &str,
    f: impl FnOnce(&NativeProcess) -> R,
) -> Result<R, QuoinError> {
    v.with_native_state::<NativeProcess, _, _>(f)
        .map_err(|_| QuoinError::TypeError {
            expected: "Process".to_string(),
            got: "a non-Process value".to_string(),
            msg: format!("{who} requires a Process receiver"),
        })
}

pub fn build_process_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[OS]Process", Some("Object"))
        .construct_with("use [OS]Process.run: / [OS]Process.start:")
        .class_doc(
            "Subprocesses, on the scheduler: `run:` parks the calling task (other tasks \
             keep running) until the child exits, answering a ProcessResult; `start:` \
             spawns for streaming and answers this handle — read `stdout`/`stderr` like a \
             socket, write with `writeStdin:`/`closeStdin`, `wait`/`kill`/`terminate` it. \
             The command is a List (program + arguments) — there is NO shell, so nothing \
             splits, globs, or injects. An undetached child dies with its handle (and a \
             cancelled `run:` kills its child); `detach` opts out.\n\n\
             ```\n\
             ([OS]Process.run:#( 'echo' 'hi' )).stdout    \"* -> 'hi\\n'\n\
             ```",
        )
        .class_method("primRun:env:dir:input:", |vm, mc, _r, args| {
            let (program, argv) = argv_of(args[0], "run:")?;
            let env = env_of(args[1], "run:")?;
            let dir = dir_of(args[2], "run:")?;
            let input = input_of(args[3], "run:")?;
            match vm.await_io(IoRequest::RunProcess {
                program,
                args: argv,
                env,
                dir,
                input,
            })? {
                IoResult::ProcDone {
                    code,
                    signal,
                    stdout,
                    stderr,
                } => {
                    let stdout = vm.new_bytes(mc, stdout);
                    let stderr = vm.new_bytes(mc, stderr);
                    let code_v = match code {
                        Some(c) => vm.new_int(mc, c as i64),
                        None => vm.new_nil(mc),
                    };
                    let signal_v = match signal {
                        Some(s) => vm.new_int(mc, s as i64),
                        None => vm.new_nil(mc),
                    };
                    Ok(vm.new_map(
                        mc,
                        vec![
                            ("stdout".to_string(), stdout),
                            ("stderr".to_string(), stderr),
                            ("exitCode".to_string(), code_v),
                            ("signal".to_string(), signal_v),
                        ],
                    ))
                }
                IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
                other => Err(QuoinError::Other(format!(
                    "run:: unexpected io result {other:?}"
                ))),
            }
        })
        .doc(
            "Internal: the nil-tolerant kitchen sink behind the `run:` family (see \
             `run:`, `run:input:`, `run:env:`, `run:dir:`, `run:input:env:dir:`), \
             answering the raw result Map the ProcessResult wraps.",
        )
        .class_method("primStart:env:dir:", |vm, mc, _r, args| {
            let (program, argv) = argv_of(args[0], "start:")?;
            let env = env_of(args[1], "start:")?;
            let dir = dir_of(args[2], "start:")?;
            match vm.await_io(IoRequest::SpawnProcess {
                program,
                args: argv,
                env,
                dir,
            })? {
                IoResult::ProcSpawned {
                    child,
                    pid,
                    stdin,
                    stdout,
                    stderr,
                } => Ok(make_process(vm, mc, child, pid, stdin, stdout, stderr)),
                IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
                other => Err(QuoinError::Other(format!(
                    "start:: unexpected io result {other:?}"
                ))),
            }
        })
        .doc(
            "Internal: the nil-tolerant kitchen sink behind the `start:` family (see \
             `start:`, `start:env:`, `start:dir:`, `start:env:dir:`).",
        )
        .instance_method("wait", |vm, mc, receiver, _args| {
            let (cached, child_id) = with_proc(receiver, "wait", |p| (p.exit.get(), p.child_id))?;
            let (code, signal) = match cached {
                Some(pair) => pair,
                None => match vm.await_io(IoRequest::ChildWait { id: child_id })? {
                    IoResult::ProcExited { code, signal } => {
                        with_proc(receiver, "wait", |p| p.exit.set(Some((code, signal))))?;
                        (code, signal)
                    }
                    IoResult::Err(e) => return Err(QuoinError::from_io_error(&e)),
                    other => {
                        return Err(QuoinError::Other(format!(
                            "wait: unexpected io result {other:?}"
                        )));
                    }
                },
            };
            let _ = signal;
            Ok(match code {
                Some(c) => vm.new_int(mc, c as i64),
                None => vm.new_nil(mc),
            })
        })
        .returns("Integer?")
        .doc(
            "Park until the child exits (idempotent once it has); answers the exit code, \
             or nil when a signal ended it (`signal` says which). One task waits at a \
             time — a concurrent second wait throws.",
        )
        .instance_method("exitCode", |vm, mc, receiver, _args| {
            let cached = with_proc(receiver, "exitCode", |p| p.exit.get())?;
            Ok(match cached {
                Some((Some(c), _)) => vm.new_int(mc, c as i64),
                _ => vm.new_nil(mc),
            })
        })
        .returns("Integer?")
        .doc("The exit code a completed `wait` observed — nil before the wait, and nil for a signal-terminated child.")
        .instance_method("signal", |vm, mc, receiver, _args| {
            let cached = with_proc(receiver, "signal", |p| p.exit.get())?;
            Ok(match cached {
                Some((_, Some(s))) => vm.new_int(mc, s as i64),
                _ => vm.new_nil(mc),
            })
        })
        .returns("Integer?")
        .doc("The signal that ended the child, per a completed `wait` — nil otherwise.")
        .instance_method("pid", |vm, mc, receiver, _args| {
            let pid = with_proc(receiver, "pid", |p| p.pid)?;
            Ok(vm.new_int(mc, pid as i64))
        })
        .returns("Integer")
        .doc("The operating-system process id.")
        .instance_method("running?", |vm, mc, receiver, _args| {
            let child_id = with_proc(receiver, "running?", |p| p.child_id)?;
            Ok(vm.new_bool(mc, vm.io.backend.child_running(child_id)))
        })
        .returns("Boolean")
        .doc("Whether the child is still running — exact (a non-blocking status probe), not a pid guess.")
        .instance_method("kill", |vm, mc, receiver, _args| {
            let child_id = with_proc(receiver, "kill", |p| p.child_id)?;
            vm.io
                .backend
                .child_signal(child_id, SIGKILL)
                .map_err(|e| QuoinError::from_io_error(&e))?;
            let _ = mc;
            Ok(receiver)
        })
        .doc(
            "SIGKILL the child (unblockable). A parked `wait` resolves with the signal \
             exit. A no-op once the child has exited; answers the receiver.",
        )
        .instance_method("terminate", |vm, mc, receiver, _args| {
            let child_id = with_proc(receiver, "terminate", |p| p.child_id)?;
            vm.io
                .backend
                .child_signal(child_id, SIGTERM)
                .map_err(|e| QuoinError::from_io_error(&e))?;
            let _ = mc;
            Ok(receiver)
        })
        .doc(
            "SIGTERM the child — the polite `kill` (the child may catch it to shut down \
             cleanly, or ignore it). Answers the receiver.",
        )
        .instance_method("detach", |vm, mc, receiver, _args| {
            let child_id = with_proc(receiver, "detach", |p| {
                p.detached.set(true);
                p.child_id
            })?;
            vm.io.backend.child_detach(child_id);
            let _ = mc;
            Ok(receiver)
        })
        .doc(
            "Let the child outlive this handle AND the VM (neither collection nor exit \
             kills it). Its pipes still close when the handle goes — a detached child \
             that keeps writing gets EPIPE, so drain or redirect first. Answers the \
             receiver.",
        )
        .instance_method("stdout", |vm, mc, receiver, _args| {
            let id = with_proc(receiver, "stdout", |p| p.stdout_id.take())?;
            match id {
                Some(id) => Ok(crate::runtime::streams::make_byte_stream(vm, mc, id)),
                None => Err(QuoinError::ValueError(
                    "stdout: this pipe's stream was already taken".to_string(),
                )),
            }
        })
        .returns("ByteStream")
        .doc(
            "The child's standard output as a ByteStream (like a socket's). One stream \
             per pipe: a second take throws. Reading only stdout while the child floods \
             stderr can deadlock on the pipe buffer — drain both (two tasks), or use \
             `run:`, which does.",
        )
        .instance_method("stderr", |vm, mc, receiver, _args| {
            let id = with_proc(receiver, "stderr", |p| p.stderr_id.take())?;
            match id {
                Some(id) => Ok(crate::runtime::streams::make_byte_stream(vm, mc, id)),
                None => Err(QuoinError::ValueError(
                    "stderr: this pipe's stream was already taken".to_string(),
                )),
            }
        })
        .returns("ByteStream")
        .doc("The child's standard error as a ByteStream — see `stdout` for the one-stream-per-pipe rule and the two-pipe deadlock note.")
        .instance_method("stdoutText", |vm, mc, receiver, _args| {
            let id = with_proc(receiver, "stdoutText", |p| p.stdout_id.take())?;
            match id {
                Some(id) => Ok(crate::runtime::streams::make_string_stream(
                    vm,
                    mc,
                    id,
                    Vec::new(),
                )),
                None => Err(QuoinError::ValueError(
                    "stdoutText: this pipe's stream was already taken".to_string(),
                )),
            }
        })
        .returns("StringStream")
        .doc(
            "The child's standard output as a StringStream (`readLine`/`eachLine:`). \
             Same one-stream-per-pipe rule as `stdout`.\n\n\
             ```\n\
             var p = [OS]Process.start:#( 'printf' 'a\\nb\\n' )\n\
             p.stdoutText.readLine     \"* -> 'a'\n\
             ```",
        )
        .instance_method("stderrText", |vm, mc, receiver, _args| {
            let id = with_proc(receiver, "stderrText", |p| p.stderr_id.take())?;
            match id {
                Some(id) => Ok(crate::runtime::streams::make_string_stream(
                    vm,
                    mc,
                    id,
                    Vec::new(),
                )),
                None => Err(QuoinError::ValueError(
                    "stderrText: this pipe's stream was already taken".to_string(),
                )),
            }
        })
        .returns("StringStream")
        .doc("The child's standard error as a StringStream — see `stdoutText`.")
        .instance_method("writeStdin:", |vm, mc, receiver, args| {
            let bytes = match input_of(args[0], "writeStdin:")? {
                Some(b) => b,
                None => {
                    return Err(QuoinError::TypeError {
                        expected: "String or Bytes".to_string(),
                        got: "Nil".to_string(),
                        msg: "writeStdin: takes a String or Bytes".to_string(),
                    });
                }
            };
            let id = with_proc(receiver, "writeStdin:", |p| p.stdin_id.get())?;
            let Some(id) = id else {
                return Err(QuoinError::ValueError(
                    "writeStdin:: stdin is closed".to_string(),
                ));
            };
            match vm.await_io(IoRequest::Write { id, bytes })? {
                IoResult::Wrote(_) => {
                    let _ = mc;
                    Ok(receiver)
                }
                IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
                other => Err(QuoinError::Other(format!(
                    "writeStdin:: unexpected io result {other:?}"
                ))),
            }
        })
        .doc(
            "Write a String (UTF-8) or Bytes to the child's standard input; answers the \
             receiver. Finish with `closeStdin` — most filters read until EOF.",
        )
        .instance_method("closeStdin", |vm, mc, receiver, _args| {
            let id = with_proc(receiver, "closeStdin", |p| p.stdin_id.take())?;
            if let Some(id) = id {
                vm.io.socket_reap.borrow_mut().push(id);
            }
            let _ = mc;
            Ok(receiver)
        })
        .doc(
            "Close the child's standard input — the child sees EOF. Idempotent; answers \
             the receiver.",
        )
        .instance_method("s", |vm, mc, receiver, _args| {
            let pid = with_proc(receiver, "s", |p| p.pid)?;
            Ok(vm.new_string(mc, format!("[OS]Process(pid {pid})")))
        })
        .doc("The inspect string: the class and pid.")
}
