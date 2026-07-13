//! The target-independent front half of running a program unit: compiler setup and
//! the compile-into-a-live-VM sequence, shared by the native runner modes
//! (run/test/benchmark/repl) and the coroutine-less `direct_runner` so the two can't
//! drift. The back half — *driving* the started block — is what differs per target:
//! the native runner wraps it in a corosensei coroutine under the scheduler, the
//! direct runner steps `run_dispatch` on the caller's stack.

use crate::compiler::Compiler;
use crate::parser::NodeValue;
use crate::parser::ast::Node;
use crate::vm::VmState;
use gc_arena::Mutation;
use std::sync::Arc;

/// The once-per-unit compiler: template ids for shared inline caches, plus AOT
/// candidate collection when `QN_AOT=1` (docs/internal/AOT_ARCH.md).
pub fn unit_compiler() -> Compiler {
    let c = Compiler::new().with_template_ids();
    if crate::tuning::aot_enabled() {
        c.with_aot()
    } else {
        c
    }
}

/// Compile and register this unit's AOT candidates (no-op when `QN_AOT=0`).
/// METHOD candidates compile eagerly (few, hot by construction); BLOCK
/// templates stash as pending and compile lazily on first invocation
/// (B3a — eager compilation of every literal cost ~+34ms startup).
pub fn compile_unit_aot(vm: &mut VmState, compiler: &mut Compiler) {
    if !crate::tuning::aot_enabled() {
        return;
    }
    vm.register_aot_candidates(compiler.take_aot_candidates());
}

/// The unit failed to compile; the diagnostic was already rendered through the VM's
/// stderr sink. Deliberately payload-free — callers only branch on it.
#[derive(Debug)]
pub struct CompileReported;

/// Compile one parsed program into `vm` and start its main block — the shared
/// per-unit sequence every execution mode runs before its driver takes over. A
/// compile error is a *user* error (a typo, an undeclared local, a reassigned
/// `let`): it is reported through the VM's stderr sink and returns
/// `Err(CompileReported)` rather than panicking.
pub fn compile_and_start<'gc>(
    mc: &Mutation<'gc>,
    vm: &mut VmState<'gc>,
    ast: &Node,
) -> Result<(), CompileReported> {
    let program_node = match &ast.value {
        NodeValue::Program(p) => p,
        _ => {
            panic!("Error: Root AST node is not a ProgramNode");
        }
    };

    let mut compiler = unit_compiler();
    compiler.set_seen_types(vm.options.seen_types.clone());
    compiler.set_class_table(vm.options.class_table.clone());
    crate::class_table::populate_from_vm(vm, &vm.options.class_table);
    let program = match compiler.compile_program(program_node) {
        Ok(p) => p,
        Err(e) => {
            // Rendered through the VM's stderr sink: `file:line:col: error: …`
            // plus the offending line and caret, like a type warning.
            vm.report_compile_error(&e);
            return Err(CompileReported);
        }
    };
    vm.report_type_warnings(compiler.diagnostics());
    compile_unit_aot(vm, &mut compiler);

    let main_block = vm.block_from_template(mc, Arc::new(program), None, None);
    vm.start_block(mc, main_block, Vec::new(), None, None);
    Ok(())
}
