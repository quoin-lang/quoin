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
use crate::highlighter::highlight_to_ansi;
use crate::runtime::pretty;
use crate::symbol::{Symbol, self_symbol};
use crate::value::{SourceInfo, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
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
    /// Exception types to break on (`--break-on-throw=Type,…`). Empty ⇒ off. A thrown value
    /// whose class is (a subclass of) any of these pauses the debugger at the throw site
    /// (first-chance: fires whether or not the throw is ultimately caught).
    pub break_on_throw: HashSet<String>,
    /// Exception types to break on only when *uncaught* (`--break-on-uncaught=Type,…`). Like
    /// `break_on_throw`, but pauses solely when no enclosing `catch:` has a handler whose declared
    /// type matches — decided at the innermost throw site via `VmState.handler_stack`.
    pub break_on_uncaught: HashSet<String>,
    /// Set by `debug_check_throw` when a break-on-throw fires: a one-line description of the
    /// exception, shown by `announce_pause` then cleared.
    pub pause_throw: Option<String>,
    /// Whether the current pause is a break-on-throw (post-dispatch): the throwing frame's `ip`
    /// has already advanced past the failing send, so its displayed line uses `ip - 1` (the
    /// failing instruction), matching `annotate_error`. Set in `debug_check_throw`, cleared on
    /// resume by `apply_debug_action`. A normal breakpoint/step pause is pre-dispatch (`false`).
    pub at_throw: bool,
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

        let pause = match &self.instrumentation.debug {
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
        let action = match self.instrumentation.debug.as_mut() {
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
        // Leaving the pause: a throw pause (post-dispatch) is over, so the next pause is a
        // normal pre-dispatch breakpoint/step until another break-on-throw fires.
        if let Some(d) = self.instrumentation.debug.as_mut() {
            d.at_throw = false;
        }
        match action {
            DebugAction::Continue => {
                if let Some(d) = self.instrumentation.debug.as_mut() {
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
        if let Some(d) = self.instrumentation.debug.as_mut() {
            d.step = Some(mode);
            d.origin = pos.map(|(file, line)| StepOrigin { file, line, depth });
        }
    }

    /// The instruction index to *display* for frame `idx`. The innermost frame at a normal
    /// (pre-dispatch) breakpoint/step pause is about to run `ip`, so its current line is
    /// `source_map[ip]`. Every other case shows the *last-executed* instruction (`ip - 1`): a
    /// caller frame sits at its live call site (`exec_send` bumps the caller's `ip` before
    /// dispatching), and at a break-on-throw the innermost frame's `ip` has likewise advanced
    /// past the failing send. This mirrors `annotate_error`'s `ip - 1` so a debugger location
    /// at a throw matches the error trace.
    fn frame_display_ip(&self, idx: usize) -> usize {
        let ip = self.frames[idx].ip;
        let is_innermost = idx + 1 == self.frames.len();
        let at_throw = self
            .instrumentation
            .debug
            .as_ref()
            .is_some_and(|d| d.at_throw);
        if is_innermost && !at_throw {
            ip
        } else {
            ip.saturating_sub(1)
        }
    }

    /// The source position of the instruction the top frame is paused at, if mapped — used to
    /// label a pause and as the default file for `$break LINE`. See [`Self::frame_display_ip`].
    pub(crate) fn debug_current_pos(&self) -> Option<(String, usize)> {
        let idx = self.frames.len().checked_sub(1)?;
        let ip = self.frame_display_ip(idx);
        self.frames[idx]
            .block
            .source_map
            .get(ip)
            .and_then(|o| o.as_ref())
            .map(|si| (si.filename.clone(), si.line))
    }

    /// Begin a pause: reset the focus to the top (innermost) frame.
    pub(crate) fn debug_enter_pause(&mut self) {
        let top = self.frames.len().saturating_sub(1);
        if let Some(d) = self.instrumentation.debug.as_mut() {
            d.focus = top;
        }
    }

    /// The current focus-frame index (clamped to a valid frame), or `None` with no frames.
    pub(crate) fn debug_focus(&self) -> Option<usize> {
        if self.frames.is_empty() {
            return None;
        }
        let top = self.frames.len() - 1;
        Some(
            self.instrumentation
                .debug
                .as_ref()
                .map_or(top, |d| d.focus.min(top)),
        )
    }

    /// Move the focus toward the caller (`delta < 0`, `$up`) or the callee (`delta > 0`,
    /// `$down`), clamped. Returns the new focus index (or `None` with no frames).
    pub(crate) fn debug_move_focus(&mut self, delta: isize) -> Option<usize> {
        let cur = self.debug_focus()?;
        let top = self.frames.len() - 1;
        let next = (cur as isize + delta).clamp(0, top as isize) as usize;
        if let Some(d) = self.instrumentation.debug.as_mut() {
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
            .get(self.frame_display_ip(idx))
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

    /// The variables in scope at frame `idx` as `(name, Value)`: the frame's own locals, then
    /// `self`, then `self`'s instance variables (`@x`), in declaration order. The (name, Value)
    /// source for both `debug_locals` (which renders them) and the DAP `variables` request.
    pub(crate) fn debug_frame_variables(&self, idx: usize) -> Vec<(String, Value<'gc>)> {
        let Some(frame) = self.frames.get(idx) else {
            return Vec::new();
        };
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
        pairs
    }

    /// The variables in scope at frame `idx`, each as `(name, rendered_value)`. Values are
    /// rendered structurally via the pretty-printer — no `Value` method is invoked, so this never
    /// runs user code or re-enters the suspended VM. For `$locals`.
    pub(crate) fn debug_locals(&self, idx: usize) -> Vec<(String, String)> {
        let width = self.options.console_width.map(|w| w as usize).unwrap_or(80);
        self.debug_frame_variables(idx)
            .into_iter()
            .map(|(name, val)| (name, pretty::render(val, width, false)))
            .collect()
    }

    /// Variables for the DAP `variables` request: each row is `(name, rendered_value, expandable)`.
    /// `path` empty = frame `idx`'s top-level scope (locals + `self` + `@ivars`); each subsequent
    /// index descends into a child value (via `pretty::value_children`). `expandable` = the child
    /// itself has children, so the DAP layer can mint a `variablesReference` handle for it. Live
    /// values are re-fetched from the (rooted) frames each call — nothing is held across the pause.
    pub(crate) fn debug_variables(&self, idx: usize, path: &[usize]) -> Vec<(String, String, bool)> {
        let width = self.options.console_width.map(|w| w as usize).unwrap_or(80);
        let children: Vec<(String, pretty::PpChild<'gc>)> = if path.is_empty() {
            self.debug_frame_variables(idx)
                .into_iter()
                .map(|(name, v)| (name, pretty::PpChild::Val(v)))
                .collect()
        } else {
            match self.debug_descend(idx, path) {
                Some(v) => pretty::value_children(v),
                None => return Vec::new(),
            }
        };
        children
            .into_iter()
            .map(|(name, child)| match child {
                pretty::PpChild::Val(v) => {
                    let rendered = pretty::render(v, width, false);
                    let expandable = !pretty::value_children(v).is_empty();
                    (name, rendered, expandable)
                }
                pretty::PpChild::Text(s, role) => {
                    let rendered = match role {
                        pretty::PpRole::Str => format!("'{s}'"),
                        pretty::PpRole::Number => s,
                    };
                    (name, rendered, false)
                }
            })
            .collect()
    }

    /// Resolve the live value at frame `idx` following `path`: `path[0]` selects a top-level
    /// variable, each subsequent index a child (via `pretty::value_children`). `None` if any index
    /// is out of range or lands on a pre-rendered text leaf.
    fn debug_descend(&self, idx: usize, path: &[usize]) -> Option<Value<'gc>> {
        let (first, rest) = path.split_first()?;
        let mut value = self.debug_frame_variables(idx).into_iter().nth(*first)?.1;
        for &k in rest {
            match pretty::value_children(value).into_iter().nth(k)?.1 {
                pretty::PpChild::Val(v) => value = v,
                pretty::PpChild::Text(..) => return None,
            }
        }
        Some(value)
    }

    /// Look up `name` as a local visible at frame `idx` — its own bindings, then up the
    /// lexical (closure) chain. Used for `$print <bare-local>`, which `eval:self:` can't see
    /// (it has no access to frame locals until `eval:bindings:` lands).
    pub(crate) fn debug_lookup_local(&self, idx: usize, name: &str) -> Option<Value<'gc>> {
        let mut env = Some(self.frames.get(idx)?.env);
        while let Some(e) = env {
            let borrowed = e.borrow();
            if let Some((_, val)) = borrowed.vars.iter().find(|(sym, _)| sym.as_str() == name) {
                return Some(*val);
            }
            env = borrowed.parent;
        }
        None
    }

    /// Look up `@name` (instance variable `name`, given without the `@`) on the receiver of
    /// frame `idx`. A direct field read — `eval:self:` does not expose `@ivars` to eval'd code,
    /// so `$print @x` resolves them here instead.
    pub(crate) fn debug_lookup_ivar(&self, idx: usize, name: &str) -> Option<Value<'gc>> {
        let Value::Object(obj) = self.frames.get(idx)?.receiver? else {
            return None;
        };
        let borrowed = obj.borrow();
        let slot = *borrowed.class.borrow().field_slots.get(name)?;
        borrowed.fields.get(slot).copied()
    }

    /// The locals visible at frame `idx` as `(Symbol, Value)` bindings — its own bindings, then
    /// up the lexical (closure) chain, inner shadowing outer, excluding `self`. Passed to
    /// `debug_eval` so a `$print` expression can reference frame locals.
    pub(crate) fn debug_frame_bindings(&self, idx: usize) -> Vec<(Symbol, Value<'gc>)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut env = self.frames.get(idx).map(|f| f.env);
        while let Some(e) = env {
            let borrowed = e.borrow();
            for (sym, val) in &borrowed.vars {
                if *sym != self_symbol() && seen.insert(sym.as_str()) {
                    out.push((*sym, *val));
                }
            }
            env = borrowed.parent;
        }
        out
    }

    /// Evaluate `expr` in the focus frame's context: a fresh compilation unit with `self_val`
    /// bound as `self` (so `self`/`@ivars`/`self.method` resolve) and `bindings` seeded as
    /// locals (the frame's locals), so `$print` over arbitrary frame state works.
    ///
    /// Run isolated from the paused task: the debug session is suspended (so the eval's own
    /// execution doesn't re-trip the checkpoint) and the coroutine **yielder is cleared** so
    /// `execute_block` runs synchronously instead of cooperatively suspending the (already
    /// suspended) paused coroutine. `execute_block` stops at the frame baseline, so it never
    /// runs the paused frames; on error the frame/stack are truncated back and the stray
    /// exception cleared — so a failed `$print` never corrupts the paused task. (A `$print`
    /// expression that tries to `await` fails cleanly: there is no scheduler to park on.)
    pub(crate) fn debug_eval(
        &mut self,
        mc: &Mutation<'gc>,
        expr: &str,
        self_val: Option<Value<'gc>>,
        bindings: &[(Symbol, Value<'gc>)],
    ) -> Result<Value<'gc>, String> {
        let saved_debug = self.instrumentation.debug.take();
        let saved_yielder = self.sched.yielder.take();
        let base_frames = self.frames.len();
        let base_stack = self.stack.len();
        let outcome = match crate::runtime::runtime::eval_string(
            self, mc, expr, "<debug>", self_val, bindings,
        ) {
            Ok(v) => Ok(v),
            // A `throw` in the expression: render the thrown value before it's cleared.
            Err(QuoinError::Thrown) => {
                let thrown = self.exceptions.active;
                Err(thrown
                    .map(|v| self.debug_render(v))
                    .unwrap_or_else(|| "<thrown>".to_string()))
            }
            Err(e) => Err(format!("{e}")),
        };
        if outcome.is_err() {
            self.frames.truncate(base_frames);
            self.stack.truncate(base_stack);
            self.exceptions.active = None;
        }
        self.sched.yielder = saved_yielder;
        self.instrumentation.debug = saved_debug;
        outcome
    }

    /// Whether an interactive session is watching for thrown exception types (`--break-on-throw`).
    /// A cheap gate so the `catch:` / uncaught chokepoints cost ~nothing unless a throw or an
    /// uncaught break is armed.
    pub(crate) fn has_break_on_throw(&self) -> bool {
        self.instrumentation
            .debug
            .as_ref()
            .is_some_and(|d| !d.break_on_throw.is_empty() || !d.break_on_uncaught.is_empty())
    }

    /// Whether some enclosing `catch:` (per `handler_stack`) has a handler whose declared type
    /// catches `val` — i.e. the thrown value will be caught rather than escape uncaught. A
    /// `"Object"`/untyped handler catches everything.
    fn exception_has_handler(&self, val: Value<'gc>) -> bool {
        self.exceptions.handler_stack.iter().any(|types| {
            types
                .iter()
                .any(|t| t == "Object" || self.value_matches_type(val, t))
        })
    }

    /// True if `val`'s class is (a subclass of) any of `names` — walking its class hierarchy.
    fn debug_value_class_matches(&self, val: Value<'gc>, names: &HashSet<String>) -> bool {
        let Some(mut class) = self.get_class_for_lookup(val) else {
            // A primitive (a thrown String / Integer / …): match its class name directly.
            return names.contains(&val.class_name());
        };
        loop {
            let (cname, parent) = {
                let cb = class.borrow();
                (cb.name.name.clone(), cb.parent)
            };
            if names.contains(&cname) {
                return true;
            }
            match parent {
                Some(p) => class = p,
                None => return false,
            }
        }
    }

    /// Break-on-throw checkpoint, called where an error is about to be caught (`catch:`) or to
    /// escape uncaught (`run_vm_loop`) — both with the throw-site frames still live. If a debug
    /// session is watching for the error's type, record a banner and pause via `DebugBreak`
    /// (reusing the normal pause path); on `$continue` the throw resumes its normal course.
    /// `Cancelled` / non-local-return are control flow, never exceptions — skipped.
    pub(crate) fn debug_check_throw(&mut self, mc: &Mutation<'gc>, e: &QuoinError) {
        if matches!(e, QuoinError::Cancelled | QuoinError::NonLocalReturn) {
            return;
        }
        let (throw_names, uncaught_names) = match &self.instrumentation.debug {
            Some(d) => (d.break_on_throw.clone(), d.break_on_uncaught.clone()),
            None => return,
        };
        if throw_names.is_empty() && uncaught_names.is_empty() {
            return;
        }
        // The thrown value: a user throw parks it in `active_exception`; a structured error is
        // materialized to its typed `Error` object (so `TypeError` etc. match uniformly).
        let val = match self.exceptions.active {
            Some(v) => v,
            None => self.quoinerror_to_value(mc, e),
        };
        // First-chance: break at any matching throw, caught or not.
        let throw_hit =
            !throw_names.is_empty() && self.debug_value_class_matches(val, &throw_names);
        // Uncaught: break only when the type matches AND no enclosing `catch:` will take it. The
        // `reraised` guard fires this once — at the innermost throw site, before re-raises bubble
        // the error through outer catches — where the throw-site frames are still live.
        let uncaught_hit = !uncaught_names.is_empty()
            && !self.exceptions.reraised
            && self.debug_value_class_matches(val, &uncaught_names)
            && !self.exception_has_handler(val);
        if !throw_hit && !uncaught_hit {
            return;
        }
        let label = if throw_hit {
            "broke on throw"
        } else {
            "broke on uncaught"
        };
        let banner = format!("→ {label}: {}", self.debug_render(val));
        if let Some(d) = self.instrumentation.debug.as_mut() {
            d.pause_throw = Some(banner);
            d.at_throw = true;
        }
        // Pause with frames intact (the same DebugBreak seam as a breakpoint). A clear of the
        // checkpoint isn't needed — we don't re-enter here, and `$continue` resumes the throw.
        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::DebugBreak);
        }
    }

    /// Render a value the way the debugger displays it — structurally, via the pretty-printer
    /// (no `Value` method is invoked).
    pub(crate) fn debug_render(&self, value: Value<'gc>) -> String {
        let width = self.options.console_width.map(|w| w as usize).unwrap_or(80);
        pretty::render(value, width, false)
    }

    /// A source window around frame `idx`'s current line: `context` lines on each side, with
    /// the current line marked. `None` if the source file can't be read. For `$list` and the
    /// auto-display at each pause.
    pub(crate) fn debug_source_window(&self, idx: usize, context: usize) -> Option<String> {
        let frame = self.frames.get(idx)?;
        let si = frame
            .block
            .source_map
            .get(self.frame_display_ip(idx))
            .and_then(|o| o.as_ref())?;
        let content = std::fs::read_to_string(&si.filename).ok()?;
        Some(render_source_window(
            &content,
            si.line,
            context,
            self.options.supports_color,
        ))
    }
}

/// Render the `context`-line window of `content` around 1-indexed line `cur`, with the current
/// line marked. When `colorize`, the source is syntax-highlighted (the whole file is run through
/// `highlight_to_ansi` once — ANSI codes carry no newlines, so the per-line split stays exact);
/// the marker and line number stay uncolored. Pure, so it's unit-testable without a VM.
fn render_source_window(content: &str, cur: usize, context: usize, colorize: bool) -> String {
    let highlighted = colorize.then(|| highlight_to_ansi(content));
    let lines: Vec<&str> = highlighted.as_deref().unwrap_or(content).lines().collect();
    let lo = cur.saturating_sub(context).max(1);
    let hi = (cur + context).min(lines.len());
    let mut out = String::new();
    for ln in lo..=hi {
        let marker = if ln == cur { "→" } else { " " };
        let text = lines.get(ln - 1).copied().unwrap_or("");
        out.push_str(&format!("{marker} {ln:>4} │ {text}\n"));
    }
    out
}

#[cfg(test)]
#[path = "debug_tests.rs"]
mod tests;
