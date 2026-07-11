//! The [CLI]Spec production `parse` entry, end to end through a real qn child:
//! `-h` prints help and exits 0, misuse prints message + usage to stderr and
//! exits 2, and a good command line reaches the program. Also proves the
//! hyphen pass-through: the flags on qn's own command line reach the SCRIPT.

use std::process::{Command, Output};

const TOOL: &str = "\
var cli = [CLI]Spec.new:'greet' about:'says hello'
cli.flag:'shout' short:'s' help:'LOUDLY'
cli.positional:'name' help:'whom to greet'
var args = cli.parse
var word = (args.flag?:'shout').if:{ 'HELLO' } else:{ 'hello' }
('%1, %2' % #( word (args.at:'name') )).print
";

fn run_tool(args: &[&str]) -> Output {
    let path = std::env::temp_dir().join(format!("quoin_cli_{}.qn", std::process::id()));
    std::fs::write(&path, TOOL).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);
    out
}

#[test]
fn a_good_command_line_reaches_the_program() {
    let out = run_tool(&["-s", "quoin"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("HELLO, quoin"),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn help_prints_usage_and_exits_zero() {
    let out = run_tool(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("usage: greet [options] <name>") && stdout.contains("says hello"),
        "stdout: {stdout}"
    );
}

#[test]
fn misuse_prints_usage_to_stderr_and_exits_two() {
    let out = run_tool(&["--shot", "quoin"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown option --shot") && stderr.contains("usage: greet"),
        "stderr: {stderr}"
    );
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
}
