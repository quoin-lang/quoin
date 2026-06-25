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
            // A breakpoint/step paused this task. Run the debugger command loop (v0: a stub
            // that records the pause and continues), then resume the *same* task in place —
            // no park, so the whole VM stays stopped while paused. The coroutine resumes
            // past the suspend point in `debug_checkpoint` and dispatches the instruction.
            vm.debug_on_pause();
            Ok(RunStep::Running)
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
fn drive_main_task(arena: &mut ReplArena) -> Result<(), QuoinError> {
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
                Ok(RunStep::Finished) => break,
                Err(e) => return Err(e),
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
}

impl VmRunnerOptions {
    pub fn parse(args: &[String]) -> Self {
        let mut mode = VmRunnerMode::Run;
        let mut target_path = None;
        let mut vm_args = Vec::new();

        if let Some(arg) = args.get(1) {
            if arg == "highlight" {
                mode = VmRunnerMode::Highlight;
                target_path = args.get(2).cloned();
                if args.len() > 3 {
                    vm_args = args[3..].to_vec();
                }
            } else if arg == "test" {
                mode = VmRunnerMode::Test;
                if args.len() > 2 {
                    vm_args = args[2..].to_vec();
                }
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
            } else {
                mode = VmRunnerMode::Run;
                target_path = Some(arg.clone());
                if args.len() > 2 {
                    vm_args = args[2..].to_vec();
                }
            }
        }

        // The REPL is interactive: colorize errors/output when stdout is a terminal.
        let supports_color = mode == VmRunnerMode::Repl && std::io::stdout().is_terminal();

        Self {
            mode,
            target_path,
            vm_options: VmOptions {
                arguments: vm_args,
                supports_color,
                console_width: None,
            },
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
    use crate::debug::DebugState;
    use std::collections::{HashMap, HashSet};

    /// Run `source` to completion under the real scheduler with a debug session attached
    /// (line `breakpoints` as `(file, line)`), returning the `pause_log` the v0 stub driver
    /// recorded. Exercises the full mechanism end-to-end: the step-loop hook fires, suspends
    /// via `DebugBreak`, the driver handles it, and the task resumes in place to completion.
    fn run_with_breakpoints(
        source: &str,
        filename: &str,
        breakpoints: &[(&str, usize)],
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
                ..Default::default()
            });
            vm.start_block(mc, block, Vec::new(), None, None);
            install_main_task(mc, vm);
        });
        drive_main_task(&mut arena).expect("fixture runs to completion");
        arena.mutate_root(|_mc, vm| {
            vm.debug
                .as_ref()
                .map(|d| d.pause_log.clone())
                .unwrap_or_default()
        })
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
        let log = run_with_breakpoints(source, "fixture.qn", &[("fixture.qn", 2)]);
        assert_eq!(
            log,
            vec![("fixture.qn".to_string(), 2), ("fixture.qn".to_string(), 2),],
        );
    }

    #[test]
    fn no_breakpoints_never_pauses() {
        let log = run_with_breakpoints("x = 1;\ny = 2;\nx + y\n", "fixture.qn", &[]);
        assert!(log.is_empty());
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
