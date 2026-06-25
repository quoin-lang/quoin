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
//! or step stops only at a *line boundary* — when execution arrives at a line, not on every
//! instruction of it. "Arrival" means the `(file, line)` changed from the previous mapped
//! instruction, *or* we entered a new frame (so a loop body re-entered via a fresh block
//! invocation re-fires each iteration). See `docs/DEBUGGER_ARCH.md`.

use crate::error::QuoinError;
use crate::fiber::YieldReason;
use crate::vm::VmState;

use std::collections::{HashMap, HashSet};

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
    /// Where the current step started (for line / depth comparisons).
    pub origin: Option<StepOrigin>,
    /// The previous mapped instruction's `(file, line, frame_id)`, for line-boundary
    /// detection. Updated every checkpoint.
    pub prev: Option<(String, usize, usize)>,
    /// Pause locations recorded by the v0 stub driver — `(file, line)` — so a test can
    /// assert the mechanism fired. Superseded by the interactive `$`-command loop (Slice 3).
    pub pause_log: Vec<(String, usize)>,
}

impl DebugState {
    /// Whether the instruction at `pos` in frame `frame_id` is the *arrival* at a line —
    /// the line/file differs from the previous mapped instruction, or we are in a new
    /// frame. An unmapped instruction (`None`) is never an arrival.
    fn at_line_boundary(&self, pos: Option<(&str, usize)>, frame_id: usize) -> bool {
        match (pos, &self.prev) {
            (Some((f, l)), Some((pf, pl, pid))) => l != *pl || f != pf.as_str() || frame_id != *pid,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// Decide whether to pause *before* executing the instruction at `pos` (`(file, line)`,
    /// or `None` when unmapped) with the frame stack at `depth`. `at_boundary` is the
    /// arrival flag from [`at_line_boundary`]. Pure — the single source of truth for
    /// breakpoint and single-step semantics.
    pub fn should_pause(
        &self,
        at_boundary: bool,
        pos: Option<(&str, usize)>,
        depth: usize,
    ) -> bool {
        // Step-out is depth-only and fires the moment we return to a shallower frame,
        // regardless of line boundary (it pauses once, then the driver clears the step).
        if matches!(self.step, Some(StepMode::Out)) && depth < self.origin_depth() {
            return true;
        }
        // Everything else stops only on arrival at a line (once per line, not per
        // instruction; once per iteration for a loop body re-entered in a new frame).
        if !at_boundary {
            return false;
        }
        // A boundary implies a mapped instruction (see `at_line_boundary`), so a missing
        // position here means nothing to stop on.
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
            // Into: any newly-arrived line, at whatever depth (descends into calls).
            Some(StepMode::Into) => true,
            // Over: only in the origin frame or a shallower one (skip deeper calls).
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
        let frame_id = self.frames[frame_idx].id;
        let pos = self.frames[frame_idx]
            .block
            .source_map
            .get(ip)
            .and_then(|o| o.as_ref())
            .map(|si| (si.filename.as_str(), si.line));

        let pause = match &self.debug {
            Some(d) => d.should_pause(d.at_line_boundary(pos, frame_id), pos, depth),
            None => false,
        };

        // Record this line as the new "previous" for the next boundary check (mapped
        // instructions only; an unmapped one leaves the last known line in place).
        if let Some((file, line)) = pos {
            let prev = (file.to_string(), line, frame_id);
            if let Some(d) = self.debug.as_mut() {
                d.prev = Some(prev);
            }
        }

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

    /// Driver-side handler when a `DebugBreak` reaches the scheduler. **v0 stub:** record the
    /// pause location and continue (clear any armed step). Slice 3 replaces this body with
    /// the interactive `$`-command loop (which reads commands, inspects/evaluates against the
    /// live — still-rooted — paused frames, sets the next step/continue, and resumes).
    pub(crate) fn debug_on_pause(&mut self) {
        let pos = self.debug_current_pos();
        if let Some(d) = self.debug.as_mut() {
            if let Some((file, line)) = &pos {
                d.pause_log.push((file.clone(), *line));
            }
            d.step = None;
            d.origin = None;
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
