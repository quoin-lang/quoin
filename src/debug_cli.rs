//! Interactive `$`-command frontend for the debugger (`qn debug <file>`).
//!
//! When a paused session is `interactive`, the driver (`drive_main_task`) bubbles a
//! `DebugBreak` up as `RunStep::DebugPaused` and runs a command loop here: it reads a line
//! with [`DebugFrontend`] (outside the GC arena, so history/editor state persists), then
//! executes it against the live paused VM via [`exec_command`] (inside `mutate_root`). This
//! mirrors the REPL's split — line I/O outside the arena, evaluation inside it.
//!
//! Commands: control + breakpoints (`$continue`/`$step`/`$next`/`$finish`/`$break`/`$delete`/
//! `$quit`, Slice 3a); inspection + source (`$frames`/`$up`/`$down`/`$locals`/`$list`/
//! `$source`, 3b); and `$print` / bare-expression eval-in-frame (3c). See
//! `docs/DEBUGGER_ARCH.md`.

use crate::debug::DebugAction;
use crate::vm::VmState;

use gc_arena::Mutation;
use std::path::PathBuf;

/// What a command line at a pause asks the driver to do next.
pub(crate) enum CommandOutcome {
    /// Keep prompting (a query ran, a breakpoint was set, or input was invalid).
    Stay,
    /// Resume execution — a continue/step verb has been applied to `DebugState`.
    Resume,
    /// Stop debugging and end the program.
    Quit,
}

/// The persistent line editor for a debug session — held by the driver across pauses (so
/// history survives) and built lazily on the first pause. Saves history on drop.
pub(crate) struct DebugFrontend {
    editor: rustyline::DefaultEditor,
    history: Option<PathBuf>,
}

impl DebugFrontend {
    pub(crate) fn new() -> Option<Self> {
        let mut editor = match rustyline::DefaultEditor::new() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("debug: failed to start line editor: {e}");
                return None;
            }
        };
        let history =
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".quoin_debug_history"));
        if let Some(p) = &history {
            let _ = editor.load_history(p);
        }
        Some(Self { editor, history })
    }

    /// Read one command line, recording non-blank input in history. Propagates the rustyline
    /// signals (`Interrupted` = Ctrl-C, `Eof` = Ctrl-D) for the caller to interpret.
    pub(crate) fn readline(&mut self) -> rustyline::Result<String> {
        let line = self.editor.readline("$ ")?;
        if !line.trim().is_empty() {
            let _ = self.editor.add_history_entry(line.as_str());
        }
        Ok(line)
    }
}

impl Drop for DebugFrontend {
    fn drop(&mut self) {
        if let Some(p) = &self.history {
            let _ = self.editor.save_history(p);
        }
    }
}

/// Begin a pause: reset the focus to the top frame, print where we stopped, and (when
/// `$source` is on) the surrounding source.
pub(crate) fn announce_pause(vm: &mut VmState<'_>) {
    vm.debug_enter_pause();
    // If a break-on-throw fired, say what was thrown.
    if let Some(banner) = vm.debug.as_mut().and_then(|d| d.pause_throw.take()) {
        println!("{banner}");
    }
    match vm.debug_focus().and_then(|f| vm.debug_frame_location(f)) {
        Some((file, line, label)) => println!("→ paused at {file}:{line}  (in {label})"),
        None => println!("→ paused (no source location)"),
    }
    if vm.debug.as_ref().is_some_and(|d| d.show_source)
        && let Some(win) = vm.debug_focus().and_then(|f| vm.debug_source_window(f, 2))
    {
        print!("{win}");
    }
}

/// Print the focused frame's header (and source, if enabled) — after `$up`/`$down`.
fn print_focus(vm: &VmState<'_>) {
    let Some(f) = vm.debug_focus() else { return };
    if let Some((file, line, label)) = vm.debug_frame_location(f) {
        println!("#{f}  {file}:{line}  (in {label})");
    }
    if vm.debug.as_ref().is_some_and(|d| d.show_source)
        && let Some(win) = vm.debug_source_window(f, 2)
    {
        print!("{win}");
    }
}

/// Execute one debugger command line against the paused VM, returning what the driver should
/// do next. A leading `$` is a meta-command; anything else is an expression to evaluate in the
/// focus frame.
pub(crate) fn exec_command<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    line: &str,
) -> CommandOutcome {
    let line = line.trim();
    if line.is_empty() {
        return CommandOutcome::Stay;
    }
    let Some(rest) = line.strip_prefix('$') else {
        print_expr(vm, mc, line);
        return CommandOutcome::Stay;
    };
    let verb = rest.split_whitespace().next().unwrap_or("");
    let arg = rest.split_whitespace().nth(1);
    // Everything after the verb, untrimmed of internal spaces — for `$print <expr>`.
    let rest_args = rest[verb.len()..].trim();
    match verb {
        "continue" | "c" => {
            vm.apply_debug_action(DebugAction::Continue);
            CommandOutcome::Resume
        }
        "step" | "s" => {
            vm.apply_debug_action(DebugAction::StepInto);
            CommandOutcome::Resume
        }
        "next" | "n" => {
            vm.apply_debug_action(DebugAction::StepOver);
            CommandOutcome::Resume
        }
        "finish" | "fin" => {
            vm.apply_debug_action(DebugAction::StepOut);
            CommandOutcome::Resume
        }
        "break" | "b" => {
            set_breakpoint(vm, arg, true);
            CommandOutcome::Stay
        }
        "delete" | "d" => {
            set_breakpoint(vm, arg, false);
            CommandOutcome::Stay
        }
        "frames" | "bt" => {
            for line in vm.debug_backtrace() {
                println!("{line}");
            }
            CommandOutcome::Stay
        }
        "up" => {
            vm.debug_move_focus(-1);
            print_focus(vm);
            CommandOutcome::Stay
        }
        "down" => {
            vm.debug_move_focus(1);
            print_focus(vm);
            CommandOutcome::Stay
        }
        "locals" | "l" => {
            match vm.debug_focus() {
                Some(f) => {
                    let locals = vm.debug_locals(f);
                    if locals.is_empty() {
                        println!("(no locals)");
                    }
                    for (name, value) in locals {
                        println!("  {name} = {value}");
                    }
                }
                None => println!("(no frame)"),
            }
            CommandOutcome::Stay
        }
        "list" => {
            match vm.debug_focus().and_then(|f| vm.debug_source_window(f, 5)) {
                Some(win) => print!("{win}"),
                None => println!("(source unavailable)"),
            }
            CommandOutcome::Stay
        }
        "source" => {
            match arg {
                Some("on") | Some("off") => {
                    let on = arg == Some("on");
                    if let Some(d) = vm.debug.as_mut() {
                        d.show_source = on;
                    }
                    println!("source display {}", if on { "on" } else { "off" });
                }
                _ => println!("usage: $source on|off"),
            }
            CommandOutcome::Stay
        }
        "print" | "p" => {
            if rest_args.is_empty() {
                println!("usage: $print <expr>");
            } else {
                print_expr(vm, mc, rest_args);
            }
            CommandOutcome::Stay
        }
        "quit" | "q" => CommandOutcome::Quit,
        "help" => {
            print_help();
            CommandOutcome::Stay
        }
        other => {
            println!("unknown command: ${other}  ($help for the list)");
            CommandOutcome::Stay
        }
    }
}

/// Show `expr` evaluated in the focus frame. A bare local / `@ivar` is read directly (a
/// side-effect-free fast path); any other expression is evaluated with the frame's `self` bound
/// and its locals seeded as bindings, so compound expressions over `self`/`@ivars`/locals (e.g.
/// `@total + n`) resolve.
fn print_expr<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>, expr: &str) {
    let expr = expr.trim();
    let focus = vm.debug_focus();
    // Fast path for a bare local / `@ivar`: read it directly rather than re-entering the VM to
    // eval (the eval path below would resolve these too, but a direct read has no side effects).
    if is_bare_ident(expr)
        && let Some(val) = focus.and_then(|f| vm.debug_lookup_local(f, expr))
    {
        println!("{}", vm.debug_render(val));
        return;
    }
    if let Some(ivar) = expr.strip_prefix('@')
        && is_bare_ident(ivar)
        && let Some(val) = focus.and_then(|f| vm.debug_lookup_ivar(f, ivar))
    {
        println!("{}", vm.debug_render(val));
        return;
    }
    // Any other expression: evaluate with the frame's `self` bound and its locals seeded as
    // bindings, so `n + 1`, `@total + step`, `self.method`, etc. all resolve.
    let self_val = focus.and_then(|f| vm.frames.get(f).and_then(|fr| fr.receiver));
    let bindings = focus
        .map(|f| vm.debug_frame_bindings(f))
        .unwrap_or_default();
    match vm.debug_eval(mc, expr, self_val, &bindings) {
        Ok(val) => println!("{}", vm.debug_render(val)),
        Err(msg) => println!("error: {msg}"),
    }
}

/// Whether `s` is a single bare identifier (so it can be looked up as a frame local).
fn is_bare_ident(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Add or remove a `(file, line)` breakpoint from `spec` — either `FILE:LINE`, or `LINE`
/// (the file of the current pause).
fn set_breakpoint(vm: &mut VmState<'_>, spec: Option<&str>, add: bool) {
    let Some(spec) = spec else {
        println!("usage: $break FILE:LINE  (or $break LINE for the current file)");
        return;
    };
    let (file, line) = match spec.rsplit_once(':') {
        Some((f, l)) => (f.to_string(), l.parse::<usize>().ok()),
        None => (
            vm.debug_current_pos().map(|(f, _)| f).unwrap_or_default(),
            spec.parse::<usize>().ok(),
        ),
    };
    let Some(line) = line else {
        println!("not a line number: {spec}");
        return;
    };
    if file.is_empty() {
        println!("no current file — give $break FILE:LINE");
        return;
    }
    if let Some(d) = vm.debug.as_mut() {
        if add {
            d.breakpoints.entry(file.clone()).or_default().insert(line);
            println!("breakpoint set at {file}:{line}");
        } else {
            let removed = d
                .breakpoints
                .get_mut(&file)
                .is_some_and(|lines| lines.remove(&line));
            println!(
                "{}",
                if removed {
                    format!("breakpoint cleared at {file}:{line}")
                } else {
                    format!("no breakpoint at {file}:{line}")
                }
            );
        }
    }
}

#[cfg(test)]
#[path = "debug_cli_tests.rs"]
mod tests;

fn print_help() {
    println!(
        "\
debugger commands:
  $continue, $c       resume execution
  $step, $s           step into the next line
  $next, $n           step over (skip calls)
  $finish, $fin       run until the current frame returns
  $break FILE:LINE    set a line breakpoint ($b; $break LINE = current file)
  $delete FILE:LINE   clear a breakpoint ($d)
  $frames, $bt        backtrace (innermost first); the focus frame is marked
  $up / $down         move the focus to the caller / callee frame
  $locals, $l         locals, self, and self's @ivars of the focus frame
  $list               source around the focus frame's current line
  $source on|off      auto-show source at each pause (default on)
  $print EXPR, $p     evaluate EXPR in the focus frame (or just type it)
  $quit, $q           stop debugging and exit
  $help               this list
$print evaluates EXPR in the focus frame — `self`, `@ivars`, and locals all resolve
(e.g. `@total + n`)."
    );
}
