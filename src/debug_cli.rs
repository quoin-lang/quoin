//! Interactive `$`-command frontend for the debugger (`qn debug <file>`), Slice 3a.
//!
//! When a paused session is `interactive`, the driver (`drive_main_task`) bubbles a
//! `DebugBreak` up as `RunStep::DebugPaused` and runs a command loop here: it reads a line
//! with [`DebugFrontend`] (outside the GC arena, so history/editor state persists), then
//! executes it against the live paused VM via [`exec_command`] (inside `mutate_root`). This
//! mirrors the REPL's split — line I/O outside the arena, evaluation inside it.
//!
//! Slice 3a covers control + breakpoints (`$continue`/`$step`/`$next`/`$finish`/`$break`/
//! `$delete`/`$quit`). Inspection (`$frames`/`$locals`/`$list`) and expression eval-in-frame
//! land in 3b/3c. See `docs/DEBUGGER_ARCH.md`.

use crate::debug::DebugAction;
use crate::vm::VmState;

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

/// Print where execution paused (the instruction the top frame is about to run).
pub(crate) fn announce_pause(vm: &VmState<'_>) {
    match vm.debug_current_pos() {
        Some((file, line)) => println!("→ paused at {file}:{line}"),
        None => println!("→ paused (no source location)"),
    }
}

/// Execute one debugger command line against the paused VM, returning what the driver should
/// do next. A leading `$` is a meta-command; anything else is (eventually) an expression to
/// evaluate in the current frame — not yet wired in 3a.
pub(crate) fn exec_command<'gc>(vm: &mut VmState<'gc>, line: &str) -> CommandOutcome {
    let line = line.trim();
    if line.is_empty() {
        return CommandOutcome::Stay;
    }
    let Some(rest) = line.strip_prefix('$') else {
        println!("expression evaluation lands in a later slice — use $-commands ($help).");
        return CommandOutcome::Stay;
    };
    let mut parts = rest.split_whitespace();
    let verb = parts.next().unwrap_or("");
    let arg = parts.next();
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
  $quit, $q           stop debugging and exit
  $help               this list
inspection ($frames/$locals/$list) and expression eval arrive in later slices."
    );
}
