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
use crate::runtime::pretty;
use crate::symbol::self_symbol;
use crate::value::{SourceInfo, Value};
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
    /// `qn debug` sets this: a pause bubbles to the driver's interactive `$`-command loop
    /// (`src/debug_cli.rs`). When `false` (tests, scripted runs), a pause is handled in place
    /// from `script` (or just continues). Default `false`.
    pub interactive: bool,
    /// The frame `$locals`/`$list`/`$print` act on (a `frames` index; the top frame is
    /// `frames.len() - 1`). Reset to the top frame at each pause; moved by `$up`/`$down`.
    pub focus: usize,
    /// Whether to auto-print the source around the current line at each pause
    /// (`$source on|off`). `qn debug` sets it `true`; default `false`.
    pub show_source: bool,
    /// The non-interactive driver: actions applied at successive pauses (one popped per pause;
    /// empty ⇒ continue). The interactive frontend supplies actions from `$`-commands instead,
    /// but reuses the same [`VmState::apply_debug_action`] core.
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

    /// Begin a pause: reset the focus to the top (innermost) frame.
    pub(crate) fn debug_enter_pause(&mut self) {
        let top = self.frames.len().saturating_sub(1);
        if let Some(d) = self.debug.as_mut() {
            d.focus = top;
        }
    }

    /// The current focus-frame index (clamped to a valid frame), or `None` with no frames.
    pub(crate) fn debug_focus(&self) -> Option<usize> {
        if self.frames.is_empty() {
            return None;
        }
        let top = self.frames.len() - 1;
        Some(self.debug.as_ref().map_or(top, |d| d.focus.min(top)))
    }

    /// Move the focus toward the caller (`delta < 0`, `$up`) or the callee (`delta > 0`,
    /// `$down`), clamped. Returns the new focus index (or `None` with no frames).
    pub(crate) fn debug_move_focus(&mut self, delta: isize) -> Option<usize> {
        let cur = self.debug_focus()?;
        let top = self.frames.len() - 1;
        let next = (cur as isize + delta).clamp(0, top as isize) as usize;
        if let Some(d) = self.debug.as_mut() {
            d.focus = next;
        }
        Some(next)
    }

    /// The `(file, line, label)` of frame `idx` — `label` is the method selector or block
    /// name. `None` if the frame or its source mapping is missing.
    pub(crate) fn debug_frame_location(&self, idx: usize) -> Option<(String, usize, String)> {
        let frame = self.frames.get(idx)?;
        let si = frame
            .block
            .source_map
            .get(frame.ip)
            .and_then(|o| o.as_ref())?;
        let label = frame
            .selector
            .map(|s| s.as_str().to_string())
            .or_else(|| frame.block.name.clone())
            .unwrap_or_else(|| "<block>".to_string());
        Some((si.filename.clone(), si.line, label))
    }

    /// The backtrace, innermost frame first: one `"#i  file:line  label"` line each, with
    /// the focus frame marked. For `$frames`.
    pub(crate) fn debug_backtrace(&self) -> Vec<String> {
        let focus = self.debug_focus();
        (0..self.frames.len())
            .rev()
            .map(|i| {
                let marker = if Some(i) == focus { "→" } else { " " };
                match self.debug_frame_location(i) {
                    Some((file, line, label)) => {
                        format!("{marker} #{i}  {file}:{line}  {label}")
                    }
                    None => format!("{marker} #{i}  <no source>"),
                }
            })
            .collect()
    }

    /// The variables in scope at frame `idx`, each as `(name, rendered_value)`: the frame's
    /// own locals, then `self`, then `self`'s instance variables (`@x`). Values are rendered
    /// structurally via the pretty-printer — no `Value` method is invoked, so this never runs
    /// user code or re-enters the suspended VM. For `$locals`.
    pub(crate) fn debug_locals(&self, idx: usize) -> Vec<(String, String)> {
        let Some(frame) = self.frames.get(idx) else {
            return Vec::new();
        };
        // Collect (name, value) under the borrows, then render after dropping them.
        let mut pairs: Vec<(String, Value<'gc>)> = Vec::new();
        {
            let env = frame.env.borrow();
            for (sym, val) in &env.vars {
                if *sym != self_symbol() {
                    pairs.push((sym.as_str().to_string(), *val));
                }
            }
        }
        if let Some(receiver) = frame.receiver {
            pairs.push(("self".to_string(), receiver));
            if let Value::Object(obj) = receiver {
                let borrowed = obj.borrow();
                let class = borrowed.class.borrow();
                // `field_slots` covers the full hierarchy; sort by slot for declaration order.
                let mut ivars: Vec<(&String, usize)> =
                    class.field_slots.iter().map(|(n, s)| (n, *s)).collect();
                ivars.sort_by_key(|&(_, slot)| slot);
                for (name, slot) in ivars {
                    if let Some(val) = borrowed.fields.get(slot) {
                        pairs.push((format!("@{name}"), *val));
                    }
                }
            }
        }
        let width = self.options.console_width.map(|w| w as usize).unwrap_or(80);
        pairs
            .into_iter()
            .map(|(name, val)| (name, pretty::render(val, width, false)))
            .collect()
    }

    /// A source window around frame `idx`'s current line: `context` lines on each side, with
    /// the current line marked. `None` if the source file can't be read. For `$list` and the
    /// auto-display at each pause.
    pub(crate) fn debug_source_window(&self, idx: usize, context: usize) -> Option<String> {
        let frame = self.frames.get(idx)?;
        let si = frame
            .block
            .source_map
            .get(frame.ip)
            .and_then(|o| o.as_ref())?;
        let content = std::fs::read_to_string(&si.filename).ok()?;
        let lines: Vec<&str> = content.lines().collect();
        let cur = si.line; // 1-indexed
        let lo = cur.saturating_sub(context).max(1);
        let hi = (cur + context).min(lines.len());
        let mut out = String::new();
        for ln in lo..=hi {
            let marker = if ln == cur { "→" } else { " " };
            let text = lines.get(ln - 1).copied().unwrap_or("");
            out.push_str(&format!("{marker} {ln:>4} │ {text}\n"));
        }
        Some(out)
    }
}

#[cfg(test)]
#[path = "debug_tests.rs"]
mod tests;
