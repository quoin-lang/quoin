use crate::compiler::Compiler;
use crate::error::QuoinError;
use crate::fiber::{Fiber, VMContext, YieldReason, run_vm_loop};
use crate::gc;
use crate::gcl;
use crate::highlighter::highlight_to_ansi;
use crate::introspect::{self, ClassInfo, GlobalInfo, GlobalKind, ValueInfo};
use crate::io_backend::{IoBackend, IoRequest, IoResult, StreamId};
use crate::parser::ast::Node;
use crate::parser::{NodeValue, parse_quoin_file, try_parse_quoin_string_named};
use crate::repl_complete::{CompletionIndex, build_completion_index, complete_input};
use crate::runtime::runtime::build_block;
use crate::runtime::{fiber as fiber_class, pretty};
use crate::value::{EnvFrame, NamespacedName, ObjectPayload, Value};
use crate::vm::{Task, TaskId, VmOptions, VmState, VmStatus, Wake};
use std::sync::Arc;

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
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, exit};
use std::sync::Once;
use std::time::Instant;

/// The prelude AST: the stdlib's `prelude` unit, whose `use core/*` loads the core
/// stdlib (00-bootstrap … 11-plan) in sorted order. Every runner mode loads this first,
/// so the prelude composition lives in Quoin rather than a hardcoded glob here. Read
/// from the embedded stdlib (or `QUOIN_STDLIB`), never from the CWD — an installed `qn`
/// has no `qnlib/` to find.
pub(crate) fn prelude_asts() -> impl Iterator<Item = Node> {
    once_with(|| {
        let source = crate::packages::read_stdlib_unit("prelude").unwrap_or_else(|| {
            eprintln!("qn: cannot load the stdlib prelude");
            exit(1);
        });
        parse_source_or_exit(&source, "prelude.qn")
    })
}

/// Parse Quoin source that did not come from a user-named file (the prelude, the
/// synthesized `qn test` entry). A failure here is a bug in the shipped stdlib rather
/// than user error, so it reports cleanly and exits rather than panicking.
fn parse_source_or_exit(source: &str, display: &str) -> Node {
    match try_parse_quoin_string_named(source, display) {
        Ok(node) => node,
        Err(e) => {
            eprintln!(
                "qn: parse error in {display} at line {}, col {}: {}",
                e.line,
                e.column + 1,
                e.message
            );
            exit(1);
        }
    }
}

// The builtin registry and the per-unit compile sequence live in `src/registry.rs` /
// `src/runner_core.rs` so the wasm build (which compiles this runner out) shares them;
// re-exported here so the `crate::runner::…` paths keep working.
pub(crate) use crate::registry::register_builtins;
pub(crate) use crate::runner_core::{compile_and_start, compile_unit_aot, unit_compiler};

/// The persistent REPL arena: one `VmState` kept alive across all lines.
pub(crate) type ReplArena = Arena<Rootable![VmState<'_>]>;

// `runner` is split across sibling `runner_*.rs` files, declared here as `#[path]` child modules so
// the public API (`VmRunner` / `VmRunnerOptions` / `register_builtins`) stays at `crate::runner::…`.
#[path = "runner_dap.rs"]
mod runner_dap;
#[path = "runner_doc.rs"]
mod runner_doc;
#[path = "runner_driver.rs"]
mod runner_driver;
#[path = "runner_repl.rs"]
mod runner_repl;

use runner_dap::{DapFrontend, PendingProgram};
use runner_driver::drive_with_frontend;
pub(crate) use runner_driver::{drive_main_task, install_main_task};
use runner_repl::{eval_once, load_quoinrc, run_repl_interactive, run_repl_piped};

/// Step status for the benchmark driver, which runs a single fiber to completion
/// with no async I/O (the run/test driver uses `RunStep` and the task scheduler).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutionStatus {
    Running,
    Finished,
    Yeeted,
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
    /// `qn fmt --check`: report unformatted files and exit non-zero instead of writing.
    pub fmt_check: bool,
    /// `qn fmt --dry-run`: print formatted source to stdout instead of rewriting in place.
    pub fmt_dry_run: bool,
    /// `qn fmt --diff`: show a unified diff of what would change, without writing.
    pub fmt_diff: bool,
    /// `qn doc --json`: also emit the raw model as `model.json`.
    pub doc_json: bool,
    /// `qn doc --coverage`: report undocumented classes/selectors instead of generating.
    pub doc_coverage: bool,
    /// `qn highlight --html`: emit a standalone HTML page instead of ANSI.
    pub highlight_html: bool,
    /// `qn doc --check`: run the documentation's fenced examples instead of generating.
    pub doc_check: bool,
    /// `qn doc --md`: render markdown paths to HTML pages with Quoin highlighting.
    pub doc_md: bool,
    /// `qn doc --stdlib` (hidden): document the shipped stdlib instead of a project.
    pub doc_stdlib: bool,
    /// `qn doc --stdlib-path PREFIX` (hidden): link prefix (relative path or URL) for
    /// stdlib types in project docs.
    pub doc_stdlib_path: Option<String>,
    /// `qn check --json` (hidden): emit diagnostics as a JSON array on stdout instead
    /// of rendered text — the machine contract the language server consumes.
    pub check_json: bool,
}

/// One `qn check --json` diagnostic — the machine-readable form of a checker warning,
/// compile error, or parse error. `line` is 1-based and `column` 0-based (the
/// `SourceInfo` convention, which is also the LSP's); `start`/`end` are byte offsets
/// into `file`, the authoritative span. Location fields are absent when a diagnostic
/// has no attributable site.
#[derive(serde::Serialize)]
struct CheckDiag {
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    /// `"error"` (parse / compile) or `"warning"` (checker diagnostics).
    severity: &'static str,
    /// The `WARNING_KINDS` slug, or `"parse-error"` / `"compile-error"`.
    kind: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<usize>,
    /// Why-chain notes (provenance), each at its own span.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<CheckNote>,
}

#[derive(serde::Serialize)]
struct CheckNote {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<usize>,
}

impl CheckDiag {
    fn new(
        severity: &'static str,
        kind: &str,
        message: String,
        span: Option<&quoin_syntax::SourceInfo>,
    ) -> Self {
        let mut d = CheckDiag {
            file: None,
            severity,
            kind: kind.to_string(),
            message,
            line: None,
            column: None,
            start: None,
            end: None,
            notes: Vec::new(),
        };
        if let Some(s) = span {
            d.file = Some(s.filename.clone());
            d.line = Some(s.line);
            d.column = Some(s.column);
            d.start = Some(s.start);
            d.end = Some(s.end);
        }
        d
    }

    fn from_warning(w: &crate::compiler::Diagnostic) -> Self {
        let mut d = Self::new("warning", w.kind, w.message.clone(), w.span.as_ref());
        d.notes = w
            .notes
            .iter()
            .map(|n| CheckNote {
                message: n.message.clone(),
                file: n.span.as_ref().map(|s| s.filename.clone()),
                line: n.span.as_ref().map(|s| s.line),
                column: n.span.as_ref().map(|s| s.column),
                start: n.span.as_ref().map(|s| s.start),
                end: n.span.as_ref().map(|s| s.end),
            })
            .collect();
        d
    }

    fn from_compile_error(e: &crate::compiler::CompileError) -> Self {
        Self::new("error", "compile-error", e.message.clone(), e.span.as_ref())
    }
}

/// Recursively collect `.qn` files under `dir`, in sorted order, skipping `target`/`.git`.
/// Used by `qn fmt <dir>`.
fn collect_qn_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "target" || name == ".git" {
                continue;
            }
            collect_qn_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("qn") {
            out.push(path);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmRunnerMode {
    /// `qn worker-serve <sock> <unit> [<serviceClass>]` — the child side of
    /// a PROCESS-backed worker (docs/internal/CONCURRENCY_ARCH.md §13.1).
    WorkerServe,
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
    /// `qn fmt [--check|--dry-run|--diff] <path>…`: format Quoin source in place. The paths are
    /// carried in `VmRunnerOptions::vm_options.arguments`.
    Fmt,
    /// `qn check <file>…`: type-check each file (report diagnostics) without running it. The paths
    /// are carried in `VmRunnerOptions::vm_options.arguments`; exits non-zero if any diagnostic.
    Check,
    /// `qn doc [PATH…]`: generate the API reference (docs/internal/DOCS_ARCH.md). The output directory
    /// is carried in `target_path`, extra units in `vm_options.arguments`.
    Doc,
    /// `qn` with no arguments: print usage. (`--help` / `--version` are answered by
    /// the argument parser itself, which prints and exits before a mode is chosen.)
    Help,
}

/// Shown under the generated help. The parser knows the flags; these are the things it
/// can't infer.
const AFTER_HELP: &str = "\
Arguments after a program are passed to it (`Runtime.arguments`); separate flags
meant for the program with `--`, e.g. `qn app.qn -- --verbose`.

Environment:
  QUOIN_STDLIB=DIR  Load the stdlib from DIR, not the copy embedded in the binary
  QUOIN_PATH=DIRS   Extra roots searched for extension packages";

/// Coverage collection, shared by a program run and `qn test`.
#[derive(clap::Args, Clone, Debug, Default)]
struct CoverageArgs {
    /// Collect coverage; FORMAT is `lcov` (default) or `cobertura`
    //
    // A `///` second paragraph would print in `--help`; this note is for maintainers.
    // `require_equals` matters: without it a bare `--coverage` would swallow the next
    // positional as its format, so `qn app.qn --coverage extra` would lose `extra`.
    #[arg(long, value_name = "FORMAT", num_args = 0..=1, require_equals = true, default_missing_value = "lcov")]
    coverage: Option<String>,
    /// Write the coverage report to PATH instead of stdout (implies --coverage)
    #[arg(long, value_name = "PATH")]
    coverage_out: Option<String>,
}

impl CoverageArgs {
    /// `None` when coverage was not requested. An unsupported format warns and falls back
    /// to lcov rather than failing the run — coverage is a reporting side-channel.
    fn config(self) -> Option<crate::coverage::CoverageConfig> {
        use crate::coverage::CoverageFormat;
        if self.coverage.is_none() && self.coverage_out.is_none() {
            return None;
        }
        let format = match self.coverage.as_deref() {
            Some("cobertura") => CoverageFormat::Cobertura,
            Some("lcov") | None => CoverageFormat::Lcov,
            Some(other) => {
                eprintln!("qn: unsupported coverage format '{other}', using lcov");
                CoverageFormat::Lcov
            }
        };
        Some(crate::coverage::CoverageConfig {
            format,
            out: self.coverage_out,
        })
    }
}

/// The `qn` command line. A bare `qn` prints help; `qn <file.qn>` runs a program; every
/// other verb is a subcommand.
#[derive(clap::Parser, Debug)]
#[command(
    name = "qn",
    version,
    about = "Quoin — a small object-oriented language on a bytecode VM.",
    after_help = AFTER_HELP,
    disable_help_subcommand = true,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    /// Evaluate one expression and print its result
    #[arg(short = 'e', value_name = "EXPR", conflicts_with = "file")]
    eval: Option<String>,
    /// Quoin program to run
    #[arg(value_name = "FILE")]
    file: Option<String>,
    /// Arguments passed to the program
    // `allow_hyphen_values`: everything after FILE reaches the program verbatim —
    // `qn tool.qn --verbose` (and `./tool.qn --verbose` via a shebang) hands
    // `--verbose` to the SCRIPT, not to qn's own parser. qn's flags go before FILE.
    #[arg(
        value_name = "ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    args: Vec<String>,
    #[command(flatten)]
    coverage: CoverageArgs,
}

#[derive(clap::Subcommand, Debug)]
enum Cmd {
    /// Run the test suites in DIR (each file registers its suites as it loads)
    Test {
        /// Directory of `.qn` test files
        #[arg(value_name = "DIR")]
        dir: Option<String>,
        #[command(flatten)]
        coverage: CoverageArgs,
    },
    /// Interactive read-eval-print loop
    Repl,
    /// Type-check each path, reporting diagnostics, without running it
    Check {
        #[arg(value_name = "PATH", required = true)]
        paths: Vec<String>,
        /// Emit diagnostics as JSON on stdout (the language server's contract).
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Generate the project's API reference (HTML, and JSON with --json)
    Doc {
        /// Project roots to document (files or directories); default: the current
        /// directory's tree, minus tests/ and shebang scripts
        #[arg(value_name = "PATH")]
        paths: Vec<String>,
        /// Output directory
        #[arg(long, value_name = "DIR", default_value = "qn-docs")]
        out: String,
        /// Also write the raw doc model as model.json
        #[arg(long)]
        json: bool,
        /// Report undocumented classes/selectors instead of generating
        #[arg(long)]
        coverage: bool,
        /// Run the documentation's fenced examples instead of generating: with PATHs,
        /// markdown files/dirs (blocks tagged `quoin`); without, the project's doc examples
        #[arg(long)]
        check: bool,
        /// Render markdown PATHs (files or directories) to HTML pages instead —
        /// fenced `quoin` blocks through the syntax highlighter
        #[arg(long)]
        md: bool,
        /// Document the shipped stdlib instead of a project (the reference-publishing
        /// mode this repository uses) — not an end-user flag
        #[arg(long, hide = true)]
        stdlib: bool,
        /// Link prefix (a relative path or full URL) for stdlib types in project docs —
        /// not an end-user flag
        #[arg(long, value_name = "PREFIX", hide = true)]
        stdlib_path: Option<String>,
    },
    /// Format Quoin source in place
    Fmt {
        /// Exit non-zero if any file is not already formatted
        #[arg(long)]
        check: bool,
        /// Report what would change without writing
        #[arg(long)]
        dry_run: bool,
        /// Print a unified diff of the changes
        #[arg(long)]
        diff: bool,
        /// Files or directories (`-` reads stdin)
        #[arg(value_name = "PATH", required = true)]
        paths: Vec<String>,
    },
    /// Run a program under the debugger
    Debug {
        /// Pause when one of these exception types is thrown
        #[arg(long, value_name = "TYPES")]
        break_on_throw: Option<String>,
        /// Pause only when a matching exception will go uncaught
        #[arg(long, value_name = "TYPES")]
        break_on_uncaught: Option<String>,
        /// Speak the Debug Adapter Protocol on stdio instead of the `$`-command loop
        #[arg(long)]
        dap: bool,
        #[arg(value_name = "FILE")]
        file: Option<String>,
        #[arg(value_name = "ARGS", trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Print syntax-highlighted source
    Highlight {
        /// Emit a standalone HTML page instead of ANSI (shares the doc generator's code styles)
        #[arg(long)]
        html: bool,
        #[arg(value_name = "FILE")]
        file: String,
        #[arg(value_name = "ARGS", trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Run the built-in benchmarks (needs a Quoin source tree)
    Benchmark {
        #[arg(value_name = "ARGS", trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Install and list Quoin packages ($QUOIN_HOME, default ~/.quoin)
    Pkg {
        #[command(subcommand)]
        cmd: crate::runner_pkg::PkgCmd,
    },
    /// The child side of a process-backed worker (internal)
    #[command(name = "worker-serve", hide = true)]
    WorkerServe {
        sock: String,
        unit: String,
        service: Option<String>,
        lanes: Option<u32>,
    },
}

/// `--break-on-throw=TypeError, Error` — comma-separated, whitespace-trimmed.
fn split_types(types: Option<String>) -> Vec<String> {
    types
        .into_iter()
        .flat_map(|t| {
            t.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// `print!` for the CLI's bulk/report output (`highlight`, `fmt`, `doc`, the `-e` result
/// echo): `println!` panics when the reader hangs up (`qn highlight big.qn | head`), but a
/// broken pipe is not a program error — exit quietly with the conventional SIGPIPE status
/// (128+13), like the guest-output path (`VmState::write_std_guest`). Any other stdout
/// failure is real: report it and fail. Only for arena-free paths — exiting here skips
/// `Drop` teardown, so callers holding a live arena map `BrokenPipe` themselves.
pub(crate) fn print_or_exit(text: &str) {
    use std::io::Write;
    if let Err(e) = std::io::stdout().write_all(text.as_bytes()) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            exit(141);
        }
        eprintln!("qn: cannot write to stdout: {e}");
        exit(1);
    }
}

/// The synthesized `qn test` entry unit: pull in the test framework, glob the caller's
/// test directory (each suite self-registers as it loads), then run the registry.
///
/// `dir` is interpolated into a `use` path, so it is validated first: a missing or empty
/// directory would otherwise glob to zero suites and *pass*, silently greening a CI run.
fn test_entry_source(dir: &str) -> String {
    let dir = dir.trim_end_matches('/');
    if !Path::new(dir).is_dir() {
        eprintln!("qn test: no test directory '{dir}'");
        eprintln!("       pass one explicitly: qn test <dir>");
        exit(1);
    }
    // The path lands in Quoin source as a bare `use` token, so keep it to characters the
    // grammar accepts — anything else would surface as a confusing parse error.
    if !dir
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
    {
        eprintln!("qn test: test directory '{dir}' must be a plain relative path");
        exit(1);
    }
    let has_tests = std::fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("qn"))
    });
    if !has_tests {
        eprintln!("qn test: no .qn test files in '{dir}'");
        exit(1);
    }
    format!("use test;\nuse self:{dir}/*;\n[Test]Main.run\n")
}

/// The directory `use self:…` resolves against for a script: the script's own directory,
/// so `qn /srv/app/main.qn` finds `/srv/app/lib/…` regardless of the invoking CWD — with
/// one convention on top: a script living in a `bin/` directory anchors at bin's PARENT,
/// so an installable tool laid out as `<root>/bin/quern` + `<root>/lib/*.qn` loads its
/// own library (`use self:lib/*`) from any invoking directory. Falls back to
/// CWD-relative (an empty path) when the script path has no parent or cannot be
/// canonicalized — the run will fail on the missing script anyway.
fn script_self_root(script: &str) -> PathBuf {
    std::fs::canonicalize(script)
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .map(|dir| {
            if dir.file_name().is_some_and(|n| n == "bin")
                && let Some(root) = dir.parent()
            {
                root.to_path_buf()
            } else {
                dir
            }
        })
        .unwrap_or_default()
}

/// How a `compile_and_run_asts*` run ended, for the mode drivers' exit-code decisions.
/// Returned (not `exit()`ed in place) so the arena has dropped — extension children and
/// sockets torn down — before the caller exits the process.
enum UnitOutcome {
    /// Every unit ran to completion; `last_truthy` is the final program's result value
    /// (`qn test` gates on it — main.qn ends in "every suite passed").
    Finished { last_truthy: bool },
    /// A unit aborted on an uncaught error (already reported to stderr).
    Aborted,
    /// The guest requested process exit (`Runtime.exit:`) with this status.
    ExitRequested(i32),
}

/// The index of the program FILE in raw argv for a plain file run, or `None`
/// when this invocation is clap's whole business (a subcommand, `-e`, bare
/// `qn`, `--help`/`--version` with no file). Walks qn's own pre-file flags:
/// `--coverage-out` takes a separate value; everything else is either
/// self-contained (`--coverage[=FMT]`, glued short values) or a bare flag.
fn file_run_split(args: &[String]) -> Option<usize> {
    use clap::CommandFactory;
    let first = args.get(1)?;
    if !first.starts_with('-') {
        // A subcommand invocation (including aliases) is entirely clap's.
        let cmd = Cli::command();
        let is_sub = cmd.get_subcommands().any(|c| {
            c.get_name() == first.as_str() || c.get_all_aliases().any(|a| a == first.as_str())
        });
        return if is_sub { None } else { Some(1) };
    }
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-e" {
            // Expression mode has no FILE; clap owns the rest.
            return None;
        }
        if a == "--coverage-out" {
            i += 2;
            continue;
        }
        if !a.starts_with('-') {
            return Some(i);
        }
        i += 1;
    }
    None
}

impl VmRunnerOptions {
    /// Parse `argv`. `--help` / `--version` and any usage error are answered by the
    /// parser itself (printing, then exiting 0 or 2); everything that returns here is a
    /// runnable command.
    pub fn parse(args: &[String]) -> Self {
        use clap::Parser;
        // File-run pass-through: locate the program FILE in raw argv and keep
        // everything after it AWAY from clap entirely — `trailing_var_arg` alone
        // still lets clap intercept `--help`/`--version` mid-capture, and a
        // script's `--help` must be the SCRIPT's (`qn tool.qn --help`, or
        // `./tool.qn --help` through a shebang). qn's own flags go before FILE.
        let (clap_args, passthrough) = match file_run_split(args) {
            Some(i) => (args[..=i].to_vec(), args[i + 1..].to_vec()),
            None => (args.to_vec(), Vec::new()),
        };
        let cli = match Cli::try_parse_from(&clap_args) {
            Ok(cli) => cli,
            Err(e) => e.exit(),
        };

        let mut break_on_throw = Vec::new();
        let mut break_on_uncaught = Vec::new();
        let mut dap = false;
        let mut fmt_check = false;
        let mut fmt_dry_run = false;
        let mut fmt_diff = false;
        let mut doc_json = false;
        let mut doc_coverage = false;
        let mut doc_check = false;
        let mut doc_md = false;
        let mut doc_stdlib = false;
        let mut doc_stdlib_path = None;
        let mut highlight_html = false;
        let mut check_json = false;
        let mut target_path = None;
        let mut vm_args = Vec::new();
        let mut coverage = None;

        let mode = match cli.command {
            Some(Cmd::Test { dir, coverage: cov }) => {
                target_path = dir;
                coverage = cov.config();
                VmRunnerMode::Test
            }
            Some(Cmd::Repl) => VmRunnerMode::Repl,
            // Pure filesystem work — no VM. Runs and exits here.
            Some(Cmd::Pkg { cmd }) => {
                std::process::exit(crate::runner_pkg::run(cmd));
            }
            Some(Cmd::Check { paths, json }) => {
                vm_args = paths;
                check_json = json;
                VmRunnerMode::Check
            }
            Some(Cmd::Doc {
                paths,
                out,
                json,
                coverage: cov,
                check,
                md,
                stdlib,
                stdlib_path,
            }) => {
                target_path = Some(out);
                vm_args = paths;
                doc_json = json;
                doc_coverage = cov;
                doc_check = check;
                doc_md = md;
                doc_stdlib = stdlib;
                doc_stdlib_path = stdlib_path;
                VmRunnerMode::Doc
            }
            Some(Cmd::Fmt {
                check,
                dry_run,
                diff,
                paths,
            }) => {
                fmt_check = check;
                fmt_dry_run = dry_run;
                fmt_diff = diff;
                vm_args = paths;
                VmRunnerMode::Fmt
            }
            Some(Cmd::Debug {
                break_on_throw: bot,
                break_on_uncaught: bou,
                dap: is_dap,
                file,
                args,
            }) => {
                break_on_throw = split_types(bot);
                break_on_uncaught = split_types(bou);
                dap = is_dap;
                target_path = file;
                vm_args = args;
                VmRunnerMode::Debug
            }
            Some(Cmd::Highlight { html, file, args }) => {
                highlight_html = html;
                target_path = Some(file);
                vm_args = args;
                VmRunnerMode::Highlight
            }
            Some(Cmd::Benchmark { args }) => {
                vm_args = args;
                VmRunnerMode::Benchmark
            }
            // `target_path` is the socket; the unit (and optional service class) ride in
            // `arguments`, matching how the parent spawns the child.
            Some(Cmd::WorkerServe {
                sock,
                unit,
                service,
                lanes,
            }) => {
                target_path = Some(sock);
                vm_args = std::iter::once(unit)
                    .chain(service)
                    .chain(lanes.map(|n| n.to_string()))
                    .collect();
                VmRunnerMode::WorkerServe
            }
            None => match (cli.eval, cli.file) {
                (Some(expr), _) => {
                    target_path = Some(expr);
                    vm_args = cli.args;
                    VmRunnerMode::Eval
                }
                (None, Some(file)) => {
                    target_path = Some(file);
                    // With a pre-split, cli.args is empty and the program's
                    // arguments arrive verbatim from raw argv.
                    vm_args = cli.args;
                    vm_args.extend(passthrough);
                    coverage = cli.coverage.config();
                    VmRunnerMode::Run
                }
                // Bare `qn`: print usage rather than running a scratch script.
                (None, None) => VmRunnerMode::Help,
            },
        };

        // Interactive modes (REPL, debugger) colorize errors/output when stdout is a terminal.
        // DAP owns stdout (program output is sent as plain-text `output` events), so never there.
        let supports_color = !dap
            && matches!(mode, VmRunnerMode::Repl | VmRunnerMode::Debug)
            && std::io::stdout().is_terminal();

        // `use self:…` resolves against the entry script's directory. The script-less modes
        // (repl, -e, test, benchmark) have nothing to anchor to and stay CWD-relative.
        // A worker child's script is its *unit* (the first vm arg), not `target_path`
        // (which holds the socket path), so it anchors the same way its parent did.
        let self_root = match mode {
            VmRunnerMode::Run | VmRunnerMode::Debug => target_path
                .as_deref()
                .map(script_self_root)
                .unwrap_or_default(),
            VmRunnerMode::WorkerServe => vm_args
                .first()
                .map(|u| script_self_root(u))
                .unwrap_or_default(),
            _ => PathBuf::new(),
        };

        Self {
            mode,
            target_path,
            vm_options: VmOptions {
                arguments: vm_args,
                supports_color,
                console_width: None,
                console_height: None,
                // The single shared class-name accumulator for this run — cloned (Rc) into
                // every VM and top-level compile, so units see each other's classes.
                seen_types: crate::types::SeenTypes::with_builtins(),
                class_table: crate::class_table::ClassTable::new(),
                self_root,
            },
            break_on_throw,
            break_on_uncaught,
            dap,
            coverage,
            fmt_check,
            fmt_dry_run,
            fmt_diff,
            doc_json,
            doc_coverage,
            doc_check,
            doc_md,
            doc_stdlib,
            doc_stdlib_path,
            highlight_html,
            check_json,
        }
    }
}

/// Compile the Quoin program at `path` and install it as the debugger's main task #0, armed for
/// interactive (DAP) pausing. Shared by `run_dap`'s eager (CLI-path) install and the DAP `launch`
/// handler's deferred (program-from-protocol) install. Returns a human-readable error on failure
/// so the DAP layer can report it as a failed `launch` rather than killing the process.
pub(crate) fn install_dap_program(
    arena: &mut ReplArena,
    path: &str,
    break_on_throw: &[String],
    break_on_uncaught: &[String],
) -> Result<(), String> {
    use crate::debug::DebugState;
    let source = read_to_string(path).map_err(|e| format!("error reading {path}: {e}"))?;
    let node = try_parse_quoin_string_named(&source, path).map_err(|pe| {
        format!(
            "parse error at {}:{}:{}: {}",
            path,
            pe.line,
            pe.column + 1,
            pe.message
        )
    })?;
    let mut compile_err = None;
    let installed = arena.mutate_root(|mc, vm| {
        let NodeValue::Program(p) = &node.value else {
            compile_err = Some("expected a Program node".to_string());
            return false;
        };
        // Capture output as DAP `output` events from here on — crucially before the compile, so
        // the resolver's type warnings below reach the client instead of the adapter's raw stderr.
        vm.output.capture = true;
        let mut compiler = unit_compiler();
        compiler.set_seen_types(vm.options.seen_types.clone());
        compiler.set_class_table(vm.options.class_table.clone());
        crate::class_table::populate_from_vm(vm, &vm.options.class_table);
        let sb = match compiler.compile_program(p) {
            Ok(sb) => sb,
            Err(e) => {
                compile_err = Some(format!("compile error: {e}"));
                return false;
            }
        };
        vm.report_type_warnings(compiler.diagnostics());
        compile_unit_aot(vm, &mut compiler);
        let block = build_block(mc, &sb);
        // Run to a breakpoint (`stopOnEntry` is honored at `launch`). Program output is captured
        // and re-emitted as DAP `output` events rather than written to fd 1/2.
        vm.instrumentation.debug = Some(DebugState {
            // `interactive` = "bubble a pause up to the driver frontend" (here, the DAP adapter's
            // `on_pause`) rather than auto-applying the scripted action in place.
            interactive: true,
            show_source: false,
            step: None,
            break_on_throw: break_on_throw.iter().cloned().collect(),
            break_on_uncaught: break_on_uncaught.iter().cloned().collect(),
            ..Default::default()
        });
        vm.start_block(mc, block, Vec::new(), None, None);
        install_main_task(mc, vm);
        true
    });
    if installed {
        Ok(())
    } else {
        Err(compile_err.unwrap_or_else(|| "failed to install program".to_string()))
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
                if self.options.highlight_html {
                    // A standalone page over the shared code stylesheet — the same classes and
                    // colors the doc generator's fenced examples use (docs/internal/DOCS_ARCH.md §8).
                    print_or_exit(&format!(
                        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\n<title>{}</title>\n{}\n\
                         <style>\nbody {{ margin: 2rem auto; max-width: 60rem; padding: 0 1rem; \
                         background: #ffffff; color: #1a1a1a; }}\n\
                         @media (prefers-color-scheme: dark) {{ body {{ background: #1e1e1e; \
                         color: #d4d4d4; }} }}\n\
                         pre.qn-code {{ font: 14px/1.5 ui-monospace, monospace; }}\n{}</style>\n\
                         </head>\n<body>\n{}\n</body></html>\n",
                        path,
                        crate::highlighter::code_font_links(),
                        crate::highlighter::code_stylesheet(),
                        crate::highlighter::highlight_to_html(&source)
                    ));
                } else {
                    print_or_exit(&highlight_to_ansi(&source));
                }
                Ok(())
            }
            VmRunnerMode::WorkerServe => {
                let Some(sock) = self.options.target_path.clone() else {
                    eprintln!("usage: qn worker-serve <sock> <unit|@none> [@block|@job [<lanes>]]");
                    exit(2);
                };
                let args = &self.options.vm_options.arguments;
                let Some(unit) = args.first() else {
                    eprintln!("usage: qn worker-serve <sock> <unit|@none> [@block|@job [<lanes>]]");
                    exit(2);
                };
                let service = args.get(1).map(|s| s.as_str());
                let lanes = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
                exit(crate::worker::worker_serve_main(
                    &sock, unit, service, lanes,
                ));
            }
            VmRunnerMode::Test => {
                // `qn test [DIR]` runs the CALLER's suites: the entry unit is synthesized
                // around their test directory, not shipped with the stdlib. Each test file
                // self-registers into `[Test]Suites` as the glob loads it, then
                // `[Test]Main.run` (qnlib/test.qn) runs the registry; its boolean value is
                // the program's value, which `UnitOutcome::Finished` gates the exit code on.
                let dir = self.options.target_path.as_deref().unwrap_or("tests");
                let entry = test_entry_source(dir);
                let ast_iter =
                    prelude_asts().chain(once_with(move || parse_source_or_exit(&entry, "<test>")));

                match self.compile_and_run_asts(ast_iter) {
                    UnitOutcome::ExitRequested(code) => exit(code),
                    UnitOutcome::Finished { last_truthy: true } => Ok(()),
                    UnitOutcome::Finished { last_truthy: false } | UnitOutcome::Aborted => exit(1),
                }
            }
            VmRunnerMode::Benchmark => {
                // `benchmark.qn` is deliberately not embedded — the benchmarks are a
                // source-tree feature, like the test suite. Say so plainly.
                let Some(root) = crate::packages::source_tree_root() else {
                    eprintln!(
                        "qn benchmark: needs a Quoin source tree (qnlib/benchmark.qn).\n\
                         \x20      Run from a checkout, or set QUOIN_STDLIB=/path/to/qnlib"
                    );
                    exit(1);
                };
                let bench = root.join("benchmark.qn");
                let ast_iter = prelude_asts().chain(once_with(move || parse_quoin_file(&bench)));

                self.compile_and_benchmark(ast_iter);
                Ok(())
            }
            VmRunnerMode::Help => {
                use clap::CommandFactory;
                Cli::command().print_help().ok();
                println!();
                Ok(())
            }
            VmRunnerMode::Run => {
                let script_path = self
                    .options
                    .target_path
                    .clone()
                    .expect("Run mode always carries a script path");
                let unit = std::fs::canonicalize(&script_path)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| script_path.clone());
                let ast_iter = prelude_asts().chain(once_with(move || {
                    parse_quoin_file(&PathBuf::from(&script_path))
                }));

                match self.compile_and_run_asts_as_unit(ast_iter, Some(unit)) {
                    UnitOutcome::ExitRequested(code) => exit(code),
                    // A script that runs to completion exits 0 whatever its final
                    // value; only an uncaught error (already printed) fails the run.
                    UnitOutcome::Finished { .. } => Ok(()),
                    UnitOutcome::Aborted => exit(1),
                }
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
                if self.options.dap {
                    // `qn debug --dap [FILE]`: the file is optional. When omitted, the program
                    // path is taken from the DAP `launch` request instead (IDE integration).
                    self.run_dap(self.options.target_path.as_deref());
                } else {
                    let Some(ref path) = self.options.target_path else {
                        eprintln!("Usage: qn debug FILE");
                        exit(2);
                    };
                    self.run_debug(path);
                }
                Ok(())
            }
            VmRunnerMode::Fmt => {
                self.run_fmt();
                Ok(())
            }
            VmRunnerMode::Check => {
                self.run_check();
                Ok(())
            }
            VmRunnerMode::Doc => self.run_doc(),
        }
    }

    /// `qn check <file-or-dir>…`: type-check each file — reporting the checker's diagnostics —
    /// without running it. A directory argument is searched recursively for `.qn` files (like
    /// `fmt`). Exits non-zero if any file emitted a diagnostic, so it works as a CI gate.
    fn run_check(&self) {
        let args = &self.options.vm_options.arguments;
        if args.is_empty() {
            eprintln!("Usage: qn check <file-or-dir>…");
            exit(2);
        }
        // Expand directory arguments to their `.qn` files, recursively (like `fmt`).
        let mut files = Vec::new();
        for p in args {
            let path = Path::new(p);
            if path.is_dir() {
                collect_qn_files(path, &mut files);
            } else {
                files.push(path.to_path_buf());
            }
        }
        if files.is_empty() {
            eprintln!("qn check: no .qn files found in {}", args.join(" "));
            exit(2);
        }

        // Parse each file WITHOUT panicking: a read or syntax error is reported and that file is
        // skipped, so one bad file (common when checking a whole tree) doesn't abort the rest.
        // Under `--json`, parse errors and checker diagnostics collect into one array on
        // stdout (read errors stay on stderr — operational, not diagnostics); the exit-code
        // contract is unchanged.
        let json = self.options.check_json;
        let mut sink: Vec<CheckDiag> = Vec::new();
        let mut had_error = false;
        let mut asts = Vec::new();
        for f in &files {
            let name = f.display().to_string();
            let source = match read_to_string(f) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{name}: {e}");
                    had_error = true;
                    continue;
                }
            };
            let source = source.strip_prefix('\u{FEFF}').unwrap_or(&source);
            match try_parse_quoin_string_named(source, &name) {
                Ok(node) => asts.push(node),
                Err(e) => {
                    if json {
                        sink.push(CheckDiag {
                            file: Some(name.clone()),
                            severity: "error",
                            kind: "parse-error".to_string(),
                            message: e.message.clone(),
                            line: Some(e.line),
                            column: Some(e.column),
                            start: Some(e.start),
                            end: Some(e.end),
                            notes: Vec::new(),
                        });
                    } else {
                        eprintln!("{name}: parse error: {e}");
                    }
                    had_error = true;
                }
            }
        }

        let had_diagnostics = self.compile_and_check_asts(
            prelude_asts(),
            asts.into_iter(),
            if json { Some(&mut sink) } else { None },
        );
        if json {
            println!(
                "{}",
                serde_json::to_string(&sink).expect("check diagnostics serialize")
            );
        }
        if had_error || had_diagnostics {
            exit(1);
        }
    }

    /// `qn fmt [--check|--dry-run|--diff] <path>…`: format each named file in place (directories
    /// are searched recursively for `.qn` files). `--dry-run` prints formatted source to stdout,
    /// `--diff` shows a unified diff of what would change, and `--check` lists files that aren't
    /// formatted — the last three write nothing and exit non-zero when anything differs. A parse
    /// error is reported and fails the run but doesn't abort the batch.
    fn run_fmt(&self) {
        let paths = &self.options.vm_options.arguments;
        if paths.is_empty() {
            eprintln!("Usage: qn fmt [--check|--dry-run|--diff] <file-or-dir>… | qn fmt -");
            exit(2);
        }
        // `qn fmt -`: read source from stdin, write formatted source to stdout (the editor
        // interface — format an unsaved buffer without touching disk). It stands alone.
        if paths.len() == 1 && paths[0] == "-" {
            self.run_fmt_stdin();
            return;
        }
        let modes = [
            self.options.fmt_check,
            self.options.fmt_dry_run,
            self.options.fmt_diff,
        ];
        if modes.iter().filter(|&&m| m).count() > 1 {
            eprintln!("qn fmt: --check, --dry-run, and --diff are mutually exclusive");
            exit(2);
        }

        let mut files = Vec::new();
        for p in paths {
            let path = Path::new(p);
            if path.is_dir() {
                collect_qn_files(path, &mut files);
            } else {
                files.push(path.to_path_buf());
            }
        }

        let mut had_error = false;
        let mut unformatted = false;
        for (i, file) in files.iter().enumerate() {
            let name = file.display().to_string();
            let source = match read_to_string(file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("qn fmt: cannot read {name}: {e}");
                    had_error = true;
                    continue;
                }
            };
            let formatted = match quoin_fmt::format_source(&source, &name) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("qn fmt: {name}: {e}");
                    had_error = true;
                    continue;
                }
            };
            if self.options.fmt_check {
                if formatted != source {
                    print_or_exit(&format!("{name}\n"));
                    unformatted = true;
                }
            } else if self.options.fmt_dry_run {
                print_or_exit(&formatted);
            } else if self.options.fmt_diff {
                if formatted != source {
                    unformatted = true;
                    had_error |= !self.show_fmt_diff(file, &name, &formatted, i);
                }
            } else if formatted != source {
                // Default: rewrite in place, only when something actually changed.
                if let Err(e) = std::fs::write(file, &formatted) {
                    eprintln!("qn fmt: cannot write {name}: {e}");
                    had_error = true;
                } else {
                    eprintln!("formatted {name}");
                }
            }
        }

        if had_error || ((self.options.fmt_check || self.options.fmt_diff) && unformatted) {
            exit(1);
        }
    }

    /// `qn fmt -`: read Quoin source from stdin, write the formatted source to stdout. The editor
    /// interface for formatting an in-memory (possibly unsaved) buffer. On a parse or self-check
    /// failure, write the message to stderr and exit non-zero, emitting nothing on stdout — so a
    /// caller can leave the buffer untouched. Never returns (always exits).
    fn run_fmt_stdin(&self) {
        use std::io::Read;
        let mut source = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut source) {
            eprintln!("qn fmt: cannot read stdin: {e}");
            exit(2);
        }
        match quoin_fmt::format_source(&source, "<stdin>") {
            Ok(formatted) => {
                print_or_exit(&formatted);
                exit(0);
            }
            Err(e) => {
                eprintln!("qn fmt: {e}");
                exit(1);
            }
        }
    }

    /// Write `formatted` to a temp file, print a unified diff of `file` against it (via the
    /// system `diff -u`), then remove the temp file. `index` disambiguates the temp path within
    /// one run. Returns whether the diff was produced successfully.
    fn show_fmt_diff(&self, file: &Path, name: &str, formatted: &str, index: usize) -> bool {
        let tmp = std::env::temp_dir().join(format!("qn-fmt-{}-{index}.qn", std::process::id()));
        if let Err(e) = std::fs::write(&tmp, formatted) {
            eprintln!("qn fmt: cannot write temp file for diff: {e}");
            return false;
        }
        let result = Command::new("diff").arg("-u").arg(file).arg(&tmp).output();
        let _ = std::fs::remove_file(&tmp);
        match result {
            Ok(out) => {
                // Replace the temp path in diff's `+++` header with a readable label.
                let text = String::from_utf8_lossy(&out.stdout)
                    .replace(&tmp.display().to_string(), &format!("{name} (formatted)"));
                print_or_exit(&text);
                true
            }
            Err(e) => {
                eprintln!("qn fmt: could not run `diff`: {e}");
                false
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
            let mut compiler = unit_compiler();
            compiler.set_seen_types(vm.options.seen_types.clone());
            compiler.set_class_table(vm.options.class_table.clone());
            crate::class_table::populate_from_vm(vm, &vm.options.class_table);
            let sb = match compiler.compile_program(p) {
                Ok(sb) => sb,
                Err(e) => {
                    vm.report_compile_error(&e);
                    return false;
                }
            };
            vm.report_type_warnings(compiler.diagnostics());
            compile_unit_aot(vm, &mut compiler);
            let block = build_block(mc, &sb);
            // Stop at entry: an armed `StepInto` halts at the first line start. Source is
            // shown at each pause by default ($source off to silence). `--break-on-throw` types
            // additionally pause at a matching throw; `--break-on-uncaught` only when it escapes.
            vm.instrumentation.debug = Some(DebugState {
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
            if let QuoinError::ExitRequested(code) = e {
                drop(arena);
                exit(code);
            }
            eprintln!("VM execution error: {e}");
        }
    }

    /// `qn debug --dap <file>`: drive the program as a DAP adapter over stdio. Mirrors
    /// [`run_debug`] but speaks the Debug Adapter Protocol instead of the `$`-command loop — the
    /// debuggee's output is rerouted to DAP `output` events and stdout is reserved for the protocol
    /// stream (a stray Rust print can't corrupt it; see `src/dap.rs`).
    fn run_dap(&self, path: Option<&str>) {
        // Reserve stdout for the protocol BEFORE anything can print; stray prints go to stderr.
        let protocol = match crate::dap::redirect_protocol_stdout() {
            Ok(f) => f,
            Err(e) => {
                eprintln!("dap: could not redirect stdout: {e}");
                exit(1);
            }
        };
        let Some(mut arena) = self.build_repl_arena() else {
            exit(1);
        };
        let conn = crate::dap::Connection::new(std::io::BufReader::new(std::io::stdin()), protocol);
        let mut frontend = match path {
            // `qn debug --dap FILE`: install eagerly; the launch request carries no program.
            Some(path) => {
                if let Err(e) = install_dap_program(
                    &mut arena,
                    path,
                    &self.options.break_on_throw,
                    &self.options.break_on_uncaught,
                ) {
                    eprintln!("dap: {e}");
                    exit(1);
                }
                DapFrontend::new(conn)
            }
            // `qn debug --dap`: the program path arrives in the DAP `launch` request.
            None => DapFrontend::with_pending(
                conn,
                PendingProgram {
                    break_on_throw: self.options.break_on_throw.clone(),
                    break_on_uncaught: self.options.break_on_uncaught.clone(),
                },
            ),
        };
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
        let requested = if stdin().is_terminal() {
            // `~/.quoinrc` is interactive-only, like a shell rc file (a piped script or a
            // one-shot `-e` doesn't run it).
            load_quoinrc(&mut arena);
            run_repl_interactive(&mut arena)
        } else {
            run_repl_piped(&mut arena)
        };
        // A guest `Runtime.exit:` — drop the arena first so teardown `Drop`s run.
        if let Some(code) = requested {
            drop(arena);
            exit(code);
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
        arena.metrics().set_pacing(crate::vm::gc_pacing());

        // Load the core stdlib into the persistent VM (prelude `use core/*`).
        for ast in prelude_asts() {
            let mut failed = false;
            arena.mutate_root(|mc, vm| {
                let NodeValue::Program(p) = &ast.value else {
                    return;
                };
                match Compiler::new().with_template_ids().compile_program(p) {
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
        let res = eval_once(&mut arena, expr);
        // A guest `Runtime.exit:` wins over result/error printing; drop the arena
        // first so teardown `Drop`s (extension children, sockets) run before exiting.
        if let Some(code) = arena.mutate_root(|_mc, vm| vm.requested_exit) {
            drop(arena);
            exit(code);
        }
        match res {
            Ok(Some(out)) => {
                use std::io::Write;
                if let Err(e) = std::io::stdout().write_all(format!("{out}\n").as_bytes()) {
                    // The reader hung up on the result echo: quiet SIGPIPE status (141),
                    // arena dropped first so teardown runs (as the exit path above).
                    let broken = e.kind() == std::io::ErrorKind::BrokenPipe;
                    if !broken {
                        eprintln!("qn: cannot write to stdout: {e}");
                    }
                    drop(arena);
                    exit(if broken { 141 } else { 1 });
                }
            }
            Ok(None) => {}
            Err(msg) => {
                eprintln!("{msg}");
                exit(1);
            }
        }
    }

    /// Runs each program AST in turn. For `qn test` the `Finished` outcome's
    /// `last_truthy` is main.qn's `results.none?:{…}` boolean (true iff every suite
    /// passed), so the Test driver can gate the process exit code on it.
    fn compile_and_run_asts(&self, ast_iter: impl Iterator<Item = Node>) -> UnitOutcome {
        self.compile_and_run_asts_as_unit(ast_iter, None)
    }

    fn compile_and_run_asts_as_unit(
        &self,
        ast_iter: impl Iterator<Item = Node>,
        unit_path: Option<String>,
    ) -> UnitOutcome {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, self.options.vm_options.clone());
            register_builtins(mc, &mut vm);
            vm.unit_path = unit_path.clone();
            // Attach the coverage collector before any user code runs, so every
            // line-start crossing from here on is recorded.
            if self.options.coverage.is_some() {
                vm.instrumentation.coverage = Some(crate::coverage::CoverageState::new());
            }
            vm
        });
        arena.metrics().set_pacing(crate::vm::gc_pacing());

        // `Some` once a unit aborted or requested exit; the remaining ASTs are skipped.
        let mut ended: Option<UnitOutcome> = None;
        for ast in ast_iter {
            if ended.is_some() {
                break;
            }

            // A compile error inside `compile_and_start` is a *user* error — a typo, an
            // undeclared local, a reassigned `let` — so it is reported and aborts the run
            // like any other (no `panic!`: that printed a Rust backtrace note and exited
            // 101, on exactly the two mistakes strict `var`/`let` invites).
            let compiled = arena.mutate_root(|mc, vm| {
                compile_and_start(mc, vm, &ast)?;
                // Run this program unit as scheduler task #0; driven to completion below.
                install_main_task(mc, vm);
                Ok::<(), crate::runner_core::CompileReported>(())
            });
            if compiled.is_err() {
                ended = Some(UnitOutcome::Aborted);
                break;
            }

            // Drive the unit to completion through the shared scheduler (async I/O, sleep,
            // tasks, fibers). An error aborts the remaining ASTs (and fails a test run);
            // a guest `Runtime.exit:` ends the run silently with its status.
            if let Err(e) = drive_main_task(&mut arena) {
                if let QuoinError::ExitRequested(code) = e {
                    ended = Some(UnitOutcome::ExitRequested(code));
                } else {
                    eprintln!("VM execution error: {}", e);
                    ended = Some(UnitOutcome::Aborted);
                }
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
                None => {
                    use std::io::Write;
                    // A truncated report must not exit 0 (CI consumes this stream), and
                    // the arena is live — so no `print_or_exit` here: report through the
                    // normal outcome (a guest exit already recorded in `ended` wins).
                    if let Err(e) = std::io::stdout().write_all(output.as_bytes())
                        && ended.is_none()
                    {
                        let broken = e.kind() == std::io::ErrorKind::BrokenPipe;
                        if !broken {
                            eprintln!("qn: cannot write coverage to stdout: {e}");
                        }
                        ended = Some(UnitOutcome::ExitRequested(if broken { 141 } else { 1 }));
                    }
                }
            }
        }

        // The last program run leaves its result on top of the stack; callers that
        // gate on it (`qn test`) read it off the `Finished` outcome.
        let outcome = ended.unwrap_or_else(|| UnitOutcome::Finished {
            last_truthy: arena
                .mutate_root(|_mc, vm| vm.stack.last().map(|v| v.is_truthy()).unwrap_or(false)),
        });

        arena.finish_cycle();
        outcome
    }

    /// `qn check FILE…`: run the prelude (so the checker sees the full stdlib class environment),
    /// then compile each target — running the type checker, whose diagnostics are reported —
    /// WITHOUT executing it. Returns whether any target emitted a diagnostic (→ non-zero exit).
    fn compile_and_check_asts(
        &self,
        prelude: impl Iterator<Item = Node>,
        targets: impl Iterator<Item = Node>,
        mut sink: Option<&mut Vec<CheckDiag>>,
    ) -> bool {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, self.options.vm_options.clone());
            register_builtins(mc, &mut vm);
            vm
        });
        arena.metrics().set_pacing(crate::vm::gc_pacing());

        // Execute the prelude so every stdlib class is registered for the checker to see.
        for ast in prelude {
            // The prelude is the stdlib we shipped, so a compile error here is our bug, not the
            // user's — but report it the same way rather than panicking at them about it.
            let compiled = arena.mutate_root(|mc, vm| {
                let program_node = match &ast.value {
                    NodeValue::Program(p) => p,
                    _ => panic!("Error: Root AST node is not a ProgramNode"),
                };
                let mut compiler = unit_compiler();
                compiler.set_seen_types(vm.options.seen_types.clone());
                compiler.set_class_table(vm.options.class_table.clone());
                crate::class_table::populate_from_vm(vm, &vm.options.class_table);
                let program = compiler.compile_program(program_node)?;
                vm.report_type_warnings(compiler.diagnostics());
                compile_unit_aot(vm, &mut compiler);
                let main_block = vm.block_from_template(mc, Arc::new(program), None, None);
                vm.start_block(mc, main_block, Vec::new(), None, None);
                install_main_task(mc, vm);
                Ok::<(), crate::compiler::CompileError>(())
            });
            if let Err(e) = compiled {
                eprintln!("qn check: the prelude failed to compile: {e}");
                break;
            }
            if let Err(e) = drive_main_task(&mut arena) {
                eprintln!("qn check: error loading the prelude: {}", e);
                break;
            }
        }

        // Compile-only each target: the checker runs (diagnostics reported), the program doesn't.
        let mut had_diagnostics = false;
        for ast in targets {
            had_diagnostics |= arena.mutate_root(|_mc, vm| {
                let program_node = match &ast.value {
                    NodeValue::Program(p) => p,
                    _ => panic!("Error: Root AST node is not a ProgramNode"),
                };
                let mut compiler = unit_compiler();
                compiler.set_seen_types(vm.options.seen_types.clone());
                compiler.set_class_table(vm.options.class_table.clone());
                crate::class_table::populate_from_vm(vm, &vm.options.class_table);
                match compiler.compile_program(program_node) {
                    Ok(_) => {
                        let diags = compiler.diagnostics();
                        let had = !diags.is_empty();
                        // `--json` collects diagnostics as data; text mode renders them.
                        if let Some(sink) = sink.as_deref_mut() {
                            sink.extend(diags.iter().map(CheckDiag::from_warning));
                        } else {
                            vm.report_type_warnings(diags);
                        }
                        compile_unit_aot(vm, &mut compiler);
                        had
                    }
                    Err(e) => {
                        if let Some(sink) = sink.as_deref_mut() {
                            sink.push(CheckDiag::from_compile_error(&e));
                        } else {
                            vm.report_compile_error(&e);
                        }
                        true
                    }
                }
            });
        }

        arena.finish_cycle();
        had_diagnostics
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

                // As in the scheduler driver: `execute_block` measures its headroom against
                // the coroutine it is about to run on.
                vm.stack_limit = fiber.stack_limit;

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
        arena.metrics().set_pacing(crate::vm::gc_pacing());

        let mut aborted = false;
        for ast in ast_iter {
            if aborted {
                break;
            }

            let compiled = arena.mutate_root(|mc, vm| compile_and_start(mc, vm, &ast));
            if compiled.is_err() {
                aborted = true;
                break;
            }

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
#[path = "runner_tests.rs"]
mod tests;
