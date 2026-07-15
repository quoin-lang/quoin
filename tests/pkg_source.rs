//! Source packages: a named package's `[lib]` units load through the ordinary `use`
//! pipeline (the v0.1.1 package story; `docs/internal/EXT_PACKAGING.md`, source packages).
//!
//! - `source_package_glob_loads_lib_units`: `use pkg:*` loads the `[lib]` root's units in
//!   sorted order; a later explicit `use pkg:unit` is a run-once no-op (a re-run would die
//!   on class redefinition).
//! - `source_unit_loads_siblings_via_self`: inside a package's unit, `use self:` addresses
//!   the PACKAGE's own units, not the consumer's project — the load-context rewrite.
//! - `bare_global_class_definition_is_refused`: a package unit defining a bare-global
//!   class is a load-time error naming the namespace rule; nothing runs.
//! - `extension_classes_install_before_source_units`: in a both-kind package the synthetic
//!   `*` unit (spawn + install) precedes the `[lib]` units, so source units can reopen the
//!   installed classes.
//! - `init_unit_is_never_globbed`: `init.qn` is the extension hook (evaluated by
//!   `loadPackage:`), excluded from `[lib]` listings even when the lib root contains it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn sandbox(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qn_src_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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

/// A pure-Quoin source package: two units, the second reaching the first via `use self:`.
fn mathkit(dir: &Path) {
    write(
        &dir.join("quoin.toml"),
        "[package]\nname = \"mathkit\"\n\n[lib]\nroot = \"lib\"\n",
    );
    write(
        &dir.join("lib/adder.qn"),
        "[MathKit]Adder <- { |@n|\n    init: -> { |n| @n = n }\n    plus: -> { |x| @n + x }\n}\n",
    );
    write(
        &dir.join("lib/doubler.qn"),
        "use self:adder;\n[MathKit]Doubler <- {\n    double: -> { |x| ([MathKit]Adder.new:{ var n = x }).plus:x }\n}\n",
    );
}

fn install(home: &Path, src: &Path) {
    let (ok, out) = qn(home, &["pkg", "install", src.to_str().unwrap()]);
    assert!(ok, "install failed:\n{out}");
}

fn run_expr(home: &Path, expr: &str) -> (bool, String) {
    qn(home, &["-e", expr])
}

#[test]
fn source_package_glob_loads_lib_units() {
    let sandbox = sandbox("glob");
    let home = sandbox.join("home");
    mathkit(&sandbox.join("mathkit"));
    install(&home, &sandbox.join("mathkit"));

    // The glob loads adder then doubler; the explicit re-use is a run-once no-op — if the
    // unit re-ran, the class redefinition would error.
    let (ok, out) = run_expr(
        &home,
        "use mathkit:*; use mathkit:adder; (([MathKit]Doubler.new).double:21).print",
    );
    assert!(ok && out.contains("42"), "{out}");
}

#[test]
fn source_unit_loads_siblings_via_self() {
    let sandbox = sandbox("selfref");
    let home = sandbox.join("home");
    mathkit(&sandbox.join("mathkit"));
    install(&home, &sandbox.join("mathkit"));

    // Load ONLY doubler: its `use self:adder` must resolve inside the package (the
    // consumer has no adder.qn anywhere).
    let (ok, out) = run_expr(
        &home,
        "use mathkit:doubler; (([MathKit]Doubler.new).double:5).print",
    );
    assert!(ok && out.contains("10"), "{out}");
}

#[test]
fn bare_global_class_definition_is_refused() {
    let sandbox = sandbox("bare");
    let home = sandbox.join("home");
    let src = sandbox.join("mathkit");
    mathkit(&src);
    write(
        &src.join("lib/evil.qn"),
        "Sneaky <- { hi -> { 'polluted' } }\n",
    );
    install(&home, &src);

    let (ok, out) = run_expr(&home, "use mathkit:*");
    assert!(!ok, "a bare-global class definition must fail the load");
    assert!(
        out.contains("bare-global class `Sneaky`") && out.contains("[Mathkit]Sneaky"),
        "{out}"
    );
    // And the pollution never happened: the class must not exist afterwards.
    let (ok, out) = run_expr(
        &home,
        "{ use mathkit:* }.catch:{ |e| nil }; (Class.exists?:#Sneaky).print",
    );
    assert!(ok && out.contains("false"), "{out}");
}

#[test]
fn bare_global_class_definition_is_refused_even_dynamically() {
    // The rule is enforced at the DEFINITION site (the load stack names the
    // executing package), so a definition the unit's top level runs from inside
    // a block — invisible to any top-level AST scan — is refused all the same.
    let sandbox = sandbox("bare-dyn");
    let home = sandbox.join("home");
    let src = sandbox.join("mathkit");
    mathkit(&src);
    write(
        &src.join("lib/evil.qn"),
        "true.if:{ Sneaky <- { hi -> { 'polluted' } } }\n",
    );
    install(&home, &src);

    let (ok, out) = run_expr(&home, "use mathkit:*");
    assert!(
        !ok,
        "a load-time dynamic bare definition must fail the load"
    );
    assert!(
        out.contains("bare-global class `Sneaky`") && out.contains("[Mathkit]Sneaky"),
        "{out}"
    );
}

#[test]
fn extension_classes_install_before_source_units() {
    let sandbox = sandbox("bothkind");
    let home = sandbox.join("home");
    let src = sandbox.join("veckit");
    write(
        &src.join("quoin.toml"),
        &format!(
            "[package]\nname = \"veckit\"\n\n\
             [extension]\ncommand = \"{}\"\nnamespace = \"Vec\"\n\n\
             [lib]\nroot = \"lib\"\n",
            env!("CARGO_BIN_EXE_ext_vector")
        ),
    );
    // The source unit reopens a class the EXTENSION provides — only possible if the
    // synthetic `*` unit (spawn + install) ran first.
    write(
        &src.join("lib/sugar.qn"),
        "[Vec]Vector <-- {\n    sumTwice -> { self.sum * 2 }\n}\n",
    );
    install(&home, &src);

    let (ok, out) = run_expr(
        &home,
        "use veckit:*; (([Vec]Vector.ofFloats:#( 1.0 2.0 )).sumTwice).print",
    );
    assert!(ok && out.contains("6"), "{out}");
}

#[test]
fn init_unit_is_never_globbed() {
    let sandbox = sandbox("init");
    let home = sandbox.join("home");
    let src = sandbox.join("veckit");
    // Lib root = the package root, with an init.qn sitting in it: `loadPackage:` evaluates
    // init.qn once, and the glob must NOT load it again as a source unit (the reopened
    // method would double-define).
    write(
        &src.join("quoin.toml"),
        &format!(
            "[package]\nname = \"veckit\"\n\n\
             [extension]\ncommand = \"{}\"\nnamespace = \"Vec\"\n\n\
             [lib]\n",
            env!("CARGO_BIN_EXE_ext_vector")
        ),
    );
    write(
        &src.join("init.qn"),
        "[Vec]Vector <-- {\n    tag -> { 'from-init' }\n}\n",
    );
    write(
        &src.join("sugar.qn"),
        "[Vec]Vector <-- {\n    sumTwice -> { self.sum * 2 }\n}\n",
    );
    install(&home, &src);

    let (ok, out) = run_expr(
        &home,
        "use veckit:*; var v = [Vec]Vector.ofFloats:#( 1.0 2.0 ); v.tag.print; (v.sumTwice).print",
    );
    assert!(
        ok && out.contains("from-init") && out.contains("6"),
        "{out}"
    );
}
