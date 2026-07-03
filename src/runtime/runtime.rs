use crate::arg;
use crate::compiler::Compiler;
use crate::error::QuoinError;
use crate::instruction::StaticBlock;
use crate::packages::{LoadStatus, LoadedUnit, canonical_package};
use crate::parser::ast::NodeValue;
use crate::parser::try_parse_quoin_string_named;
use crate::runtime::map::NativeMapState;
use crate::symbol::Symbol;
use crate::value::{Block, EnvFrame, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::lock::RefLock;
use gc_arena::{Gc, Mutation};
use indexmap::IndexMap;
use std::collections::HashSet;

/// An eval environment binding (a seeded local in the eval'd frame).
type Binding<'gc> = (Symbol, Value<'gc>);

pub fn build_runtime_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Runtime", Some("Object"))
        .class_method("eval:", |vm, mc, _receiver, args| {
            let code = arg!(args, String, 0);
            eval_string(vm, mc, &code, "<string>", None, &[])
        })
        .class_method("eval:self:", |vm, mc, _receiver, args| {
            let code = arg!(args, String, 0);
            let self_val = args[1];
            eval_string(vm, mc, &code, "<string>", Some(self_val), &[])
        })
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
        .class_method("arguments", |vm, mc, _receiver, _args| {
            let args_list = vm
                .options
                .arguments
                .iter()
                .map(|s| vm.new_string(mc, s.clone()))
                .collect::<Vec<_>>();
            Ok(vm.new_list(mc, args_list))
        })
        .class_method("options", |vm, mc, _receiver, _args| {
            let mut map = IndexMap::new();
            let args_list = vm
                .options
                .arguments
                .iter()
                .map(|s| vm.new_string(mc, s.clone()))
                .collect::<Vec<_>>();
            map.insert("arguments".to_string(), vm.new_list(mc, args_list));
            let supports_color_val = vm.new_bool(mc, vm.options.supports_color);
            map.insert("supports_color".to_string(), supports_color_val);
            Ok(vm.new_map(mc, map))
        })
        .class_method("supportsColor", |vm, mc, _receiver, _args| {
            Ok(vm.new_bool(mc, vm.options.supports_color))
        })
}

/// Build a runnable top-level `Block` from a freshly compiled `StaticBlock`.
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
    let decl_block = static_block.decl_block.as_ref().map(|db| {
        crate::gc!(
            mc,
            Block {
                source_info: db.source_info.clone(),
                name: db.name.clone(),
                is_nested_block: db.is_nested_block,
                param_syms: db.param_syms.clone(),
                param_types: db.param_types.clone(),
                bytecode: db.bytecode.clone(),
                parent_env: None,
                enclosing_method_id: None,
                decl_block: None,
                source_map: db.source_map.clone(),
            }
        )
    });

    crate::gc!(
        mc,
        Block {
            source_info: static_block.source_info.clone(),
            name: static_block.name.clone(),
            is_nested_block: static_block.is_nested_block,
            param_syms: static_block.param_syms.clone(),
            param_types: static_block.param_types.clone(),
            bytecode: static_block.bytecode.clone(),
            parent_env,
            enclosing_method_id: None,
            decl_block,
            source_map: static_block.source_map.clone(),
        }
    )
}

/// Extract `(Symbol, Value)` bindings from a `Map` value (keys interned), for `eval:bindings:`.
fn map_to_bindings<'gc>(map_val: Value<'gc>) -> Result<Vec<Binding<'gc>>, QuoinError> {
    map_val
        .with_native_state::<NativeMapState, _, _>(|m| {
            m.get_map()
                .iter()
                .map(|(k, v)| (Symbol::intern(k), *v))
                .collect()
        })
        .map_err(QuoinError::Other)
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
) -> Result<Value<'gc>, QuoinError> {
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
    // Share the VM's class-name accumulator so this unit sees classes earlier-compiled units
    // defined (and later units see this one's) — the basis for `unknown type Foo`.
    compiler.set_seen_types(vm.options.seen_types.clone());
    // When a `self` is supplied (`eval:self:`), don't emit the top-level `self = nil` default —
    // the frame setup binds `self` to the receiver, so `self`/`@ivars`/`self.method` resolve in
    // the eval'd code. Plain `eval:` / `use` (no receiver) keep `self == nil` at top level.
    let static_block = compiler
        .compile_program_with(program_node, self_val.is_none())
        .map_err(|e| QuoinError::ParseError(format!("Compilation error: {}", e)))?;
    crate::compiler::report_type_warnings(compiler.diagnostics());
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

pub(crate) fn eval_string<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    code: &str,
    filename: &str,
    self_val: Option<Value<'gc>>,
    bindings: &[Binding<'gc>],
) -> Result<Value<'gc>, QuoinError> {
    compile_and_execute_source(vm, mc, code, filename, self_val, bindings)
}

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
    // Bare and `std:` name the same package — canonicalize so they share one run-once
    // key instead of double-loading the same file.
    let package = canonical_package(package);
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
    vm.modules.loaded.push(LoadedUnit {
        package: package.map(|s| s.to_string()),
        path: path.to_string(),
        status: LoadStatus::InProgress,
    });
    let q = package.map(|p| format!("{p}:")).unwrap_or_default();
    let display = format!("{q}{path}.qn");
    compile_and_execute_source(vm, mc, &source, &display, None, &[])?;
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
    let package = canonical_package(package);
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
