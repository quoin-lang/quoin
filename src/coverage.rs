//! Quoin-level (`.qn`) code coverage.
//!
//! Rust `cargo llvm-cov` measures which *VM* paths run; it says nothing about which
//! `qnlib` methods and source lines a `qn test` run actually exercises — the bytecode
//! interpreter is well-covered even when the Quoin code it runs is not. This module
//! closes that gap by reusing the debugger's machinery: every compiled block carries a
//! `source_map: [Option<SourceInfo>]` parallel to its bytecode, and
//! [`is_line_start`](crate::debug::is_line_start) marks the first instruction of each
//! source line. Coverage is "the debugger that never stops" — at each line-start crossing
//! it ticks a counter, keyed by the *executing block* (so defining a method doesn't count
//! its body as run — see [`block_key`]).
//!
//! - **Numerator** ([`CoverageState`]): a `block -> line -> hit count` map, filled by
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
use crate::value::{SourceInfo, Value};
use crate::vm::VmState;

/// A stable identifier for a *block* (method body or nested closure): a hash of its
/// source span (`file`, byte `start`, byte `end`). The same logical block has the same
/// span whether seen as a runtime `Block` (the tick) or a `StaticBlock` template (the
/// denominator walk), so the two sides agree.
///
/// Why attribute hits per block rather than per `(file, line)`: *defining* a method runs
/// the enclosing code mapped to the method's own source line, which would otherwise count
/// the method's body as executed even when it is never called (acute for single-line
/// methods). The definition runs in the *enclosing* block; the body runs in the method's
/// block. Keying on the block keeps them apart — the enclosing block isn't in the
/// denominator, so its def-site hits are ignored.
fn block_key(file: &str, start: u32, end: u32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    file.hash(&mut h);
    start.hash(&mut h);
    end.hash(&mut h);
    h.finish()
}

/// The runtime hit accumulator. Plain data (no `Gc`), so it is `require_static` on
/// `VmState`. Keyed `block -> line -> count`.
#[derive(Debug, Default)]
pub struct CoverageState {
    hits: HashMap<u64, HashMap<u32, u64>>,
}

impl CoverageState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one execution of `line` within the block spanning `start..end` of `file`.
    pub fn record_line(&mut self, file: &str, start: u32, end: u32, line: u32) {
        let key = block_key(file, start, end);
        *self.hits.entry(key).or_default().entry(line).or_insert(0) += 1;
    }

    fn hit_count(&self, file: &str, start: u32, end: u32, line: u32) -> u64 {
        self.hits
            .get(&block_key(file, start, end))
            .and_then(|lines| lines.get(&line))
            .copied()
            .unwrap_or(0)
    }
}

/// The output format requested on the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageFormat {
    Lcov,
    Cobertura,
}

/// How a run should emit coverage, parsed from `--coverage[=fmt]` / `--coverage-out=PATH`.
#[derive(Debug, Clone)]
pub struct CoverageConfig {
    pub format: CoverageFormat,
    /// Output file, or `None` to write to stdout.
    pub out: Option<String>,
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

/// A block's source span — `(file, byte start, byte end)` — the key under which its line
/// hits are recorded. Threaded through the walk so each block looks up its *own* hits.
type Span<'a> = (&'a str, u32, u32);

fn span_of(si: &SourceInfo) -> Span<'_> {
    (&si.filename, si.start as u32, si.end as u32)
}

fn hit_count(hits: Hits, span: Span, line: u32) -> u64 {
    hits.map_or(0, |c| c.hit_count(span.0, span.1, span.2, line))
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

/// Record every line-start in `source_map` into the report at its hit count (looked up
/// under this block's `span`), then recurse into the nested closures the `bytecode`
/// references, each keyed by its own span.
fn walk_code(
    report: &mut CoverageReport,
    hits: Hits,
    span: Span,
    source_map: &[Option<SourceInfo>],
    bytecode: &[Instruction],
) {
    for ip in 0..source_map.len() {
        if !is_line_start(source_map, ip) {
            continue;
        }
        if let Some(Some(si)) = source_map.get(ip) {
            let line = si.line as u32;
            let count = hit_count(hits, span, line);
            report
                .files
                .entry(si.filename.clone())
                .or_default()
                .lines
                .entry(line)
                .or_insert(count);
        }
    }

    // Recurse into nested closures, each keyed by its own span.
    for inst in bytecode {
        if let Some(sb) = block_constant(inst) {
            if let Some(child) = &sb.source_info {
                walk_code(report, hits, span_of(child), &sb.source_map, &sb.bytecode);
            }
        }
    }
}

fn register_function(
    report: &mut CoverageReport,
    hits: Hits,
    class_name: &str,
    selector: &str,
    span: Span,
    source_map: &[Option<SourceInfo>],
) {
    let Some(si) = first_mapped(source_map) else {
        return;
    };
    let line = si.line as u32;
    let count = hit_count(hits, span, line);
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

fn rate(hit: usize, found: usize) -> f64 {
    if found == 0 {
        1.0
    } else {
        hit as f64 / found as f64
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Serialize a [`CoverageReport`] to the Cobertura XML format (consumed by GitLab CI,
/// Jenkins, Codecov). Each source file becomes a `<class>`; we carry no branch data, so
/// branch rates are 0. `timestamp` is fixed at 0 to keep the output deterministic.
pub fn to_cobertura(report: &CoverageReport) -> String {
    let (found, hit) = report.line_totals();
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" ?>\n");
    out.push_str(
        "<!DOCTYPE coverage SYSTEM \"http://cobertura.sourceforge.net/xml/coverage-04.dtd\">\n",
    );
    out.push_str(&format!(
        "<coverage lines-valid=\"{found}\" lines-covered=\"{hit}\" line-rate=\"{:.4}\" \
         branches-valid=\"0\" branches-covered=\"0\" branch-rate=\"0\" complexity=\"0\" \
         version=\"0\" timestamp=\"0\">\n",
        rate(hit, found)
    ));
    out.push_str("  <sources>\n    <source>.</source>\n  </sources>\n");
    out.push_str(&format!(
        "  <packages>\n    <package name=\"qnlib\" line-rate=\"{:.4}\" branch-rate=\"0\" complexity=\"0\">\n      <classes>\n",
        rate(hit, found)
    ));
    for (file, fr) in &report.files {
        let cl_found = fr.lines.len();
        let cl_hit = fr.lines.values().filter(|&&h| h > 0).count();
        let f = xml_escape(file);
        out.push_str(&format!(
            "        <class name=\"{f}\" filename=\"{f}\" line-rate=\"{:.4}\" branch-rate=\"0\" complexity=\"0\">\n",
            rate(cl_hit, cl_found)
        ));
        out.push_str("          <methods>\n");
        for (name, fr_fn) in &fr.funcs {
            out.push_str(&format!(
                "            <method name=\"{}\" signature=\"\" line-rate=\"{:.1}\" branch-rate=\"0\">\n              <lines>\n                <line number=\"{}\" hits=\"{}\"/>\n              </lines>\n            </method>\n",
                xml_escape(name),
                if fr_fn.hits > 0 { 1.0 } else { 0.0 },
                fr_fn.line,
                fr_fn.hits
            ));
        }
        out.push_str("          </methods>\n          <lines>\n");
        for (line, count) in &fr.lines {
            out.push_str(&format!(
                "            <line number=\"{line}\" hits=\"{count}\" branch=\"false\"/>\n"
            ));
        }
        out.push_str("          </lines>\n        </class>\n");
    }
    out.push_str("      </classes>\n    </package>\n  </packages>\n</coverage>\n");
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
        let map = &block.template.source_map;
        if !is_line_start(map, ip) {
            return;
        }
        let Some(Some(si)) = map.get(ip) else {
            return;
        };
        let Some(bsi) = &block.template.source_info else {
            return; // no block span to attribute the hit to
        };
        let line = si.line as u32;
        let (file, start, end) = (bsi.filename.as_str(), bsi.start as u32, bsi.end as u32);
        if let Some(cov) = self.instrumentation.coverage.as_mut() {
            cov.record_line(file, start, end, line);
        }
    }

    /// Build the coverage report by walking every registered class's methods (and the
    /// nested blocks they reference) for the *coverable* lines/functions — the
    /// denominator — and looking up each line's hit count from the attached collector.
    /// A compiled-but-never-called method appears here at 0%.
    pub fn build_coverage_report(&self) -> CoverageReport {
        let hits = self.instrumentation.coverage.as_ref();
        let mut report = CoverageReport::default();

        // Snapshot the class handles, then drop the globals borrow before walking.
        let classes: Vec<_> = self
            .globals
            .borrow()
            .values()
            .filter_map(|v| match v {
                Value::Class(c) => Some(*c),
                _ => None,
            })
            .collect();

        for class in classes {
            let class_ref = class.borrow();
            let class_name = class_ref.name.name.clone();
            // Snapshot (selector, method) so the class borrow isn't held across the
            // block lookup below.
            let methods: Vec<(String, Value)> = class_ref
                .instance_methods
                .iter()
                .chain(class_ref.class_methods.iter())
                .map(|(selector, method)| (selector.as_str().to_string(), *method))
                .collect();
            drop(class_ref);

            for (selector, method) in methods {
                // A QN method is a block (directly, or wrapped in a NativeMethodState);
                // a native (Rust) method has no block and is covered by cargo llvm-cov.
                let Some(block) = self.get_block_from_method(method) else {
                    continue;
                };
                let Some(bsi) = &block.template.source_info else {
                    continue; // no span to attribute hits to
                };
                let span = span_of(bsi);
                register_function(
                    &mut report,
                    hits,
                    &class_name,
                    &selector,
                    span,
                    &block.template.source_map,
                );
                walk_code(
                    &mut report,
                    hits,
                    span,
                    &block.template.source_map,
                    &block.template.bytecode,
                );
            }
        }
        report
    }
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;
