//! Unit tests for the pure pause-decision logic (`should_pause` / `is_line_start`). No VM /
//! arena needed — the whole point of keeping the decision pure. The `at_line_start` flag is
//! the static line-start signal the checkpoint computes via `is_line_start`; here we pass it
//! directly. The end-to-end pause/resume/step path is covered in `runner`'s tests.

use super::{DebugState, StepMode, StepOrigin, is_line_start};
use crate::value::SourceInfo;
use std::collections::{HashMap, HashSet};

fn with_breakpoints(pairs: &[(&str, usize)]) -> DebugState {
    let mut breakpoints: HashMap<String, HashSet<usize>> = HashMap::new();
    for (file, line) in pairs {
        breakpoints
            .entry((*file).to_string())
            .or_default()
            .insert(*line);
    }
    DebugState {
        breakpoints,
        ..Default::default()
    }
}

fn stepping(mode: StepMode, file: &str, line: usize, depth: usize) -> DebugState {
    DebugState {
        step: Some(mode),
        origin: Some(StepOrigin {
            file: file.to_string(),
            line,
            depth,
        }),
        ..Default::default()
    }
}

#[test]
fn breakpoint_fires_on_arrival_at_its_exact_file_and_line() {
    let d = with_breakpoints(&[("foo.qn", 3)]);
    assert!(d.should_pause(true, Some(("foo.qn", 3)), 1));
    // mid-line (not a boundary): never re-pause within the same line
    assert!(!d.should_pause(false, Some(("foo.qn", 3)), 1));
    assert!(!d.should_pause(true, Some(("foo.qn", 4)), 1)); // wrong line
    assert!(!d.should_pause(true, Some(("bar.qn", 3)), 1)); // wrong file
    assert!(!d.should_pause(true, None, 1)); // unmapped instruction
}

#[test]
fn no_step_and_no_breakpoint_never_pauses() {
    let d = DebugState::default();
    assert!(!d.should_pause(true, Some(("foo.qn", 3)), 1));
    assert!(!d.should_pause(true, None, 5));
}

#[test]
fn step_into_stops_at_the_next_line_at_any_depth() {
    let d = stepping(StepMode::Into, "foo.qn", 3, 2);
    assert!(d.should_pause(true, Some(("foo.qn", 4)), 2)); // next line, same frame
    assert!(d.should_pause(true, Some(("callee.qn", 1)), 3)); // descended into a call
    assert!(!d.should_pause(false, Some(("foo.qn", 4)), 2)); // mid-line — keep going
    assert!(!d.should_pause(true, None, 2)); // unmapped — can't tell the line
}

#[test]
fn step_over_skips_deeper_calls_but_stops_in_frame_or_shallower() {
    let d = stepping(StepMode::Over, "foo.qn", 3, 2);
    assert!(d.should_pause(true, Some(("foo.qn", 4)), 2)); // next line, same frame
    assert!(!d.should_pause(true, Some(("callee.qn", 1)), 3)); // inside a deeper call — skip
    assert!(d.should_pause(true, Some(("caller.qn", 7)), 1)); // returned shallower
    assert!(!d.should_pause(false, Some(("foo.qn", 4)), 2)); // mid-line
}

#[test]
fn step_out_stops_only_when_a_frame_is_popped() {
    let d = stepping(StepMode::Out, "foo.qn", 3, 3);
    assert!(!d.should_pause(true, Some(("foo.qn", 4)), 3)); // same depth
    assert!(!d.should_pause(true, Some(("callee.qn", 1)), 5)); // deeper
    assert!(d.should_pause(true, Some(("caller.qn", 7)), 2)); // shallower — returned
    // step-out is depth-only: it fires even mid-line / on an unmapped instruction
    assert!(d.should_pause(false, None, 2));
}

#[test]
fn a_breakpoint_wins_even_while_stepping_over_a_deeper_call() {
    // Step-over would skip a deeper frame, but a breakpoint arrival there must still pause.
    let mut d = stepping(StepMode::Over, "foo.qn", 3, 2);
    d.breakpoints
        .entry("callee.qn".to_string())
        .or_default()
        .insert(5);
    assert!(d.should_pause(true, Some(("callee.qn", 5)), 7));
}

fn si(file: &str, line: usize) -> Option<SourceInfo> {
    Some(SourceInfo {
        filename: file.to_string(),
        line,
        column: 0,
        start: 0,
        end: 0,
        source_text: None,
    })
}

#[test]
fn is_line_start_marks_the_first_instruction_of_each_line() {
    // ip:  0          1          2          3      4          5
    let map = vec![
        si("f.qn", 1),
        si("f.qn", 1),
        si("f.qn", 2),
        None,
        si("f.qn", 2),
        si("g.qn", 2),
    ];
    assert!(is_line_start(&map, 0)); // first instruction is always a line start
    assert!(!is_line_start(&map, 1)); // same line as the previous instruction
    assert!(is_line_start(&map, 2)); // line changed 1 → 2
    assert!(!is_line_start(&map, 3)); // unmapped is never a line start
    assert!(is_line_start(&map, 4)); // previous instruction unmapped ⇒ treat as a start
    assert!(is_line_start(&map, 5)); // file changed (same line number, different file)
    assert!(!is_line_start(&map, 99)); // out of range
}
