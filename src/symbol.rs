//! Interned method selectors.
//!
//! A [`Symbol`] is a `Copy` handle to a string that has been deduplicated into a
//! process-global, leak-forever interner, so two equal selectors share one
//! canonical `&'static str`. Equality and hashing go through the *pointer* rather
//! than the bytes — cheap and integer-like — which makes `Symbol` an ideal `Copy`
//! key for the method-dispatch cache, and makes [`Symbol::as_str`] free and
//! lock-free. Only interning itself (every selector at compile time, plus the rare
//! runtime selector) takes a brief lock.
//!
//! Selectors are bounded and live for the whole program, so leaking the interned
//! strings to obtain `&'static str` is the right tradeoff (the standard idiom for
//! a string interner). All `Symbol`s are constructed through [`Symbol::intern`],
//! so the pointer is always canonical and pointer-equality is sound.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use gc_arena::Collect;

/// An interned selector. Cheap to copy, compare, and hash (all by pointer).
#[derive(Copy, Clone)]
pub struct Symbol(&'static str);

// Symbol holds only a `&'static str` (no GC pointers), so it never needs tracing.
unsafe impl<'gc> Collect<'gc> for Symbol {
    const NEEDS_TRACE: bool = false;
}

fn interner() -> &'static Mutex<HashMap<&'static str, Symbol>> {
    static INTERNER: OnceLock<Mutex<HashMap<&'static str, Symbol>>> = OnceLock::new();
    INTERNER.get_or_init(|| Mutex::new(HashMap::new()))
}

impl Symbol {
    /// Intern `s`, returning the canonical [`Symbol`] for it. Equal strings always
    /// yield a `Symbol` with the same backing pointer.
    pub fn intern(s: &str) -> Symbol {
        let mut map = interner().lock().unwrap();
        if let Some(&sym) = map.get(s) {
            return sym;
        }
        // First sighting: leak a canonical copy so the pointer is stable forever.
        let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
        let sym = Symbol(leaked);
        map.insert(leaked, sym);
        sym
    }

    /// The selector's text. Free — just returns the interned `&'static str`.
    #[inline]
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

/// The interned `self` local — bound in every method frame and read on every
/// `self`/`@ivar` access, so it's worth caching past the interner lock.
pub fn self_symbol() -> Symbol {
    static SELF: OnceLock<Symbol> = OnceLock::new();
    *SELF.get_or_init(|| Symbol::intern("self"))
}

/// The interned `init` / `init:` selectors — probed in every class of the
/// hierarchy on every instantiation, so cached past the interner lock.
pub fn init_symbol() -> Symbol {
    static INIT: OnceLock<Symbol> = OnceLock::new();
    *INIT.get_or_init(|| Symbol::intern("init"))
}

pub fn init_colon_symbol() -> Symbol {
    static INIT_COLON: OnceLock<Symbol> = OnceLock::new();
    *INIT_COLON.get_or_init(|| Symbol::intern("init:"))
}

impl PartialEq for Symbol {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        // Interning guarantees one canonical pointer per distinct string, so
        // pointer identity is exactly string equality.
        std::ptr::eq(self.0.as_ptr(), other.0.as_ptr())
    }
}
impl Eq for Symbol {}

impl Hash for Symbol {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.0.as_ptr() as usize).hash(state);
    }
}

impl std::fmt::Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Symbol({:?})", self.0)
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
