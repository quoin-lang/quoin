//! Package resolution for `use` — the filesystem-agnostic seam.
//!
//! The VM never touches `std::fs` directly: it asks a [`PackageResolver`] to turn a
//! logical `(package, path)` address into Quoin source. The native CLI uses
//! [`FsResolver`] (reading `.qn` files from disk); a WASM or embedded host supplies its
//! own (in-memory, host-provided, …). See `USE_ARCH.md`.

use std::path::PathBuf;

/// Resolves a logical `(package, path)` load address to Quoin source.
///
/// `package` is `None` for the default package (the stdlib, also spellable `std`);
/// `path` is the slash-separated path with the `.qn` extension *implied*
/// (e.g. `"io/file"`). Returns the source text, or `None` if the unit isn't found.
pub trait PackageResolver {
    fn resolve(&self, package: Option<&str>, path: &str) -> Option<String>;

    /// List the `.qn` units directly in `dir` of `package` — as full unit paths without
    /// the extension (e.g. `"io/file"`), UTF-8-lexicographically sorted for a
    /// deterministic load order. `None` if the directory can't be read. Backs `use
    /// pkg:dir/*`.
    fn list(&self, package: Option<&str>, dir: &str) -> Option<Vec<String>>;
}

/// Canonical package name for the run-once key: bare (`None`) and `std:` are the same
/// package, so a file used both ways dedupes to one entry rather than double-loading
/// (which would hit "cannot redefine class").
pub fn canonical_package(package: Option<&str>) -> Option<&str> {
    match package {
        Some("std") => None,
        other => other,
    }
}

/// Filesystem-backed resolver for the native CLI. Both roots are CWD-relative in dev
/// mode: the stdlib (the default package, also spellable `std`) at `$CWD/qnlib` so its
/// source stays viewable to end-users (an installer relocates it later, e.g.
/// `/usr/local/quoin/qnlib`), and `self:` (the current project) at `$CWD`. Anchoring
/// `self_root` to the entry-point's directory can refine this later. A named package
/// (any other qualifier) is unknown for now → resolves to nothing.
pub struct FsResolver {
    stdlib_root: PathBuf,
    self_root: PathBuf,
}

impl FsResolver {
    pub fn new() -> Self {
        Self {
            stdlib_root: PathBuf::from("qnlib"),
            self_root: PathBuf::from("."),
        }
    }

    /// The filesystem root for a *file* package (the stdlib / `self`), or `None` for a named
    /// package (which is resolved as an extension package instead — see `ext_package_dir`).
    fn root_for(&self, package: Option<&str>) -> Option<&PathBuf> {
        match package {
            None | Some("std") => Some(&self.stdlib_root),
            Some("self") => Some(&self.self_root),
            Some(_) => None,
        }
    }

    /// Roots searched for a named package `<name>/`: project-local `./quoin_packages/` first, then
    /// each entry of `$QUOIN_PATH` (platform path-separated). `docs/EXT_PACKAGING.md` §6 — drop a
    /// folder on a search path; no install/registry yet.
    fn package_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.self_root.join("quoin_packages")];
        if let Some(path) = std::env::var_os("QUOIN_PATH") {
            roots.extend(std::env::split_paths(&path));
        }
        roots
    }

    /// If `package` names an **extension package** on the search path — a `<name>/` directory
    /// holding an `extension.toml` — return its absolute directory; else `None`. The first matching
    /// root wins. (`EXT_PACKAGING.md` §5: the resolver bakes the absolute dir into the synthesized
    /// `loadPackage:` glue, so there is no "where am I on disk?" problem.)
    fn ext_package_dir(&self, package: &str) -> Option<PathBuf> {
        for root in self.package_roots() {
            let dir = root.join(package);
            if dir.join("extension.toml").is_file() {
                return std::fs::canonicalize(&dir).ok();
            }
        }
        None
    }
}

impl Default for FsResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageResolver for FsResolver {
    fn resolve(&self, package: Option<&str>, path: &str) -> Option<String> {
        // A file package (stdlib / `self`): read the `.qn` file.
        if let Some(root) = self.root_for(package) {
            return std::fs::read_to_string(root.join(format!("{path}.qn"))).ok();
        }
        // A named package: an extension package resolves to one synthesized line of glue — the
        // whole-package unit is `*` (see `list`), and the resolver bakes in the absolute dir so the
        // package loads itself without a "where am I?" lookup. `Extension loadPackage:` reads the
        // manifest, spawns, installs the namespaced classes, and runs the package's `init.qn`.
        let package = package?;
        if path != "*" {
            return None;
        }
        let dir = self.ext_package_dir(package)?;
        Some(format!("Extension.loadPackage: '{}';\n", dir.display()))
    }

    fn list(&self, package: Option<&str>, dir: &str) -> Option<Vec<String>> {
        // A file package: list the `.qn` units directly in `dir` (the package root when `dir` is
        // empty — `use pkg:*`). The unit path has no leading slash for the root case.
        if let Some(root) = self.root_for(package) {
            let mut units = Vec::new();
            for entry in std::fs::read_dir(root.join(dir)).ok()? {
                let path = entry.ok()?.path();
                if path.extension().and_then(|e| e.to_str()) == Some("qn")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
                    units.push(if dir.is_empty() {
                        stem.to_string()
                    } else {
                        format!("{dir}/{stem}")
                    });
                }
            }
            units.sort(); // UTF-8 lexicographic — deterministic load order (USE_ARCH.md)
            return Some(units);
        }
        // A named extension package: `use pkg:*` (whole-package glob, empty dir) maps to one
        // synthetic unit `*`, which `resolve` turns into the `loadPackage:` glue. (A sub-glob of a
        // named package — non-empty `dir` — has no meaning here.)
        let package = package?;
        if !dir.is_empty() {
            return None;
        }
        self.ext_package_dir(package)?;
        Some(vec!["*".to_string()])
    }
}

#[cfg(test)]
#[path = "packages_tests.rs"]
mod packages_tests;

/// Whether a unit has finished loading or is still in progress — the in-progress
/// marker is what breaks cycles (a `use` that finds an in-progress entry skips,
/// seeing the partial definitions rather than recursing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadStatus {
    InProgress,
    Loaded,
}

/// One entry in the run-once registry. The registry is an ordered `Vec` (not a set)
/// because run order *is* load order, which is meaningful — see `USE_ARCH.md`.
#[derive(Debug, Clone)]
pub struct LoadedUnit {
    pub package: Option<String>,
    pub path: String,
    pub status: LoadStatus,
}
