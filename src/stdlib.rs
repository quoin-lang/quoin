//! The stdlib subset compiled into the binary, so an installed `qn` runs anywhere.
//!
//! `build.rs` generates [`UNITS`] from `qnlib/`: the prelude, the test framework, and
//! the `core/` `net/` `web/` trees. The rest of `qnlib/` — the language's own test
//! suite, `benchmark.qn`, the stress corpora — is a *source-tree* feature and is only
//! reachable from a disk stdlib (`QUOIN_STDLIB`, see [`crate::packages`]).
//!
//! Unit paths are slash-separated with the `.qn` extension implied, exactly as
//! [`crate::packages::PackageResolver`] addresses them (`"core/00-bootstrap"`).

include!(concat!(env!("OUT_DIR"), "/stdlib_table.rs"));

/// Source of the embedded unit at `path` (extension implied), or `None`.
pub fn resolve(path: &str) -> Option<&'static str> {
    UNITS
        .binary_search_by(|(unit, _)| (*unit).cmp(path))
        .ok()
        .map(|i| UNITS[i].1)
}

/// The embedded units directly in `dir` (no recursion), as full unit paths — the
/// embedded half of [`crate::packages::PackageResolver::list`], backing `use core/*`.
/// `dir` is `""` for the stdlib root. `None` when no unit lives there, mirroring the
/// filesystem resolver's unreadable-directory case.
pub fn list(dir: &str) -> Option<Vec<String>> {
    let units: Vec<String> = UNITS
        .iter()
        .map(|(unit, _)| *unit)
        .filter(|unit| {
            let rest = if dir.is_empty() {
                Some(*unit)
            } else {
                unit.strip_prefix(dir).and_then(|r| r.strip_prefix('/'))
            };
            // Directly in `dir`: no further separator.
            rest.is_some_and(|r| !r.contains('/'))
        })
        .map(str::to_string)
        .collect();
    // Already sorted: `UNITS` is emitted in UTF-8 lexicographic order.
    (!units.is_empty()).then_some(units)
}

#[cfg(test)]
#[path = "stdlib_tests.rs"]
mod stdlib_tests;
