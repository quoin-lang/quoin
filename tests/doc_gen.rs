//! `qn doc` end to end (docs/DOCS_ARCH.md §7): the generator boots a VM, walks the class
//! table, and emits HTML + JSON. What these pin:
//!
//!   * the ONE pipeline — a single class page carries docs from both worlds: `.doc(..)` text
//!     from a native builder and a `"*` comment block lifted from Quoin source;
//!   * the JSON model is the contract (`version`, class/method shape);
//!   * user units are documented alongside the stdlib;
//!   * `--coverage` reports instead of generating.

use std::path::Path;
use std::process::{Command, Output};

fn run_doc(args: &[&str], dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg("doc")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run qn doc")
}

fn fresh_out(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("qn_docgen_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn stdlib_docs_carry_both_native_and_quoin_doc_text() {
    let out_dir = fresh_out("stdlib");
    let out = run_doc(
        &["--json", "--out", out_dir.to_str().unwrap()],
        Path::new(env!("CARGO_MANIFEST_DIR")),
    );
    assert!(
        out.status.success(),
        "qn doc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let model: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out_dir.join("model.json")).unwrap())
            .expect("model.json parses");
    assert_eq!(model["version"], 1, "the model contract is versioned");

    let classes = model["classes"].as_array().unwrap();
    let class = |name: &str| {
        classes
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing from the model"))
    };
    let method_doc = |c: &serde_json::Value, side: &str, sel: &str| -> String {
        c[side]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["selector"] == sel)
            .unwrap_or_else(|| panic!("{side} {sel} missing"))["doc"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };

    // One class, both doc sources. `readLine` is a Quoin delegator whose doc is the `"*`
    // block in qnlib/core/06-io.qn; `stream` is a native method whose doc is `.doc(..)` in
    // src/runtime/io.rs. If either pipeline breaks, this page shows it.
    let stdin = class("[IO]Stdin");
    assert!(
        method_doc(stdin, "class_methods", "readLine").contains("without its terminator"),
        "Quoin comment extraction lost [IO]Stdin.readLine"
    );
    assert!(
        method_doc(stdin, "class_methods", "stream").contains("created on first use"),
        ".doc(..) text lost on the native [IO]Stdin.stream"
    );
    assert!(
        stdin["doc"]
            .as_str()
            .unwrap_or("")
            .contains("standard input"),
        ".class_doc(..) text lost on [IO]Stdin"
    );

    // The stdlib beyond core/ is loaded: [Web] and [HTTP] classes must be present.
    class("[Web]App");
    class("[HTTP]Client");

    // HTML exists, is self-contained, and cross-links resolve to emitted pages.
    let index = std::fs::read_to_string(out_dir.join("index.html")).unwrap();
    assert!(index.contains("<style>"), "the stylesheet must be inline");
    // Self-contained means no external RESOURCES (scripts, stylesheets, images) — plain
    // hyperlinks (the GitHub source links) are fine.
    for forbidden in ["<script", "<link", "src=\"http", "@import"] {
        assert!(!index.contains(forbidden), "external resource: {forbidden}");
    }
    let page = std::fs::read_to_string(out_dir.join("IO.Stdin.html")).unwrap();
    assert!(
        page.contains("without its terminator"),
        "doc text must reach the HTML"
    );

    // Source refs link to the repository named by the crate metadata; the path is the
    // stdlib's home under qnlib/, the fragment the line.
    assert!(
        page.contains("href=\"https://github.com/quoin-lang/quoin/blob/main/qnlib/core/06-io.qn#L"),
        "a stdlib source ref must link to GitHub"
    );
    // A native method says so where a Quoin method shows its source — and never as a
    // signature suffix.
    assert!(
        page.contains("<p class=\"meta-line\"><code>native</code></p>"),
        "native methods must carry the source-position label"
    );
    assert!(
        !page.contains("(native)"),
        "the (native) signature suffix must not appear in doc pages"
    );
    // Backtick spans in prose render as <code> — on class pages and in index summaries.
    assert!(
        page.contains("<code>readLine</code>"),
        "inline backticks must become <code>"
    );
    for href in index.match_indices("href=\"").map(|(i, _)| {
        let rest = &index[i + 6..];
        &rest[..rest.find('"').unwrap()]
    }) {
        assert!(
            out_dir.join(href).exists(),
            "index links to {href}, which was not emitted"
        );
    }

    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn user_units_are_documented_alongside_the_stdlib() {
    // A scratch project dir: the unit is loaded `use self:...`, CWD-relative. The class has a
    // definition doc AND a documented reopen with a fenced example — the reopen must land
    // under `extensions` (the definition supplied the class doc), and the fence must render
    // through the shared highlighter.
    let proj = fresh_out("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("shapes.qn"),
        "\"* A circle, by `radius`.\n\
         \"*\n\
         \"* ```\n\
         \"* var c = Circle.new:{ r = 2 };\n\
         \"* ```\n\
         Circle <- { |@r|\n\
         \x20   \"* The enclosing area.\n\
         \x20   area -> { @r * @r * 355 / 113 }\n\
         }\n\
         \"* Growth, added after the fact.\n\
         Circle <-- {\n\
         \x20   \"* A circle one bigger.\n\
         \x20   grown -> { Circle.new:{ r = @r + 1 } }\n\
         }\n",
    )
    .unwrap();

    let out = run_doc(&["shapes.qn", "--json", "--out", "docs-out"], &proj);
    assert!(
        out.status.success(),
        "qn doc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let model: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(proj.join("docs-out/model.json")).unwrap())
            .unwrap();
    let circle = model["classes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Circle")
        .expect("user class documented");
    assert_eq!(
        circle["doc"].as_str().unwrap().lines().next().unwrap(),
        "A circle, by `radius`."
    );
    let method_doc = |sel: &str| {
        circle["instance_methods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["selector"] == sel)
            .unwrap_or_else(|| panic!("{sel} missing"))["doc"]
            .clone()
    };
    assert_eq!(method_doc("area"), "The enclosing area.");
    // The reopen's method extracts like any other...
    assert_eq!(method_doc("grown"), "A circle one bigger.");
    // ...and the reopen site itself lists under extensions (the definition took the doc slot).
    let exts = circle["extensions"].as_array().unwrap();
    assert_eq!(exts.len(), 1, "the documented reopen must be listed");
    assert_eq!(exts[0]["doc"], "Growth, added after the fact.");

    // The fenced example rendered through the shared highlighter: real span classes, and the
    // page inlines the shared code stylesheet.
    let page = std::fs::read_to_string(proj.join("docs-out/Circle.html")).unwrap();
    assert!(
        page.contains("<pre class=\"qn-code\">") && page.contains("qn-keyword"),
        "fenced example must render through the shared highlighter"
    );
    assert!(
        page.contains(".qn-keyword {"),
        "the code stylesheet must be inlined"
    );
    assert!(
        page.contains("extended at"),
        "the reopen site must appear on the page"
    );
    // A user unit is not in the Quoin repository: its source refs stay plain text.
    assert!(
        !page.contains("github.com"),
        "a non-stdlib source ref must not link to the Quoin repo"
    );
    // ...and the backticked word renders as code.
    assert!(page.contains("<code>radius</code>"));
    let _ = std::fs::remove_dir_all(&proj);
}

/// `qn highlight --html` — the other consumer of the same stylesheet and span classes.
#[test]
fn highlight_html_shares_the_doc_generator_styles() {
    let dir = fresh_out("hl");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("sample.qn");
    std::fs::write(&src, "var x = 1;\n\"* a comment\n'text'.print;\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args(["highlight", "--html", src.to_str().unwrap()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn highlight --html");
    assert!(out.status.success());
    let html = String::from_utf8_lossy(&out.stdout);
    for needle in [
        "<pre class=\"qn-code\">",
        "qn-keyword",    // `var`
        "qn-comment",    // the "* line
        "qn-string",     // 'text'
        ".qn-keyword {", // the shared stylesheet, same classes the doc pages inline
        "prefers-color-scheme: dark",
    ] {
        assert!(html.contains(needle), "missing {needle} in:\n{html}");
    }
    // Spans balance — nothing double-emitted or dropped by the gap-walk.
    assert_eq!(
        html.matches("<span").count(),
        html.matches("</span>").count()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn coverage_reports_and_emits_nothing() {
    let out_dir = fresh_out("cov");
    let out = run_doc(
        &["--coverage", "--out", out_dir.to_str().unwrap()],
        Path::new(env!("CARGO_MANIFEST_DIR")),
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("doc coverage:"),
        "coverage summary missing:\n{stdout}"
    );
    assert!(!out_dir.exists(), "--coverage must report, not generate");
}
