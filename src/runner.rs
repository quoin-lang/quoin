use crate::compiler::Compiler;
use crate::error::QuoinError;
use crate::fiber::{Fiber, VMContext, YieldReason, run_vm_loop};
use crate::gc;
use crate::gcl;
use crate::highlighter::highlight_to_ansi;
use crate::introspect::{self, ClassInfo, GlobalInfo, GlobalKind, ValueInfo};
use crate::io_backend::{IoBackend, IoRequest, IoResult, SmolBackend, StreamId};
use crate::parser::ast::Node;
use crate::parser::{NodeValue, parse_quoin_file, try_parse_quoin_string_named};
use crate::repl_complete::{CompletionIndex, build_completion_index, complete_input};
use crate::runtime::runtime::build_block;
use crate::runtime::{
    async_rt, big_decimal, big_integer, block, boolean, bytes, channel, class, codecs, csv_fmt,
    date_time, double, duration, fiber as fiber_class, http, ids, instant, integer, io, json, list,
    map, math, method, msgpack, nil, object, pretty, regex, runtime, set, sockets, streams, string,
    symbol, task, time_zone, timer, timestamp, toml_fmt, yaml,
};
use crate::value::{Block, EnvFrame, NamespacedName, ObjectPayload, Value};
use crate::vm::{Task, TaskId, VmOptions, VmState, VmStatus, Wake};

use corosensei::CoroutineResult;
use futures_lite::StreamExt;
use futures_lite::future::block_on;
use futures_util::future::{Aborted, abortable};
use futures_util::stream::FuturesUnordered;
use gc_arena::{Arena, Gc, Mutation, Rootable, lock::RefLock};
use std::collections::HashSet;
use std::fs::read_to_string;
use std::future::Future;
use std::io::{BufRead, IsTerminal, stdin};
use std::iter::once_with;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::exit;
use std::sync::Once;
use std::time::Instant;

/// The prelude AST: a single `qnlib/prelude.qn` whose `use core/*` loads the core
/// stdlib (00-bootstrap … 06-io) in sorted order. Every runner mode loads this first,
/// so the prelude composition lives in Quoin rather than a hardcoded glob here.
fn prelude_asts() -> impl Iterator<Item = Node> {
    once_with(|| parse_quoin_file(&PathBuf::from("qnlib/prelude.qn")))
}

/// Register every native (Rust-backed) class on a fresh `VmState`. Shared by all runner
/// modes (run/test/benchmark/repl) so the builtin set can't drift between them.
pub(crate) fn register_builtins<'gc>(mc: &Mutation<'gc>, vm: &mut VmState<'gc>) {
    vm.register_native_class(mc, object::build_object_class());
    vm.register_native_class(mc, class::build_class_class());
    vm.register_native_class(mc, boolean::build_boolean_class());
    vm.register_native_class(mc, block::build_block_class());
    vm.register_native_class(mc, bytes::build_bytes_class());
    vm.register_native_class(mc, codecs::build_base64_class());
    vm.register_native_class(mc, codecs::build_hex_class());
    vm.register_native_class(mc, json::build_json_class());
    vm.register_native_class(mc, msgpack::build_message_pack_class());
    vm.register_native_class(mc, csv_fmt::build_csv_class());
    vm.register_native_class(mc, ids::build_uuid_class());
    vm.register_native_class(mc, ids::build_ulid_class());
    vm.register_native_class(mc, channel::build_channel_class());
    vm.register_native_class(mc, toml_fmt::build_toml_class());
    vm.register_native_class(mc, yaml::build_yaml_class());
    vm.register_native_class(mc, sockets::build_tcp_socket_class());
    vm.register_native_class(mc, sockets::build_tls_socket_class());
    vm.register_native_class(mc, sockets::build_tcp_listener_class());
    vm.register_native_class(mc, http::build_http_parser_class());
    vm.register_native_class(mc, streams::build_byte_stream_class());
    vm.register_native_class(mc, streams::build_string_stream_class());
    vm.register_native_class(mc, io::build_io_folder_class());
    vm.register_native_class(mc, io::build_io_file_class());
    vm.register_native_class(mc, io::build_io_handle_class());
    vm.register_native_class(mc, list::build_list_class());
    vm.register_native_class(mc, set::build_set_class());
    vm.register_native_class(mc, runtime::build_runtime_class());
    vm.register_native_class(mc, async_rt::build_async_class());
    vm.register_native_class(mc, task::build_task_class());
    vm.register_native_class(mc, method::build_method_class());
    vm.register_native_class(mc, timer::build_timer_class());
    vm.register_native_class(mc, double::build_double_class());
    vm.register_native_class(mc, integer::build_integer_class());
    vm.register_native_class(mc, math::build_math_class());
    vm.register_native_class(mc, big_decimal::build_big_decimal_class());
    vm.register_native_class(mc, big_integer::build_big_integer_class());
    vm.register_native_class(mc, duration::build_duration_class());
    vm.register_native_class(mc, instant::build_instant_class());
    vm.register_native_class(mc, time_zone::build_time_zone_class());
    vm.register_native_class(mc, timestamp::build_timestamp_class());
    vm.register_native_class(mc, date_time::build_date_time_class());
    vm.register_native_class(mc, string::build_string_class());
    vm.register_native_class(mc, symbol::build_symbol_class());
    vm.register_native_class(mc, nil::build_nil_class());
    vm.register_native_class(mc, map::build_map_class());
    vm.register_native_class(mc, map::build_key_value_pair_class());
    vm.register_native_class(mc, regex::build_regex_class());
    vm.register_native_class(mc, fiber_class::build_fiber_class());
}

/// The persistent REPL arena: one `VmState` kept alive across all lines.
type ReplArena = Arena<Rootable![VmState<'_>]>;

/// rustyline helper. Its `Validator` keeps the editor open for continuation while the input
/// is syntactically incomplete (a parse error positioned at end-of-input); a complete input
/// — or one with a *real* mid-input syntax error — submits, and the eval loop shows any
/// error. The `Highlighter` colorizes valid input. Completion/hinting are no-ops (P2).
struct ReplHelper {
    /// Tab-completion snapshot, refreshed from the live VM before each `readline` (the VM is
    /// frozen during editing, so a per-line snapshot is never stale).
    index: CompletionIndex,
}

impl rustyline::completion::Completer for ReplHelper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        Ok(complete_input(line, pos, &self.index))
    }
}
impl rustyline::hint::Hinter for ReplHelper {
    type Hint = String;
}
impl rustyline::highlight::Highlighter for ReplHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> std::borrow::Cow<'l, str> {
        // `highlight_to_ansi` is resilient (it predictively completes incomplete input and
        // never panics), so we colorize as-you-type, including partial lines.
        if line.trim().is_empty() {
            std::borrow::Cow::Borrowed(line)
        } else {
            std::borrow::Cow::Owned(highlight_to_ansi(line))
        }
    }

    fn highlight_char(
        &self,
        _line: &str,
        _pos: usize,
        _kind: rustyline::highlight::CmdKind,
    ) -> bool {
        true
    }
}
impl rustyline::Helper for ReplHelper {}

impl rustyline::validate::Validator for ReplHelper {
    fn validate(
        &self,
        ctx: &mut rustyline::validate::ValidationContext,
    ) -> rustyline::Result<rustyline::validate::ValidationResult> {
        use rustyline::validate::ValidationResult;
        let input = ctx.input();
        if input.trim().is_empty() {
            return Ok(ValidationResult::Valid(None));
        }
        match try_parse_quoin_string_named(input, "<repl>") {
            Ok(_) => Ok(ValidationResult::Valid(None)),
            Err(pe) if pe.start >= input.trim_end().len() => Ok(ValidationResult::Incomplete),
            Err(_) => Ok(ValidationResult::Valid(None)),
        }
    }
}

/// Outcome of a `$`-command.
enum ReplAction {
    Continue,
    Quit,
}

/// If `line` is a `$`-command, run it and return its action; `None` means "not a command,
/// evaluate it".
fn handle_repl_command(arena: &mut ReplArena, line: &str) -> Option<ReplAction> {
    let cmd = line.trim().strip_prefix('$')?.trim_start();
    let word = cmd.split_whitespace().next().unwrap_or("");
    let rest = cmd[word.len()..].trim();
    match word {
        "quit" | "exit" | "q" => return Some(ReplAction::Quit),
        "help" | "h" | "?" => {
            println!("Commands:");
            println!("  $type <expr>      show the class of an expression's result");
            println!("  $inspect <expr>   evaluate and show the value's class + fields");
            println!("  $time <expr>      evaluate and report wall-clock time");
            println!("  $globals [pre]    list defined classes and values (optional prefix)");
            println!("  $class <Name>     show a class: parent, mixins, ivars, methods");
            println!("  $load <file.qn>   run a .qn file into the session");
            println!("  $reset            clear session locals");
            println!("  $help             this help");
            println!("  $quit / $exit     leave the REPL (also Ctrl-D)");
            println!("Anything else is evaluated as Quoin; definitions and lowercase");
            println!("variables persist across lines.");
        }
        "reset" => {
            arena.mutate_root(|mc, vm| {
                vm.repl_env = Some(gcl!(mc, EnvFrame::new(None)));
            });
            println!("Session locals cleared.");
        }
        "type" if !rest.is_empty() => {
            if let Some(out) = eval_repl_type(arena, rest) {
                println!("{out}");
            }
        }
        "type" => eprintln!("usage: $type <expr>"),
        "inspect" if !rest.is_empty() => {
            if let Some(out) = eval_repl_inspect(arena, rest) {
                println!("{out}");
            }
        }
        "inspect" => eprintln!("usage: $inspect <expr>"),
        "globals" => {
            // Pure read: pull the owned infos out of the arena, format outside it. `rest`
            // (possibly empty) filters by name prefix. Wrap to the detected terminal width.
            let (infos, width) = arena.mutate_root(|_mc, vm| {
                (
                    introspect::globals(vm),
                    vm.options.console_width.map(|w| w as usize),
                )
            });
            print!("{}", format_globals(&infos, rest, width.unwrap_or(80)));
        }
        "class" if !rest.is_empty() => {
            match arena.mutate_root(|_mc, vm| introspect::describe_class(vm, rest)) {
                Some(info) => print!("{}", format_class(&info)),
                None => eprintln!("$class: no class named {rest}"),
            }
        }
        "class" => eprintln!("usage: $class <Name>"),
        "time" if !rest.is_empty() => {
            let start = Instant::now();
            let out = eval_repl_input(arena, rest);
            let elapsed = start.elapsed();
            if let Some(out) = out {
                println!("{out}");
            }
            println!("   ({:.3} ms)", elapsed.as_secs_f64() * 1000.0);
        }
        "time" => eprintln!("usage: $time <expr>"),
        "load" if !rest.is_empty() => match read_to_string(rest) {
            Ok(src) => {
                if let Some(out) = eval_repl_input(arena, &src) {
                    println!("{out}");
                }
                println!("loaded {rest}");
            }
            Err(e) => eprintln!("$load: cannot read {rest}: {e}"),
        },
        "load" => eprintln!("usage: $load <file.qn>"),
        other => eprintln!("Unknown command: ${other} (try $help)"),
    }
    Some(ReplAction::Continue)
}

/// Evaluate one complete REPL input and return the line to print (`=> <value>`, an error,
/// or `None` for a value-less `nil` result). The result is rendered with `.pp` (structural,
/// width-aware) — the canonical repr a REPL wants (quoted strings, instance vars), like a
/// `repr`/`inspect` rather than `.s`.
fn eval_repl_input(arena: &mut ReplArena, input: &str) -> Option<String> {
    eval_value(arena, input, |vm, _mc, val| {
        // Suppress a bare `nil` result (a value-less statement).
        if val.type_name() == "Nil" {
            None
        } else {
            let width = vm.options.console_width.map(|w| w as usize).unwrap_or(80);
            Some(format!(
                "=> {}",
                pretty::render(val, width, vm.options.supports_color)
            ))
        }
    })
}

/// `$type` form: evaluate `input` and return its class (the `.class`), never suppressed.
fn eval_repl_type(arena: &mut ReplArena, input: &str) -> Option<String> {
    eval_value(arena, input, |vm, mc, val| {
        let cls = vm.call_method(mc, val, "class", Vec::new()).unwrap_or(val);
        Some(render_value(vm, mc, cls))
    })
}

/// `$inspect` form: evaluate `input` and show its value, class, and (for an object) its
/// instance fields — the surface metadata from `introspect::describe_value`.
fn eval_repl_inspect(arena: &mut ReplArena, input: &str) -> Option<String> {
    eval_value(arena, input, |vm, mc, val| {
        let info = introspect::describe_value(vm, val);
        let repr = render_value(vm, mc, val);
        Some(format_inspect(&repr, &info))
    })
}

/// The session's top-level binding names, seeded into the compiler so references to them
/// resolve as locals (`LoadLocal` -> `repl_env`) rather than globals.
fn repl_locals(vm: &VmState<'_>) -> HashSet<String> {
    vm.repl_env
        .map(|env| {
            env.borrow()
                .vars
                .iter()
                .map(|(s, _)| s.as_str().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Compile one REPL program and run it **through the scheduler** as task #0 in the persistent
/// `repl_env`, so the line can do async I/O, sleep, spawn tasks, and resume fibers — and its
/// top-level bindings persist across lines. The outcome (`Ok(value)` on a clean finish,
/// `Err(..)` for a compile or runtime error) is handed to `finish` inside the final arena
/// borrow, so `finish` may call back into the VM (e.g. `.s`/`.class`). A non-program node
/// yields `nil`. Shared by `eval_value` (interactive/piped REPL) and `eval_once` (`-e`,
/// `~/.quoinrc`); both used to call the synchronous `execute_repl_line`, which had no
/// scheduler and so failed on any async op.
fn run_scheduled_line<F, R>(arena: &mut ReplArena, node: &Node, finish: F) -> R
where
    F: for<'gc> FnOnce(&mut VmState<'gc>, &Mutation<'gc>, Result<Value<'gc>, QuoinError>) -> R,
{
    enum Setup {
        Ready(usize, usize),
        Empty,
        CompileErr(String),
    }

    // Setup: compile in the session's locals, start the line as the live task #0 in
    // `repl_env`, recording the frame/stack baseline to restore afterward.
    let setup = arena.mutate_root(|mc, vm| {
        let NodeValue::Program(p) = &node.value else {
            return Setup::Empty;
        };
        let sb = match Compiler::new_with_locals(repl_locals(vm)).compile_program(p) {
            Ok(sb) => sb,
            Err(e) => return Setup::CompileErr(format!("Compile error: {e}")),
        };
        let block = build_block(mc, &sb);
        let (base_frames, base_stack) = vm.begin_repl_line(block);
        install_main_task(mc, vm);
        Setup::Ready(base_frames, base_stack)
    });

    match setup {
        Setup::Empty => arena.mutate_root(|mc, vm| {
            let nil = vm.new_nil(mc);
            finish(vm, mc, Ok(nil))
        }),
        Setup::CompileErr(msg) => {
            arena.mutate_root(|mc, vm| finish(vm, mc, Err(QuoinError::Other(msg))))
        }
        Setup::Ready(base_frames, base_stack) => {
            // Drive the scheduler outside any arena borrow (it interleaves `mutate_root`s
            // with the reactor `.await`). `step` already source-annotated any error.
            let drive = drive_main_task(arena);
            arena.mutate_root(|mc, vm| {
                let outcome = match drive {
                    Ok(()) => Ok(vm.end_repl_line(mc, base_frames, base_stack, true)),
                    Err(e) => {
                        vm.end_repl_line(mc, base_frames, base_stack, false);
                        Err(e)
                    }
                };
                finish(vm, mc, outcome)
            })
        }
    }
}

/// Parse and run one complete REPL input, then hand the result `Value` to `render` (inside the
/// GC borrow, so it may call back into the VM — e.g. `.s`/`.class`) to produce the line to
/// print. Parse/compile/runtime failures short-circuit to an error string; `render` only sees
/// a successful value.
fn eval_value<F>(arena: &mut ReplArena, input: &str, render: F) -> Option<String>
where
    F: for<'gc> FnOnce(&mut VmState<'gc>, &Mutation<'gc>, Value<'gc>) -> Option<String>,
{
    let node = match try_parse_quoin_string_named(input, "<repl>") {
        Ok(n) => n,
        Err(pe) => {
            return Some(format!(
                "Parse error at line {}, col {}: {}",
                pe.line, pe.column, pe.message
            ));
        }
    };
    run_scheduled_line(arena, &node, |vm, mc, outcome| match outcome {
        Ok(val) => render(vm, mc, val),
        Err(e) => Some(format!("{e}")),
    })
}

/// Parse and run one complete input, returning the rendered non-`nil` result (`Ok(Some)`),
/// `Ok(None)` for a `nil` result, or an error message. Unlike `eval_value` — which folds
/// errors into the printed string for the interactive loop — this keeps success and failure
/// distinct, so `qn -e` / `~/.quoinrc` can act on a real error.
fn eval_once(arena: &mut ReplArena, input: &str) -> Result<Option<String>, String> {
    let node = try_parse_quoin_string_named(input, "<eval>").map_err(|pe| {
        format!(
            "Parse error at line {}, col {}: {}",
            pe.line, pe.column, pe.message
        )
    })?;
    run_scheduled_line(arena, &node, |vm, mc, outcome| match outcome {
        Ok(val) if val.type_name() == "Nil" => Ok(None),
        Ok(val) => Ok(Some(render_value(vm, mc, val))),
        Err(e) => Err(format!("{e}")),
    })
}

/// A `QN_*` boolean toggle, using the same convention as the internal `tuning` knobs: set and
/// not `""`/`"0"`/`"false"` → on. These are user-facing REPL toggles, so they live here rather
/// than in the (explicitly non-user-facing) `tuning` module.
fn env_flag(name: &str) -> bool {
    matches!(std::env::var(name), Ok(v) if !matches!(v.as_str(), "" | "0" | "false"))
}

/// Run `~/.quoinrc` into the session if it exists. Missing/unreadable/empty is fine (silent);
/// a successful run is silent too (its definitions just persist); an error is reported but
/// non-fatal — the prompt still opens.
fn load_quoinrc(arena: &mut ReplArena) {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let path = PathBuf::from(home).join(".quoinrc");
    let src = match read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return,
    };
    if src.trim().is_empty() {
        return;
    }
    if let Err(msg) = eval_once(arena, &src) {
        eprintln!("~/.quoinrc: {msg}");
    }
}

/// `$globals` output: surface globals split into classes (flowed to `width` columns) and
/// values (one `name: Class` per line), filtered by `prefix` (empty = all).
fn format_globals(infos: &[GlobalInfo], prefix: &str, width: usize) -> String {
    let mut classes: Vec<&str> = Vec::new();
    let mut values: Vec<(&str, &str)> = Vec::new();
    for info in infos {
        if !info.name.starts_with(prefix) {
            continue;
        }
        match &info.kind {
            GlobalKind::Class => classes.push(&info.name),
            GlobalKind::Value { class } => values.push((&info.name, class)),
        }
    }
    let mut out = String::new();
    if !classes.is_empty() {
        out.push_str(&format!("Classes ({}):\n", classes.len()));
        out.push_str(&flow_names(&classes, width));
    }
    if !values.is_empty() {
        out.push_str(&format!("Values ({}):\n", values.len()));
        for (name, class) in &values {
            out.push_str(&format!("  {name}: {class}\n"));
        }
    }
    if out.is_empty() {
        out.push_str(if prefix.is_empty() {
            "(no globals)\n"
        } else {
            "(none matching prefix)\n"
        });
    }
    out
}

/// `$class` output: a header line (`Name < Parent (mixins…) [flags]`) followed by indented
/// ivars and method signatures (own instance methods, then class methods).
fn format_class(info: &ClassInfo) -> String {
    let mut out = String::new();
    // Quoin-style inheritance: `Parent <- Child` (the arrow points at the subclass, as in a
    // `Parent <- Child <- { … }` definition). Mixins/flags trail the class name.
    if let Some(parent) = &info.parent {
        out.push_str(&format!("{parent} <- "));
    }
    out.push_str(&info.name);
    if !info.mixins.is_empty() {
        out.push_str(&format!(" ({})", info.mixins.join(", ")));
    }
    let mut flags: Vec<&str> = Vec::new();
    if info.is_sealed {
        flags.push("sealed");
    }
    if info.is_abstract {
        flags.push("abstract");
    }
    if !flags.is_empty() {
        out.push_str(&format!(" [{}]", flags.join(" ")));
    }
    out.push('\n');

    if !info.instance_vars.is_empty() {
        out.push_str(&format!("  ivars: {}\n", info.instance_vars.join(", ")));
    }
    let mut section = |label: &str, methods: &[introspect::MethodInfo]| {
        if methods.is_empty() {
            return;
        }
        out.push_str(&format!("  {label}:\n"));
        for m in methods {
            for v in &m.variants {
                out.push_str(&format!("    {}\n", introspect::signature(&m.selector, v)));
            }
        }
    };
    section("methods", &info.instance_methods);
    section("class methods", &info.class_methods);
    out
}

/// `$inspect` output: the value's repr + class, then one `@field: Class` line per instance
/// field (objects only; scalars and fieldless objects show just the first line).
fn format_inspect(repr: &str, info: &ValueInfo) -> String {
    let mut out = format!("{repr}  (class {})", info.class);
    for (name, class) in &info.fields {
        out.push_str(&format!("\n  @{name}: {class}"));
    }
    out
}

/// Wrap `names` into indented lines no wider than `width` columns (two-space indent, two
/// spaces between names). Names are ASCII identifiers, so byte length is display width.
fn flow_names(names: &[&str], width: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    for name in names {
        if col == 0 {
            // Line start: two-space indent, no separator.
            out.push_str("  ");
            col = 2;
        } else if col + 2 + name.len() <= width {
            // Fits on the current line: two-space separator before it.
            out.push_str("  ");
            col += 2;
        } else {
            // Wrap to a fresh indented line.
            out.push_str("\n  ");
            col = 2;
        }
        out.push_str(name);
        col += name.len();
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Render a result value for display via its `.s` method (so a user-defined `s` override is
/// honored), falling back to the `Value` `Display` if `.s` errors or returns a non-string.
/// Safe to run here: a `mutate_root` closure holds the `Mutation`, so no GC occurs while
/// `val` sits only on the Rust stack.
fn render_value<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>, val: Value<'gc>) -> String {
    match vm.call_method(mc, val, "s", Vec::new()) {
        Ok(Value::Object(o)) => match &o.borrow().payload {
            ObjectPayload::String(s) => s.to_string(),
            _ => format!("{val}"),
        },
        Ok(other) => format!("{other}"),
        Err(_) => format!("{val}"),
    }
}

/// Interactive loop: rustyline editing, history, multiline via the `Validator`, and Ctrl-C
/// to abandon an in-progress input (Ctrl-D to exit).
fn run_repl_interactive(arena: &mut ReplArena) {
    // `List` completion shows all candidates beneath the prompt (vs the default circular
    // cycle-on-Tab), which suits selector/global menus.
    let config = rustyline::Config::builder()
        .completion_type(rustyline::CompletionType::List)
        .build();
    let mut editor =
        match rustyline::Editor::<ReplHelper, rustyline::history::DefaultHistory>::with_config(
            config,
        ) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("repl: failed to start line editor: {e}");
                return;
            }
        };
    editor.set_helper(Some(ReplHelper {
        index: CompletionIndex::default(),
    }));

    let history =
        std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".quoin_history"));
    if let Some(ref p) = history {
        let _ = editor.load_history(p);
    }

    // `QN_NO_BANNER` (matching the `QN_*` tuning knobs) suppresses the greeting; `QN_NO_BANNER=0`
    // still shows it.
    if !env_flag("QN_NO_BANNER") {
        println!("Quoin REPL — $help for commands, $quit (or Ctrl-D) to exit.");
    }
    // `QN_PROMPT` overrides the prompt string (default `qn> `).
    let prompt = std::env::var("QN_PROMPT").unwrap_or_else(|_| "qn> ".to_string());

    loop {
        // Refresh the completion snapshot from the VM as it stands before this line (it can't
        // change until we eval below). `helper_mut` and `arena` are disjoint borrows.
        let index = arena.mutate_root(|_mc, vm| build_completion_index(vm));
        if let Some(helper) = editor.helper_mut() {
            helper.index = index;
        }
        match editor.readline(&prompt) {
            Ok(input) => {
                if input.trim().is_empty() {
                    continue;
                }
                let _ = editor.add_history_entry(input.as_str());
                if let Some(action) = handle_repl_command(arena, &input) {
                    match action {
                        ReplAction::Quit => break,
                        ReplAction::Continue => continue,
                    }
                }
                if let Some(out) = eval_repl_input(arena, &input) {
                    println!("{out}");
                }
            }
            // Ctrl-C abandons the in-progress input; Ctrl-D / EOF exits.
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("repl: input error: {e}");
                break;
            }
        }
    }

    if let Some(ref p) = history {
        let _ = editor.save_history(p);
    }
}

/// Non-interactive loop (piped / redirected stdin): no editor, no prompts. Accumulates
/// lines until the buffer parses (or errors mid-input), then evaluates. Enables
/// `echo '…' | qn repl` and `qn repl < script.qn`.
fn run_repl_piped(arena: &mut ReplArena) {
    let stdin = stdin();
    let mut buffer = String::new();
    loop {
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if buffer.is_empty() && line.trim().is_empty() {
            continue;
        }
        if buffer.is_empty() {
            if let Some(action) = handle_repl_command(arena, &line) {
                match action {
                    ReplAction::Quit => break,
                    ReplAction::Continue => continue,
                }
            }
        }
        buffer.push_str(&line);
        match try_parse_quoin_string_named(&buffer, "<repl>") {
            // Incomplete — keep accumulating.
            Err(pe) if pe.start >= buffer.trim_end().len() => continue,
            _ => {
                if let Some(out) = eval_repl_input(arena, &buffer) {
                    println!("{out}");
                }
                buffer.clear();
            }
        }
    }
}

/// Step status for the benchmark driver, which runs a single fiber to completion
/// with no async I/O (the run/test driver uses `RunStep` and the task scheduler).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutionStatus {
    Running,
    Finished,
    Yeeted,
}

/// What a completed background future tells the driver to do for the task that owns it.
/// The scheduler keeps these in a `FuturesUnordered`; `.next().await` is the one reactor
/// wait. Both arms are `abortable` so `cancel` (and a won race) interrupts them promptly.
enum TaskWakeup {
    /// An async I/O op finished (`Ok`), or was aborted by `cancel` (`Err(Aborted)`).
    Io(Result<IoResult, Aborted>),
    /// A `JoinTimed` deadline timer elapsed. Carries the joined `target` and the park
    /// `epoch` captured at park time, so `deliver_deadline` can ignore a stale firing.
    Deadline { target: TaskId, epoch: u64 },
}

/// A boxed, single-threaded background future tagged with the task that is waiting on it.
type IoTaskFuture = Pin<Box<dyn Future<Output = (TaskId, TaskWakeup)>>>;

/// A tiny deterministic PRNG (SplitMix64) for `QN_SCHED_STRESS`. Seeded so a
/// randomized scheduling failure can be replayed exactly. Not used outside stress.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform index in `0..n` (caller ensures `n > 0`).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// What the current task did when the run/test scheduler resumed it once.
enum RunStep {
    /// Suspended at a cooperative-yield boundary (between VM steps). Mailboxes are
    /// empty here, so this is the one point it is safe to *preempt* the task — the
    /// scheduler stress mode does exactly that. Without stress, it just keeps running.
    Yielded,
    /// Made progress mid-work — a block call or a guest-fiber switch — and is still
    /// the current task. Not a safe preemption point (a fiber switch leaves a value
    /// in the `fiber_transfer` mailbox the target has not consumed yet), so the
    /// driver always keeps running it.
    Running,
    /// Parked on async I/O. Its context is already saved; the driver fulfills `req`
    /// and resumes it later with the result.
    ParkedIo(IoRequest),
    /// Parked waiting on other tasks — a `gather` batch, or a `join` — which were
    /// already wired up (children/waiters enqueued, context saved) inside the resume.
    /// The driver just picks the next ready task; the wakeup comes from a completion.
    Parked,
    /// Parked in `JoinTimed` on `target` with a deadline of `ms` ms: like `Parked`, but
    /// the driver must also arm a deadline timer that wakes this task if `target` has not
    /// finished in time (`Async.timeout:do:`). The joiner is already a waiter on `target`.
    ParkedJoinTimed { target: TaskId, ms: u64 },
    /// A non-main task finished; its waker(s) were already enqueued to `ready`.
    Done,
    /// The main task (#0) finished — the program is done; its result is on the stack.
    Finished,
    /// An interactive debug session hit a breakpoint/step. The driver runs the `$`-command
    /// loop (which reads commands outside the arena and applies them inside it), then
    /// re-resumes this same task in place. Only produced when `debug.interactive` is set.
    DebugPaused,
}

/// Resume the current task's coroutine once and classify what happened. The guest
/// `Fiber` switches (`ResumeFiber`/`YieldFiber`) and the GC-cooperative yield stay
/// internal to the task; only I/O, gather, and completion surface to the driver.
fn resume_current_task<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
) -> Result<RunStep, QuoinError> {
    // Resume the coroutine of the currently-running fiber: a guest `Fiber` if one is
    // active in this task, otherwise the task's own root coroutine.
    let coro_holder = match vm.sched.current_fiber {
        None => match vm
            .sched
            .tasks
            .get(vm.sched.current_task.0)
            .and_then(|t| t.as_ref())
        {
            Some(task) => task.coro,
            None => return Ok(RunStep::Finished),
        },
        Some(fv) => fv
            .with_native_state::<fiber_class::NativeFiberState, _, _>(|s| s.coro())
            .map_err(QuoinError::Other)?,
    };

    // Point `vm.sched.yielder` at the coroutine we're about to run, sourced from its
    // own GC-rooted slot, so it never dangles.
    vm.sched.yielder = vm.current_fiber_yielder();

    let ctx = VMContext {
        vm: vm as *mut _,
        mc: mc as *const _,
    };
    let res = {
        let mut opt = coro_holder.coroutine.borrow_mut();
        let coro = opt.as_mut().expect("Coroutine already finished");
        coro.resume(ctx)
    };

    match res {
        CoroutineResult::Yield(YieldReason::CooperativeYield) => Ok(RunStep::Yielded),
        CoroutineResult::Yield(YieldReason::CallBlock { .. }) => Ok(RunStep::Running),
        CoroutineResult::Yield(YieldReason::ResumeFiber { fiber, arg }) => {
            vm.do_resume_switch(mc, fiber, arg)?;
            Ok(RunStep::Running)
        }
        CoroutineResult::Yield(YieldReason::YieldFiber { value }) => {
            vm.do_yield_switch(mc, value)?;
            Ok(RunStep::Running)
        }
        CoroutineResult::Yield(YieldReason::AwaitIo { req }) => {
            // Park: stash this task's context so another can run while I/O is in flight.
            vm.save_task_context(vm.sched.current_task);
            Ok(RunStep::ParkedIo(req))
        }
        CoroutineResult::Yield(YieldReason::Gather { blocks }) => {
            // Park the parent on its gather; children are enqueued inside spawn_gather.
            vm.spawn_gather(mc, blocks);
            Ok(RunStep::Parked)
        }
        CoroutineResult::Yield(YieldReason::Join { .. }) => {
            // The joiner already added itself to the target's waiter list in await_join;
            // park its context until the target completes and wakes it.
            vm.save_task_context(vm.sched.current_task);
            Ok(RunStep::Parked)
        }
        CoroutineResult::Yield(YieldReason::JoinTimed { task, ms }) => {
            // Like Join (the joiner is already a waiter on `task`), but the driver also
            // arms a deadline timer — carry the target and `ms` up to it.
            vm.save_task_context(vm.sched.current_task);
            Ok(RunStep::ParkedJoinTimed { target: task, ms })
        }
        CoroutineResult::Yield(YieldReason::ChannelPark) => {
            // The task already enqueued itself in the channel's waiter queue (in
            // `channel_send`/`channel_recv`); park its context until a counterpart or
            // `close` sets its `wake` and re-enqueues it to `ready`.
            vm.save_task_context(vm.sched.current_task);
            Ok(RunStep::Parked)
        }
        CoroutineResult::Yield(YieldReason::DebugBreak) => {
            // A breakpoint/step paused this task. Interactive sessions bubble up to the
            // driver's `$`-command loop (where the line editor lives); non-interactive ones
            // (tests / scripted runs) apply the next scripted action in place. Either way the
            // VM stays stopped — no park — and the coroutine resumes past the suspend point in
            // `debug_checkpoint` and dispatches the instruction.
            if vm.debug.as_ref().is_some_and(|d| d.interactive) {
                Ok(RunStep::DebugPaused)
            } else {
                vm.debug_on_pause();
                Ok(RunStep::Running)
            }
        }
        CoroutineResult::Yield(YieldReason::Return(val)) => complete_current_task(vm, mc, Ok(val)),
        CoroutineResult::Return(res) => {
            if vm.sched.current_fiber.is_some() {
                // A guest fiber's block returned; hand the result back to its resumer
                // and keep running this same task.
                vm.do_fiber_done(mc, res)?;
                Ok(RunStep::Running)
            } else {
                complete_current_task(vm, mc, res)
            }
        }
    }
}

/// The current task's root coroutine completed with `result`. Dispatch by kind: a
/// gather child reports into its parent's batch; a detached task writes its outcome to
/// its handle and wakes joiners; the main task ends the program, leaving its result on
/// the stack. The first two enqueue any woken task to `ready` themselves.
fn complete_current_task<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    result: Result<Value<'gc>, QuoinError>,
) -> Result<RunStep, QuoinError> {
    let cur = vm.sched.current_task;
    let task = vm.sched.tasks[cur.0]
        .as_ref()
        .expect("completing task slot is empty");
    if task.parent.is_some() {
        vm.complete_child(mc, cur, result);
        Ok(RunStep::Done)
    } else if task.handle.is_some() {
        vm.complete_detached(mc, cur, result);
        Ok(RunStep::Done)
    } else {
        vm.sched.tasks[cur.0] = None;
        match result {
            Ok(val) => {
                vm.push(val);
                Ok(RunStep::Finished)
            }
            Err(err) => Err(err),
        }
    }
}

/// Install the already-started top-level block as scheduler task #0: wrap `run_vm_loop` in
/// a fiber, make it the sole task, and mark it current. The block must already be live on
/// `vm`'s frames (via `start_block` or `push_repl_frame`); the task is pre-started, so its
/// context lives in `VmState` until it parks. Shared by the file runner and the REPL so
/// every top-level unit runs under the scheduler.
fn install_main_task<'gc>(mc: &Mutation<'gc>, vm: &mut VmState<'gc>) {
    let fiber = Fiber::new(|yielder, ctx| run_vm_loop(yielder, ctx));
    vm.sched.tasks = vec![Some(Task {
        coro: gc!(mc, fiber),
        root_yielder: None,
        block: None,
        started: true,
        stack: Vec::new(),
        frames: Vec::new(),
        native_args: Vec::new(),
        current_fiber: None,
        resume_stack: Vec::new(),
        wake: None,
        parent: None,
        gather: None,
        handle: None,
        waiters: Vec::new(),
        cancel_requested: false,
        abort_handle: None,
        joining: None,
        park_epoch: 0,
        deadline_abort: None,
        parked_on_channel: false,
    })];
    vm.sched.current_task = TaskId(0);
}

/// Drive the scheduler until the main task (#0) — already installed via `install_main_task`
/// — finishes. Background I/O overlaps on a fresh `SmolBackend`; the single
/// `futures.next().await` is the one reactor wait. The runnable set is `vm.sched.ready` (so a
/// native `spawn` enqueues directly); `QN_SCHED_STRESS` preempts at every cooperative yield
/// and picks ready tasks at random, otherwise the scheduler is run-to-block. On a clean
/// finish the unit's result is on the VM stack (pushed by `complete_current_task`); a runtime
/// error is returned (already source-annotated by `step`). Shared by the file runner, the
/// REPL, `-e`, and `~/.quoinrc` so every top-level run gets async I/O, sleep, tasks, fibers.
/// Outcome of servicing a debug pause: resume the paused task, or stop the session.
enum DebugFlow {
    Resume,
    Quit,
}

/// The frontend the driver consults at debug touchpoints — the interactive CLI (`CliFrontend`)
/// or the DAP adapter. One scheduler loop ([`drive_with_frontend`]) serves both; a non-debug run
/// uses `CliFrontend` and simply never pauses.
trait DriverFrontend {
    /// Run once before the task starts. Return `false` to abort without running. (DAP: the
    /// `initialize`/`setBreakpoints`/`configurationDone` handshake; CLI: nothing.)
    fn configure(&mut self, _arena: &mut ReplArena) -> Result<bool, QuoinError> {
        Ok(true)
    }
    /// Called after each step to surface program output captured since the last call (DAP:
    /// `output` events). No-op when not capturing.
    fn on_output(&mut self, _arena: &mut ReplArena) -> Result<(), QuoinError> {
        Ok(())
    }
    /// A `DebugPaused`: service the frontend until the user resumes or quits.
    fn on_pause(&mut self, arena: &mut ReplArena) -> Result<DebugFlow, QuoinError>;
    /// The task finished (`err` = `None`) or escaped with an uncaught error.
    fn on_finished(
        &mut self,
        _arena: &mut ReplArena,
        _err: Option<&QuoinError>,
    ) -> Result<(), QuoinError> {
        Ok(())
    }
}

/// The interactive `$`-command frontend (`qn debug`), and the default for every non-debug run
/// (where it never pauses). The rustyline editor is built lazily on the first pause.
#[derive(Default)]
struct CliFrontend {
    editor: Option<crate::debug_cli::DebugFrontend>,
}

impl DriverFrontend for CliFrontend {
    fn on_pause(&mut self, arena: &mut ReplArena) -> Result<DebugFlow, QuoinError> {
        // The whole VM is stopped. Run the `$`-command loop: read each line with the editor
        // (outside the arena, so history persists), then execute it against the live paused VM
        // inside `mutate_root`. Loop until a continue/step verb (`Resume`) or `$quit`.
        use crate::debug_cli::{CommandOutcome, DebugFrontend, announce_pause, exec_command};
        use rustyline::error::ReadlineError;
        if self.editor.is_none() {
            self.editor = DebugFrontend::new();
        }
        let Some(editor) = self.editor.as_mut() else {
            // No usable editor — degrade to "continue" so the run still completes.
            arena.mutate_root(|_mc, vm| vm.apply_debug_action(crate::debug::DebugAction::Continue));
            return Ok(DebugFlow::Resume);
        };
        arena.mutate_root(|_mc, vm| announce_pause(vm));
        loop {
            let line = match editor.readline() {
                Ok(l) => l,
                Err(ReadlineError::Interrupted) => continue, // Ctrl-C: re-prompt
                Err(ReadlineError::Eof) => "$quit".to_string(), // Ctrl-D: quit
                Err(e) => {
                    eprintln!("debug: input error: {e}");
                    "$quit".to_string()
                }
            };
            match arena.mutate_root(|mc, vm| exec_command(vm, mc, &line)) {
                CommandOutcome::Stay => continue,
                CommandOutcome::Resume => return Ok(DebugFlow::Resume),
                CommandOutcome::Quit => return Ok(DebugFlow::Quit),
            }
        }
    }
}

/// Map a DAP wire I/O error into the driver's error type.
fn dap_io(e: std::io::Error) -> QuoinError {
    QuoinError::Other(format!("DAP I/O: {e}"))
}

/// Best-effort `stopped` reason from the paused debug state.
fn dap_stop_reason(vm: &VmState<'_>) -> &'static str {
    let Some(d) = vm.debug.as_ref() else {
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
        if let Some(d) = vm.debug.as_mut() {
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

/// The DAP adapter frontend: translates the driver's debug touchpoints to/from the Debug Adapter
/// Protocol over its [`Connection`](crate::dap::Connection). `configure` runs the handshake +
/// breakpoint setup through `configurationDone`; `on_output` flushes program output as `output`
/// events; `on_pause` emits `stopped` and services requests until the client resumes/disconnects.
/// Generic over the streams so tests can drive it over in-memory buffers.
struct DapFrontend<R: std::io::BufRead, W: std::io::Write> {
    conn: crate::dap::Connection<R, W>,
    /// Per-pause `variablesReference` table: handle (1-based) -> the frame whose `Locals` scope it
    /// expands. Cleared at each pause (a DAP handle is valid only for the current stop).
    handles: Vec<usize>,
}

impl<R: std::io::BufRead, W: std::io::Write> DapFrontend<R, W> {
    fn new(conn: crate::dap::Connection<R, W>) -> Self {
        Self {
            conn,
            handles: Vec::new(),
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
                    let stop_on_entry = req
                        .arguments
                        .get("stopOnEntry")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if stop_on_entry {
                        arena.mutate_root(|_mc, vm| {
                            if let Some(d) = vm.debug.as_mut() {
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
                    self.handles.push(frame);
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
                    let vars = match self.handles.get(var_ref.wrapping_sub(1)).copied() {
                        Some(frame) => arena.mutate_root(|_mc, vm| {
                            vm.debug_frame_variables(frame)
                                .into_iter()
                                .map(|(name, val)| {
                                    json!({
                                        "name": name,
                                        "value": vm.debug_render(val),
                                        "variablesReference": 0,
                                    })
                                })
                                .collect::<Vec<_>>()
                        }),
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

/// The interactive/normal driver: a [`CliFrontend`] over the shared scheduler loop. Used by
/// `qn debug`, plain `qn <file>`, the REPL, and the debug fixtures.
fn drive_main_task(arena: &mut ReplArena) -> Result<(), QuoinError> {
    drive_with_frontend(arena, &mut CliFrontend::default())
}

/// The cooperative scheduler loop, parameterized by a [`DriverFrontend`] for the debug
/// touchpoints (configuration, program output, pause, completion). Resumes the current task,
/// services background I/O / deadlines via the reactor, and hands a `DebugPaused` to the
/// frontend. Shared by the CLI debugger, normal/REPL runs, and the DAP adapter.
fn drive_with_frontend<F: DriverFrontend>(
    arena: &mut ReplArena,
    frontend: &mut F,
) -> Result<(), QuoinError> {
    let backend = SmolBackend::new();
    let mut futures: FuturesUnordered<IoTaskFuture> = FuturesUnordered::new();
    let mut rng = crate::tuning::sched_stress().map(SplitMix64::new);
    // Announce the seed once per process so a failing run is reproducible with the same
    // `QN_SCHED_STRESS=<seed>`.
    if let Some(seed) = crate::tuning::sched_stress() {
        static ANNOUNCED: Once = Once::new();
        ANNOUNCED.call_once(|| eprintln!("scheduler stress enabled (seed={seed})"));
    }
    // Task #0 starts current and already live; nothing to load on first resume.
    let mut current: Option<TaskId> = Some(TaskId(0));
    let mut needs_load = false;

    block_on(async {
        // Pre-run configuration (the DAP handshake + breakpoints; a no-op for the CLI). Abort
        // cleanly if the frontend declines to run.
        if !frontend.configure(arena)? {
            return Ok(());
        }
        let mut step_count = 0;
        loop {
            // Acquire a task to run after the previous one parked or finished: pick from
            // `ready` (random under stress); if none are ready but I/O is in flight, await a
            // completion, which feeds `ready`, and retry.
            if current.is_none() {
                let picked = arena.mutate_root(|_mc, vm| {
                    let n = vm.sched.ready.len();
                    if n == 0 {
                        None
                    } else {
                        let idx = rng.as_mut().map(|r| r.below(n)).unwrap_or(0);
                        Some(vm.sched.ready.remove(idx).expect("idx within ready"))
                    }
                });
                match picked {
                    Some(tid) => {
                        current = Some(tid);
                        needs_load = true;
                    }
                    None => {
                        if futures.is_empty() {
                            break; // nothing ready and nothing in flight
                        }
                        // The single reactor wait: park until some background future (I/O op
                        // or deadline timer) lands.
                        let (tid, wakeup) = futures.next().await.expect("futures is non-empty");
                        arena.mutate_root(|_mc, vm| match wakeup {
                            TaskWakeup::Io(result) => {
                                {
                                    let t = vm.sched.tasks[tid.0]
                                        .as_mut()
                                        .expect("woken task slot is empty");
                                    t.abort_handle = None; // the future is done
                                    // On `Err(Aborted)` the task was cancelled: leave `wake`
                                    // unset — `await_io` raises `Cancelled` instead.
                                    if let Ok(io_result) = result {
                                        t.wake = Some(Wake::Io { result: io_result });
                                    }
                                }
                                vm.sched.ready.push_back(tid);
                            }
                            // A deadline elapsed: `deliver_deadline` resolves the race and
                            // enqueues the joiner if it won.
                            TaskWakeup::Deadline { target, epoch } => {
                                vm.deliver_deadline(tid, target, epoch);
                            }
                        });
                        continue;
                    }
                }
            }
            let cur = current.expect("current task set above");
            if needs_load {
                arena.mutate_root(|mc, vm| vm.load_task_context(mc, cur));
                needs_load = false;
            }

            let step = arena.mutate_root(|mc, vm| resume_current_task(vm, mc));
            // Surface any program output this step produced before reacting to the step.
            frontend.on_output(arena)?;
            match step {
                Ok(RunStep::Yielded) => {
                    // A clean cooperative-yield boundary. Under stress, preempt: stash and
                    // requeue so the save/load round-trip runs every step and ordering varies.
                    if rng.is_some() {
                        arena.mutate_root(|_mc, vm| {
                            vm.save_task_context(cur);
                            vm.sched.ready.push_back(cur);
                        });
                        current = None;
                    }
                }
                Ok(RunStep::Running) => {}
                Ok(RunStep::ParkedIo(req)) => {
                    // Hand the op to the backend; the future is tagged with the parked task so
                    // its result routes back, and wrapped in `abortable` so `cancel` can
                    // interrupt it. Stash the abort handle for `request_cancel`.
                    let (fut, abort_handle) = abortable(backend.perform(req));
                    arena.mutate_root(|_mc, vm| {
                        vm.sched.tasks[cur.0]
                            .as_mut()
                            .expect("parked task slot is empty")
                            .abort_handle = Some(abort_handle);
                    });
                    futures.push(Box::pin(async move { (cur, TaskWakeup::Io(fut.await)) }));
                    current = None;
                }
                Ok(RunStep::ParkedJoinTimed { target, ms }) => {
                    // Arm the deadline alongside the join: a `Sleep` timer tagged with this
                    // joiner + the park epoch, wrapped in `abortable` so a normal completion /
                    // cancel can disarm it. `deliver_deadline` ignores a stale firing.
                    let (fut, abort_handle) = abortable(backend.perform(IoRequest::Sleep { ms }));
                    let epoch = arena.mutate_root(|_mc, vm| {
                        let t = vm.sched.tasks[cur.0]
                            .as_mut()
                            .expect("timed-join parked task slot is empty");
                        t.deadline_abort = Some(abort_handle);
                        t.park_epoch
                    });
                    futures.push(Box::pin(async move {
                        let _ = fut.await; // resolved (Slept) or aborted; either way
                        (cur, TaskWakeup::Deadline { target, epoch })
                    }));
                    current = None;
                }
                // Parked on a gather batch or a join, or finished: any task that became
                // runnable was already enqueued to `ready` in the resume.
                Ok(RunStep::Parked) | Ok(RunStep::Done) => {
                    current = None;
                }
                Ok(RunStep::Finished) => {
                    frontend.on_finished(arena, None)?;
                    break;
                }
                Ok(RunStep::DebugPaused) => match frontend.on_pause(arena)? {
                    // Re-resume the same task: its context is live (it parked nothing).
                    DebugFlow::Resume => {}
                    DebugFlow::Quit => return Ok(()),
                },
                Err(e) => {
                    frontend.on_finished(arena, Some(&e))?;
                    return Err(e);
                }
            }
            step_count += 1;
            if crate::tuning::gc_stress() || step_count % 10 == 0 {
                arena.collect_debt();
                // Reap fds whose handle was closed or collected — both enqueue on
                // `socket_reap`; close them now, outside the arena borrow.
                let reaped: Vec<StreamId> =
                    arena.mutate_root(|_mc, vm| vm.socket_reap.borrow_mut().drain(..).collect());
                for id in reaped {
                    backend.close(id);
                }
            }
        }
        Ok(())
    })
}

pub struct VmRunnerOptions {
    pub mode: VmRunnerMode,
    pub target_path: Option<String>,
    pub vm_options: VmOptions,
    /// Exception types from `qn debug --break-on-throw=Type,…` (empty otherwise).
    pub break_on_throw: Vec<String>,
    /// Exception types from `qn debug --break-on-uncaught=Type,…` — break only when uncaught.
    pub break_on_uncaught: Vec<String>,
    /// `qn debug --dap`: drive the program as a DAP adapter over stdio instead of the CLI loop.
    pub dap: bool,
    /// Quoin-level coverage output, from `--coverage[=fmt]` / `--coverage-out=PATH`
    /// (on `qn test` or `qn <file>`). `None` when coverage wasn't requested.
    pub coverage: Option<crate::coverage::CoverageConfig>,
}

/// Pull `--coverage[=fmt]` and `--coverage-out=PATH` out of an argument slice, pushing
/// every other argument onto `vm_args`. Shared by the `test` and `run` arms.
fn take_coverage_flags(
    args: &[String],
    vm_args: &mut Vec<String>,
    enabled: &mut bool,
    out: &mut Option<String>,
    format: &mut crate::coverage::CoverageFormat,
) {
    use crate::coverage::CoverageFormat;
    for a in args {
        if a == "--coverage" {
            *enabled = true;
        } else if let Some(fmt) = a.strip_prefix("--coverage=") {
            *enabled = true;
            match fmt {
                "lcov" => *format = CoverageFormat::Lcov,
                "cobertura" => *format = CoverageFormat::Cobertura,
                other => eprintln!("qn: unsupported coverage format '{other}', using lcov"),
            }
        } else if let Some(path) = a.strip_prefix("--coverage-out=") {
            *enabled = true;
            *out = Some(path.to_string());
        } else {
            vm_args.push(a.clone());
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmRunnerMode {
    Highlight,
    Test,
    Benchmark,
    Run,
    Repl,
    /// `qn -e '<expr>'`: evaluate one expression and print its result. The expression source
    /// is carried in `VmRunnerOptions::target_path`.
    Eval,
    /// `qn debug <file>`: run a program under the interactive debugger. The path is carried in
    /// `VmRunnerOptions::target_path`.
    Debug,
}

impl VmRunnerOptions {
    pub fn parse(args: &[String]) -> Self {
        let mut mode = VmRunnerMode::Run;
        let mut target_path = None;
        let mut vm_args = Vec::new();
        let mut break_on_throw = Vec::new();
        let mut break_on_uncaught = Vec::new();
        let mut dap = false;
        let mut coverage_enabled = false;
        let mut coverage_out = None;
        let mut coverage_format = crate::coverage::CoverageFormat::Lcov;

        if let Some(arg) = args.get(1) {
            if arg == "highlight" {
                mode = VmRunnerMode::Highlight;
                target_path = args.get(2).cloned();
                if args.len() > 3 {
                    vm_args = args[3..].to_vec();
                }
            } else if arg == "test" {
                mode = VmRunnerMode::Test;
                take_coverage_flags(
                    &args[2..],
                    &mut vm_args,
                    &mut coverage_enabled,
                    &mut coverage_out,
                    &mut coverage_format,
                );
            } else if arg == "benchmark" {
                mode = VmRunnerMode::Benchmark;
                if args.len() > 2 {
                    vm_args = args[2..].to_vec();
                }
            } else if arg == "repl" {
                mode = VmRunnerMode::Repl;
                if args.len() > 2 {
                    vm_args = args[2..].to_vec();
                }
            } else if arg == "-e" {
                // `qn -e '<expr>'`: the next arg is the expression source; anything after it
                // is passed through as VM arguments.
                mode = VmRunnerMode::Eval;
                target_path = args.get(2).cloned();
                if args.len() > 3 {
                    vm_args = args[3..].to_vec();
                }
            } else if arg == "debug" {
                // `qn debug [--break-on-throw=Type,…] <file> [vm-args…]`: run under the
                // interactive debugger. The first non-flag arg is the file; the rest pass through.
                mode = VmRunnerMode::Debug;
                for a in &args[2..] {
                    if let Some(types) = a.strip_prefix("--break-on-throw=") {
                        break_on_throw = types
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    } else if let Some(types) = a.strip_prefix("--break-on-uncaught=") {
                        break_on_uncaught = types
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    } else if a == "--dap" {
                        dap = true;
                    } else if target_path.is_none() {
                        target_path = Some(a.clone());
                    } else {
                        vm_args.push(a.clone());
                    }
                }
            } else {
                mode = VmRunnerMode::Run;
                target_path = Some(arg.clone());
                take_coverage_flags(
                    &args[2..],
                    &mut vm_args,
                    &mut coverage_enabled,
                    &mut coverage_out,
                    &mut coverage_format,
                );
            }
        }

        let coverage = coverage_enabled.then(|| crate::coverage::CoverageConfig {
            format: coverage_format,
            out: coverage_out,
        });

        // Interactive modes (REPL, debugger) colorize errors/output when stdout is a terminal.
        // DAP owns stdout (program output is sent as plain-text `output` events), so never there.
        let supports_color = !dap
            && matches!(mode, VmRunnerMode::Repl | VmRunnerMode::Debug)
            && std::io::stdout().is_terminal();

        Self {
            mode,
            target_path,
            vm_options: VmOptions {
                arguments: vm_args,
                supports_color,
                console_width: None,
            },
            break_on_throw,
            break_on_uncaught,
            dap,
            coverage,
        }
    }
}

pub struct VmRunner {
    options: VmRunnerOptions,
}

impl VmRunner {
    pub fn new(options: VmRunnerOptions) -> Self {
        Self { options }
    }

    pub fn run(&self) -> Result<(), QuoinError> {
        match self.options.mode {
            VmRunnerMode::Highlight => {
                let Some(ref path) = self.options.target_path else {
                    eprintln!("Usage: cargo run -- highlight FILE");
                    exit(2);
                };
                let source = match read_to_string(path) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Error reading {}: {}", path, e);
                        exit(1);
                    }
                };
                print!("{}", highlight_to_ansi(&source));
                Ok(())
            }
            VmRunnerMode::Test => {
                // prelude, then the test entry — main.qn `use`s the framework + suites.
                let ast_iter = prelude_asts().chain(once_with(|| {
                    parse_quoin_file(&PathBuf::from("qnlib/main.qn"))
                }));

                if !self.compile_and_run_asts(ast_iter) {
                    exit(1);
                }
                Ok(())
            }
            VmRunnerMode::Benchmark => {
                let ast_iter = prelude_asts().chain(once_with(|| {
                    parse_quoin_file(&PathBuf::from("qnlib/benchmark.qn"))
                }));

                self.compile_and_benchmark(ast_iter);
                Ok(())
            }
            VmRunnerMode::Run => {
                let script_path = self
                    .options
                    .target_path
                    .clone()
                    .unwrap_or_else(|| "qnlib/testscript.qn".to_string());
                let ast_iter = prelude_asts().chain(once_with(move || {
                    parse_quoin_file(&PathBuf::from(&script_path))
                }));

                self.compile_and_run_asts(ast_iter);
                Ok(())
            }
            VmRunnerMode::Repl => {
                self.run_repl();
                Ok(())
            }
            VmRunnerMode::Eval => {
                let Some(ref expr) = self.options.target_path else {
                    eprintln!("Usage: qn -e '<expr>'");
                    exit(2);
                };
                self.run_eval(expr);
                Ok(())
            }
            VmRunnerMode::Debug => {
                let Some(ref path) = self.options.target_path else {
                    eprintln!("Usage: qn debug FILE");
                    exit(2);
                };
                if self.options.dap {
                    self.run_dap(path);
                } else {
                    self.run_debug(path);
                }
                Ok(())
            }
        }
    }

    /// `qn debug <file>`: run a program under the interactive debugger. The prelude loads
    /// undebugged (via `build_repl_arena`); then the program is installed as task #0 with an
    /// `interactive` debug session armed to stop at the first line ("stop at entry"), and
    /// driven through `drive_main_task` — whose `DebugPaused` handler runs the `$`-command
    /// loop. See `src/debug_cli.rs`.
    fn run_debug(&self, path: &str) {
        use crate::debug::{DebugState, StepMode};

        let source = match read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading {path}: {e}");
                exit(1);
            }
        };
        let node = match try_parse_quoin_string_named(&source, path) {
            Ok(n) => n,
            Err(pe) => {
                eprintln!(
                    "Parse error at {}:{}:{}: {}",
                    path,
                    pe.line,
                    pe.column + 1,
                    pe.message
                );
                exit(1);
            }
        };
        let Some(mut arena) = self.build_repl_arena() else {
            exit(1);
        };
        let installed = arena.mutate_root(|mc, vm| {
            let NodeValue::Program(p) = &node.value else {
                return false;
            };
            let sb = match Compiler::new().compile_program(p) {
                Ok(sb) => sb,
                Err(e) => {
                    eprintln!("Compile error: {e}");
                    return false;
                }
            };
            let block = build_block(mc, &sb);
            // Stop at entry: an armed `StepInto` halts at the first line start. Source is
            // shown at each pause by default ($source off to silence). `--break-on-throw` types
            // additionally pause at a matching throw; `--break-on-uncaught` only when it escapes.
            vm.debug = Some(DebugState {
                interactive: true,
                show_source: true,
                step: Some(StepMode::Into),
                break_on_throw: self.options.break_on_throw.iter().cloned().collect(),
                break_on_uncaught: self.options.break_on_uncaught.iter().cloned().collect(),
                ..Default::default()
            });
            vm.start_block(mc, block, Vec::new(), None, None);
            install_main_task(mc, vm);
            true
        });
        if !installed {
            exit(1);
        }
        println!("Quoin debugger — $help for commands, $continue to run, $quit to exit.");
        if let Err(e) = drive_main_task(&mut arena) {
            eprintln!("VM execution error: {e}");
        }
    }

    /// `qn debug --dap <file>`: drive the program as a DAP adapter over stdio. Mirrors
    /// [`run_debug`] but speaks the Debug Adapter Protocol instead of the `$`-command loop — the
    /// debuggee's output is rerouted to DAP `output` events and stdout is reserved for the protocol
    /// stream (a stray Rust print can't corrupt it; see `src/dap.rs`).
    fn run_dap(&self, path: &str) {
        use crate::debug::DebugState;

        // Reserve stdout for the protocol BEFORE anything can print; stray prints go to stderr.
        let protocol = match crate::dap::redirect_protocol_stdout() {
            Ok(f) => f,
            Err(e) => {
                eprintln!("dap: could not redirect stdout: {e}");
                exit(1);
            }
        };
        let source = match read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("dap: error reading {path}: {e}");
                exit(1);
            }
        };
        let node = match try_parse_quoin_string_named(&source, path) {
            Ok(n) => n,
            Err(pe) => {
                eprintln!(
                    "dap: parse error at {}:{}:{}: {}",
                    path,
                    pe.line,
                    pe.column + 1,
                    pe.message
                );
                exit(1);
            }
        };
        let Some(mut arena) = self.build_repl_arena() else {
            exit(1);
        };
        let installed = arena.mutate_root(|mc, vm| {
            let NodeValue::Program(p) = &node.value else {
                return false;
            };
            let sb = match Compiler::new().compile_program(p) {
                Ok(sb) => sb,
                Err(e) => {
                    eprintln!("dap: compile error: {e}");
                    return false;
                }
            };
            let block = build_block(mc, &sb);
            // Run to a breakpoint (`stopOnEntry` is honored at `launch`). Program output is
            // captured and re-emitted as DAP `output` events rather than written to fd 1/2.
            vm.debug = Some(DebugState {
                // `interactive` = "bubble a pause up to the driver frontend" (here, the DAP
                // adapter's `on_pause`) rather than auto-applying the scripted action in place.
                interactive: true,
                show_source: false,
                step: None,
                break_on_throw: self.options.break_on_throw.iter().cloned().collect(),
                break_on_uncaught: self.options.break_on_uncaught.iter().cloned().collect(),
                ..Default::default()
            });
            vm.capture_output = true;
            vm.start_block(mc, block, Vec::new(), None, None);
            install_main_task(mc, vm);
            true
        });
        if !installed {
            exit(1);
        }
        let conn = crate::dap::Connection::new(std::io::BufReader::new(std::io::stdin()), protocol);
        let mut frontend = DapFrontend::new(conn);
        if let Err(e) = drive_with_frontend(&mut arena, &mut frontend) {
            // stdout is the protocol channel — log to stderr.
            eprintln!("dap: VM error: {e}");
        }
    }

    /// Interactive read-eval-print loop (`qn repl`). One VM is built and the prelude
    /// loaded; a persistent `repl_env` holds top-level bindings so they survive across
    /// lines. Each input is parsed without panicking (`try_parse`), compiled, and run in
    /// that env. Incomplete input re-prompts (`... `); parse/compile/runtime failures are
    /// shown and the loop continues. A line starting with `$` is a REPL command.
    fn run_repl(&self) {
        let Some(mut arena) = self.build_repl_arena() else {
            return;
        };

        // Interactive terminals get the line editor; piped/redirected stdin uses the
        // promptless accumulation loop (rustyline's editor/validator only apply to a tty).
        if stdin().is_terminal() {
            // `~/.quoinrc` is interactive-only, like a shell rc file (a piped script or a
            // one-shot `-e` doesn't run it).
            load_quoinrc(&mut arena);
            run_repl_interactive(&mut arena);
        } else {
            run_repl_piped(&mut arena);
        }
    }

    /// Build a fresh session arena: a `VmState` with the native builtins registered, the core
    /// stdlib prelude loaded, and an empty persistent `repl_env`. Returns `None` (after
    /// printing the failure) if the prelude fails to load. Shared by `run_repl` and `run_eval`.
    fn build_repl_arena(&self) -> Option<ReplArena> {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, self.options.vm_options.clone());
            register_builtins(mc, &mut vm);
            vm
        });

        // Load the core stdlib into the persistent VM (prelude `use core/*`).
        for ast in prelude_asts() {
            let mut failed = false;
            arena.mutate_root(|mc, vm| {
                let NodeValue::Program(p) = &ast.value else {
                    return;
                };
                match Compiler::new().compile_program(p) {
                    Ok(sb) => {
                        let block = build_block(mc, &sb);
                        if let Err(e) = vm.execute_block(mc, block, Vec::new(), None) {
                            eprintln!("repl: failed to load prelude: {}", e);
                            failed = true;
                        }
                    }
                    Err(e) => {
                        eprintln!("repl: prelude compile error: {}", e);
                        failed = true;
                    }
                }
            });
            if failed {
                return None;
            }
        }

        // The session's persistent top-level environment.
        arena.mutate_root(|mc, vm| {
            vm.repl_env = Some(gcl!(mc, EnvFrame::new(None)));
        });

        Some(arena)
    }

    /// `qn -e '<expr>'`: evaluate one expression in a fresh prelude-loaded session and print
    /// its result via `.s` (a `nil` result prints nothing). Parse/compile/runtime errors go to
    /// stderr with a non-zero exit, so `-e` composes in pipelines.
    fn run_eval(&self, expr: &str) {
        let Some(mut arena) = self.build_repl_arena() else {
            exit(1);
        };
        match eval_once(&mut arena, expr) {
            Ok(Some(out)) => println!("{out}"),
            Ok(None) => {}
            Err(msg) => {
                eprintln!("{msg}");
                exit(1);
            }
        }
    }

    /// Runs each program AST in turn. Returns `true` if the run completed without a
    /// VM error and the last program's result value was truthy. For `qn test` that
    /// last value is main.qn's `results.none?:{…}` boolean (true iff every suite
    /// passed), so the Test driver can gate the process exit code on it.
    fn compile_and_run_asts(&self, ast_iter: impl Iterator<Item = Node>) -> bool {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, self.options.vm_options.clone());
            register_builtins(mc, &mut vm);
            // Attach the coverage collector before any user code runs, so every
            // line-start crossing from here on is recorded.
            if self.options.coverage.is_some() {
                vm.coverage = Some(crate::coverage::CoverageState::new());
            }
            vm
        });

        let mut aborted = false;
        for ast in ast_iter {
            if aborted {
                break;
            }

            arena.mutate_root(|mc, vm| {
                let program_node = match &ast.value {
                    NodeValue::Program(p) => p,
                    _ => {
                        panic!("Error: Root AST node is not a ProgramNode");
                    }
                };

                let mut compiler = Compiler::new();
                let program = match compiler.compile_program(program_node) {
                    Ok(p) => p,
                    Err(e) => {
                        panic!("Compilation error: {}", e);
                    }
                };

                let decl_block = program.decl_block.as_ref().map(|db| {
                    gc!(
                        mc,
                        Block {
                            name: db.name.clone(),
                            is_nested_block: db.is_nested_block,
                            param_syms: db.param_syms.clone(),
                            param_types: db.param_types.clone(),
                            bytecode: db.bytecode.clone(),
                            parent_env: None,
                            enclosing_method_id: None,
                            source_info: db.source_info.clone(),
                            decl_block: None,
                            source_map: db.source_map.clone(),
                        }
                    )
                });
                let main_block = gc!(
                    mc,
                    Block {
                        name: program.name.clone(),
                        is_nested_block: program.is_nested_block,
                        param_syms: program.param_syms.clone(),
                        param_types: program.param_types.clone(),
                        bytecode: program.bytecode.clone(),
                        parent_env: None,
                        enclosing_method_id: None,
                        source_info: program.source_info.clone(),
                        decl_block,
                        source_map: program.source_map.clone(),
                    }
                );
                vm.start_block(mc, main_block, Vec::new(), None, None);
                // Run this program unit as scheduler task #0; driven to completion below.
                install_main_task(mc, vm);
            });

            // Drive the unit to completion through the shared scheduler (async I/O, sleep,
            // tasks, fibers). An error aborts the remaining ASTs (and fails a test run).
            if let Err(e) = drive_main_task(&mut arena) {
                eprintln!("VM execution error: {}", e);
                aborted = true;
            }
        }

        // Emit coverage after the run (all classes are loaded, all hits recorded),
        // regardless of pass/fail — a failing suite still wants its coverage.
        if let Some(cfg) = self.options.coverage.clone() {
            let output = arena.mutate_root(|_mc, vm| {
                let report = vm.build_coverage_report();
                let (found, hit) = report.line_totals();
                eprintln!(
                    "coverage: {hit}/{found} lines ({:.1}%)",
                    if found == 0 {
                        100.0
                    } else {
                        100.0 * hit as f64 / found as f64
                    }
                );
                match cfg.format {
                    crate::coverage::CoverageFormat::Lcov => crate::coverage::to_lcov(&report),
                    crate::coverage::CoverageFormat::Cobertura => {
                        crate::coverage::to_cobertura(&report)
                    }
                }
            });
            match &cfg.out {
                Some(path) => {
                    if let Err(e) = std::fs::write(path, &output) {
                        eprintln!("qn: failed to write coverage to {path}: {e}");
                    } else {
                        eprintln!("coverage written to {path}");
                    }
                }
                None => print!("{output}"),
            }
        }

        // The last program run leaves its result on top of the stack. Treat a VM
        // error (abort) as failure too, so callers can gate purely on the return.
        let passed = !aborted
            && arena.mutate_root(|_mc, vm| vm.stack.last().map(|v| v.is_truthy()).unwrap_or(false));

        arena.finish_cycle();
        passed
    }

    fn run_benchmark_iteration(
        &self,
        arena: &mut Arena<Rootable![VmState<'_>]>,
        receiver_name: &str,
        selector: &str,
        arg_ints: Vec<i64>,
    ) -> (u128, usize, usize) {
        let _initial_frame_count = arena.mutate_root(|mc, vm| {
            let receiver = vm
                .globals
                .borrow()
                .get(&NamespacedName::parse(receiver_name))
                .copied()
                .unwrap_or_else(|| panic!("{} not found", receiver_name));
            let args = arg_ints
                .iter()
                .map(|&i| vm.new_int(mc, i))
                .collect::<Vec<_>>();
            vm.start_method_call(mc, receiver, selector, args)
                .unwrap_or_else(|e| panic!("failed to start {}.{}: {}", receiver_name, selector, e))
        });

        arena.mutate_root(|mc, vm| {
            let fiber = Fiber::new(move |yielder, mut ctx| {
                let (vm, _mc) = unsafe { ctx.get() };
                vm.sched.yielder = Some(yielder as *const _ as *const ());

                loop {
                    let (vm, _mc) = unsafe { ctx.get() };
                    match vm.step(_mc) {
                        Ok(VmStatus::Running) => {
                            vm.sched.yielder = None;
                            ctx = yielder.suspend(YieldReason::CooperativeYield);
                            let (vm, _mc) = unsafe { ctx.get() };
                            vm.sched.yielder = Some(yielder as *const _ as *const ());
                        }
                        Ok(VmStatus::Finished(val)) => {
                            vm.sched.yielder = None;
                            return Ok(val);
                        }
                        Ok(VmStatus::Yeeted(val)) => {
                            vm.sched.yielder = None;
                            return Err(QuoinError::Other(format!("Uncaught exception: {}", val)));
                        }
                        Err(err) => {
                            vm.sched.yielder = None;
                            return Err(err);
                        }
                    }
                }
            });
            vm.sched.active_fiber = Some(gc!(mc, fiber));
        });

        let alloc_before = arena.mutate_root(|mc, _| mc.metrics().total_gc_allocation());
        let start_time = Instant::now();

        let mut step_count = 0;
        loop {
            let is_done = arena.mutate_root(|mc, vm| {
                let Some(fiber) = vm.sched.active_fiber else {
                    return Ok(true);
                };

                let mut opt = fiber.coroutine.borrow_mut();
                let coro = opt.as_mut().expect("Coroutine already finished");

                let ctx = VMContext {
                    vm: vm as *mut _,
                    mc: mc as *const _,
                };

                match coro.resume(ctx) {
                    CoroutineResult::Yield(YieldReason::CooperativeYield) => Ok(false),
                    CoroutineResult::Yield(YieldReason::CallBlock { .. }) => Ok(false),
                    // Guest fibers are not used by the benchmark harness.
                    CoroutineResult::Yield(YieldReason::ResumeFiber { .. })
                    | CoroutineResult::Yield(YieldReason::YieldFiber { .. }) => {
                        panic!("guest fibers are not supported in benchmark mode")
                    }
                    CoroutineResult::Yield(YieldReason::AwaitIo { .. })
                    | CoroutineResult::Yield(YieldReason::Gather { .. })
                    | CoroutineResult::Yield(YieldReason::Join { .. })
                    | CoroutineResult::Yield(YieldReason::JoinTimed { .. })
                    | CoroutineResult::Yield(YieldReason::ChannelPark)
                    | CoroutineResult::Yield(YieldReason::DebugBreak) => {
                        panic!("async I/O is not supported in benchmark mode")
                    }
                    CoroutineResult::Yield(YieldReason::Return(val)) => {
                        vm.sched.active_fiber = None;
                        vm.push(val);
                        Ok(true)
                    }
                    CoroutineResult::Return(res) => {
                        vm.sched.active_fiber = None;
                        match res {
                            Ok(val) => {
                                vm.push(val);
                                Ok(true)
                            }
                            Err(err) => Err(err),
                        }
                    }
                }
            });

            match is_done {
                Ok(true) => break,
                Ok(false) => {
                    step_count += 1;
                    if crate::tuning::gc_stress() || step_count % 10 == 0 {
                        arena.collect_debt();
                    }
                }
                Err(e) => {
                    panic!("VM execution error: {}", e);
                }
            }
        }

        let elapsed = start_time.elapsed().as_millis();

        arena.mutate_root(|_mc, vm| {
            let _ = vm.pop().expect("Failed to pop benchmark result");
        });

        let alloc_after = arena.mutate_root(|mc, _| mc.metrics().total_gc_allocation());

        (elapsed, alloc_before, alloc_after)
    }

    fn compile_and_benchmark(&self, ast_iter: impl Iterator<Item = Node>) {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, self.options.vm_options.clone());
            register_builtins(mc, &mut vm);
            vm
        });

        let mut aborted = false;
        for ast in ast_iter {
            if aborted {
                break;
            }

            arena.mutate_root(|mc, vm| {
                let program_node = match &ast.value {
                    NodeValue::Program(p) => p,
                    _ => {
                        panic!("Error: Root AST node is not a ProgramNode");
                    }
                };

                let mut compiler = Compiler::new();
                let program = match compiler.compile_program(program_node) {
                    Ok(p) => p,
                    Err(e) => {
                        panic!("Compilation error: {}", e);
                    }
                };

                let decl_block = program.decl_block.as_ref().map(|db| {
                    gc!(
                        mc,
                        Block {
                            name: db.name.clone(),
                            is_nested_block: db.is_nested_block,
                            param_syms: db.param_syms.clone(),
                            param_types: db.param_types.clone(),
                            bytecode: db.bytecode.clone(),
                            parent_env: None,
                            enclosing_method_id: None,
                            source_info: db.source_info.clone(),
                            decl_block: None,
                            source_map: db.source_map.clone(),
                        }
                    )
                });
                let main_block = gc!(
                    mc,
                    Block {
                        name: program.name.clone(),
                        is_nested_block: program.is_nested_block,
                        param_syms: program.param_syms.clone(),
                        param_types: program.param_types.clone(),
                        bytecode: program.bytecode.clone(),
                        parent_env: None,
                        enclosing_method_id: None,
                        source_info: program.source_info.clone(),
                        decl_block,
                        source_map: program.source_map.clone(),
                    }
                );
                vm.start_block(mc, main_block, Vec::new(), None, None);
            });

            let mut step_count = 0;
            loop {
                let status = arena.mutate_root(|mc, vm| match vm.step(mc) {
                    Ok(VmStatus::Running) => Ok(ExecutionStatus::Running),
                    Ok(VmStatus::Finished(_)) => Ok(ExecutionStatus::Finished),
                    Ok(VmStatus::Yeeted(val)) => {
                        println!("VM execution terminated with uncaught exception: {}", val);
                        Ok(ExecutionStatus::Yeeted)
                    }
                    Err(e) => Err(e),
                });
                match status {
                    Ok(ExecutionStatus::Running) => {
                        step_count += 1;
                        if crate::tuning::gc_stress() || step_count % 10 == 0 {
                            arena.collect_debt();
                        }
                    }
                    Ok(ExecutionStatus::Finished) => {
                        break;
                    }
                    Ok(ExecutionStatus::Yeeted) => {
                        aborted = true;
                        break;
                    }
                    Err(e) => {
                        eprintln!("VM execution error: {}", e);
                        aborted = true;
                        break;
                    }
                }
            }
        }

        if aborted {
            println!("Initialization aborted. Cannot run benchmarks.");
            return;
        }

        println!("==================================================");
        println!("RUST-TIMED BENCHMARK RUNNER (WITH GC)");
        println!("==================================================");

        let benchmarks = vec![
            ("Fibonacci (n = 20)", "Fib", "value:", vec![20]),
            (
                "Sieve of Eratosthenes (limit = 10000)",
                "Sieve",
                "primesUpTo:",
                vec![10000],
            ),
            (
                "Binary Trees (depth = 10)",
                "TreeBenchmark",
                "run:",
                vec![10],
            ),
        ];

        let mut averages = Vec::new();

        for &(name, receiver_name, selector, ref args) in &benchmarks {
            println!("Running: {}", name);
            let mut total_time = 0;
            let mut initial_alloc = 0;
            let mut final_alloc = 0;
            for iter in 1..=2 {
                let (elapsed, alloc_before, alloc_after) =
                    self.run_benchmark_iteration(&mut arena, receiver_name, selector, args.clone());
                if iter == 1 {
                    initial_alloc = alloc_before;
                }
                if iter == 2 {
                    final_alloc = alloc_after;
                }
                println!(
                    "  Iteration {}: {} ms (Heap: {} KB -> {} KB)",
                    iter,
                    elapsed,
                    alloc_before / 1024,
                    alloc_after / 1024
                );
                total_time += elapsed;
            }
            let avg = total_time / 2;
            averages.push((name, avg));
            println!("  Average: {} ms", avg);
            println!(
                "  Heap delta over iterations: {} KB -> {} KB (difference: {} KB)",
                initial_alloc / 1024,
                final_alloc / 1024,
                (final_alloc as i64 - initial_alloc as i64) / 1024
            );
            println!("--------------------------------------------------");
        }

        println!();
        println!("==================================================");
        println!("BENCHMARK SUMMARY (RUST-TIMED)");
        println!("==================================================");
        for &(name, avg) in &averages {
            println!("{:<38} {} ms", name.to_string() + ":", avg);
        }
        println!("==================================================");

        arena.finish_cycle();
    }
}

#[cfg(test)]
mod tests {
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
            vm.debug = Some(DebugState {
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
            vm.debug
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
            vm.debug = Some(DebugState {
                breakpoints: bps,
                interactive: true, // bubble pauses to the DAP frontend
                ..Default::default()
            });
            vm.capture_output = true;
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
}
