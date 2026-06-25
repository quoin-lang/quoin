//! Quoin-level (`.qn`) code coverage.
//!
//! Rust `cargo llvm-cov` measures which *VM* paths run; it says nothing about which
//! `qnlib` methods and source lines a `qn test` run actually exercises — the bytecode
//! interpreter is well-covered even when the Quoin code it runs is not. This module
//! closes that gap by reusing the debugger's machinery: every compiled block carries a
//! `source_map: [Option<SourceInfo>]` parallel to its bytecode, and
//! [`is_line_start`](crate::debug::is_line_start) marks the first instruction of each
//! source line. Coverage is "the debugger that never stops" — at each line-start crossing
//! it ticks a counter for `(file, line)`.
//!
//! - **Numerator** ([`CoverageState`]): a `(file, line) -> hit count` map, filled by
//!   [`VmState::coverage_tick`] from the per-instruction seam in `step_internal` (gated by
//!   `VmState::coverage`, one bool load when off — same cost model as the debugger).
//! - **Denominator** ([`VmState::build_coverage_report`]): after the run, walk every
//!   registered class's methods (and the nested blocks they reference) to enumerate *all*
//!   coverable line-starts and functions — so a method that was compiled but never called
//!   shows up at 0%, which Rust coverage structurally cannot see.
//! - **Output**: [`to_lcov`] serializes a [`CoverageReport`] to the LCOV `.info` format
//!   (the lingua franca: genhtml, Codecov, Coveralls, VS Code). The report is
//!   format-agnostic, so other writers (e.g. Cobertura XML) can be added later.

use std::collections::{BTreeMap, HashMap};

use crate::debug::is_line_start;
use crate::instruction::{Constant, Instruction, StaticBlock};
use crate::value::{ObjectPayload, SourceInfo, Value};
use crate::vm::VmState;

/// The runtime hit accumulator. Plain data (no `Gc`), so it is `require_static` on
/// `VmState`. Keyed `file -> line -> count`; `get_mut` on the hot path means only the
/// first hit per file allocates the filename.
#[derive(Debug, Default)]
pub struct CoverageState {
    hits: HashMap<String, HashMap<u32, u64>>,
}

impl CoverageState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one execution of `(file, line)`.
    pub fn record_line(&mut self, file: &str, line: u32) {
        if let Some(lines) = self.hits.get_mut(file) {
            *lines.entry(line).or_insert(0) += 1;
        } else {
            let mut lines = HashMap::new();
            lines.insert(line, 1);
            self.hits.insert(file.to_string(), lines);
        }
    }

    fn hit_count(&self, file: &str, line: u32) -> u64 {
        self.hits
            .get(file)
            .and_then(|lines| lines.get(&line))
            .copied()
            .unwrap_or(0)
    }
}

/// A format-agnostic coverage result. `BTreeMap`s keep the output deterministic (stable
/// LCOV ordering, so the writer is testable).
#[derive(Debug, Default)]
pub struct CoverageReport {
    pub files: BTreeMap<String, FileReport>,
}

#[derive(Debug, Default)]
pub struct FileReport {
    /// line number -> execution count (0 = coverable but never hit).
    pub lines: BTreeMap<u32, u64>,
    /// `Class#selector` -> its definition line and hit count.
    pub funcs: BTreeMap<String, FnReport>,
}

#[derive(Debug)]
pub struct FnReport {
    pub line: u32,
    pub hits: u64,
}

impl CoverageReport {
    /// Total coverable lines, and how many were hit.
    pub fn line_totals(&self) -> (usize, usize) {
        let mut found = 0;
        let mut hit = 0;
        for fr in self.files.values() {
            found += fr.lines.len();
            hit += fr.lines.values().filter(|&&h| h > 0).count();
        }
        (found, hit)
    }
}

/// The hit map, threaded read-only through the denominator walk. `None` when coverage
/// wasn't recorded (every line then reports 0 hits — a pure structural report).
type Hits<'a> = Option<&'a CoverageState>;

fn hit_count(hits: Hits, file: &str, line: u32) -> u64 {
    hits.map_or(0, |c| c.hit_count(file, line))
}

fn first_mapped(source_map: &[Option<SourceInfo>]) -> Option<&SourceInfo> {
    source_map.iter().flatten().next()
}

/// A block literal compiles to a `Constant::Block`, materialized by `Push` or one of the
/// fused `Send*Const` superinstructions. Pull the nested block template out of any
/// instruction that carries one, so the walk recurses into closures (`.if:{…}`, `.each:{…}`).
fn block_constant(inst: &Instruction) -> Option<&StaticBlock> {
    match inst {
        Instruction::Push(Constant::Block(sb))
        | Instruction::SendConst(Constant::Block(sb), ..)
        | Instruction::SendLocalConst(_, Constant::Block(sb), ..) => Some(sb),
        _ => None,
    }
}

/// Record every line-start in `source_map` into the report at its hit count, then recurse
/// into the nested blocks the `bytecode` references (`Instruction::Block`). The same
/// `(file, line)` reached from two blocks resolves to the same global hit count, so the
/// per-line count is the total times that source line was crossed.
fn walk_code(
    report: &mut CoverageReport,
    hits: Hits,
    source_map: &[Option<SourceInfo>],
    bytecode: &[Instruction],
) {
    for ip in 0..source_map.len() {
        if !is_line_start(source_map, ip) {
            continue;
        }
        if let Some(Some(si)) = source_map.get(ip) {
            let line = si.line as u32;
            let count = hit_count(hits, &si.filename, line);
            report
                .files
                .entry(si.filename.clone())
                .or_default()
                .lines
                .entry(line)
                .or_insert(count);
        }
    }

    for inst in bytecode {
        if let Some(sb) = block_constant(inst) {
            walk_code(report, hits, &sb.source_map, &sb.bytecode);
        }
    }
}

fn register_function(
    report: &mut CoverageReport,
    hits: Hits,
    class_name: &str,
    selector: &str,
    source_map: &[Option<SourceInfo>],
) {
    let Some(si) = first_mapped(source_map) else {
        return;
    };
    let line = si.line as u32;
    let count = hit_count(hits, &si.filename, line);
    report
        .files
        .entry(si.filename.clone())
        .or_default()
        .funcs
        .entry(format!("{class_name}#{selector}"))
        .or_insert(FnReport { line, hits: count });
}

/// Serialize a [`CoverageReport`] to the LCOV tracefile format (`.info`). One `SF` record
/// per file, with `FN`/`FNDA`/`FNF`/`FNH` (functions) and `DA`/`LF`/`LH` (lines).
pub fn to_lcov(report: &CoverageReport) -> String {
    let mut out = String::new();
    for (file, fr) in &report.files {
        out.push_str("TN:\n");
        out.push_str(&format!("SF:{file}\n"));
        for (name, f) in &fr.funcs {
            out.push_str(&format!("FN:{},{}\n", f.line, name));
        }
        for (name, f) in &fr.funcs {
            out.push_str(&format!("FNDA:{},{}\n", f.hits, name));
        }
        out.push_str(&format!("FNF:{}\n", fr.funcs.len()));
        out.push_str(&format!(
            "FNH:{}\n",
            fr.funcs.values().filter(|f| f.hits > 0).count()
        ));
        for (line, count) in &fr.lines {
            out.push_str(&format!("DA:{line},{count}\n"));
        }
        out.push_str(&format!("LF:{}\n", fr.lines.len()));
        out.push_str(&format!(
            "LH:{}\n",
            fr.lines.values().filter(|&&h| h > 0).count()
        ));
        out.push_str("end_of_record\n");
    }
    out
}

impl<'gc> VmState<'gc> {
    /// Per-instruction coverage hook, called from `step_internal` beside the debugger
    /// checkpoint while a collector is attached. Records a hit when `ip` is the first
    /// instruction of a source line (a static bytecode property), so a line counts once
    /// per arrival — including each loop iteration, but not the mid-line return after a
    /// call. Cheap: a no-op for non-line-start instructions, otherwise one map bump.
    pub(crate) fn coverage_tick(&mut self, frame_idx: usize, ip: usize) {
        let block = self.frames[frame_idx].block;
        let map = &block.source_map;
        if !is_line_start(map, ip) {
            return;
        }
        let Some(Some(si)) = map.get(ip) else {
            return;
        };
        let line = si.line as u32;
        let file = si.filename.as_str();
        if let Some(cov) = self.coverage.as_mut() {
            cov.record_line(file, line);
        }
    }

    /// Build the coverage report by walking every registered class's methods (and the
    /// nested blocks they reference) for the *coverable* lines/functions — the
    /// denominator — and looking up each line's hit count from the attached collector.
    /// A compiled-but-never-called method appears here at 0%.
    pub fn build_coverage_report(&self) -> CoverageReport {
        let hits = self.coverage.as_ref();
        let mut report = CoverageReport::default();
        let globals = self.globals.borrow();
        for val in globals.values() {
            let Value::Class(class) = val else {
                continue;
            };
            let class_ref = class.borrow();
            let class_name = class_ref.name.name.clone();
            for (selector, method) in class_ref
                .instance_methods
                .iter()
                .chain(class_ref.class_methods.iter())
            {
                let Value::Object(obj) = method else {
                    continue;
                };
                let obj_ref = obj.borrow();
                let ObjectPayload::Block(block) = &obj_ref.payload else {
                    continue; // native (Rust) method — covered by cargo llvm-cov instead
                };
                register_function(&mut report, hits, &class_name, selector, &block.source_map);
                walk_code(&mut report, hits, &block.source_map, &block.bytecode);
            }
        }
        report
    }
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;
