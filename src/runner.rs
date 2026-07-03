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
use crate::runtime::{
    array, async_rt, big_decimal, big_integer, block, boolean, bytes, channel, class, codecs,
    csv_fmt, date_time, double, duration, extension, fiber as fiber_class, http, ids, instant,
    integer, io, json, list, map, math, method, msgpack, nil, object, pretty, regex, runtime, set,
    sockets, streams, string, symbol, task, time_zone, timer, timestamp, toml_fmt, yaml,
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
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, exit};
use std::sync::Once;
use std::time::Instant;

/// The prelude AST: a single `qnlib/prelude.qn` whose `use core/*` loads the core
/// stdlib (00-bootstrap … 06-io) in sorted order. Every runner mode loads this first,
/// so the prelude composition lives in Quoin rather than a hardcoded glob here.
pub(crate) fn prelude_asts() -> impl Iterator<Item = Node> {
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
    vm.register_native_class(mc, array::build_array_class());
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
    vm.register_native_class(mc, extension::build_extension_class());
}

/// The persistent REPL arena: one `VmState` kept alive across all lines.
pub(crate) type ReplArena = Arena<Rootable![VmState<'_>]>;

// `runner` is split across sibling `runner_*.rs` files, declared here as `#[path]` child modules so
// the public API (`VmRunner` / `VmRunnerOptions` / `register_builtins`) stays at `crate::runner::…`.
#[path = "runner_dap.rs"]
mod runner_dap;
#[path = "runner_driver.rs"]
mod runner_driver;
#[path = "runner_repl.rs"]
mod runner_repl;

use runner_dap::{DapFrontend, PendingProgram};
use runner_driver::{drive_main_task, drive_with_frontend, install_main_task};
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
        let mut fmt_check = false;
        let mut fmt_dry_run = false;
        let mut fmt_diff = false;

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
            } else if arg == "fmt" {
                // `qn fmt [--check|--dry-run] <file-or-dir>…`: format Quoin source in place.
                // Flags are pulled out; every other argument is a path (collected in `vm_args`).
                mode = VmRunnerMode::Fmt;
                for a in &args[2..] {
                    match a.as_str() {
                        "--check" => fmt_check = true,
                        "--dry-run" => fmt_dry_run = true,
                        "--diff" => fmt_diff = true,
                        _ => vm_args.push(a.clone()),
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
                // The single shared class-name accumulator for this run — cloned (Rc) into
                // every VM and top-level compile, so units see each other's classes.
                seen_types: crate::types::SeenTypes::with_builtins(),
                class_table: crate::class_table::ClassTable::new(),
            },
            break_on_throw,
            break_on_uncaught,
            dap,
            coverage,
            fmt_check,
            fmt_dry_run,
            fmt_diff,
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
        let mut compiler = Compiler::new();
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
                    println!("{name}");
                    unformatted = true;
                }
            } else if self.options.fmt_dry_run {
                print!("{formatted}");
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
                print!("{formatted}");
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
                print!("{text}");
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
            let mut compiler = Compiler::new();
            compiler.set_seen_types(vm.options.seen_types.clone());
            compiler.set_class_table(vm.options.class_table.clone());
            crate::class_table::populate_from_vm(vm, &vm.options.class_table);
            let sb = match compiler.compile_program(p) {
                Ok(sb) => sb,
                Err(e) => {
                    eprintln!("Compile error: {e}");
                    return false;
                }
            };
            vm.report_type_warnings(compiler.diagnostics());
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
                vm.instrumentation.coverage = Some(crate::coverage::CoverageState::new());
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
                compiler.set_seen_types(vm.options.seen_types.clone());
                compiler.set_class_table(vm.options.class_table.clone());
                crate::class_table::populate_from_vm(vm, &vm.options.class_table);
                let program = match compiler.compile_program(program_node) {
                    Ok(p) => p,
                    Err(e) => {
                        panic!("Compilation error: {}", e);
                    }
                };
                vm.report_type_warnings(compiler.diagnostics());

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
                compiler.set_seen_types(vm.options.seen_types.clone());
                compiler.set_class_table(vm.options.class_table.clone());
                crate::class_table::populate_from_vm(vm, &vm.options.class_table);
                let program = match compiler.compile_program(program_node) {
                    Ok(p) => p,
                    Err(e) => {
                        panic!("Compilation error: {}", e);
                    }
                };
                vm.report_type_warnings(compiler.diagnostics());

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
#[path = "runner_tests.rs"]
mod tests;
