//! `qn doc` — the documentation generator (docs/DOCS_ARCH.md §7), PROJECT-first.
//!
//! Discovery is runtime, never static: boot a VM the way `qn -e` does (embedded stdlib
//! prelude), LOAD the code, then walk the *class table* through `introspect` — the one
//! source of truth that unifies Quoin classes, native classes, mixins, and extension
//! classes. Docs come from two places: a native method carries its `.doc(..)` text in the
//! introspection result already; a Quoin method's doc is the `"*` block above its
//! `SourceLoc`, lifted lazily by `crate::docs`.
//!
//! Two modes, one mechanism, split by PROVENANCE (which unit defined each class/method):
//!
//!   * Default: document the current project. The tree's `.qn` units (minus `tests/`,
//!     `bin/`, and shebang scripts — a command's top level IS the program) load via
//!     `use self:…`; classes defined in `self:` units are the subject, and project methods
//!     added to PLATFORM classes (open-class reopens like `[OS]Process <-- …`) render as
//!     "Extensions" — for many Quoin libraries that is the API. `bin/` files with a `#!…qn`
//!     line list as commands (flag docs are future work — QUOIN_TODO); `README.md` opens
//!     the index. Platform type names link to a published stdlib reference when
//!     `--stdlib-path PREFIX` (hidden; a relative path or full URL) is given.
//!
//!   * `--stdlib` (hidden): document the shipping stdlib itself — the reference-publishing
//!     mode this repository uses.
//!
//! Output: one self-contained HTML page per class (plus one per extension host) and an
//! index (no JS, one inline stylesheet); with `--json` the raw model (`model.json`,
//! `{"version": 2, …}`) — the contract other renderers consume, so the HTML here is just
//! one consumer of the model, not the model.

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
    /// True for a project model (the default); false for `--stdlib`.
    project: bool,
    /// The index title: the project directory's name, or "Quoin reference".
    title: String,
    /// The project's README.md, verbatim markdown (rendered on the index).
    readme: Option<String>,
    /// `bin/` entries with a `#!…qn` line: the project's commands.
    commands: Vec<CommandDoc>,
    classes: Vec<ClassDoc>,
    /// Project methods added to platform classes, grouped per host.
    extensions: Vec<ExtensionGroup>,
    /// Link prefix for platform types (`--stdlib-path`) — relative path or URL.
    stdlib_path: Option<String>,
    /// Every class name the session knows (link resolution) — not part of the contract.
    #[serde(skip)]
    known: Vec<String>,
}

/// A `bin/` command: its name and the `"*` block under its shebang.
#[derive(Serialize)]
struct CommandDoc {
    name: String,
    doc: Option<String>,
}

/// The project's additions to one platform class — method-level provenance:
/// only `self:`-defined variants appear here.
#[derive(Serialize)]
struct ExtensionGroup {
    /// The host class, e.g. `[OS]Process`.
    host: String,
    /// The doc block above the project's reopen site, when present.
    doc: Option<String>,
    /// The project's reopen sites, as `file:line`.
    sources: Vec<String>,
    instance_methods: Vec<MethodDoc>,
    class_methods: Vec<MethodDoc>,
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
        if self.options.doc_check {
            return self.run_doc_check();
        }
        let out_dir = PathBuf::from(
            self.options
                .target_path
                .clone()
                .unwrap_or_else(|| "qn-docs".to_string()),
        );
        let want_json = self.options.doc_json;
        let want_coverage = self.options.doc_coverage;
        let user_units: Vec<String> = self.options.vm_options.arguments.clone();
        let mut model = build_model(self.collect_inputs(&user_units));
        model.stdlib_path = self.options.doc_stdlib_path.clone();

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
        for group in &model.extensions {
            write_out(
                &out_dir.join(ext_page_name(&group.host)),
                &render_extension(group, &model),
            );
        }
        let mut parts = vec![format!("{} classes", model.classes.len())];
        if !model.extensions.is_empty() {
            parts.push(format!("{} extended classes", model.extensions.len()));
        }
        if !model.commands.is_empty() {
            parts.push(format!("{} commands", model.commands.len()));
        }
        crate::runner::print_or_exit(&format!(
            "qn doc: {} -> {}\n",
            parts.join(", "),
            out_dir.display()
        ));
        Ok(())
    }

    /// Boot a VM, load the subject (the project tree by default; the shipping stdlib under
    /// `--stdlib`), and walk the class table into plain data. Shared by generation,
    /// coverage, and `--check`.
    fn collect_inputs(&self, user_units: &[String]) -> DocInputs {
        let Some(mut arena) = self.build_repl_arena() else {
            exit(1);
        };
        let project = !self.options.doc_stdlib;
        let units = if project {
            let units = discover_project_units(user_units);
            if units.is_empty() {
                eprintln!(
                    "qn doc: no .qn units under {} (tests/, bin/, and shebang scripts \
                     don't count; see --help)",
                    if user_units.is_empty() {
                        "."
                    } else {
                        "the given paths"
                    }
                );
                exit(2);
            }
            units
        } else {
            // The prelude loads `core/*` only; the rest of the shipping stdlib is use-loaded.
            for unit in [
                "use std:net/*",
                "use std:web/*",
                "use std:lang/*",
                "use test",
            ] {
                if let Err(e) = runner_repl::eval_once(&mut arena, unit) {
                    eprintln!("qn doc: loading the stdlib: {e}");
                    exit(1);
                }
            }
            user_units
                .iter()
                .map(|p| p.strip_suffix(".qn").unwrap_or(p).to_string())
                .collect()
        };
        // Loading IS discovery (and for a project, `self:` in each unit's provenance is
        // what marks it as the subject). `self_root` is the CWD in the script-less modes,
        // so relative paths resolve as written. A DISCOVERED unit that doesn't load
        // standalone isn't part of the documentable library (a Quernfile-style config
        // script, say, that its tool loads after its own library) — warn and move on;
        // an explicitly named path still fails hard.
        let explicit = !user_units.is_empty() || !project;
        let mut loaded: Vec<String> = Vec::new();
        for unit in &units {
            if Path::new(unit).is_absolute() {
                eprintln!("qn doc: {unit}: give a path relative to the current directory");
                exit(2);
            }
            match runner_repl::eval_once(&mut arena, &format!("use self:{unit}")) {
                Ok(_) => loaded.push(unit.clone()),
                Err(e) if explicit => {
                    eprintln!("qn doc: loading {unit}.qn: {e}");
                    exit(1);
                }
                Err(e) => {
                    let first = e.to_string();
                    let first = first.lines().next().unwrap_or_default().to_string();
                    eprintln!("qn doc: skipping {unit}.qn (does not load standalone): {first}");
                }
            }
        }
        let units = loaded;
        let (infos, known) = arena.mutate_root(|_mc, vm| {
            let names: Vec<String> = introspect::globals(vm)
                .into_iter()
                .filter(|g| g.kind == GlobalKind::Class && !is_internal(&g.name))
                .map(|g| g.name)
                .collect();
            let mut infos: Vec<ClassInfo> = names
                .iter()
                .filter_map(|n| introspect::describe_class(vm, n))
                .collect();
            infos.sort_by(|a, b| a.name.cmp(&b.name));
            (infos, names)
        });
        DocInputs {
            infos,
            project,
            units,
            commands: if project {
                collect_commands()
            } else {
                Vec::new()
            },
            readme: if project {
                std::fs::read_to_string("README.md").ok()
            } else {
                None
            },
            known,
        }
    }
}

/// What one collection pass hands the model builder.
struct DocInputs {
    infos: Vec<ClassInfo>,
    project: bool,
    /// The `use self:` units that were loaded (— `--check` preloads them again so a
    /// project example doesn't have to repeat the project's own `use` line).
    units: Vec<String>,
    commands: Vec<CommandDoc>,
    readme: Option<String>,
    known: Vec<String>,
}

/// A `self:`-loaded unit — the provenance test that partitions project from platform.
fn is_project_file(file: &str) -> bool {
    file.starts_with("self:")
}

/// The project's `use self:` paths: every `.qn` under the roots (default `.`), sorted —
/// minus `tests/` (suites aren't API), `bin/` (commands, sniffed separately), the output
/// directory, hidden directories, and shebang-first files (a script's top level IS the
/// program; loading would run it). A directory holding files whose names the `use` path
/// grammar can't spell (hyphens, leading digits — `lib/00-task.qn`) loads as ONE glob
/// (`self:lib/*`), exactly how the stdlib loads its own core; name-safe files load
/// individually, which keeps the warn-and-skip granular. A root-level glob-only name
/// can't be reached either way and is skipped with a warning.
fn discover_project_units(roots: &[String]) -> Vec<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    if roots.is_empty() {
        walk_units(Path::new("."), &mut files);
    } else {
        for root in roots {
            let p = Path::new(root);
            if p.is_dir() {
                walk_units(p, &mut files);
            } else {
                files.push(p.to_path_buf());
            }
        }
    }
    // Group by directory; decide glob-vs-individual per group.
    let mut by_dir: Vec<(String, Vec<String>)> = Vec::new();
    for f in &files {
        let Some(rel) = f.to_str().map(|s| s.trim_start_matches("./")) else {
            continue;
        };
        let unit = rel.strip_suffix(".qn").unwrap_or(rel).to_string();
        let dir = unit
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();
        match by_dir.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, v)) => v.push(unit),
            None => by_dir.push((dir, vec![unit])),
        }
    }
    let mut units: Vec<String> = Vec::new();
    for (dir, group) in &by_dir {
        if group.iter().all(|u| unit_name_safe(u)) {
            units.extend(group.iter().cloned());
        } else if dir.is_empty() {
            for u in group {
                if unit_name_safe(u) {
                    units.push(u.clone());
                } else {
                    eprintln!(
                        "qn doc: skipping {u}.qn — its name can't be spelled as a `use` \
                         path (hyphen or leading digit); move it into a subdirectory"
                    );
                }
            }
        } else {
            units.push(format!("{dir}/*"));
        }
    }
    units.sort();
    units.dedup();
    units
}

/// Whether every path segment is a spellable `use` identifier.
fn unit_name_safe(unit: &str) -> bool {
    unit.split('/').all(|seg| {
        let mut chars = seg.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn walk_units(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    children.sort();
    for child in children {
        let name = child
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if child.is_dir() {
            if name.starts_with('.') || name == "tests" || name == "bin" || name == "qn-docs" {
                continue;
            }
            walk_units(&child, out);
        } else if name.ends_with(".qn") && !starts_with_shebang(&child) {
            out.push(child);
        }
    }
}

fn starts_with_shebang(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|t| t.starts_with("#!"))
        .unwrap_or(false)
}

/// The project's commands: `bin/` entries whose first line is a `#!` mentioning `qn`,
/// with the `"*` block under the shebang as the doc. Flag-level docs need static
/// discovery and are future work (QUOIN_TODO).
fn collect_commands() -> Vec<CommandDoc> {
    let Ok(entries) = std::fs::read_dir("bin") else {
        return Vec::new();
    };
    let mut out: Vec<CommandDoc> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_str()?.to_string();
            let text = std::fs::read_to_string(&path).ok()?;
            let mut lines = text.lines();
            let first = lines.next()?;
            if !(first.starts_with("#!") && first.contains("qn")) {
                return None;
            }
            let doc_lines: Vec<&str> = lines
                .take_while(|l| l.starts_with("\"*"))
                .map(|l| {
                    l.strip_prefix("\"* ")
                        .or_else(|| l.strip_prefix("\"*"))
                        .unwrap_or(l)
                })
                .collect();
            Some(CommandDoc {
                name,
                doc: (!doc_lines.is_empty()).then(|| doc_lines.join("\n")),
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn write_out(path: &Path, text: &str) {
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("qn doc: cannot write {}: {e}", path.display());
        exit(1);
    }
}

/// The `MethodDoc`s for a method list — one doc per selector: the first variant that has
/// one (native `.doc(..)` text first, else the comment block above the first located
/// variant), sorted by selector.
fn method_docs(
    list: &[MethodInfo],
    doc_at: &mut impl FnMut(&str, usize, Option<&str>) -> Option<String>,
) -> Vec<MethodDoc> {
    let mut out: Vec<MethodDoc> = list
        .iter()
        .map(|m| {
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
}

/// Assemble the model: introspection data plus lazily-extracted Quoin docs. Source files are
/// read once each (`cache`), through the same resolution `$doc` will use. In project mode
/// the class table is PARTITIONED by provenance: `self:`-defined classes are the subject;
/// platform classes carrying `self:`-defined methods become extension groups; everything
/// else is background (link targets only).
fn build_model(inputs: DocInputs) -> DocModel {
    let DocInputs {
        infos,
        project,
        units: _,
        commands,
        readme,
        known,
    } = inputs;
    let (subjects, background): (Vec<ClassInfo>, Vec<ClassInfo>) = if project {
        infos
            .into_iter()
            .partition(|i| i.source.as_ref().is_some_and(|s| is_project_file(&s.file)))
    } else {
        (infos, Vec::new())
    };
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

    let classes: Vec<ClassDoc> = subjects
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
                instance_methods: method_docs(&info.instance_methods, &mut doc_at),
                class_methods: method_docs(&info.class_methods, &mut doc_at),
            }
        })
        .collect();

    // Method-level provenance: a platform class with `self:`-defined variants is a host
    // the project extended. Only the project's variants document here.
    let mut extensions: Vec<ExtensionGroup> = Vec::new();
    for info in &background {
        let project_methods = |list: &[MethodInfo]| -> Vec<MethodInfo> {
            list.iter()
                .filter_map(|m| {
                    let variants: Vec<_> = m
                        .variants
                        .iter()
                        .filter(|v| v.source.as_ref().is_some_and(|s| is_project_file(&s.file)))
                        .cloned()
                        .collect();
                    (!variants.is_empty()).then(|| MethodInfo {
                        selector: m.selector.clone(),
                        variants,
                    })
                })
                .collect()
        };
        let inst = project_methods(&info.instance_methods);
        let cls = project_methods(&info.class_methods);
        let sources: Vec<&introspect::SourceLoc> = info
            .extension_sources
            .iter()
            .filter(|s| is_project_file(&s.file))
            .collect();
        if inst.is_empty() && cls.is_empty() && sources.is_empty() {
            continue;
        }
        let doc = sources
            .iter()
            .find_map(|src| doc_at(&src.file, src.line, None));
        extensions.push(ExtensionGroup {
            host: info.name.clone(),
            doc,
            sources: sources
                .iter()
                .map(|s| format!("{}:{}", s.file, s.line))
                .collect(),
            instance_methods: method_docs(&inst, &mut doc_at),
            class_methods: method_docs(&cls, &mut doc_at),
        });
    }
    extensions.sort_by(|a, b| a.host.cmp(&b.host));

    let title = if project {
        std::env::current_dir()
            .ok()
            .and_then(|d| d.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "Project".to_string())
    } else {
        "Quoin reference".to_string()
    };
    DocModel {
        version: 2,
        project,
        title,
        readme,
        commands,
        classes,
        extensions,
        stdlib_path: None, // the caller stamps the flag in
        known,
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
            crate::runner::print_or_exit(&format!("undocumented class: {}\n", class.name));
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
                    crate::runner::print_or_exit(&format!(
                        "undocumented: {} {}{}\n",
                        class.name, side, m.selector
                    ));
                }
            }
        }
    }
    for group in &model.extensions {
        for (side, list) in [
            ("", &group.instance_methods),
            (".meta ", &group.class_methods),
        ] {
            for m in list {
                total += 1;
                if m.doc.is_some() {
                    documented += 1;
                } else {
                    crate::runner::print_or_exit(&format!(
                        "undocumented extension: {} {}{}\n",
                        group.host, side, m.selector
                    ));
                }
            }
        }
    }
    let pct = if total == 0 {
        100.0
    } else {
        documented as f64 * 100.0 / total as f64
    };
    crate::runner::print_or_exit(&format!("doc coverage: {documented}/{total} ({pct:.1}%)\n"));
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

/// Link `name` (bare or nullable `Foo?`): to its local page when the model documents it,
/// to the published stdlib reference when `--stdlib-path` names one and the session knows
/// the class, else plain text.
fn type_link(name: &str, model: &DocModel) -> String {
    let bare = name.strip_suffix('?').unwrap_or(name);
    if model.classes.iter().any(|c| c.name == bare) {
        return format!(r#"<a href="{}">{}</a>"#, page_name(bare), esc(name));
    }
    if let Some(prefix) = &model.stdlib_path
        && model.known.iter().any(|k| k == bare)
    {
        return format!(
            r#"<a href="{}/{}">{}</a>"#,
            prefix.trim_end_matches('/'),
            page_name(bare),
            esc(name)
        );
    }
    esc(name)
}

/// The extension page for a host class: `ext.OS.Process.html`.
fn ext_page_name(host: &str) -> String {
    format!("ext.{}", page_name(host))
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

    // A README that opens with its own `# title` renders as the page heading;
    // otherwise the model's title does.
    let readme_has_title = model
        .readme
        .as_deref()
        .and_then(|r| r.lines().find(|l| !l.trim().is_empty()))
        .is_some_and(|l| l.starts_with("# "));
    let mut body = if readme_has_title {
        String::new()
    } else {
        format!("<h1>{}</h1>\n", esc(&model.title))
    };
    if let Some(readme) = &model.readme {
        body.push_str(&md_html(readme));
        body.push_str("<hr>\n");
    }
    if !model.commands.is_empty() {
        body.push_str("<h2>Commands</h2>\n<ul class=\"classlist\">\n");
        for cmd in &model.commands {
            let summary = cmd
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
                "<li><code>bin/{}</code>{}</li>\n",
                esc(&cmd.name),
                summary
            );
        }
        body.push_str("</ul>\n");
    }
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
    if !model.extensions.is_empty() {
        body.push_str("<h2>Extensions</h2>\n<ul class=\"classlist\">\n");
        for group in &model.extensions {
            let n = group.instance_methods.len() + group.class_methods.len();
            let _ = write!(
                body,
                "<li><a href=\"{}\"><code>{}</code></a><span class=\"summary\">{} added \
                 method{}</span></li>\n",
                ext_page_name(&group.host),
                esc(&group.host),
                n,
                if n == 1 { "" } else { "s" }
            );
        }
        body.push_str("</ul>\n");
    }
    page(&model.title, &body)
}

/// Render an extension group as a class-like page: the project's additions to a platform
/// class, with the host linked (locally never — it isn't a subject — so via
/// `--stdlib-path` when set).
fn render_extension(group: &ExtensionGroup, model: &DocModel) -> String {
    let mut body = String::new();
    let _ = write!(body, "<p><a href=\"index.html\">← index</a></p>\n");
    let _ = write!(
        body,
        "<h1>Extensions to <code>{}</code></h1>\n",
        type_link(&group.host, model)
    );
    if !group.sources.is_empty() {
        let srcs: Vec<String> = group.sources.iter().map(|s| source_html(s)).collect();
        let _ = write!(
            body,
            "<p class=\"meta-line\">extended at {}</p>\n",
            srcs.join(" · ")
        );
    }
    if let Some(doc) = &group.doc {
        body.push_str(&doc_html(doc));
    }
    for (title, anchor, list) in [
        ("Class methods", "c", &group.class_methods),
        ("Instance methods", "i", &group.instance_methods),
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
            }
            body.push_str("</div></div>\n");
        }
    }
    page(&format!("Extensions to {}", group.host), &body)
}

/// Minimal markdown for the README preamble: ATX headers, fenced code (highlighted when
/// tagged `quoin` or untagged-but-Quoin-looking is NOT attempted — only `quoin` fences
/// highlight, the rest render plain), paragraphs, and the inline layer (backtick spans and
/// `[text](url)` links). Everything else renders as prose — honest, not a full renderer.
fn md_html(md: &str) -> String {
    let mut out = String::new();
    let mut para: Vec<&str> = Vec::new();
    let mut items: Vec<String> = Vec::new();
    let mut fence: Option<(String, Vec<&str>)> = None;
    let flush_para = |out: &mut String, para: &mut Vec<&str>| {
        if !para.is_empty() {
            let _ = write!(out, "<p>{}</p>\n", inline_md(&para.join(" ")));
            para.clear();
        }
    };
    let flush_items = |out: &mut String, items: &mut Vec<String>| {
        if !items.is_empty() {
            out.push_str("<ul>\n");
            for item in items.iter() {
                let _ = write!(out, "<li>{}</li>\n", inline_md(item));
            }
            out.push_str("</ul>\n");
            items.clear();
        }
    };
    for line in md.lines() {
        if let Some((tag, body)) = &mut fence {
            if line.trim_start().starts_with("```") {
                if tag.starts_with("quoin") {
                    let _ = write!(
                        out,
                        "{}\n",
                        crate::highlighter::highlight_to_html(&body.join("\n"))
                    );
                } else {
                    let _ = write!(out, "<pre>{}</pre>\n", esc(&body.join("\n")));
                }
                fence = None;
            } else {
                body.push(line);
            }
        } else if let Some(info) = line.trim_start().strip_prefix("```") {
            flush_para(&mut out, &mut para);
            flush_items(&mut out, &mut items);
            fence = Some((info.trim().to_string(), Vec::new()));
        } else if let Some(rest) = line.strip_prefix('#') {
            flush_para(&mut out, &mut para);
            flush_items(&mut out, &mut items);
            let level = 1 + rest.chars().take_while(|&c| c == '#').count().min(4);
            let text = rest.trim_start_matches('#').trim();
            let _ = write!(out, "<h{level}>{}</h{level}>\n", inline_md(text));
        } else if let Some(item) = line.trim_start().strip_prefix("- ") {
            flush_para(&mut out, &mut para);
            items.push(item.to_string());
        } else if line.trim().is_empty() {
            flush_para(&mut out, &mut para);
            flush_items(&mut out, &mut items);
        } else if !items.is_empty() {
            // A wrapped continuation of the previous bullet.
            let last = items.len() - 1;
            items[last].push(' ');
            items[last].push_str(line.trim());
        } else {
            para.push(line);
        }
    }
    if let Some((_, body)) = fence {
        let _ = write!(out, "<pre>{}</pre>\n", esc(&body.join("\n")));
    }
    flush_para(&mut out, &mut para);
    flush_items(&mut out, &mut items);
    out
}

/// The inline layer plus `**bold**` and `[text](url)` links (applied on the escaped text,
/// so an `&` in a URL is already `&amp;` — valid HTML).
fn inline_md(text: &str) -> String {
    let mut s = inline(text);
    // **bold**: pairs only; an odd trailing ** stays literal.
    while let Some(a) = s.find("**") {
        let Some(b) = s[a + 2..].find("**") else {
            break;
        };
        let b = a + 2 + b;
        s = format!(
            "{}<strong>{}</strong>{}",
            &s[..a],
            &s[a + 2..b],
            &s[b + 2..]
        );
    }
    let mut out = String::new();
    let mut rest = s.as_str();
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](") else {
            break;
        };
        let close = open + close;
        let Some(end) = rest[close + 2..].find(')') else {
            break;
        };
        let end = close + 2 + end;
        let label = &rest[open + 1..close];
        let url = &rest[close + 2..end];
        out.push_str(&rest[..open]);
        let _ = write!(out, r#"<a href="{url}">{label}</a>"#);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
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

// ---- `qn doc --check`: run the documentation's examples ---------------------------------
//
// One engine, two corpora (docs/DOCS_ARCH.md phase 3 + the RELEASE_PREP Tier 2 harness):
//
//   * With PATHs: markdown files/directories. A fenced block tagged `quoin` runs; `quoin
//     norun` displays only. Untagged fences are prose/output samples and never run.
//   * Without PATHs: the fenced examples inside the stdlib's own doc comments. No tags exist
//     there, so the rule is the one the corpus was written under: a block runs iff it carries
//     at least one `"* -> value` annotation — the verified examples annotate, the
//     illustrative ones (live sockets, HTTP) deliberately do not.
//
// Each block runs in a fresh session (prelude only — an example must be runnable as pasted,
// including its own `use` lines), statement by statement with persistent bindings, exactly
// like the REPL. An annotated statement's rendered value must match the annotation text —
// or, failing that, render equal to the annotation evaluated as a literal (`-> nil`,
// `-> 'quoted\nstring'`), which absorbs both annotation conventions the docs use.

/// One runnable example, with a label good enough to find it again.
struct ExampleBlock {
    label: String,
    source: String,
}

impl VmRunner {
    pub(crate) fn run_doc_check(&self) -> Result<(), QuoinError> {
        let paths: Vec<String> = self.options.vm_options.arguments.clone();
        // Project examples run with the project preloaded (the subject shouldn't have to
        // `use` itself); stdlib and markdown examples must be runnable as pasted.
        let mut preload: Vec<String> = Vec::new();
        let blocks = if paths.is_empty() {
            let inputs = self.collect_inputs(&[]);
            if inputs.project {
                preload = inputs.units.clone();
            }
            let model = build_model(inputs);
            doc_example_blocks(&model)
        } else {
            let mut blocks = Vec::new();
            for p in &paths {
                collect_markdown_blocks(Path::new(p), &mut blocks);
            }
            blocks
        };

        let total = blocks.len();
        let mut checked_annotations = 0usize;
        let mut failures: Vec<String> = Vec::new();
        for block in &blocks {
            match self.run_example(block, &preload) {
                Ok(n) => checked_annotations += n,
                Err(msg) => failures.push(msg),
            }
        }

        for f in &failures {
            eprintln!("{f}\n");
        }
        crate::runner::print_or_exit(&format!(
            "qn doc --check: {total} examples, {checked_annotations} annotations checked, {} failed\n",
            failures.len()
        ));
        if !failures.is_empty() {
            exit(1);
        }
        Ok(())
    }

    /// Run one example block: parse (only to find statement boundaries), then evaluate the
    /// block line-group by line-group in a fresh session, checking each `"* -> value`
    /// annotation. Returns the number of annotations checked, or a formatted failure.
    ///
    /// Groups are WHOLE LINES between consecutive statements' start lines — not the parser's
    /// byte spans, which exclude wrapping parentheses (`(a + b).s` would slice to `a + b).s`)
    /// and may bleed past a statement's own text. Whole lines also carry the trailing
    /// annotation comment along harmlessly, and a standalone `"* -> …` line after the last
    /// statement lands in that statement's group.
    fn run_example(&self, block: &ExampleBlock, preload: &[String]) -> Result<usize, String> {
        use crate::parser::{NodeValue, try_parse_quoin_string_named};

        let node = try_parse_quoin_string_named(&block.source, &block.label).map_err(|pe| {
            format!(
                "FAIL {} — example does not parse (line {}, col {}): {}",
                block.label, pe.line, pe.column, pe.message
            )
        })?;
        let NodeValue::Program(program) = &node.value else {
            return Err(format!("FAIL {} — not a program", block.label));
        };
        let lines: Vec<&str> = block.source.lines().collect();
        // 1-based start lines, deduped: statements sharing a line run as one group.
        let mut starts: Vec<usize> = program
            .expressions
            .iter()
            .filter_map(|stmt| stmt.source_info.as_ref().map(|si| si.line))
            .collect();
        starts.dedup();

        let annotation_of = |first: usize, last: usize| -> Option<&str> {
            lines[first - 1..last]
                .iter()
                .rev()
                .find(|l| !l.trim().is_empty())
                .and_then(|l| l.split_once("\"* ->").map(|(_, v)| v.trim()))
        };
        // When only the FINAL statement carries an annotation, run the block as ONE program —
        // exactly how `qn -e` verified it. Concurrency examples need this: each eval drives
        // the scheduler to completion, so a `Task.spawn:` evaluated on its own line finishes
        // before a later line's `join` (a REPL user sees the same thing; a program does not).
        let group_bounds: Vec<(usize, usize)> = starts
            .iter()
            .enumerate()
            .map(|(i, &st)| (st, starts.get(i + 1).map(|&n| n - 1).unwrap_or(lines.len())))
            .collect();
        let intermediate_annotated = group_bounds
            .iter()
            .rev()
            .skip(1)
            .any(|&(f, l)| annotation_of(f, l).is_some());
        if !intermediate_annotated {
            let mut arena = self.example_session(&block.label, &block.source, preload)?;
            let rendered = match runner_repl::eval_once(&mut arena, &block.source) {
                Ok(r) => r.unwrap_or_default(),
                Err(e) => return Err(format!("FAIL {} — {}", block.label, e)),
            };
            let Some(expected) = group_bounds.last().and_then(|&(f, l)| annotation_of(f, l)) else {
                return Ok(0);
            };
            if rendered == expected {
                return Ok(1);
            }
            let expected_rendered = runner_repl::eval_once(&mut arena, expected)
                .map(|r| r.unwrap_or_default())
                .ok();
            if expected_rendered.as_deref() == Some(rendered.as_str()) {
                return Ok(1);
            }
            return Err(format!(
                "FAIL {} — final value:\n  expected: {}\n  got:      {}",
                block.label, expected, rendered
            ));
        }

        let mut arena = self.example_session(&block.label, &block.source, preload)?;
        let mut checked = 0usize;
        for (i, &start) in starts.iter().enumerate() {
            let end = starts.get(i + 1).map(|&n| n - 1).unwrap_or(lines.len());
            let group = lines[start - 1..end].join("\n");
            let rendered = match runner_repl::eval_once(&mut arena, &group) {
                Ok(r) => r.unwrap_or_default(),
                Err(e) => {
                    return Err(format!(
                        "FAIL {} (line {start}) — `{}`:\n  {}",
                        block.label,
                        first_code_line(&group),
                        e
                    ));
                }
            };
            // The group's annotation: on its LAST line — trailing after the expression, or a
            // standalone `"* -> …` comment line.
            let Some(expected) = annotation_of(start, end) else {
                continue;
            };
            checked += 1;
            if rendered == expected {
                continue;
            }
            // Fallback: the annotation as a literal, rendered by the same pipeline — absorbs
            // `-> nil` (renders empty) and `-> 'quoted\nstrings'`.
            let expected_rendered = runner_repl::eval_once(&mut arena, expected)
                .map(|r| r.unwrap_or_default())
                .ok();
            if expected_rendered.as_deref() == Some(rendered.as_str()) {
                continue;
            }
            return Err(format!(
                "FAIL {} (line {start}) — `{}`:\n  expected: {}\n  got:      {}",
                block.label,
                first_code_line(&group),
                expected,
                rendered
            ));
        }
        Ok(checked)
    }
}

impl VmRunner {
    /// A fresh example session. The reference documents the whole shipping stdlib, so an
    /// example assumes the class it sits on is loadable (the class doc states any needed
    /// `use` once; method examples don't repeat it) — but only the prelude is free, so the
    /// net/web/test units load only when the example's source names them. A miss shows up as
    /// a visible NameError failure, never a silent pass.
    fn example_session(
        &self,
        label: &str,
        source: &str,
        preload: &[String],
    ) -> Result<ReplArena, String> {
        let Some(mut arena) = self.build_repl_arena() else {
            return Err(format!("FAIL {label} — no session arena"));
        };
        let mut load = |unit: &str| -> Result<(), String> {
            runner_repl::eval_once(&mut arena, unit)
                .map(|_| ())
                .map_err(|e| format!("FAIL {label} — loading the stdlib: {e}"))
        };
        for unit in preload {
            load(&format!("use self:{unit}"))?;
        }
        if source.contains("[HTTP]") || source.contains("TcpServer") {
            load("use std:net/*")?;
        }
        if source.contains("[Web]") {
            load("use std:net/*")?;
            load("use std:web/*")?;
        }
        if source.contains("TestSuite") {
            load("use test")?;
        }
        Ok(arena)
    }
}

/// The first non-blank line of a group, for failure messages.
fn first_code_line(group: &str) -> &str {
    group
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// Corpus B: every runnable fenced example in the subject's doc comments — class docs,
/// extension docs, and method docs, labeled by owner.
fn doc_example_blocks(model: &DocModel) -> Vec<ExampleBlock> {
    let mut blocks = Vec::new();
    let mut add = |label: &str, doc: &Option<String>| {
        if let Some(doc) = doc {
            for source in fenced_blocks(doc) {
                // Runs iff annotated (see the module comment): the verified/illustrative
                // split the corpus was written under.
                if source.contains("\"* ->") {
                    blocks.push(ExampleBlock {
                        label: label.to_string(),
                        source,
                    });
                }
            }
        }
    };
    for class in &model.classes {
        add(&class.name, &class.doc);
        for ext in &class.extensions {
            add(
                &format!("{} (extension)", class.name),
                &Some(ext.doc.clone()),
            );
        }
        for (side, list) in [
            ("", &class.instance_methods),
            (".meta ", &class.class_methods),
        ] {
            for m in list {
                add(&format!("{} {side}{}", class.name, m.selector), &m.doc);
            }
        }
    }
    for group in &model.extensions {
        add(&format!("extensions to {}", group.host), &group.doc);
        for (side, list) in [
            ("", &group.instance_methods),
            (".meta ", &group.class_methods),
        ] {
            for m in list {
                add(&format!("{} {side}{}", group.host, m.selector), &m.doc);
            }
        }
    }
    blocks
}

/// Corpus A: fenced blocks tagged `quoin` in markdown files (recursively for a directory).
/// `quoin norun` displays without running; untagged fences are prose/output samples.
fn collect_markdown_blocks(path: &Path, out: &mut Vec<ExampleBlock>) {
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        children.sort();
        for child in children {
            collect_markdown_blocks(&child, out);
        }
        return;
    }
    if path.extension().and_then(|e| e.to_str()) != Some("md") {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let mut in_block: Option<(usize, Vec<String>)> = None;
    for (i, line) in text.lines().enumerate() {
        // Fences inside blockquotes (the book's Gotcha boxes) count too: strip the
        // quote marker before matching, and from body lines before running.
        let line = line
            .trim_start()
            .strip_prefix('>')
            .map(|r| r.strip_prefix(' ').unwrap_or(r))
            .unwrap_or(line)
            .to_string();
        let trimmed = line.trim_start();
        if let Some((start, body)) = &mut in_block {
            if trimmed.starts_with("```") {
                out.push(ExampleBlock {
                    label: format!("{}:{}", path.display(), *start + 1),
                    source: body.join("\n"),
                });
                in_block = None;
            } else {
                body.push(line.clone());
            }
        } else if let Some(info) = trimmed.strip_prefix("```") {
            let info = info.trim();
            if info == "quoin" || (info.starts_with("quoin") && !info.contains("norun")) {
                in_block = Some((i, Vec::new()));
            }
        }
    }
}

/// The fenced code blocks inside a doc text (post-extraction: the leading `"*` markers are
/// already stripped; fences are bare ``` lines).
fn fenced_blocks(doc: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            match current.take() {
                Some(body) => blocks.push(body.join("\n")),
                None => current = Some(Vec::new()),
            }
        } else if let Some(body) = &mut current {
            body.push(line);
        }
    }
    blocks
}
