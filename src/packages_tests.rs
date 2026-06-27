//! Unit tests for `FsResolver` package resolution — the `use pkg:*` whole-package glob in both
//! forms: a *file* package (lists the `.qn` units in the package root) and an *extension* package
//! (synthesized `Extension.loadPackage:` glue). `cargo test`'s CWD is the crate root, so `qnlib/`
//! and the bundled `quoin_packages/adbc/` are on the search path.

use super::{FsResolver, PackageResolver};

#[test]
fn root_glob_of_a_file_package_lists_bare_stems() {
    let r = FsResolver::new();
    // `use std:*` globs the stdlib ROOT (empty dir): unit names are bare stems, never `/stem`
    // (the leading-slash bug that empty-dir listing used to produce).
    let units = r.list(Some("std"), "").expect("qnlib root lists");
    assert!(!units.is_empty(), "qnlib root has .qn units");
    assert!(
        units.iter().all(|u| !u.starts_with('/')),
        "no leading slash: {units:?}"
    );
    // a known stdlib-root unit, and it round-trips back to source through `resolve`
    assert!(units.iter().any(|u| u == "prelude"), "units: {units:?}");
    assert!(r.resolve(Some("std"), "prelude").is_some());
}

#[test]
fn extension_package_glob_synthesizes_loadpackage_glue() {
    let r = FsResolver::new();
    // The bundled `quoin_packages/adbc/` is on the default search root, so `use adbc:*` maps to the
    // one synthetic unit `*`, which `resolve` turns into a single line of `loadPackage:` glue.
    assert_eq!(r.list(Some("adbc"), ""), Some(vec!["*".to_string()]));
    let glue = r.resolve(Some("adbc"), "*").expect("adbc glue");
    assert!(
        glue.contains("Extension.loadPackage:") && glue.contains("adbc"),
        "glue: {glue}"
    );
    // a sub-path of a named extension package has no units; only the whole-package `*` resolves.
    assert!(r.list(Some("adbc"), "sub").is_none());
}

#[test]
fn an_unknown_named_package_is_unresolved() {
    let r = FsResolver::new();
    assert!(r.list(Some("definitely_not_a_package"), "").is_none());
    assert!(r.resolve(Some("definitely_not_a_package"), "*").is_none());
}
