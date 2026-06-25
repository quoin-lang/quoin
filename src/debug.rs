//! In-VM debugger state and the pause-decision logic (debugger v0).
//!
//! A [`DebugState`] lives in `VmState::debug` (`None` for a normal run, so the step loop
//! pays one bool load when no session is attached). The step loop's checkpoint
//! ([`VmState::debug_checkpoint`]) runs once per instruction while a session is attached;
//! on a hit it suspends the coroutine with [`YieldReason::DebugBreak`], handing control to
//! the driver ([`VmState::debug_on_pause`]) which resumes the task *in place* after the
//! command loop.
//!
//! The pause reuses the same suspend/resume seam as `await_io` (a direct `yielder.suspend`
//! deep in the step loop), so a breakpoint works at any call depth — even inside a block
//! run synchronously by a native method — without threading a "paused" status back up
//! through the VM's inner loops.
//!
//! **Line granularity.** One source line compiles to several instructions, so a breakpoint
//! or step stops only at a *line start* — the first instruction of a line (see
//! [`is_line_start`]), a static property of the bytecode. This matches a debugger's "once
//! per line" intent for free: a loop's back-edge lands on the body's line-start instruction
//! (so a breakpoint there fires every iteration), while the instruction after a call
//! returns lands mid-line (so a breakpoint on the call's line does not re-fire on return).
//! Step-out is the exception — it is depth-only and fires the moment the frame returns. See
//! `docs/DEBUGGER_ARCH.md`.

use crate::error::QuoinError;
use crate::fiber::YieldReason;
use crate::value::SourceInfo;
use crate::vm::VmState;

use std::collections::{HashMap, HashSet, VecDeque};

/// What to do when resuming from a pause. Produced by the frontend (the interactive
/// `$`-command loop in Slice 3; a pre-loaded script in v0) and applied by
/// [`VmState::apply_debug_action`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugAction {
    /// Run to the next breakpoint (clear any armed step).
    Continue,
    /// Stop at the next source line, descending into calls.
    StepInto,
    /// Stop at the next source line in this frame or a shallower one (skip calls).
    StepOver,
    /// Run until this frame returns, then stop in the caller.
    StepOut,
}

/// The armed single-step kind, or absence of one (run freely to the next breakpoint).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMode {
    /// Stop at the next source line, descending into calls (step-into).
    Into,
    /// Stop at the next source line in the current frame or a shallower one — skip over
    /// calls (step-over / `next`).
    Over,
    /// Stop when execution returns to a frame shallower than the origin (step-out /
    /// `finish`).
    Out,
}

/// Where a step started — captured when the step command is issued — so line and depth
/// comparisons have a reference point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepOrigin {
    pub file: String,
    pub line: usize,
    pub depth: usize,
}

/// In-VM debugger session state. Holds only plain data (no `Gc`), so it is
/// `require_static` on `VmState`.
#[derive(Debug, Default)]
pub struct DebugState {
    /// Line breakpoints, keyed file → set of 1-indexed lines.
    pub breakpoints: HashMap<String, HashSet<usize>>,
    /// The armed single-step, or `None` to run to the next breakpoint.
    pub step: Option<StepMode>,
    /// Where the current step started (its frame depth is the reference for step
    /// over / out).
    pub origin: Option<StepOrigin>,
    /// The v0 non-interactive driver: actions applied at successive pauses (one popped per
    /// pause; empty ⇒ continue). Slice 3's interactive frontend supplies actions from
    /// `$`-commands instead, but reuses the same [`VmState::apply_debug_action`] core.
    pub script: VecDeque<DebugAction>,
    /// Pause locations recorded by the driver — `(file, line)` — so a test (and, later, a
    /// session log) can see where execution stopped.
    pub pause_log: Vec<(String, usize)>,
}

/// Whether the instruction at `ip` is the *start of a source line* within its block — its
/// mapped line differs from the immediately preceding instruction's (or it is the first
/// instruction, or the previous one is unmapped). This is a static property of the
/// bytecode: the loop-back edge of a loop lands on a line-start (so a breakpoint there
/// fires every iteration), while the instruction after a call returns lands *mid-line* (so
/// a breakpoint on the call's line does not re-fire on return). An unmapped instruction is
/// never a line start.
pub fn is_line_start(map: &[Option<SourceInfo>], ip: usize) -> bool {
    let Some(Some(cur)) = map.get(ip) else {
        return false;
    };
    if ip == 0 {
        return true;
    }
    match map.get(ip - 1) {
        Some(Some(prev)) => prev.line != cur.line || prev.filename != cur.filename,
        _ => true, // previous instruction unmapped / out of range ⇒ treat as a start
    }
}

impl DebugState {
    /// Decide whether to pause *before* executing the instruction at `pos` (`(file, line)`,
    /// or `None` when unmapped) with the frame stack at `depth`. `at_line_start` is the
    /// static line-start flag from [`is_line_start`]. Pure — the single source of truth for
    /// breakpoint and single-step semantics. Stops land only on a line's first instruction
    /// (so a breakpoint or step fires once per arrival at a line, including each loop
    /// iteration, but not on the mid-line continuation after a call returns) — except
    /// step-out, which is depth-only and fires the moment the frame returns.
    pub fn should_pause(
        &self,
        at_line_start: bool,
        pos: Option<(&str, usize)>,
        depth: usize,
    ) -> bool {
        // Step-out fires the moment we return to a shallower frame (it pauses once, then the
        // driver clears the step), regardless of where on the line we land.
        if matches!(self.step, Some(StepMode::Out)) && depth < self.origin_depth() {
            return true;
        }
        if !at_line_start {
            return false;
        }
        // A line start is always a mapped instruction, so a missing position is a no-op.
        let Some((file, line)) = pos else {
            return false;
        };
        if self
            .breakpoints
            .get(file)
            .is_some_and(|lines| lines.contains(&line))
        {
            return true;
        }
        match self.step {
            // Into: the next line start, at any depth (descends into calls, follows returns).
            Some(StepMode::Into) => true,
            // Over: the next line start in the origin frame or a shallower one (deeper line
            // starts — the insides of a call — are skipped).
            Some(StepMode::Over) => depth <= self.origin_depth(),
            Some(StepMode::Out) | None => false,
        }
    }

    fn origin_depth(&self) -> usize {
        self.origin.as_ref().map_or(0, |o| o.depth)
    }
}

impl<'gc> VmState<'gc> {
    /// Debugger checkpoint, called from `step_internal` before dispatching the instruction
    /// at `ip` in frame `frame_idx` — only while a debug session is attached. If a
    /// breakpoint or armed single-step says to stop here, suspend the coroutine with
    /// [`YieldReason::DebugBreak`]: the driver runs the command loop and resumes the task
    /// past this point (the same seam as `await_io`, so it works at any call depth). A
    /// cancel requested while paused is honored on resume.
    pub(crate) fn debug_checkpoint(
        &mut self,
        frame_idx: usize,
        ip: usize,
    ) -> Result<(), QuoinError> {
        let depth = self.frames.len();
        let block = self.frames[frame_idx].block;
        let map = &block.source_map;
        let pos = map
            .get(ip)
            .and_then(|o| o.as_ref())
            .map(|si| (si.filename.as_str(), si.line));
        let at_line_start = is_line_start(map, ip);

        let pause = match &self.debug {
            Some(d) => d.should_pause(at_line_start, pos, depth),
            None => false,
        };
        if !pause {
            return Ok(());
        }
        // Suspend and hand control to the driver. No yielder ⇒ no scheduler (benchmark
        // mode), where a debug session never runs — just proceed.
        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::DebugBreak);
        } else {
            return Ok(());
        }
        if self.sched.cancel_current {
            return Err(self.take_cancellation());
        }
        Ok(())
    }

    /// Driver-side handler when a `DebugBreak` reaches the scheduler: record where we paused,
    /// then apply the next action. **v0:** the action comes from the pre-loaded `script` (or
    /// `Continue` when it is empty). Slice 3 replaces the *source* of the action with the
    /// interactive `$`-command loop — which reads commands, inspects/evaluates against the
    /// live (still-rooted) paused frames, then calls [`apply_debug_action`] for the
    /// continue/step verb — but the arming below is unchanged.
    ///
    /// [`apply_debug_action`]: VmState::apply_debug_action
    pub(crate) fn debug_on_pause(&mut self) {
        let pos = self.debug_current_pos();
        let action = match self.debug.as_mut() {
            Some(d) => {
                if let Some((file, line)) = &pos {
                    d.pause_log.push((file.clone(), *line));
                }
                d.script.pop_front().unwrap_or(DebugAction::Continue)
            }
            None => return,
        };
        self.apply_debug_action(action);
    }

    /// Apply a resume decision: continue (clear the armed step) or arm one of the three
    /// single-steps from the current frame. Shared by the v0 script driver and Slice 3's
    /// interactive frontend.
    pub(crate) fn apply_debug_action(&mut self, action: DebugAction) {
        match action {
            DebugAction::Continue => {
                if let Some(d) = self.debug.as_mut() {
                    d.step = None;
                    d.origin = None;
                }
            }
            DebugAction::StepInto => self.arm_step(StepMode::Into),
            DebugAction::StepOver => self.arm_step(StepMode::Over),
            DebugAction::StepOut => self.arm_step(StepMode::Out),
        }
    }

    /// Arm `mode`, capturing the current top frame's `(file, line, depth)` as the step
    /// origin (the reference point `should_pause` compares against).
    fn arm_step(&mut self, mode: StepMode) {
        let depth = self.frames.len();
        let pos = self.debug_current_pos();
        if let Some(d) = self.debug.as_mut() {
            d.step = Some(mode);
            d.origin = pos.map(|(file, line)| StepOrigin { file, line, depth });
        }
    }

    /// The source position of the instruction the top frame is about to execute (its
    /// current `ip`), if mapped — used to label a pause.
    pub(crate) fn debug_current_pos(&self) -> Option<(String, usize)> {
        let frame = self.frames.last()?;
        frame
            .block
            .source_map
            .get(frame.ip)
            .and_then(|o| o.as_ref())
            .map(|si| (si.filename.clone(), si.line))
    }
}

#[cfg(test)]
#[path = "debug_tests.rs"]
mod tests;
