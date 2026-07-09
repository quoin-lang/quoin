//! Unit tests for the embedded stdlib table (`build.rs` -> `stdlib.rs`).

use super::{UNITS, list, resolve};

#[test]
fn table_is_sorted_so_binary_search_is_sound() {
    let mut sorted: Vec<&str> = UNITS.iter().map(|(u, _)| *u).collect();
    let as_emitted = sorted.clone();
    sorted.sort_unstable();
    assert_eq!(as_emitted, sorted, "UNITS must be lexicographically sorted");
}

#[test]
fn resolves_the_shipping_units() {
    // The prelude and the test framework are the two root units.
    assert!(resolve("prelude").expect("prelude").contains("use core/*"));
    assert!(resolve("test").expect("test").contains("TestSuite"));
    assert!(resolve("core/00-bootstrap").is_some());
    assert!(resolve("net/http").is_some());
}

#[test]
fn source_tree_only_units_are_not_embedded() {
    // The language's own suite, benchmarks and fixtures stay on disk — an installed
    // `qn test` must run the *caller's* tests, not ours.
    assert!(resolve("main").is_none());
    assert!(resolve("benchmark").is_none());
    assert!(resolve("tests/01-iterate").is_none());
    assert!(list("tests").is_none());
}

#[test]
fn list_returns_direct_children_only() {
    let core = list("core").expect("core lists");
    assert!(core.iter().all(|u| u.starts_with("core/")));
    assert!(core.contains(&"core/00-bootstrap".to_string()));
    // Sorted, so `use core/*` loads 00-bootstrap before 01-case.
    let mut sorted = core.clone();
    sorted.sort();
    assert_eq!(core, sorted);

    // The root holds bare stems, never `dir/stem`.
    let root = list("").expect("root lists");
    assert!(root.iter().all(|u| !u.contains('/')), "root: {root:?}");
    assert!(root.contains(&"prelude".to_string()));

    // A prefix that is not a directory boundary must not match (`cor` != `core`).
    assert!(list("cor").is_none());
    assert!(list("nope").is_none());
}
