use super::*;

fn sig(
    parent: Option<&str>,
    mixins: &[&str],
    own: &[&str],
    sealed: bool,
    catch_all: bool,
) -> ClassSig {
    ClassSig {
        parent: parent.map(Arc::from),
        mixins: mixins.iter().map(|m| Arc::from(*m)).collect(),
        own_selectors: own.iter().map(|s| Arc::from(*s)).collect(),
        sealed,
        has_catch_all: catch_all,
        from_vm: false,
        method_params: HashMap::new(),
        method_returns: HashMap::new(),
    }
}

/// Animal ← Dog; Fish ← Animal + mixin Swimmer.
fn table() -> ClassTable {
    let t = ClassTable::new();
    t.insert("Animal", sig(None, &[], &["eat", "sound"], false, false));
    t.insert(
        "Dog",
        sig(Some("Animal"), &[], &["fetch", "sound"], true, false),
    );
    t.insert("Swimmer", sig(None, &[], &["swim"], false, false));
    t.insert(
        "Fish",
        sig(Some("Animal"), &["Swimmer"], &["gills"], false, false),
    );
    t
}

#[test]
fn responds_walks_own_mixins_parent_like_dispatch() {
    let t = table();
    assert_eq!(t.responds_to("Dog", "fetch"), Some(true)); // own
    assert_eq!(t.responds_to("Dog", "eat"), Some(true)); // inherited from parent
    assert_eq!(t.responds_to("Dog", "sound"), Some(true)); // own override of inherited
    assert_eq!(t.responds_to("Fish", "swim"), Some(true)); // via a mixin
    assert_eq!(t.responds_to("Fish", "eat"), Some(true)); // inherited via parent
    assert_eq!(t.responds_to("Dog", "fly"), Some(false)); // nowhere in the chain → definite no
}

#[test]
fn responds_stays_silent_when_unsure() {
    let t = table();
    // Class not in the table.
    assert_eq!(t.responds_to("Ghost", "boo"), None);
    // A class whose parent is unknown — can't be sure it doesn't inherit the selector…
    t.insert(
        "Orphan",
        sig(Some("MissingParent"), &[], &["x"], false, false),
    );
    assert_eq!(t.responds_to("Orphan", "y"), None);
    // …but an own method is still a definite yes.
    assert_eq!(t.responds_to("Orphan", "x"), Some(true));
    // A catch-all handler responds to everything → never MNU.
    t.insert("Proxy", sig(None, &[], &[], false, true));
    assert_eq!(t.responds_to("Proxy", "anything"), None);
    // A catch-all *ancestor* also silences MNU for the descendant.
    t.insert("SubProxy", sig(Some("Proxy"), &[], &["own"], false, false));
    assert_eq!(t.responds_to("SubProxy", "whatever"), None);
    assert_eq!(t.responds_to("SubProxy", "own"), Some(true));
}

#[test]
fn subtyping_follows_parent_and_mixin_chains() {
    let t = table();
    assert_eq!(t.is_subtype("Dog", "Animal"), Some(true));
    assert_eq!(t.is_subtype("Dog", "Dog"), Some(true));
    assert_eq!(t.is_subtype("Animal", "Dog"), Some(false));
    assert_eq!(t.is_subtype("Fish", "Swimmer"), Some(true)); // via mixin
    assert_eq!(t.is_subtype("Fish", "Animal"), Some(true)); // via parent
    assert_eq!(t.is_subtype("Ghost", "Animal"), None); // unknown class
}

#[test]
fn cycles_terminate() {
    let t = ClassTable::new();
    t.insert("A", sig(Some("B"), &[], &["a"], false, false));
    t.insert("B", sig(Some("A"), &[], &["b"], false, false));
    assert_eq!(t.responds_to("A", "b"), Some(true));
    assert_eq!(t.responds_to("A", "zzz"), Some(false));
    assert_eq!(t.is_subtype("A", "B"), Some(true));
    assert_eq!(t.is_subtype("A", "C"), Some(false));
}

fn returns(pairs: &[(&str, Type)]) -> HashMap<Arc<str>, Type> {
    pairs
        .iter()
        .map(|(s, t)| (Arc::from(*s), t.clone()))
        .collect()
}

#[test]
fn add_returns_upserts_and_merges_without_disturbing_the_sig() {
    let t = ClassTable::new();
    // Upsert onto an absent class: a bare entry is created carrying just the returns.
    t.add_returns("Ghost", returns(&[("boo", Type::String)]));
    let g = t.get("Ghost").unwrap();
    assert_eq!(g.method_returns.get("boo"), Some(&Type::String));
    assert!(g.own_selectors.is_empty());

    // Merge onto an existing sig: returns accumulate, structural fields untouched.
    t.insert("Dog", sig(Some("Animal"), &[], &["fetch"], true, false));
    t.add_returns("Dog", returns(&[("fetch", Type::Int)]));
    let d = t.get("Dog").unwrap();
    assert_eq!(d.method_returns.get("fetch"), Some(&Type::Int));
    assert_eq!(d.parent.as_deref(), Some("Animal"));
    assert!(d.sealed);
}

#[test]
fn insert_preserves_accumulated_returns_across_a_returnless_overwrite() {
    let t = ClassTable::new();
    // AST-recorded returns land first…
    t.add_returns("Widget", returns(&[("size", Type::Int)]));
    // …then a later `from_vm`-style sig (no returns of its own) overwrites the structural fields.
    t.insert("Widget", sig(None, &[], &["size", "grow"], false, false));
    let w = t.get("Widget").unwrap();
    assert_eq!(w.method_returns.get("size"), Some(&Type::Int)); // preserved
    assert!(w.own_selectors.contains(&Arc::from("grow"))); // structural fields applied
}
