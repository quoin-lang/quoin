//! `qn doc` — the reference-documentation generator (docs/DOCS_ARCH.md §7).
//!
//! Boots a VM the way `qn -e` does (embedded stdlib prelude), loads the rest of the shipping
//! stdlib (`use std:net/*; use std:web/*`) plus any user units, then walks the *class table*
//! through `introspect` — the one source of truth that unifies Quoin classes, native classes,
//! mixins, and extension classes. Docs come from two places: a native method carries its
//! `.doc(..)` text in the introspection result already; a Quoin method's doc is the `"*`
//! block above its `SourceLoc`, lifted lazily by `crate::docs`.
//!
//! Output: one self-contained HTML page per class plus a namespace-grouped index (no JS, one
//! inline stylesheet), and with `--json` the raw model (`model.json`, `{"version": 1, …}`) —
//! the contract other renderers (LSP hover, a website) consume so the HTML here is just one
//! consumer of the model, not the model.

use super::*;
use crate::docs;
use crate::introspect::{self, ClassInfo, GlobalKind, MethodInfo};
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Write as _;

/// The serialized doc model. `version` bumps on breaking shape changes.
#[derive(Serialize)]
struct DocModel {
    version: u32,
    classes: Vec<ClassDoc>,
}

#[derive(Serialize)]
struct ClassDoc {
    /// Display name, e.g. `[IO]File`.
    name: String,
    /// Namespace path, e.g. `["IO"]`; empty for core classes.
    namespace: Vec<String>,
    parent: Option<String>,
    mixins: Vec<String>,
    is_sealed: bool,
    is_abstract: bool,
    doc: Option<String>,
    source: Option<String>,
    /// Documented reopen sites (`Name <-- { … }`) beyond the one that supplied `doc`.
    extensions: Vec<ExtensionDoc>,
    instance_methods: Vec<MethodDoc>,
    class_methods: Vec<MethodDoc>,
}

#[derive(Serialize)]
struct ExtensionDoc {
    source: String,
    doc: String,
}

#[derive(Serialize)]
struct MethodDoc {
    selector: String,
    /// Rendered signatures, one per variant (`at:Integer put:`). Unlike the REPL's rendering,
    /// no `(native)` suffix — nativeness is the `native` field, shown where a Quoin method
    /// shows its source location.
    signatures: Vec<String>,
    doc: Option<String>,
    source: Option<String>,
    /// Every variant is Rust-implemented (no source to link).
    native: bool,
}

/// Classes that exist as machinery, not API: per-instance eigenclass shells and the
/// boolean singletons' `$`-prefixed backing classes.
fn is_internal(name: &str) -> bool {
    name.starts_with('$')
}

impl VmRunner {
    pub(crate) fn run_doc(&self) -> Result<(), QuoinError> {
        let out_dir = PathBuf::from(
            self.options
                .target_path
                .clone()
                .unwrap_or_else(|| "qn-docs".to_string()),
        );
        let want_json = self.options.doc_json;
        let want_coverage = self.options.doc_coverage;
        let user_units: Vec<String> = self.options.vm_options.arguments.clone();

        let Some(mut arena) = self.build_repl_arena() else {
            exit(1);
        };

        // The prelude loads `core/*` only; the rest of the shipping stdlib is use-loaded.
        for unit in ["use std:net/*", "use std:web/*", "use test"] {
            if let Err(e) = runner_repl::eval_once(&mut arena, unit) {
                eprintln!("qn doc: loading the stdlib: {e}");
                exit(1);
            }
        }
        // User units: `qn doc app/util.qn` loads `use self:app/util` (self_root = CWD in the
        // script-less modes, so relative paths resolve as written).
        for path in &user_units {
            let unit = path.strip_suffix(".qn").unwrap_or(path);
            if Path::new(unit).is_absolute() {
                eprintln!("qn doc: {path}: give a path relative to the current directory");
                exit(2);
            }
            if let Err(e) = runner_repl::eval_once(&mut arena, &format!("use self:{unit}")) {
                eprintln!("qn doc: loading {path}: {e}");
                exit(1);
            }
        }

        // Walk the class table into plain data; everything after this is VM-free.
        let infos: Vec<ClassInfo> = arena.mutate_root(|_mc, vm| {
            let mut infos: Vec<ClassInfo> = introspect::globals(vm)
                .into_iter()
                .filter(|g| g.kind == GlobalKind::Class && !is_internal(&g.name))
                .filter_map(|g| introspect::describe_class(vm, &g.name))
                .collect();
            infos.sort_by(|a, b| a.name.cmp(&b.name));
            infos
        });

        let model = build_model(infos);

        if want_coverage {
            report_coverage(&model);
            return Ok(());
        }

        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            eprintln!("qn doc: cannot create {}: {e}", out_dir.display());
            exit(1);
        }
        if want_json {
            let json = serde_json::to_string_pretty(&model).expect("doc model serializes");
            write_out(&out_dir.join("model.json"), &json);
        }
        write_out(&out_dir.join("index.html"), &render_index(&model));
        for class in &model.classes {
            write_out(
                &out_dir.join(page_name(&class.name)),
                &render_class(class, &model),
            );
        }
        println!(
            "qn doc: {} classes -> {}",
            model.classes.len(),
            out_dir.display()
        );
        Ok(())
    }
}

fn write_out(path: &Path, text: &str) {
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("qn doc: cannot write {}: {e}", path.display());
        exit(1);
    }
}

/// Assemble the model: introspection data plus lazily-extracted Quoin docs. Source files are
/// read once each (`cache`), through the same resolution `$doc` will use.
fn build_model(infos: Vec<ClassInfo>) -> DocModel {
    let mut cache: HashMap<String, Option<String>> = HashMap::new();
    // `selector: Some(..)` routes through the wrapped-header-aware anchor (docs.rs); class and
    // extension sites anchor on their own line.
    let mut doc_at = |file: &str, line: usize, selector: Option<&str>| -> Option<String> {
        let text = cache
            .entry(file.to_string())
            .or_insert_with(|| docs::unit_source(file));
        text.as_deref().and_then(|t| match selector {
            Some(sel) => docs::method_doc_above(t, line, sel),
            None => docs::doc_above(t, line),
        })
    };

    let classes = infos
        .into_iter()
        .map(|info| {
            // Same precedence the runtime `X.doc` uses (introspect::doc_of_class): native
            // `.class_doc(..)`, else the block above the definition, else the block above the
            // first documented reopen. Remaining documented reopens list under `extensions`.
            let mut ext_docs: Vec<ExtensionDoc> = info
                .extension_sources
                .iter()
                .filter_map(|src| {
                    Some(ExtensionDoc {
                        source: format!("{}:{}", src.file, src.line),
                        doc: doc_at(&src.file, src.line, None)?,
                    })
                })
                .collect();
            let class_doc = info
                .doc
                .clone()
                .or_else(|| {
                    let src = info.source.as_ref()?;
                    doc_at(&src.file, src.line, None)
                })
                .or_else(|| (!ext_docs.is_empty()).then(|| ext_docs.remove(0).doc));
            let mut methods = |list: &[MethodInfo]| -> Vec<MethodDoc> {
                let mut out: Vec<MethodDoc> = list
                    .iter()
                    .map(|m| {
                        // One doc per selector: the first variant that has one — native text
                        // first, else the comment block above the first located variant.
                        let native_doc = m.variants.iter().find_map(|v| v.doc.clone());
                        let quoin_doc = m.variants.iter().find_map(|v| {
                            let src = v.source.as_ref()?;
                            doc_at(&src.file, src.line, Some(&m.selector))
                        });
                        let source = m
                            .variants
                            .iter()
                            .find_map(|v| v.source.as_ref())
                            .map(|s| format!("{}:{}", s.file, s.line));
                        MethodDoc {
                            selector: m.selector.clone(),
                            signatures: m
                                .variants
                                .iter()
                                .map(|v| {
                                    let sig = introspect::signature(&m.selector, v);
                                    sig.strip_suffix(" (native)")
                                        .map(String::from)
                                        .unwrap_or(sig)
                                })
                                .collect(),
                            doc: native_doc.or(quoin_doc),
                            source,
                            native: m.variants.iter().all(|v| v.native),
                        }
                    })
                    .collect();
                out.sort_by(|a, b| a.selector.cmp(&b.selector));
                out
            };
            let ns = NamespacedName::parse(&info.name);
            ClassDoc {
                name: info.name.clone(),
                namespace: ns.path.clone(),
                parent: info.parent.clone(),
                mixins: info.mixins.clone(),
                is_sealed: info.is_sealed,
                is_abstract: info.is_abstract,
                doc: class_doc,
                source: info
                    .source
                    .as_ref()
                    .map(|s| format!("{}:{}", s.file, s.line)),
                extensions: ext_docs,
                instance_methods: methods(&info.instance_methods),
                class_methods: methods(&info.class_methods),
            }
        })
        .collect();
    DocModel {
        version: 1,
        classes,
    }
}

/// `--coverage`: what has no doc. A report, not a gate — the corpus is adopted incrementally,
/// so exiting non-zero here would only train people to ignore it.
fn report_coverage(model: &DocModel) {
    let mut documented = 0usize;
    let mut total = 0usize;
    for class in &model.classes {
        total += 1;
        if class.doc.is_some() {
            documented += 1;
        } else {
            println!("undocumented class: {}", class.name);
        }
        for (side, list) in [
            ("", &class.instance_methods),
            (".meta ", &class.class_methods),
        ] {
            for m in list {
                total += 1;
                if m.doc.is_some() {
                    documented += 1;
                } else {
                    println!("undocumented: {} {}{}", class.name, side, m.selector);
                }
            }
        }
    }
    let pct = if total == 0 {
        100.0
    } else {
        documented as f64 * 100.0 / total as f64
    };
    println!("doc coverage: {documented}/{total} ({pct:.1}%)");
}

// ---- HTML rendering -------------------------------------------------------------------

/// `[IO]File` -> `IO.File.html`; `Point` -> `Point.html`.
fn page_name(class: &str) -> String {
    let ns = NamespacedName::parse(class);
    if ns.path.is_empty() {
        format!("{}.html", ns.name)
    } else {
        format!("{}.{}.html", ns.path.join("."), ns.name)
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape, then render `backtick spans` as `<code>` — the one piece of inline markup doc
/// prose uses. An unpaired trailing backtick renders literally.
fn inline(text: &str) -> String {
    let escaped = esc(text);
    let mut out = String::new();
    let mut parts = escaped.split('`');
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    let rest: Vec<&str> = parts.collect();
    let mut i = 0;
    while i < rest.len() {
        if i + 1 < rest.len() {
            out.push_str("<code>");
            out.push_str(rest[i]);
            out.push_str("</code>");
            out.push_str(rest[i + 1]);
            i += 2;
        } else {
            // Odd trailing segment: the backtick was literal.
            out.push('`');
            out.push_str(rest[i]);
            i += 1;
        }
    }
    out
}

/// The GitHub blob URL for a `file:line` source ref, when the file is part of the shipped
/// stdlib (which lives under `qnlib/` in the repository named by the crate metadata). User
/// units are not in that repository, so they render as plain text.
fn source_href(source: &str) -> Option<String> {
    let repo = option_env!("CARGO_PKG_REPOSITORY")?;
    let (file, line) = source.rsplit_once(':')?;
    let unit = file.strip_suffix(".qn").unwrap_or(file);
    let key = unit.strip_prefix("std:").unwrap_or(unit);
    // In the embed table <=> shipped from qnlib/ in the repo.
    crate::stdlib::resolve(key)?;
    Some(format!(
        "{}/blob/main/qnlib/{key}.qn#L{line}",
        repo.trim_end_matches('/')
    ))
}

/// A source ref as HTML: `<code>file:line</code>`, linked to GitHub when the file is in the
/// repository. Styled like the surrounding meta text — underline only on hover (`a.src`).
fn source_html(source: &str) -> String {
    match source_href(source) {
        Some(href) => format!(
            r#"<a class="src" href="{href}"><code>{}</code></a>"#,
            esc(source)
        ),
        None => format!("<code>{}</code>", esc(source)),
    }
}

/// Link `name` if the model documents a class by that name (bare or nullable `Foo?`).
fn type_link(name: &str, model: &DocModel) -> String {
    let bare = name.strip_suffix('?').unwrap_or(name);
    if model.classes.iter().any(|c| c.name == bare) {
        format!(r#"<a href="{}">{}</a>"#, page_name(bare), esc(name))
    } else {
        esc(name)
    }
}

/// Doc text -> HTML: paragraphs split on blank lines; fenced blocks render through the shared
/// highlighter (docs/DOCS_ARCH.md §8) — the same classes and colors as `qn highlight --html`.
fn doc_html(doc: &str) -> String {
    let mut out = String::new();
    let mut para: Vec<&str> = Vec::new();
    let mut fence: Option<Vec<&str>> = None;
    let flush_para = |out: &mut String, para: &mut Vec<&str>| {
        if !para.is_empty() {
            let _ = write!(out, "<p>{}</p>\n", inline(&para.join(" ")));
            para.clear();
        }
    };
    for line in doc.lines() {
        if let Some(body) = &mut fence {
            if line.trim_start().starts_with("```") {
                let _ = write!(
                    out,
                    "{}\n",
                    crate::highlighter::highlight_to_html(&body.join("\n"))
                );
                fence = None;
            } else {
                body.push(line);
            }
        } else if line.trim_start().starts_with("```") {
            flush_para(&mut out, &mut para);
            fence = Some(Vec::new());
        } else if line.trim().is_empty() {
            flush_para(&mut out, &mut para);
        } else {
            para.push(line);
        }
    }
    if let Some(body) = fence {
        // Unclosed fence: render what we have rather than losing it.
        let _ = write!(
            out,
            "{}\n",
            crate::highlighter::highlight_to_html(&body.join("\n"))
        );
    }
    flush_para(&mut out, &mut para);
    out
}

/// One stylesheet for every page. Light and dark via `prefers-color-scheme`.
const STYLE: &str = r#"
:root { --fg: #1a1a1a; --bg: #ffffff; --dim: #666; --line: #e2e2e2; --accent: #0550ae;
        --code-bg: #f6f8fa; }
@media (prefers-color-scheme: dark) {
  :root { --fg: #d4d4d4; --bg: #1e1e1e; --dim: #999; --line: #3a3a3a; --accent: #6cb6ff;
          --code-bg: #2a2a2a; }
}
* { box-sizing: border-box; }
body { color: var(--fg); background: var(--bg); font: 16px/1.55 system-ui, sans-serif;
       max-width: 52rem; margin: 0 auto; padding: 2rem 1rem 4rem; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
h1, h2, h3 { line-height: 1.25; }
h1 { font-size: 1.6rem; }
h2 { font-size: 1.15rem; border-bottom: 1px solid var(--line); padding-bottom: .3rem;
     margin-top: 2.2rem; }
code, pre { font: 0.9em ui-monospace, monospace; }
pre { background: var(--code-bg); padding: .75rem 1rem; border-radius: 6px; overflow-x: auto; }
.sig { font-family: ui-monospace, monospace; font-weight: 600; }
.meta-line { color: var(--dim); font-size: .85rem; }
a.src { color: inherit; }
a.src:hover { text-decoration: underline; }
.method { margin: 1.4rem 0; }
.method > .body { margin: .3rem 0 0 1rem; }
.badge { font-size: .75rem; color: var(--dim); border: 1px solid var(--line);
         border-radius: 4px; padding: 0 .35rem; margin-left: .5rem; vertical-align: middle; }
ul.classlist { list-style: none; padding-left: 0; }
ul.classlist li { margin: .15rem 0; }
ul.classlist .summary { color: var(--dim); margin-left: .5rem; }
"#;

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n{}\n<style>{}\n{}</style>\n</head>\n<body>\n{}\n</body></html>\n",
        esc(title),
        crate::highlighter::code_font_links(),
        STYLE,
        crate::highlighter::code_stylesheet(),
        body
    )
}

fn render_index(model: &DocModel) -> String {
    // Group by namespace; core (no namespace) first.
    let mut groups: Vec<(String, Vec<&ClassDoc>)> = Vec::new();
    for class in &model.classes {
        let key = if class.namespace.is_empty() {
            "Core".to_string()
        } else {
            format!("[{}]", class.namespace.join("/"))
        };
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(class),
            None => groups.push((key, vec![class])),
        }
    }
    groups.sort_by(|a, b| (a.0 != "Core").cmp(&(b.0 != "Core")).then(a.0.cmp(&b.0)));

    let mut body = String::from("<h1>Quoin reference</h1>\n");
    for (ns, classes) in groups {
        let _ = write!(body, "<h2>{}</h2>\n<ul class=\"classlist\">\n", esc(&ns));
        for c in classes {
            let summary = c
                .doc
                .as_deref()
                .map(|d| {
                    format!(
                        "<span class=\"summary\">{}</span>",
                        inline(docs::summary(d))
                    )
                })
                .unwrap_or_default();
            let _ = write!(
                body,
                "<li><a href=\"{}\"><code>{}</code></a>{}</li>\n",
                page_name(&c.name),
                esc(&c.name),
                summary
            );
        }
        body.push_str("</ul>\n");
    }
    page("Quoin reference", &body)
}

fn render_class(class: &ClassDoc, model: &DocModel) -> String {
    let mut body = String::new();
    let _ = write!(body, "<p><a href=\"index.html\">← index</a></p>\n");
    let mut badges = String::new();
    if class.is_abstract {
        badges.push_str("<span class=\"badge\">abstract</span>");
    }
    if class.is_sealed {
        badges.push_str("<span class=\"badge\">sealed</span>");
    }
    let _ = write!(
        body,
        "<h1><code>{}</code>{}</h1>\n",
        esc(&class.name),
        badges
    );

    let mut lineage: Vec<String> = Vec::new();
    if let Some(p) = &class.parent {
        lineage.push(format!("inherits {}", type_link(p, model)));
    }
    if !class.mixins.is_empty() {
        let mixed: Vec<String> = class.mixins.iter().map(|m| type_link(m, model)).collect();
        lineage.push(format!("mixes in {}", mixed.join(", ")));
    }
    if let Some(src) = &class.source {
        lineage.push(format!("defined at {}", source_html(src)));
    }
    if !lineage.is_empty() {
        let _ = write!(body, "<p class=\"meta-line\">{}</p>\n", lineage.join(" · "));
    }
    if let Some(doc) = &class.doc {
        body.push_str(&doc_html(doc));
    }
    for ext in &class.extensions {
        let _ = write!(
            body,
            "<div class=\"method\"><span class=\"meta-line\">extended at {}</span>\
             <div class=\"body\">{}</div></div>\n",
            source_html(&ext.source),
            doc_html(&ext.doc)
        );
    }

    for (title, anchor, list) in [
        ("Class methods", "c", &class.class_methods),
        ("Instance methods", "i", &class.instance_methods),
    ] {
        if list.is_empty() {
            continue;
        }
        let _ = write!(body, "<h2>{title}</h2>\n");
        for m in list {
            let sigs: Vec<String> = m.signatures.iter().map(|s| esc(s)).collect();
            let _ = write!(
                body,
                "<div class=\"method\" id=\"{}-{}\"><span class=\"sig\">{}</span>",
                anchor,
                esc(&m.selector),
                sigs.join("<br>")
            );
            let _ = write!(body, "<div class=\"body\">");
            if let Some(doc) = &m.doc {
                body.push_str(&doc_html(doc));
            }
            if let Some(src) = &m.source {
                let _ = write!(body, "<p class=\"meta-line\">{}</p>", source_html(src));
            } else if m.native {
                let _ = write!(body, "<p class=\"meta-line\"><code>native</code></p>");
            }
            body.push_str("</div></div>\n");
        }
    }
    page(&class.name, &body)
}
