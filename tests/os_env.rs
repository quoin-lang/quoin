//! `[OS]Env` cases that need a *controlled* environment, which Quoin cannot build for itself —
//! the class is read-only by design. Each spawns `qn -e` with an environment we set.
//!
//! The distinction under test: an **unset** variable reads as `nil`, while a variable with an
//! **empty value** reads as `''`. `FOO=` is set. Collapsing the two would make `at:ifAbsent:`
//! fire for a variable the user deliberately blanked.

use std::process::Command;

/// Run `qn -e expr` with `env` applied on top of a cleared environment (plus `PATH`, which the
/// binary itself needs), returning trimmed stdout.
fn eval_with_env(expr: &str, env: &[(&str, &str)]) -> String {
    let path = std::env::var("PATH").unwrap_or_default();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
    cmd.arg("-e")
        .arg(expr)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env_clear()
        .env("PATH", path);
    // The workspace `.cargo/config.toml` points the stdlib at the source tree; `env_clear` drops
    // it, so the child falls back to the copy embedded in the binary. Both are fine here.
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run qn -e");
    assert!(
        out.status.success(),
        "qn -e failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn a_set_variable_reads_back_its_value() {
    assert_eq!(
        eval_with_env("([OS]Env.at:'QN_T').print", &[("QN_T", "hello")]),
        "hello"
    );
    assert_eq!(
        eval_with_env("([OS]Env.contains?:'QN_T').print", &[("QN_T", "hello")]),
        "true"
    );
}

#[test]
fn an_empty_value_is_the_empty_string_not_nil() {
    // `.pp` renders '' and nil distinguishably; `.print` would show both as an empty line.
    assert_eq!(
        eval_with_env("([OS]Env.at:'QN_T').pp", &[("QN_T", "")]),
        "''"
    );
    assert_eq!(
        eval_with_env("([OS]Env.contains?:'QN_T').print", &[("QN_T", "")]),
        "true",
        "`FOO=` is set, with an empty value"
    );
}

#[test]
fn an_unset_variable_is_nil() {
    assert_eq!(eval_with_env("([OS]Env.at:'QN_T').pp", &[]), "nil");
    assert_eq!(
        eval_with_env("([OS]Env.contains?:'QN_T').print", &[]),
        "false"
    );
}

#[test]
fn if_absent_fires_for_unset_but_not_for_an_empty_value() {
    let expr = "([OS]Env.at:'QN_T' ifAbsent:{ 'FELL-BACK' }).pp";
    assert_eq!(eval_with_env(expr, &[]), "'FELL-BACK'", "unset -> fallback");
    assert_eq!(
        eval_with_env(expr, &[("QN_T", "")]),
        "''",
        "an empty value is a value: the fallback must not fire"
    );
    assert_eq!(eval_with_env(expr, &[("QN_T", "v")]), "'v'");
}

#[test]
fn keys_are_sorted_and_asmap_agrees() {
    let env = &[("QN_T_C", "3"), ("QN_T_A", "1"), ("QN_T_B", "2")];
    let keys = eval_with_env("([OS]Env.keys.select:{ |k| k.starts?:'QN_T_' }).print", env);
    assert_eq!(
        keys, "#(QN_T_A QN_T_B QN_T_C)",
        "sorted, not insertion order"
    );

    let from_map = eval_with_env(
        "([OS]Env.asMap.keys.select:{ |k| k.starts?:'QN_T_' }).print",
        env,
    );
    assert_eq!(from_map, keys);
}

#[test]
fn a_non_utf8_variable_is_skipped_rather_than_mangled() {
    // A Quoin String is UTF-8; a silently-corrupted name would be worse than an absent one.
    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let path = std::env::var("PATH").unwrap_or_default();
        let out = Command::new(env!("CARGO_BIN_EXE_qn"))
            .arg("-e")
            .arg("([OS]Env.keys.select:{ |k| k.starts?:'QN_T' }).count.print")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env_clear()
            .env("PATH", path)
            .env("QN_T_OK", "fine")
            .env("QN_T_BAD", OsString::from_vec(vec![0xff, 0xfe]))
            .output()
            .expect("run qn -e");
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "1",
            "the valid name is listed, the non-UTF-8 value is skipped"
        );
    }
}
