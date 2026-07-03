//! The interactive REPL (`qn repl` / `qn -e`): the rustyline editor integration, `$`-introspection
//! commands, scheduled-line evaluation, and value/inspection formatting. Split out of `runner.rs`.

use super::runner_driver::{drive_main_task, install_main_task};
use super::*;
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
        let mut compiler = Compiler::new_with_locals(repl_locals(vm));
        // Share the session's class accumulator so a class defined on an earlier line (and the
        // prelude's classes) are known here, then surface any type warnings for this line.
        compiler.set_seen_types(vm.options.seen_types.clone());
        let sb = match compiler.compile_program(p) {
            Ok(sb) => sb,
            Err(e) => return Setup::CompileErr(format!("Compile error: {e}")),
        };
        crate::compiler::report_type_warnings(compiler.diagnostics());
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
pub(crate) fn eval_once(arena: &mut ReplArena, input: &str) -> Result<Option<String>, String> {
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
pub(crate) fn load_quoinrc(arena: &mut ReplArena) {
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
pub(crate) fn flow_names(names: &[&str], width: usize) -> String {
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
pub(crate) fn run_repl_interactive(arena: &mut ReplArena) {
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
pub(crate) fn run_repl_piped(arena: &mut ReplArena) {
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
