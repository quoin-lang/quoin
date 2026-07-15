//! Unit-cache behavior: chained keys, hit/miss across fresh sessions, and
//! source-edit invalidation. Sessions here mirror production sharing: every
//! arena clones the SAME `VmOptions` (so `SeenTypes`/`ClassTable` are shared
//! handles), which is the precondition that makes hit-skipping sound.

use std::sync::{Arc, Mutex};

use gc_arena::{Arena, Rootable};

use crate::packages::PackageResolver;
use crate::registry::register_builtins;
use crate::runtime::runtime::load_unit;
use crate::runtime::unit_cache;
use crate::value::NamespacedName;
use crate::vm::{VmOptions, VmState};

#[test]
fn advance_chains_identity_and_source() {
    let a = unit_cache::advance(0, None, "core/list", "src-a");
    assert_eq!(a, unit_cache::advance(0, None, "core/list", "src-a"));
    // Any ingredient changes the key…
    assert_ne!(a, unit_cache::advance(0, None, "core/list", "src-B"));
    assert_ne!(a, unit_cache::advance(0, None, "core/set", "src-a"));
    assert_ne!(a, unit_cache::advance(0, Some("web"), "core/list", "src-a"));
    // …and so does everything loaded BEFORE (the chained-context property):
    // the same unit after a different predecessor gets a different key.
    let after_p = unit_cache::advance(unit_cache::advance(0, None, "p", "sp"), None, "u", "su");
    let after_q = unit_cache::advance(unit_cache::advance(0, None, "q", "sq"), None, "u", "su");
    assert_ne!(after_p, after_q);
}

/// Resolves exactly one unit, `probe`, to whatever source it currently holds —
/// the mutable source is the "edited qnlib under a daemon" stand-in.
struct OneUnit(Arc<Mutex<String>>);

impl PackageResolver for OneUnit {
    fn resolve(&self, package: Option<&str>, path: &str) -> Option<String> {
        (package.is_none() && path == "probe").then(|| self.0.lock().unwrap().clone())
    }

    fn list(&self, _package: Option<&str>, _dir: &str) -> Option<Vec<String>> {
        None
    }
}

fn boot_and_load(options: &VmOptions, source: &Arc<Mutex<String>>) -> bool {
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, options.clone());
        register_builtins(mc, &mut vm);
        vm
    });
    arena.mutate_root(|mc, vm| {
        vm.modules.resolver = Box::new(OneUnit(source.clone()));
        load_unit(vm, mc, None, "probe").expect("probe unit loads");
        vm.globals
            .borrow()
            .contains_key(&NamespacedName::new(Vec::new(), "CacheProbe".to_string()))
    })
}

#[test]
fn fresh_sessions_hit_and_edits_miss() {
    let source = Arc::new(Mutex::new("CacheProbe <- { answer -> { 41 } }".to_string()));
    let options = VmOptions::default();

    let base = unit_cache::hits();
    assert!(
        boot_and_load(&options, &source),
        "session 1 defines the class"
    );
    assert_eq!(unit_cache::hits(), base, "first load is a miss (fills)");

    assert!(
        boot_and_load(&options, &source),
        "a hit still executes the unit"
    );
    assert_eq!(
        unit_cache::hits(),
        base + 1,
        "second session reuses the compile"
    );

    // An edited source under the same identity chains to a new key: no stale hit.
    *source.lock().unwrap() = "CacheProbe <- { answer -> { 42 } }".to_string();
    assert!(boot_and_load(&options, &source));
    assert_eq!(unit_cache::hits(), base + 1, "edited unit recompiles");

    // And the edited version is itself cached.
    assert!(boot_and_load(&options, &source));
    assert_eq!(unit_cache::hits(), base + 2);
}
