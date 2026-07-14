//! `qn pkg` — installing packages under `$QUOIN_HOME` (default `~/.quoin`; the tests
//! sandbox it to a temp dir):
//!
//! - `pkg_install_extension_package_resolves_via_use`: an installed extension package
//!   loads with `use name:*` and NO `QUOIN_PATH` — the per-user root is a built-in
//!   search root.
//! - `pkg_install_links_bins_onto_path`: a `[bin]` entry links into `$QUOIN_HOME/bin`
//!   executable (a `#!/usr/bin/env qn` script runs through the link).
//! - `pkg_reinstall_replaces_and_list_reports`: reinstall replaces the previous copy
//!   whole; `qn pkg list` reflects the new manifest.
//! - `pkg_install_rejects_bad_names`: a manifest name that isn't one plain path
//!   component is refused before anything is written.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh sandbox `$QUOIN_HOME` (and scratch space) per test.
fn sandbox(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qn_pkg_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `qn` with the sandboxed home; returns (success, stdout+stderr).
fn qn(home: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args(args)
        .env("QUOIN_HOME", home)
        .env_remove("QUOIN_PATH")
        .output()
        .expect("run qn");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// An extension package folder around the `ext_vector` fixture binary.
fn vector_package(dir: &Path) {
    write(
        &dir.join("quoin.toml"),
        &format!(
            "[package]\nname = \"vecpkg\"\nversion = \"0.1.0\"\n\n\
             [extension]\ncommand = \"{}\"\nnamespace = \"Vec\"\n",
            env!("CARGO_BIN_EXE_ext_vector")
        ),
    );
}

#[test]
fn pkg_install_extension_package_resolves_via_use() {
    let sandbox = sandbox("use");
    let home = sandbox.join("home");
    let src = sandbox.join("vecpkg");
    vector_package(&src);

    let (ok, out) = qn(&home, &["pkg", "install", src.to_str().unwrap()]);
    assert!(ok, "install failed:\n{out}");
    assert!(home.join("packages/vecpkg/quoin.toml").is_file());

    // `use vecpkg:*` must resolve from $QUOIN_HOME/packages alone (QUOIN_PATH is
    // removed, and the CWD has no quoin_packages/vecpkg).
    let script = sandbox.join("use_it.qn");
    write(
        &script,
        "use vecpkg:*;\n(([Vec]Vector.ofFloats:#( 1.0 2.0 )).sum == 3.0)\n    .if:{ 'PASS'.print } else:{ 'FAIL'.print }\n",
    );
    let (ok, out) = qn(&home, &[script.to_str().unwrap()]);
    assert!(ok && out.contains("PASS"), "use failed:\n{out}");
}

#[test]
fn pkg_install_links_bins_onto_path() {
    let sandbox = sandbox("bins");
    let home = sandbox.join("home");
    let src = sandbox.join("hello-tool");
    write(
        &src.join("quoin.toml"),
        "[package]\nname = \"hello-tool\"\nversion = \"0.2.0\"\n\n[bin]\nhello = \"bin/hello\"\n",
    );
    // Deliberately not executable on disk — the installer must chmod the copy.
    write(
        &src.join("bin/hello"),
        "#!/usr/bin/env qn\n'hello from an installed tool'.print\n",
    );

    let (ok, out) = qn(&home, &["pkg", "install", src.to_str().unwrap()]);
    assert!(ok, "install failed:\n{out}");
    let link = home.join("bin/hello");
    assert!(link.exists(), "no bin link at {}", link.display());

    // The link runs via its shebang; `qn` must be on PATH for `env` to find it.
    let qn_dir = Path::new(env!("CARGO_BIN_EXE_qn")).parent().unwrap();
    let path = std::env::join_paths(
        std::iter::once(qn_dir.to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let out = Command::new(&link)
        .env("PATH", path)
        .output()
        .expect("run linked bin");
    assert!(
        out.status.success()
            && String::from_utf8_lossy(&out.stdout).contains("hello from an installed tool"),
        "linked bin failed: {:?}\n{}{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn pkg_reinstall_replaces_and_list_reports() {
    let sandbox = sandbox("list");
    let home = sandbox.join("home");
    let src = sandbox.join("vecpkg");
    vector_package(&src);

    let (ok, _) = qn(&home, &["pkg", "install", src.to_str().unwrap()]);
    assert!(ok);
    // Reinstall with a bumped version and a stray file removed: the copy is replaced whole.
    write(&src.join("stale.txt"), "old");
    let (ok, _) = qn(&home, &["pkg", "install", src.to_str().unwrap()]);
    assert!(ok);
    std::fs::remove_file(src.join("stale.txt")).unwrap();
    write(
        &src.join("quoin.toml"),
        &format!(
            "[package]\nname = \"vecpkg\"\nversion = \"0.9.0\"\ndescription = \"vectors\"\n\n\
             [extension]\ncommand = \"{}\"\nnamespace = \"Vec\"\n",
            env!("CARGO_BIN_EXE_ext_vector")
        ),
    );
    let (ok, _) = qn(&home, &["pkg", "install", src.to_str().unwrap()]);
    assert!(ok);
    assert!(
        !home.join("packages/vecpkg/stale.txt").exists(),
        "reinstall must replace the folder whole, not merge"
    );

    let (ok, out) = qn(&home, &["pkg", "list"]);
    assert!(ok, "list failed:\n{out}");
    assert!(
        out.contains("vecpkg 0.9.0") && out.contains("vectors"),
        "list output wrong:\n{out}"
    );
}

#[test]
fn pkg_install_rejects_bad_names() {
    let sandbox = sandbox("badname");
    let home = sandbox.join("home");
    let src = sandbox.join("evil");
    write(&src.join("quoin.toml"), "[package]\nname = \"../escape\"\n");
    let (ok, out) = qn(&home, &["pkg", "install", src.to_str().unwrap()]);
    assert!(!ok, "a path-escaping name must be refused");
    assert!(out.contains("invalid package name"), "{out}");
    assert!(
        !home.join("packages").exists()
            || std::fs::read_dir(home.join("packages"))
                .map(|mut d| d.next().is_none())
                .unwrap_or(true),
        "nothing may be written for a refused install"
    );
}
