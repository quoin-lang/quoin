//! `qn pkg` — install and list Quoin packages under the per-user home.
//!
//! The home is `$QUOIN_HOME` (default `$HOME/.quoin`): `packages/<name>/` is the last `use`
//! search root (`FsResolver::package_roots`), so an installed package needs no `QUOIN_PATH`
//! entry, and each `[bin]` manifest entry links into `bin/` — the one directory a user puts
//! on `PATH`. This is `docs/internal/EXT_PACKAGING.md` §9's tooling, v1-scoped to
//! `install` + `list` (no registry/fetch/versions; uninstall deferred).
//!
//! A package here is anything with a `quoin.toml` — an extension package (`[extension]`
//! launch spec, loaded by `use name:*`) or a pure-Quoin program like quern (`[bin]` only;
//! its executables land on the PATH, and `use` does not apply).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::packages::quoin_home;

#[derive(clap::Subcommand, Debug)]
pub enum PkgCmd {
    /// Install a package folder (its quoin.toml names it) into $QUOIN_HOME/packages,
    /// linking any [bin] entries into $QUOIN_HOME/bin
    Install {
        /// The package directory to install (must contain quoin.toml)
        #[arg(value_name = "DIR")]
        dir: String,
    },
    /// List the installed packages
    List,
}

/// Run one `qn pkg` subcommand; the returned code is the process exit code.
pub fn run(cmd: PkgCmd) -> i32 {
    let result = match cmd {
        PkgCmd::Install { dir } => install(Path::new(&dir)),
        PkgCmd::List => list(),
    };
    match result {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("qn pkg: {message}");
            1
        }
    }
}

/// The slice of `quoin.toml` the installer needs: identity for the destination folder and
/// the `[bin]` links. (`[extension]` is the *loader's* concern — `Extension loadPackage:` /
/// `use` — and is deliberately not required here.)
struct PkgManifest {
    name: String,
    version: Option<String>,
    description: Option<String>,
    /// `[bin]` — executable name -> package-relative path.
    bins: BTreeMap<String, String>,
}

fn read_manifest(dir: &Path) -> Result<PkgManifest, String> {
    let path = dir.join("quoin.toml");
    let text = fs::read_to_string(&path).map_err(|e| {
        format!(
            "cannot read {}: {e} (a package needs a quoin.toml)",
            path.display()
        )
    })?;
    let value: toml::Value = text
        .parse()
        .map_err(|e| format!("invalid {}: {e}", path.display()))?;

    let dir_name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("package");
    let package = value.get("package");
    let name = package
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(dir_name)
        .to_string();
    // The name becomes a directory under packages/ and `use`'s package name — keep it a
    // single plain path component.
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(format!(
            "invalid package name '{name}' in {}",
            path.display()
        ));
    }
    let get_str = |key: &str| {
        package
            .and_then(|p| p.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };

    let mut bins = BTreeMap::new();
    if let Some(table) = value.get("bin") {
        let table = table.as_table().ok_or_else(|| {
            format!(
                "[bin] must be a table of name = \"path\" in {}",
                path.display()
            )
        })?;
        for (bin_name, rel) in table {
            let rel = rel.as_str().ok_or_else(|| {
                format!("[bin] {bin_name} must be a package-relative path string")
            })?;
            if bin_name.is_empty() || bin_name.contains(['/', '\\']) {
                return Err(format!("invalid [bin] name '{bin_name}'"));
            }
            bins.insert(bin_name.clone(), rel.to_string());
        }
    }

    Ok(PkgManifest {
        name,
        version: get_str("version"),
        description: get_str("description"),
        bins,
    })
}

/// Copy `src` into `dst` recursively (which must not exist yet). `.git` is skipped — it is
/// repository state, not package content; everything else ships verbatim.
fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        if entry.file_name() == ".git" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn home_or_err() -> Result<PathBuf, String> {
    quoin_home().ok_or_else(|| "no home: set $QUOIN_HOME (or $HOME)".to_string())
}

fn install(dir: &Path) -> Result<(), String> {
    let src = fs::canonicalize(dir)
        .map_err(|e| format!("cannot resolve package dir '{}': {e}", dir.display()))?;
    let manifest = read_manifest(&src)?;

    // Every [bin] target must exist in the SOURCE before anything is written.
    for (bin_name, rel) in &manifest.bins {
        let target = src.join(rel);
        if !target.is_file() {
            return Err(format!(
                "[bin] {bin_name} points at '{rel}', which is not a file in the package"
            ));
        }
    }

    let home = home_or_err()?;
    let packages = home.join("packages");
    fs::create_dir_all(&packages)
        .map_err(|e| format!("cannot create {}: {e}", packages.display()))?;

    // Staged copy + rename: a failed copy never leaves a half-installed package; a
    // reinstall replaces the previous copy whole.
    let staging = packages.join(format!(".staging-{}", manifest.name));
    let dest = packages.join(&manifest.name);
    let _ = fs::remove_dir_all(&staging);
    copy_dir(&src, &staging).map_err(|e| {
        let _ = fs::remove_dir_all(&staging);
        format!("copy failed: {e}")
    })?;
    let _ = fs::remove_dir_all(&dest);
    fs::rename(&staging, &dest).map_err(|e| format!("cannot move into place: {e}"))?;

    let version = manifest.version.as_deref().unwrap_or("0.0.0");
    println!(
        "installed {} {} -> {}",
        manifest.name,
        version,
        dest.display()
    );

    if !manifest.bins.is_empty() {
        let bin_dir = home.join("bin");
        fs::create_dir_all(&bin_dir)
            .map_err(|e| format!("cannot create {}: {e}", bin_dir.display()))?;
        for (bin_name, rel) in &manifest.bins {
            let target = dest.join(rel);
            make_executable(&target)?;
            let link = bin_dir.join(bin_name);
            let _ = fs::remove_file(&link);
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &link)
                .map_err(|e| format!("cannot link {}: {e}", link.display()))?;
            #[cfg(not(unix))]
            fs::copy(&target, &link)
                .map(|_| ())
                .map_err(|e| format!("cannot copy {}: {e}", link.display()))?;
            println!("linked {} -> {}", link.display(), target.display());
        }
        warn_if_off_path(&bin_dir);
    }
    Ok(())
}

/// Shebang scripts arrive from checkouts/tarballs with arbitrary modes; a linked
/// executable must actually execute.
fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta =
            fs::metadata(path).map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(path, perms)
            .map_err(|e| format!("cannot chmod {}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// One line, once per install, when the bin dir isn't on PATH — the single manual step.
fn warn_if_off_path(bin_dir: &Path) {
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|entry| entry == bin_dir))
        .unwrap_or(false);
    if !on_path {
        println!(
            "note: {} is not on your PATH — add it to run the linked executables",
            bin_dir.display()
        );
    }
}

fn list() -> Result<(), String> {
    let packages = home_or_err()?.join("packages");
    let entries = match fs::read_dir(&packages) {
        Ok(entries) => entries,
        Err(_) => {
            println!("no packages installed in {}", packages.display());
            return Ok(());
        }
    };
    let mut any = false;
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.join("quoin.toml").is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        let m = match read_manifest(&dir) {
            Ok(m) => m,
            Err(message) => {
                eprintln!("qn pkg: skipping {}: {message}", dir.display());
                continue;
            }
        };
        any = true;
        let version = m.version.as_deref().unwrap_or("0.0.0");
        let bins = if m.bins.is_empty() {
            String::new()
        } else {
            format!(
                "  [bin: {}]",
                m.bins.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        };
        let description = m.description.as_deref().unwrap_or("");
        println!("{} {}{}  {}", m.name, version, bins, description);
    }
    if !any {
        println!("no packages installed in {}", packages.display());
    }
    Ok(())
}
