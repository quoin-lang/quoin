//! Tests for the debugger command dispatch (`exec_command`): each `$`-verb maps to the right
//! `DebugState` mutation / `CommandOutcome`. The interactive line I/O and the driver's
//! `DebugPaused` loop are exercised manually (piped `qn debug`); here we pin the parsing.

use super::{CommandOutcome, exec_command};
use crate::debug::{DebugState, StepMode};
use crate::vm::{VmOptions, VmState};
use gc_arena::{Arena, Rootable};

#[test]
fn commands_map_to_breakpoints_step_modes_and_outcomes() {
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        vm.instrumentation.debug = Some(DebugState {
            interactive: true,
            ..Default::default()
        });
        vm
    });

    arena.mutate_root(|mc, vm| {
        let bps = |vm: &VmState<'_>, f: &str, l: usize| {
            vm.instrumentation
                .debug
                .as_ref()
                .unwrap()
                .breakpoints
                .get(f)
                .is_some_and(|s| s.contains(&l))
        };
        let step = |vm: &VmState<'_>| vm.instrumentation.debug.as_ref().unwrap().step;

        // $break adds a (file, line); $delete clears it.
        assert!(matches!(
            exec_command(vm, mc, "$break foo.qn:7"),
            CommandOutcome::Stay
        ));
        assert!(bps(vm, "foo.qn", 7));
        assert!(matches!(
            exec_command(vm, mc, "$b bar.qn:3"),
            CommandOutcome::Stay
        )); // alias
        assert!(bps(vm, "bar.qn", 3));
        exec_command(vm, mc, "$delete foo.qn:7");
        assert!(!bps(vm, "foo.qn", 7));

        // Step verbs arm the matching mode and signal Resume.
        assert!(matches!(
            exec_command(vm, mc, "$step"),
            CommandOutcome::Resume
        ));
        assert_eq!(step(vm), Some(StepMode::Into));
        assert!(matches!(
            exec_command(vm, mc, "$next"),
            CommandOutcome::Resume
        ));
        assert_eq!(step(vm), Some(StepMode::Over));
        assert!(matches!(
            exec_command(vm, mc, "$finish"),
            CommandOutcome::Resume
        ));
        assert_eq!(step(vm), Some(StepMode::Out));

        // $continue clears the armed step.
        assert!(matches!(exec_command(vm, mc, "$c"), CommandOutcome::Resume)); // alias
        assert_eq!(step(vm), None);

        // Inspection verbs are queries (Stay) even with no frames; $source toggles the flag.
        for q in ["$frames", "$bt", "$locals", "$l", "$list", "$up", "$down"] {
            assert!(
                matches!(exec_command(vm, mc, q), CommandOutcome::Stay),
                "{q}"
            );
        }
        assert!(matches!(
            exec_command(vm, mc, "$source off"),
            CommandOutcome::Stay
        ));
        assert!(!vm.instrumentation.debug.as_ref().unwrap().show_source);
        assert!(matches!(
            exec_command(vm, mc, "$source on"),
            CommandOutcome::Stay
        ));
        assert!(vm.instrumentation.debug.as_ref().unwrap().show_source);

        // $quit quits; help / unknown / a bare expression keep prompting.
        assert!(matches!(
            exec_command(vm, mc, "$quit"),
            CommandOutcome::Quit
        ));
        assert!(matches!(
            exec_command(vm, mc, "$help"),
            CommandOutcome::Stay
        ));
        assert!(matches!(
            exec_command(vm, mc, "$nope"),
            CommandOutcome::Stay
        ));
        assert!(matches!(
            exec_command(vm, mc, "1 + 1"),
            CommandOutcome::Stay
        ));
        assert!(matches!(exec_command(vm, mc, ""), CommandOutcome::Stay));
    });
}
