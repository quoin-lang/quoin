//! Executable Quoin scripts, end to end: a `#!` line pointing at the interpreter,
//! `chmod +x`, and DIRECT execution — the OS invokes qn with the script path and
//! the script's own arguments, which must reach `Runtime.arguments` verbatim,
//! hyphens included (qn's parser must not eat the script's flags).

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn a_shebang_script_executes_directly_with_hyphen_arguments() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("quoin_shebang_{}.qn", std::process::id()));
    // The shebang names the freshly built binary directly — `env qn` would need
    // qn on the test environment's PATH.
    let script = format!(
        "#!{}\n('args: %' % #( Runtime.arguments.join:' ' )).print\n",
        env!("CARGO_BIN_EXE_qn")
    );
    std::fs::write(&path, script).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();

    let out = Command::new(&path)
        .args(["--verbose", "-o", "out.txt", "input.csv"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("execute the script directly");
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("args: --verbose -o out.txt input.csv"),
        "hyphen args must reach the script verbatim\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn error_positions_are_not_shifted_by_the_shebang() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("quoin_shebang_err_{}.qn", std::process::id()));
    std::fs::write(&path, "#!/usr/bin/env qn\n'ok'.print\nnil.bogus\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(":3:1") || stderr.contains(":3:0"),
        "the failing send is on line 3, shebang included\n{stderr}"
    );
}

#[test]
fn a_bin_directory_script_anchors_self_at_the_project_root() {
    // The installable-tool convention: <root>/bin/tool + <root>/lib/*.qn — the
    // script's `use self:lib/…` must resolve from ANY invoking directory, so
    // `script_self_root` anchors a bin/-resident script at bin's parent.
    let root = std::env::temp_dir().join(format!("quoin_binroot_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::write(
        root.join("lib/probe.qn"),
        "[BinRoot]Probe <- { .meta <-- { hello -> { 'lib loaded' } } }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("bin/tool"),
        "#!/usr/bin/env qn\nuse self:lib/probe\n[BinRoot]Probe.hello.print\n",
    )
    .unwrap();
    // Invoke from an unrelated cwd (the temp root itself), extensionless path.
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(root.join("bin/tool"))
        .current_dir(std::env::temp_dir())
        .output()
        .expect("run qn");
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("lib loaded"));
}
