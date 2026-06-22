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
}

/// Filesystem-backed resolver for the native CLI. For now the stdlib (the default
/// package, also spellable `std`) lives at `$CWD/qnlib` so its source stays viewable to
/// end-users; an installer will later relocate it (e.g. `/usr/local/quoin/qnlib`).
/// `self:` and named packages aren't resolved yet (Stage 2).
pub struct FsResolver {
    stdlib_root: PathBuf,
}

impl FsResolver {
    pub fn new() -> Self {
        Self {
            stdlib_root: PathBuf::from("qnlib"),
        }
    }
}

impl Default for FsResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageResolver for FsResolver {
    fn resolve(&self, package: Option<&str>, path: &str) -> Option<String> {
        let root = match package {
            None | Some("std") => &self.stdlib_root,
            // `self:` and named packages arrive in Stage 2.
            Some(_) => return None,
        };
        let full = root.join(format!("{path}.qn"));
        std::fs::read_to_string(full).ok()
    }
}

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
