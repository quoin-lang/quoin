//! `qn doc` end to end (docs/DOCS_ARCH.md §7): the generator boots a VM, walks the class
//! table, and emits HTML + JSON. What these pin:
//!
//!   * the ONE pipeline — a single class page carries docs from both worlds: `.doc(..)` text
//!     from a native builder and a `"*` comment block lifted from Quoin source;
//!   * the JSON model is the contract (`version`, class/method shape);
//!   * an explicit path documents that project (the default mode is project-first);
//!   * `--coverage` reports instead of generating;
//!   * PROJECT mode: provenance partition, platform-class extensions, bin/ command
//!     sniffing, the README preamble, and --stdlib-path linking.

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
        &["--stdlib", "--json", "--out", out_dir.to_str().unwrap()],
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
    assert_eq!(model["version"], 2, "the model contract is versioned");

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
    // Self-containment, current stance: scripts/images/@import stay forbidden; the ONE
    // sanctioned external resource is the code font (Google Fonts <link>s, which degrade to
    // the system monospace offline), and plain hyperlinks (GitHub source refs) are fine.
    for forbidden in ["<script", "src=\"http", "@import"] {
        assert!(!index.contains(forbidden), "external resource: {forbidden}");
    }
    for (i, _) in index.match_indices("<link") {
        let end = index[i..].find('>').map(|e| i + e).unwrap_or(index.len());
        let link = &index[i..end];
        assert!(
            link.contains("cdn.jsdelivr.net"),
            "only the font host may be <link>ed: {link}"
        );
    }
    // jsDelivr's copy of the OFFICIAL Fira Code distribution, not Google Fonts — Google
    // strips the stylistic sets, which would make the ss05 `@` variant a silent no-op.
    assert!(
        index.contains("firacode@") && index.contains("'Fira Code'"),
        "the ligature code font must be linked and used"
    );
    assert!(
        index.contains("font-feature-settings: 'ss05'"),
        "the @ variant (ss05) must be enabled"
    );
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
        // Internal cross-links must resolve to emitted pages; absolute URLs (the font hosts,
        // GitHub source refs) are not files.
        if href.starts_with("http") {
            continue;
        }
        assert!(
            out_dir.join(href).exists(),
            "index links to {href}, which was not emitted"
        );
    }

    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn project_mode_partitions_extensions_commands_and_readme() {
    // A mini project with all four surfaces: a project class, a platform-class reopen
    // (method-level provenance -> an extension group), a bin/ command with a #!qn line,
    // and a README. The stdlib classes must NOT be documented; the extension host links
    // through --stdlib-path.
    let proj = fresh_out("projmode");
    std::fs::create_dir_all(proj.join("lib")).unwrap();
    std::fs::create_dir_all(proj.join("bin")).unwrap();
    std::fs::create_dir_all(proj.join("tests")).unwrap();
    std::fs::write(
        proj.join("lib/util.qn"),
        "\"* A tiny helper.\nHelper <- {\n\x20   \"* Twice the input.\n\x20   double: -> { |n| n * 2 }\n}\n\
         \"* Project seasoning for a platform class.\nString <-- {\n\x20   \"* This string, shouted.\n\x20   shout -> { .upper + '!' }\n}\n",
    )
    .unwrap();
    std::fs::write(
        proj.join("bin/tool"),
        "#!/usr/bin/env qn\n\"* Does tool things.\n'hi'.print\n",
    )
    .unwrap();
    std::fs::write(
        proj.join("tests/ignored.qn"),
        "NotDocumented <- { x -> { 1 } }\n",
    )
    .unwrap();
    std::fs::write(proj.join("README.md"), "# tooly\n\nA **great** tool.\n").unwrap();

    let out = run_doc(
        &["--json", "--stdlib-path", "../ref", "--out", "docs-out"],
        &proj,
    );
    assert!(
        out.status.success(),
        "qn doc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let model: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(proj.join("docs-out/model.json")).unwrap())
            .unwrap();
    assert_eq!(model["version"], 2);
    assert_eq!(model["project"], true);
    let classes = model["classes"].as_array().unwrap();
    assert!(
        classes.iter().any(|c| c["name"] == "Helper"),
        "the project class is the subject"
    );
    assert!(
        !classes.iter().any(|c| c["name"] == "String"),
        "platform classes are background, not subjects"
    );
    assert!(
        !classes.iter().any(|c| c["name"] == "NotDocumented"),
        "tests/ is not API"
    );
    let exts = model["extensions"].as_array().unwrap();
    assert_eq!(exts.len(), 1, "one extended platform class");
    assert_eq!(exts[0]["host"], "String");
    assert_eq!(
        exts[0]["instance_methods"].as_array().unwrap().len(),
        1,
        "only the project's method documents"
    );
    let commands = model["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["name"], "tool");
    assert_eq!(commands[0]["doc"], "Does tool things.");

    // The index: README preamble (its own h1 wins; bold renders), the command, and the
    // extension listing. The extension page links its host through --stdlib-path.
    let index = std::fs::read_to_string(proj.join("docs-out/index.html")).unwrap();
    assert!(
        index.contains("<h1 id=\"tooly\">tooly</h1>"),
        "README h1 (with its slug anchor) is the title"
    );
    assert!(
        index.contains("<strong>great</strong>"),
        "README bold renders"
    );
    assert!(index.contains("bin/tool"), "the command lists");
    assert!(index.contains("ext.String.html"), "the extension lists");
    let ext_page = std::fs::read_to_string(proj.join("docs-out/ext.String.html")).unwrap();
    assert!(
        ext_page.contains("href=\"../ref/String.html\""),
        "the host links through --stdlib-path"
    );
    assert!(ext_page.contains("This string, shouted."));
    let _ = std::fs::remove_dir_all(&proj);
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

#[test]
fn md_mode_renders_pages_with_quoin_highlighting() {
    // The book build: a directory of markdown becomes HTML pages — quoin fences (norun
    // included) through the shared highlighter, tables and rule-box blockquotes render,
    // inter-page links rewrite, README.md becomes index.html.
    let dir = fresh_out("mdmode");
    std::fs::create_dir_all(dir.join("book")).unwrap();
    std::fs::write(
        dir.join("book/README.md"),
        "# The Book\n\nStart at [chapter one](01-intro.md).\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("book/01-intro.md"),
        "# One\n\nBack to the [index](README.md).\n\n> **Rules**\n> - a `x`\n>\n> ```quoin\n> var x = 1\n> ```\n\n\
         | A | B |\n|---|---|\n| 1 | 2 |\n\n```quoin norun\nnil.bogus\n```\n",
    )
    .unwrap();
    let out = run_doc(&["--md", "book", "--out", "site"], &dir);
    assert!(
        out.status.success(),
        "qn doc --md failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("2 pages"), "{stdout}");
    let index = std::fs::read_to_string(dir.join("site/index.html")).unwrap();
    assert!(
        index.contains("href=\"01-intro.html\""),
        ".md links rewrite to rendered names"
    );
    let page = std::fs::read_to_string(dir.join("site/01-intro.html")).unwrap();
    assert!(
        page.contains("href=\"index.html\""),
        "README links map to index.html"
    );
    assert!(page.contains("<blockquote>"), "rule boxes render");
    assert!(
        page.matches("qn-code").count() >= 2,
        "quoin AND quoin norun fences highlight"
    );
    assert!(
        page.contains("<th>A</th>") && page.contains("<td>2</td>"),
        "tables render"
    );
    assert!(
        page.contains("<title>One</title>"),
        "the first h1 titles the page"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
        "firacode@", // the shared ligature font, linked not copied (jsDelivr, full features)
        "'Fira Code'",
        "font-feature-settings: 'ss05'", // the @ variant
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
        &["--stdlib", "--coverage", "--out", out_dir.to_str().unwrap()],
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

/// The `--check` harness itself (docs/DOCS_ARCH.md phase 3 + RELEASE_PREP Tier 2): fenced
/// `quoin` blocks in markdown run; annotations assert; failures name their site. Pinned on a
/// tiny fixture rather than the real corpora, which CI checks separately (they're slow).
#[test]
fn doc_check_runs_annotated_markdown_blocks() {
    let dir = fresh_out("check");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("good.md"),
        "# t\n\n```quoin\nvar x = 40;\nx + 2    \"* -> 42\n```\n\
         \n```quoin norun\nthis would explode\n```\n\
         \n```\nprose, never runs\n```\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args(["doc", "--check", dir.join("good.md").to_str().unwrap()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn doc --check");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(
        stdout.contains("1 examples, 1 annotations checked, 0 failed"),
        "norun and untagged blocks must not run:\n{stdout}"
    );

    // A wrong annotation fails, naming the file and showing expected vs got.
    std::fs::write(dir.join("bad.md"), "```quoin\n1 + 1    \"* -> 3\n```\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args(["doc", "--check", dir.join("bad.md").to_str().unwrap()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn doc --check");
    assert!(
        !out.status.success(),
        "a wrong annotation must fail the check"
    );
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(all.contains("bad.md") && all.contains("expected: 3") && all.contains("got:      2"));

    // An annotation separated from the next statement by a blank line still checks (the
    // group's last line is blank; the scan uses the last NON-blank line), and a fence inside
    // a blockquote (the book's Gotcha boxes) is not invisible.
    std::fs::write(
        dir.join("edges.md"),
        "```quoin\nvar q = 6 * 7;\nq;    \"* -> 42\n\nvar cleanup = 0;\n```\n\
         \n> quoted:\n> ```quoin\n> 1 + 2    \"* -> 3\n> ```\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args(["doc", "--check", dir.join("edges.md").to_str().unwrap()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn doc --check");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(
        stdout.contains("2 examples, 2 annotations checked, 0 failed"),
        "blank-line annotations and blockquoted fences must both check:\n{stdout}"
    );

    // A block that doesn't parse fails with its location, not a panic.
    std::fs::write(dir.join("broken.md"), "```quoin\nx = 5\n```\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args(["doc", "--check", dir.join("broken.md").to_str().unwrap()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn doc --check");
    assert!(!out.status.success());
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        all.contains("broken.md"),
        "failure must name the file:\n{all}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
