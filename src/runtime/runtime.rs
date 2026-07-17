use crate::arg;
use crate::compiler::Compiler;
use crate::error::QuoinError;
use crate::instruction::StaticBlock;
use crate::packages::{LoadStatus, LoadedUnit, canonical_package};
use crate::parser::ast::NodeValue;
use crate::parser::try_parse_quoin_string_named;
use crate::runtime::map::NativeMapState;
use crate::runtime::unit_cache::{self, CachedUnit};
use crate::symbol::Symbol;
use crate::value::{Block, EnvFrame, NativeClassBuilder, Value};
use crate::vm::VmState;
use std::sync::Arc;

use gc_arena::lock::RefLock;
use gc_arena::{Gc, Mutation};
use std::collections::HashSet;

/// An eval environment binding (a seeded local in the eval'd frame).
type Binding<'gc> = (Symbol, Value<'gc>);

pub fn build_runtime_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Runtime", Some("Object"))
        .abstract_class()
        .class_doc(
            "The running program's own runtime: command-line `arguments`, process `exit:`, \
             and the `eval:` family for compiling and running Quoin source at runtime.",
        )
        // `Runtime.exit:` / `Runtime.exit` — request process exit with a status code.
        // Uncatchable (like cancellation): the raising task unwinds through `finally`
        // blocks, and the `requested_exit` flag stops every other task at the driver,
        // which exits after normal teardown (so extension/socket `Drop`s run).
        .typed_class_method("exit:", &["Integer"], |vm, _mc, _receiver, args| {
            let code = arg!(args, Int, 0) as i32;
            vm.requested_exit = Some(code);
            Err(QuoinError::ExitRequested(code))
        })
        .doc(
            "Request process exit with the given status code. Uncatchable (like \
             cancellation): the raising task unwinds through its `finally` blocks, every \
             other task stops at the driver, and the process exits after normal teardown.",
        )
        .class_method("exit", |vm, _mc, _receiver, _args| {
            vm.requested_exit = Some(0);
            Err(QuoinError::ExitRequested(0))
        })
        .doc("As `exit:` with status 0.")
        .class_method("eval:", |vm, mc, _receiver, args| {
            let code = arg!(args, String, 0);
            eval_string(vm, mc, &code, "<string>", None, &[])
        })
        .doc(
            "Compile and run a String of Quoin source, answering its final value. A syntax \
             error raises a catchable ParseError. Definitions land in the same global scope \
             as the running program.\n\n\
             ```\n\
             Runtime.eval:'1 + 2'    \"* -> 3\n\
             ```",
        )
        .class_method("eval:self:", |vm, mc, _receiver, args| {
            let code = arg!(args, String, 0);
            let self_val = args[1];
            eval_string(vm, mc, &code, "<string>", Some(self_val), &[])
        })
        .doc(
            "As `eval:`, with `self` bound to the second argument inside the evaluated code \
             -- `self` sends and `@ivars` resolve against it.\n\n\
             ```\n\
             A <- { |@x| init -> { @x = 7 } };\n\
             Runtime.eval:'@x + 1' self:A.new    \"* -> 8\n\
             ```",
        )
        // eval:'expr' bindings:#{ 'x': 1 } — the map's entries are seeded as locals in the
        // eval'd frame, so the expression can reference them by name.
        .typed_class_method(
            "eval:bindings:",
            &["String", "Map"],
            |vm, mc, _receiver, args| {
                let code = arg!(args, String, 0);
                let bindings = map_to_bindings(args[1])?;
                eval_string(vm, mc, &code, "<string>", None, &bindings)
            },
        )
        .doc(
            "As `eval:`, with the Map's entries seeded as locals in the evaluated frame \
             (String keys become the variable names).\n\n\
             ```\n\
             Runtime.eval:'x * 2' bindings:#{ 'x': 21 }    \"* -> 42\n\
             ```",
        )
        .typed_class_method(
            "callerLocation:",
            &["Integer"],
            |vm, mc, _receiver, args| {
                // The current statement's location n frames up the interpreter
                // stack — RAW frames, blocks included: n = 0 is the frame that
                // sent callerLocation:, n = 1 its caller, and so on. Raw indexing
                // is deliberate: a caller that must skip its own plumbing (Log
                // does) walks outward comparing filenames, which stays correct
                // when a frame is missing entirely (an AOT-compiled method pushes
                // no interpreter frame) — a fixed "skip k frames" contract would
                // silently shift. Answers 'file:line:col' (diagnostic convention:
                // 1-based column), or nil past the top / no source info.
                let n = arg!(args, Int, 0);
                let Ok(n) = usize::try_from(n) else {
                    return Err(QuoinError::ValueError(format!(
                        "callerLocation: needs a frame count >= 0, got {n}"
                    )));
                };
                let loc = vm.frames.iter().rev().nth(n).and_then(|f| {
                    let frame_ip = f.ip.saturating_sub(1);
                    f.block
                        .template
                        .source_map
                        .get(frame_ip)
                        .and_then(|opt| opt.as_ref())
                        .or(f.block.template.source_info.as_ref())
                        .map(|si| format!("{}:{}:{}", si.filename, si.line, si.column + 1))
                });
                Ok(match loc {
                    Some(l) => vm.new_string(mc, l),
                    None => vm.new_nil(mc),
                })
            },
        )
        .returns("String?")
        .doc(
            "The source location ('file:line:col') of the statement executing n frames up \
             the call stack — raw frames, blocks included: 0 is the frame that sent this \
             message, 1 its caller, and so on. Nil past the top of the stack or for a \
             frame with no source info. A caller skipping its own plumbing should walk n \
             upward comparing filenames rather than hard-coding a depth (an AOT-compiled \
             frame is absent from the walk) — that is how `Log` stamps each entry.",
        )
        .typed_class_method(
            "callerLocationSkipping:",
            &["String"],
            |vm, mc, _receiver, args| {
                // The nearest call-site location whose FILE does not contain the
                // fragment — the "skip my own plumbing" walk, done natively in one
                // call because any Quoin-side loop would interpose its own frames
                // (whileDo:/each: are stdlib methods) and poison the answer.
                // Frames with no source info are skipped, not terminal.
                let frag = arg!(args, String, 0);
                let loc = vm.frames.iter().rev().find_map(|f| {
                    let frame_ip = f.ip.saturating_sub(1);
                    let si = f
                        .block
                        .template
                        .source_map
                        .get(frame_ip)
                        .and_then(|opt| opt.as_ref())
                        .or(f.block.template.source_info.as_ref())?;
                    if si.filename.contains(frag.as_str()) {
                        None
                    } else {
                        Some(format!("{}:{}:{}", si.filename, si.line, si.column + 1))
                    }
                });
                Ok(match loc {
                    Some(l) => vm.new_string(mc, l),
                    None => vm.new_nil(mc),
                })
            },
        )
        .returns("String?")
        .doc(
            "The nearest enclosing call-site location ('file:line:col') whose file does \
             NOT contain the given fragment — how a library skips its own frames when \
             stamping a caller (`Log` passes its own filename). Nil when every frame \
             matches. See `callerLocation:` for the raw indexed walk.",
        )
        .class_method("arguments", |vm, mc, _receiver, _args| {
            let args_list = vm
                .options
                .arguments
                .iter()
                .map(|s| vm.new_string(mc, s.clone()))
                .collect::<Vec<_>>();
            Ok(vm.new_list(mc, args_list))
        })
        .doc(
            "The program's command-line arguments (those after the script name), as a List \
             of Strings.",
        )
        .class_method("options", |vm, mc, _receiver, _args| {
            let mut map = Vec::new();
            let args_list = vm
                .options
                .arguments
                .iter()
                .map(|s| vm.new_string(mc, s.clone()))
                .collect::<Vec<_>>();
            map.push(("arguments".to_string(), vm.new_list(mc, args_list)));
            let supports_color_val = vm.new_bool(mc, vm.options.supports_color);
            map.push(("supports_color".to_string(), supports_color_val));
            Ok(vm.new_map(mc, map))
        })
        .doc(
            "The runtime's facts as a Map: 'arguments' (the command-line List) and \
             'supports_color' (Boolean) today.",
        )
        .class_method("supportsColor", |vm, mc, _receiver, _args| {
            Ok(vm.new_bool(mc, vm.options.supports_color))
        })
        .doc("Whether standard output supports ANSI color, as detected at startup.")
}

/// Build a runnable top-level `Block` from a freshly compiled `StaticBlock`.
// wasm32: dead only until the wasm driver (which builds the main block itself) lands.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn build_block<'gc>(
    mc: &Mutation<'gc>,
    static_block: &StaticBlock,
) -> Gc<'gc, Block<'gc>> {
    build_block_with_env(mc, static_block, None)
}

/// As [`build_block`], but with an explicit `parent_env` for the top-level block — the lexical
/// parent that `LoadLocal` walks into. Used by `eval:bindings:` to attach a frame holding the
/// seeded bindings, so the eval'd code resolves them as locals.
pub(crate) fn build_block_with_env<'gc>(
    mc: &Mutation<'gc>,
    static_block: &StaticBlock,
    parent_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
) -> Gc<'gc, Block<'gc>> {
    // Eval-path templates carry no id (their compiles don't mint them), so the
    // inline-cache cells are private — matching the one-shot nature of the blocks.
    let decl_block = static_block.decl_block.as_ref().map(|db| {
        crate::gc!(
            mc,
            Block {
                template: db.clone(),
                parent_env: None,
                enclosing_method_id: None,
                decl_block: None,
                inline_cache: crate::gcl!(mc, None),
            }
        )
    });

    crate::gc!(
        mc,
        Block {
            template: Arc::new(static_block.clone()),
            parent_env,
            enclosing_method_id: None,
            decl_block,
            inline_cache: crate::gcl!(mc, None),
        }
    )
}

/// Extract `(Symbol, Value)` bindings from a `Map` value (keys interned), for `eval:bindings:`.
fn map_to_bindings<'gc>(map_val: Value<'gc>) -> Result<Vec<Binding<'gc>>, QuoinError> {
    map_val
        .with_native_state::<NativeMapState, _, _>(|m| {
            m.entries()
                .iter()
                .map(|(_, k, v)| {
                    if let Value::Object(kobj) = k
                        && let crate::value::ObjectPayload::String(s) = &kobj.borrow().payload
                    {
                        Ok((Symbol::intern(s), *v))
                    } else {
                        Err(QuoinError::Other(
                            "eval:bindings: binding keys must be Strings".to_string(),
                        ))
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|_| QuoinError::Other("eval bindings: expected a Map".to_string()))?
}

/// How `compile_and_execute_source` is being driven: an eval (per-evaluation
/// compile, nothing cacheable) or a `use`-loaded unit (run-once — template ids,
/// AOT candidates, and a unit-cache fill under its chain key).
enum SourceMode {
    Eval,
    Unit { cache_key: u64 },
}

/// Compile `source` (named `display` for source-info / errors) into a top-level block and run
/// it to completion, returning its final value. The shared core behind `eval:` and `use`.
/// `bindings` are seeded as locals in the eval'd frame (via a parent env), and the binding
/// names are made known to the compiler so references resolve as locals (not globals).
fn compile_and_execute_source<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    source: &str,
    display: &str,
    self_val: Option<Value<'gc>>,
    bindings: &[Binding<'gc>],
    mode: SourceMode,
) -> Result<Value<'gc>, QuoinError> {
    let unit_mode = matches!(mode, SourceMode::Unit { .. });
    // Use the fallible parser so a syntax error in eval'd / `use`d source surfaces as a
    // catchable `ParseError` rather than panicking the whole VM (the panicking
    // `parse_quoin_string_named` is for the main-program entry, which fails the process).
    let ast = try_parse_quoin_string_named(source, display)
        .map_err(|e| QuoinError::ParseError(e.to_string()))?;
    let program_node = match &ast.value {
        NodeValue::Program(p) => p,
        _ => {
            return Err(QuoinError::Other(
                "Expected Program node from parser".to_string(),
            ));
        }
    };
    // Tell the compiler the binding names are locals, so `x` compiles to `LoadLocal(x)` (which
    // resolves through the env) rather than `LoadGlobal(x)`.
    let binding_names: HashSet<String> = bindings
        .iter()
        .map(|(s, _)| s.as_str().to_string())
        .collect();
    let mut compiler = if binding_names.is_empty() {
        Compiler::new()
    } else {
        Compiler::new_with_locals(binding_names)
    };
    // A `use`-loaded unit is RUN-ONCE (the load registry) — runner-shaped, not
    // eval-shaped — so it mints template ids (bounded: once per unit per
    // process) and collects AOT candidates like any runner unit. This is what
    // lets qnlib's own methods (the Iterate combinators, B2) compile natively;
    // eval/REPL/interpolation stay id-less (they compile per evaluation).
    if unit_mode && crate::tuning::aot_enabled() {
        compiler = compiler.with_template_ids().with_aot();
    } else if unit_mode {
        compiler = compiler.with_template_ids();
    }
    // Share the VM's class-name accumulator so this unit sees classes earlier-compiled units
    // defined (and later units see this one's) — the basis for `unknown type Foo`.
    compiler.set_seen_types(vm.options.seen_types.clone());
    compiler.set_class_table(vm.options.class_table.clone());
    crate::class_table::populate_from_vm(vm, &vm.options.class_table);
    // When a `self` is supplied (`eval:self:`), don't emit the top-level `self = nil` default —
    // the frame setup binds `self` to the receiver, so `self`/`@ivars`/`self.method` resolve in
    // the eval'd code. Plain `eval:` / `use` (no receiver) keep `self == nil` at top level.
    let static_block = compiler
        .compile_program_with(program_node, self_val.is_none())
        .map_err(|e| QuoinError::ParseError(format!("Compilation error: {}", e)))?;
    vm.report_type_warnings(compiler.diagnostics());
    // Fill the unit cache before AOT registration consumes the compiler (the
    // artifacts are valid even if execution below errors — a later session
    // replaying them reaches the same error).
    let unit_cache_key = match mode {
        SourceMode::Unit { cache_key } => Some(cache_key),
        SourceMode::Eval => None,
    };
    let cached_diagnostics = unit_cache_key.map(|_| compiler.diagnostics().to_vec());
    if unit_mode && crate::tuning::aot_enabled() {
        // Annotated methods compile eagerly; block templates and speculative
        // methods go pending (B3a / SPECULATIVE_AOT_ARCH S0).
        vm.register_aot_candidates(compiler.take_aot_candidates());
    }
    let static_block = Arc::new(static_block);
    if let (Some(key), Some(diagnostics)) = (unit_cache_key, cached_diagnostics) {
        unit_cache::insert(
            key,
            CachedUnit {
                program: static_block.clone(),
                diagnostics,
            },
        );
    }
    // Seed the bindings into a parent env the eval'd frame walks into.
    let parent_env = (!bindings.is_empty()).then(|| {
        let mut env = EnvFrame::new(None);
        for (sym, val) in bindings {
            env.bind(*sym, *val);
        }
        crate::gcl!(mc, env)
    });
    let block = build_block_with_env(mc, &static_block, parent_env);
    vm.execute_block(mc, block, Vec::new(), self_val)
}

/// Execute a unit-cache hit: parse and compile are skipped wholesale — see
/// `runtime::unit_cache` for why that is sound.
fn execute_cached_unit<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    unit: &CachedUnit,
) -> Result<Value<'gc>, QuoinError> {
    vm.report_type_warnings(&unit.diagnostics);
    let block = build_block_with_env(mc, &unit.program, None);
    vm.execute_block(mc, block, Vec::new(), None)
}

pub(crate) fn eval_string<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    code: &str,
    filename: &str,
    self_val: Option<Value<'gc>>,
    bindings: &[Binding<'gc>],
) -> Result<Value<'gc>, QuoinError> {
    compile_and_execute_source(vm, mc, code, filename, self_val, bindings, SourceMode::Eval)
}

/// `use self:` names the package the EXECUTING unit belongs to: inside a package's unit it
/// addresses that package's own units — so the run-once key matches however a consumer
/// spells it, and a library's internal `self:` habits work identically whether it runs as
/// a program or loads as a package (a package never wants a file from its *caller*). At
/// top level (an empty load stack) `self:` keeps meaning the entry script's root.
fn effective_package(vm: &VmState, package: Option<&str>) -> Option<String> {
    if package == Some("self")
        && let Some(ctx) = vm.modules.load_stack.last()
    {
        return ctx.clone();
    }
    package.map(str::to_string)
}

// (The bare-global-class rule for named packages is enforced at the definition
// site — `Instruction::DefineClass` consults the load stack — not by a load-time
// AST scan; the old `forbid_bare_class_definitions` pre-parse is gone.)

/// Load a unit once. Resolves `(package, path)` to source via the VM's resolver, runs
/// it in a nested top-level frame (frame-balanced), and records it in the run-once
/// registry in load order. A repeat `use` — or a cyclic one (an in-progress entry) —
/// is a no-op, so the cycle sees whatever was defined so far rather than recursing.
pub fn load_unit<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    package: Option<&str>,
    path: &str,
) -> Result<(), QuoinError> {
    let rewritten = effective_package(vm, package);
    // Bare and `std:` name the same package — canonicalize so they share one run-once
    // key instead of double-loading the same file.
    let package = canonical_package(rewritten.as_deref());
    if vm
        .modules
        .loaded
        .iter()
        .any(|u| u.package.as_deref() == package && u.path == path)
    {
        return Ok(());
    }
    let source = match vm.modules.resolver.resolve(package, path) {
        Some(s) => s,
        None => {
            let q = package.map(|p| format!("{p}:")).unwrap_or_default();
            return Err(QuoinError::Other(format!(
                "use: cannot resolve `{q}{path}`"
            )));
        }
    };
    let q = package.map(|p| format!("{p}:")).unwrap_or_default();
    let display = format!("{q}{path}.qn");
    // Advance the unit-cache chain over this unit's identity + source; the
    // resulting key covers everything loaded before it (the compile context).
    let chain = unit_cache::advance(vm.modules.unit_chain, package, path, &source);
    vm.modules.unit_chain = chain;
    let cached = unit_cache::get(chain);
    vm.modules.loaded.push(LoadedUnit {
        package: package.map(|s| s.to_string()),
        path: path.to_string(),
        status: LoadStatus::InProgress,
    });
    // The unit's package is the `self:` context for every `use` its top level executes.
    vm.modules.load_stack.push(package.map(|s| s.to_string()));
    let executed = match &cached {
        Some(unit) => execute_cached_unit(vm, mc, unit),
        None => compile_and_execute_source(
            vm,
            mc,
            &source,
            &display,
            None,
            &[],
            SourceMode::Unit { cache_key: chain },
        ),
    };
    vm.modules.load_stack.pop();
    executed?;
    if let Some(u) = vm
        .modules
        .loaded
        .iter_mut()
        .find(|u| u.package.as_deref() == package && u.path == path)
    {
        u.status = LoadStatus::Loaded;
    }
    Ok(())
}

/// Load every `.qn` unit directly in `dir` of `package` — a `use pkg:dir/*` glob — in
/// UTF-8-sorted order. Each unit still goes through `load_unit`, so run-once applies.
pub fn load_glob<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    package: Option<&str>,
    dir: &str,
) -> Result<(), QuoinError> {
    let rewritten = effective_package(vm, package);
    let package = canonical_package(rewritten.as_deref());
    let units = match vm.modules.resolver.list(package, dir) {
        Some(u) => u,
        None => {
            let q = package.map(|p| format!("{p}:")).unwrap_or_default();
            return Err(QuoinError::Other(format!(
                "use: cannot resolve glob `{q}{dir}/*`"
            )));
        }
    };
    for unit in &units {
        load_unit(vm, mc, package, unit)?;
    }
    Ok(())
}
