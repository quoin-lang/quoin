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

/// Where the stdlib (the default package, also spellable `std`) is read from.
///
/// An installed `qn` has no `qnlib/` to point at, so the shipping subset is compiled
/// into the binary ([`crate::stdlib`]). Setting `QUOIN_STDLIB` to a directory reads it
/// from disk instead — which keeps the "edit a `.qn`, no rebuild" development loop, and
/// is the only way to reach the source-tree-only units (`tests/`, `benchmark`, the
/// `usetest/`/`cyc/` fixtures). `.cargo/config.toml` sets it for every cargo-run build.
pub enum StdlibSource {
    /// Compiled into the binary — the shipping default.
    Embedded,
    /// Read from this directory (`QUOIN_STDLIB`).
    Disk(PathBuf),
}

impl StdlibSource {
    /// `QUOIN_STDLIB` if set and non-empty, else the embedded copy.
    pub fn from_env() -> Self {
        match std::env::var_os(STDLIB_ENV) {
            Some(dir) if !dir.is_empty() => StdlibSource::Disk(PathBuf::from(dir)),
            _ => StdlibSource::Embedded,
        }
    }

    fn resolve(&self, path: &str) -> Option<String> {
        match self {
            StdlibSource::Embedded => crate::stdlib::resolve(path).map(str::to_string),
            StdlibSource::Disk(root) => {
                std::fs::read_to_string(root.join(format!("{path}.qn"))).ok()
            }
        }
    }

    fn list(&self, dir: &str) -> Option<Vec<String>> {
        match self {
            StdlibSource::Embedded => crate::stdlib::list(dir),
            StdlibSource::Disk(root) => list_dir(&root.join(dir), dir),
        }
    }
}

/// Points the stdlib at a directory instead of the embedded copy.
pub const STDLIB_ENV: &str = "QUOIN_STDLIB";

/// Source of a stdlib unit (extension implied), honouring `QUOIN_STDLIB`. Used by the
/// runner to load the prelude and the test framework without going through `use`.
pub fn read_stdlib_unit(path: &str) -> Option<String> {
    StdlibSource::from_env().resolve(path)
}

/// The `qnlib/` **source tree**, for units that are deliberately not embedded because
/// they are a checkout-only feature (`benchmark.qn`). `QUOIN_STDLIB` if set, else a
/// `./qnlib` that exists. `None` when neither is present — the caller reports that the
/// mode needs a source tree, rather than emitting a confusing missing-file error.
pub fn source_tree_root() -> Option<PathBuf> {
    if let StdlibSource::Disk(root) = StdlibSource::from_env() {
        return Some(root);
    }
    let cwd = PathBuf::from("qnlib");
    cwd.is_dir().then_some(cwd)
}

/// The `.qn` units directly in `dir` on disk, as full unit paths without the extension,
/// UTF-8-lexicographically sorted. `prefix` is the logical directory the paths are
/// reported under (`""` for a package root). Shared by the disk stdlib and `self:`.
fn list_dir(dir: &std::path::Path, prefix: &str) -> Option<Vec<String>> {
    let mut units = Vec::new();
    for entry in std::fs::read_dir(dir).ok()? {
        let path = entry.ok()?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("qn")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            units.push(if prefix.is_empty() {
                stem.to_string()
            } else {
                format!("{prefix}/{stem}")
            });
        }
    }
    units.sort(); // UTF-8 lexicographic — deterministic load order (USE_ARCH.md)
    Some(units)
}

/// Resolver for the native CLI. The stdlib comes from [`StdlibSource`]; `self:` (the
/// current project) is rooted at `self_root` — the directory of the entry script, so a
/// script's `use self:lib/foo` means the same thing wherever it is invoked from. A named
/// package (any other qualifier) resolves as an extension package.
pub struct FsResolver {
    stdlib: StdlibSource,
    self_root: PathBuf,
}

/// The per-user Quoin home: `$QUOIN_HOME`, defaulting to `$HOME/.quoin`. Holds the installed
/// packages (`packages/` — the last `use` search root) and their linked executables (`bin/` —
/// the directory users put on `PATH` once). `qn pkg install` writes here; `None` only when
/// neither variable is set (then there simply is no user root).
pub fn quoin_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("QUOIN_HOME") {
        return Some(PathBuf::from(home));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".quoin"))
}

impl FsResolver {
    /// `self_root` is the directory `use self:…` resolves against (the entry script's
    /// directory; the process CWD for the script-less modes — `repl`, `-e`, `test`).
    pub fn new(self_root: PathBuf) -> Self {
        Self {
            stdlib: StdlibSource::from_env(),
            self_root,
        }
    }

    /// Roots searched for a named package `<name>/`: `./quoin_packages/` first, then each entry
    /// of `$QUOIN_PATH` (platform path-separated), then the per-user install root
    /// `$QUOIN_HOME/packages` (`qn pkg install`'s target) — project beats explicit path beats
    /// installed. `docs/internal/EXT_PACKAGING.md` §6.
    ///
    /// Deliberately CWD-relative rather than `self_root`-relative: extension packaging is deferred
    /// past v0.1 (`docs/internal/RELEASE_PREP.md`), and following the script's directory would silently
    /// change where a script run from elsewhere finds its extensions.
    fn package_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![PathBuf::from("quoin_packages")];
        if let Some(path) = std::env::var_os("QUOIN_PATH") {
            roots.extend(std::env::split_paths(&path));
        }
        if let Some(home) = quoin_home() {
            roots.push(home.join("packages"));
        }
        roots
    }

    /// If `package` names an **extension package** on the search path — a `<name>/` directory
    /// holding an `quoin.toml` — return its absolute directory; else `None`. The first matching
    /// root wins. (`EXT_PACKAGING.md` §5: the resolver bakes the absolute dir into the synthesized
    /// `loadPackage:` glue, so there is no "where am I on disk?" problem.)
    fn ext_package_dir(&self, package: &str) -> Option<PathBuf> {
        for root in self.package_roots() {
            let dir = root.join(package);
            if dir.join("quoin.toml").is_file() {
                return std::fs::canonicalize(&dir).ok();
            }
        }
        None
    }
}

impl Default for FsResolver {
    fn default() -> Self {
        Self::new(PathBuf::from("."))
    }
}

impl PackageResolver for FsResolver {
    fn resolve(&self, package: Option<&str>, path: &str) -> Option<String> {
        match package {
            // The stdlib: embedded, or from `QUOIN_STDLIB`.
            None | Some("std") => return self.stdlib.resolve(path),
            // The current project: always on disk, rooted at the entry script's directory.
            Some("self") => {
                return std::fs::read_to_string(self.self_root.join(format!("{path}.qn"))).ok();
            }
            Some(_) => {}
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
        // A file package: the `.qn` units directly in `dir` (the package root when `dir` is
        // empty — `use pkg:*`). The unit path has no leading slash for the root case.
        match package {
            None | Some("std") => return self.stdlib.list(dir),
            Some("self") => return list_dir(&self.self_root.join(dir), dir),
            Some(_) => {}
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
