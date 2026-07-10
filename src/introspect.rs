//! Read-only VM introspection: surface metadata about a running [`VmState`] as plain owned
//! structs with no `'gc` lifetime, so a caller can pull them out of an `arena.mutate_root`
//! borrow. This module owns the VM-internal walking (the [`Class`] layout, the multimethod
//! method chain, `globals`, `repl_env`) so consumers — the REPL's `$`-commands, tab
//! completion, and a future Quoin `Mirror` reflection API — stay ignorant of internals.
//!
//! Pure read: no Quoin code runs, no `Mutation` is needed. Anything heavier (a value's `.s`
//! repr, a method's source body, the real `Value`) is the caller's job; this hands back only
//! names, signatures, and flags. Design notes: `docs/INTROSPECTION.md`.

use crate::value::{Class, Value};
use crate::vm::VmState;

use gc_arena::{Gc, lock::RefLock};
use std::collections::BTreeSet;

/// A defined global: a class or a constant value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalInfo {
    pub name: String,
    pub kind: GlobalKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalKind {
    Class,
    Value { class: String },
}

/// Surface metadata for a class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassInfo {
    pub name: String,
    pub parent: Option<String>,
    pub mixins: Vec<String>,
    pub instance_vars: Vec<String>,
    /// *Own* instance methods (not inherited).
    pub instance_methods: Vec<MethodInfo>,
    /// Class-side / metaclass methods.
    pub class_methods: Vec<MethodInfo>,
    pub is_sealed: bool,
    pub is_abstract: bool,
    /// Where the class was defined (`VmState::class_meta`, recorded by `DefineClass`).
    /// `None` for native classes and `-e`/REPL definitions.
    pub source: Option<SourceLoc>,
    /// A native class's `.class_doc(..)` text. Quoin classes answer `None` here — their doc
    /// is the `"*` block above `source`, extracted lazily (docs/DOCS_ARCH.md §4/§6).
    pub doc: Option<String>,
    /// Every statically-named reopen site (`Name <-- { … }`), in load order. The doc block
    /// above a reopen documents the *extension*; for a native class this is where its qnlib
    /// class doc lives.
    pub extension_sources: Vec<SourceLoc>,
}

/// A method = its selector plus the chain of typed/guarded overloads (multimethod).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodInfo {
    pub selector: String,
    pub variants: Vec<MethodVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodVariant {
    /// Declared parameter types; `None` is an untyped param (the VM stores untyped as
    /// `"Object"`; normalized here).
    pub param_types: Vec<Option<String>>,
    /// Declared return type (Fork-1b native half), or `None`; native methods via `.returns(..)`.
    pub ret_type: Option<String>,
    pub guarded: bool,
    pub native: bool,
    pub source: Option<SourceLoc>,
    /// A native variant's `.doc(..)` text. Quoin variants answer `None` — their doc is the
    /// `"*` block above `source`, extracted lazily (docs/DOCS_ARCH.md §4/§6).
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLoc {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

/// A persistent REPL binding: its name and the class of its current value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingInfo {
    pub name: String,
    pub class: String,
}

/// An object value: its class and its instance fields (name → field value's class).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueInfo {
    pub class: String,
    pub fields: Vec<(String, String)>,
}

/// Names beginning with `$` are VM-internal singletons (e.g. `$TrueClass`) — hidden from
/// the enumeration / completion helpers, but still reachable by exact `describe_class`.
fn is_surface(name: &str) -> bool {
    !name.starts_with('$')
}

/// Every (surface) defined global, sorted by name.
pub fn globals<'gc>(vm: &VmState<'gc>) -> Vec<GlobalInfo> {
    let mut out: Vec<GlobalInfo> = vm
        .globals
        .borrow()
        .iter()
        .filter(|(key, _)| is_surface(&key.name))
        .map(|(key, val)| GlobalInfo {
            name: key.to_string(),
            kind: match val {
                Value::Class(_) => GlobalKind::Class,
                _ => GlobalKind::Value {
                    class: val.class_name(),
                },
            },
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Surface global names beginning with `prefix`, sorted (bare-word completion).
pub fn find_globals<'gc>(vm: &VmState<'gc>, prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = vm
        .globals
        .borrow()
        .iter()
        .filter(|(key, _)| is_surface(&key.name))
        .map(|(key, _)| key.to_string())
        .filter(|name| name.starts_with(prefix))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Distinct namespaces (`[IO]…` → `"IO"`) beginning with `prefix`, sorted — for completion
/// inside `[ … ]`.
pub fn find_namespaces<'gc>(vm: &VmState<'gc>, prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = vm
        .globals
        .borrow()
        .keys()
        .filter(|key| is_surface(&key.name) && !key.path.is_empty())
        .map(|key| key.path.join("/"))
        .filter(|ns| ns.starts_with(prefix))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Resolve a class by its exact (rendered) global name. Exact lookup honors `$`-internals.
/// The class named `name`, as a Gc handle — the REPL's `$doc` resolves a name once and then
/// asks per-side. Public counterpart of the private lookup below.
pub fn find_class_gc<'gc>(vm: &VmState<'gc>, name: &str) -> Option<Gc<'gc, RefLock<Class<'gc>>>> {
    find_class(vm, name)
}

fn find_class<'gc>(vm: &VmState<'gc>, name: &str) -> Option<Gc<'gc, RefLock<Class<'gc>>>> {
    vm.globals.borrow().iter().find_map(|(key, val)| match val {
        Value::Class(c) if key.to_string() == name => Some(*c),
        _ => None,
    })
}

/// Surface metadata for the class named `name`, or `None` if no such class global.
pub fn describe_class<'gc>(vm: &VmState<'gc>, name: &str) -> Option<ClassInfo> {
    let class_gc = find_class(vm, name)?;
    let class = class_gc.borrow();
    let meta = vm.class_meta.get(&class.name);
    Some(ClassInfo {
        name: class.name.to_string(),
        parent: class.parent.map(|p| p.borrow().name.to_string()),
        mixins: class
            .mixin_classes
            .iter()
            .map(|m| m.borrow().name.to_string())
            .collect(),
        instance_vars: class.instance_vars.clone(),
        instance_methods: methods_of(vm, &class.instance_methods),
        class_methods: methods_of(vm, &class.class_methods),
        is_sealed: class.is_sealed,
        is_abstract: class.is_abstract,
        source: meta.and_then(|m| m.source.as_ref()).map(|si| SourceLoc {
            file: si.filename.clone(),
            line: si.line,
            column: si.column,
        }),
        doc: meta.and_then(|m| m.doc.clone()),
        extension_sources: meta
            .map(|m| {
                m.extensions
                    .iter()
                    .map(|si| SourceLoc {
                        file: si.filename.clone(),
                        line: si.line,
                        column: si.column,
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// The reference doc for the class named `name`: a native class's `.class_doc(..)` text,
/// else the `"*` block above its definition, else the block above its first documented reopen
/// (docs/DOCS_ARCH.md §6). Lazy — source is read (embedded stdlib or disk) only when asked.
pub fn doc_of_class<'gc>(vm: &VmState<'gc>, name: &str) -> Option<String> {
    let key = crate::value::NamespacedName::parse(name);
    let meta = vm.class_meta.get(&key)?;
    if let Some(doc) = &meta.doc {
        return Some(doc.clone());
    }
    let lift = |si: &crate::value::SourceInfo| {
        crate::docs::unit_source(&si.filename).and_then(|t| crate::docs::doc_above(&t, si.line))
    };
    meta.source
        .as_ref()
        .and_then(&lift)
        .or_else(|| meta.extensions.iter().find_map(&lift))
}

/// The reference doc for `selector` on `class` — instance side, or the class side with
/// `class_side` (the `.meta.docFor:` path). Walks the hierarchy like dispatch does, then the
/// multimethod chain: a native variant's `.doc(..)` text, else the `"*` block above the first
/// located Quoin variant. Lazy, like [`doc_of_class`].
pub fn doc_of_method<'gc>(
    vm: &VmState<'gc>,
    class: gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::Class<'gc>>>,
    selector: &str,
    class_side: bool,
) -> Option<String> {
    let head = vm.lookup_in_class_hierarchy(class, selector, class_side)?;
    let mut curr = Some(head);
    while let Some(method_val) = curr {
        if let Some(doc) = vm.candidate_doc(method_val) {
            return Some(doc);
        }
        if let Some(si) = vm
            .get_block_from_method(method_val)
            .and_then(|b| b.template.source_info.clone())
            && let Some(text) = crate::docs::unit_source(&si.filename)
            && let Some(doc) = crate::docs::method_doc_above(&text, si.line, selector)
        {
            return Some(doc);
        }
        curr = vm.get_next_method_in_chain(method_val);
    }
    None
}

/// Selectors defined on `class` (and, with `include_inherited`, on its parent + mixins)
/// beginning with `prefix`, sorted — for `.`-completion.
pub fn find_selectors<'gc>(
    vm: &VmState<'gc>,
    class: &str,
    prefix: &str,
    include_inherited: bool,
) -> Vec<String> {
    let Some(class_gc) = find_class(vm, class) else {
        return Vec::new();
    };
    let mut set = BTreeSet::new();
    collect_selectors(class_gc, include_inherited, &mut set);
    set.into_iter().filter(|s| s.starts_with(prefix)).collect()
}

fn collect_selectors<'gc>(
    class_gc: Gc<'gc, RefLock<Class<'gc>>>,
    include_inherited: bool,
    out: &mut BTreeSet<String>,
) {
    let class = class_gc.borrow();
    out.extend(
        class
            .instance_methods
            .keys()
            .map(|s| s.as_str().to_string()),
    );
    if include_inherited {
        for mixin in &class.mixin_classes {
            collect_selectors(*mixin, true, out);
        }
        if let Some(parent) = class.parent {
            collect_selectors(parent, true, out);
        }
    }
}

/// The persistent REPL bindings (name + the class of each value). Empty outside the REPL.
/// The implicit top-level `self` binding is omitted.
pub fn session_locals<'gc>(vm: &VmState<'gc>) -> Vec<BindingInfo> {
    let Some(env) = vm.repl_env else {
        return Vec::new();
    };
    env.borrow()
        .vars
        .iter()
        .filter(|(name, _)| name.as_str() != "self")
        .map(|(name, val)| BindingInfo {
            name: name.as_str().to_string(),
            class: val.class_name(),
        })
        .collect()
}

/// A value's class, and (for an object) its instance fields as `(name, field's class)`.
pub fn describe_value<'gc>(_vm: &VmState<'gc>, value: Value<'gc>) -> ValueInfo {
    let class = value.class_name();
    let mut fields = Vec::new();
    if let Value::Object(obj) = value {
        let obj_ref = obj.borrow();
        let class_ref = obj_ref.class.borrow();
        // `field_slots` maps name -> slot; present fields in slot (declaration) order.
        let mut slots: Vec<(&String, usize)> = class_ref
            .field_slots
            .iter()
            .map(|(name, &slot)| (name, slot))
            .collect();
        slots.sort_by_key(|(_, slot)| *slot);
        for (name, slot) in slots {
            if let Some(field) = obj_ref.fields.get(slot) {
                fields.push((name.clone(), field.class_name()));
            }
        }
    }
    ValueInfo { class, fields }
}

/// Read each method-map entry (a multimethod chain head) into a [`MethodInfo`]; sorted by
/// selector.
fn methods_of<'gc>(
    vm: &VmState<'gc>,
    map: &rustc_hash::FxHashMap<crate::symbol::Symbol, Value<'gc>>,
) -> Vec<MethodInfo> {
    let mut out: Vec<MethodInfo> = map
        .iter()
        .map(|(selector, head)| method_info(vm, selector.as_str(), *head))
        .collect();
    out.sort_by(|a, b| a.selector.cmp(&b.selector));
    out
}

/// Walk a selector's multimethod chain into its variants.
fn method_info<'gc>(vm: &VmState<'gc>, selector: &str, head: Value<'gc>) -> MethodInfo {
    let mut variants = Vec::new();
    let mut curr = Some(head);
    while let Some(method_val) = curr {
        let block = vm.get_block_from_method(method_val);
        let param_types = vm
            .candidate_param_types(method_val)
            .into_iter()
            .map(|t| if t == "Object" { None } else { Some(t) })
            .collect();
        let guarded = block.map(|b| b.decl_block.is_some()).unwrap_or(false);
        let source = block
            .and_then(|b| b.template.source_info.clone())
            .map(|si| SourceLoc {
                file: si.filename,
                line: si.line,
                column: si.column,
            });
        variants.push(MethodVariant {
            param_types,
            ret_type: vm.candidate_ret_type(method_val),
            guarded,
            native: block.is_none(),
            source,
            doc: vm.candidate_doc(method_val),
        });
        curr = vm.get_next_method_in_chain(method_val);
    }
    MethodInfo {
        selector: selector.to_string(),
        variants,
    }
}

/// Render a variant's signature: the selector with its declared param types interleaved
/// (`at:Integer put:`, `fetch:String`, `sound`), then ` {…}` for a guarded variant and
/// ` (native)` for a Rust-backed one. Plain text — a caller may colorize separately. The
/// canonical signature rendering, shared by the REPL, completion hints, and a future Mirror.
pub fn signature(selector: &str, variant: &MethodVariant) -> String {
    // Split the selector into keyword parts (each keeps its trailing `:`) or one unary part.
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in selector.chars() {
        cur.push(c);
        if c == ':' {
            parts.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }

    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(part);
        // A keyword part takes one argument; append its type if declared (untyped = nothing).
        if part.ends_with(':')
            && let Some(Some(ty)) = variant.param_types.get(i)
        {
            out.push_str(ty);
        }
    }
    if variant.guarded {
        out.push_str(" {…}");
    }
    if variant.native {
        out.push_str(" (native)");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcl;
    use crate::parser::NodeValue;
    use crate::symbol::Symbol;
    use crate::value::EnvFrame;
    use crate::vm::VmOptions;
    use gc_arena::{Arena, Mutation, Rootable};

    /// Build a VM (native builtins, no qnlib), run `src`, then call `f` to introspect it.
    fn check<F>(src: &str, f: F)
    where
        F: for<'gc> FnOnce(&Mutation<'gc>, &mut VmState<'gc>),
    {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());
            crate::runner::register_builtins(mc, &mut vm);
            vm
        });
        arena.mutate_root(|mc, vm| {
            let node = crate::parser::parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("not a program");
            };
            let sb = crate::compiler::Compiler::new().compile_program(p).unwrap();
            let block = crate::runtime::runtime::build_block(mc, &sb);
            vm.execute_block(mc, block, Vec::new(), None).unwrap();
            f(mc, vm);
        });
    }

    const SRC: &str = "Animal <- { |@legs| sound -> { 'generic' } legs -> { @legs } }; \
         Animal <- Dog <- { sound -> { 'woof' } fetch: -> { |item:String| item } }; \
         Spot = Animal.new;";

    /// Anti-drift: the checker's `ClassTable::responds_to` must never say "does not respond" for a
    /// selector the VM's own dispatch actually resolves. Cross-checks the compile-time table
    /// (built via `populate_from_vm`) against the runtime dispatch view over a real hierarchy.
    #[test]
    fn class_table_never_false_when_dispatch_resolves() {
        use crate::class_table::{ClassTable, populate_from_vm};
        check(SRC, |_mc, vm| {
            let table = ClassTable::new();
            populate_from_vm(vm, &table);
            let classes = ["Animal", "Dog", "Object", "String", "Integer"];
            let selectors = ["sound", "legs", "fetch:", "s", "class", "nope", "zzz:"];
            for class in classes {
                let resolves: std::collections::HashSet<String> =
                    find_selectors(vm, class, "", true).into_iter().collect();
                for sel in selectors {
                    if table.responds_to(class, sel) == Some(false) {
                        assert!(
                            !resolves.contains(sel),
                            "{class}.{sel}: checker says no, but dispatch resolves it"
                        );
                    }
                }
            }
        });
    }

    /// Arg-type checking: a call to a single fully-typed method on a sealed, VM-resident class is
    /// checked against the declared param types. Mirrors the real prior-unit → later-unit flow.
    #[test]
    fn arg_check_against_a_sealed_vm_class() {
        use crate::class_table::{ClassTable, populate_from_vm};
        check(
            "Widget <- { take: -> { |n: Integer ^Integer| ^n }; .sealed! }; Widget.new;",
            |_mc, vm| {
                let table = ClassTable::new();
                populate_from_vm(vm, &table);
                assert_eq!(
                    table.own_method_params("Widget", "take:"),
                    Some(vec![crate::types::Type::Int]),
                    "the sealed method's Integer param should be captured"
                );

                // Compile a "later unit" against that table; keep only type-mismatch diagnostics.
                let mismatches = |src: &str| -> Vec<String> {
                    let node = crate::parser::parse_quoin_string(src);
                    let NodeValue::Program(p) = &node.value else {
                        panic!("not a program");
                    };
                    let mut c = crate::compiler::Compiler::new();
                    c.set_class_table(table.clone());
                    c.compile_program(p).unwrap();
                    c.diagnostics()
                        .iter()
                        .filter(|d| d.message.contains("type mismatch"))
                        .map(|d| d.message.clone())
                        .collect()
                };

                let d = mismatches("var w: Widget = Widget.new; w.take: 'oops'");
                assert!(
                    d.iter()
                        .any(|m| m.contains("expected `Integer`, found `String`")),
                    "{d:?}"
                );
                assert!(mismatches("var w: Widget = Widget.new; w.take: 5").is_empty());
            },
        );
    }

    #[test]
    fn describe_class_reads_parent_methods_and_ivars() {
        check(SRC, |_mc, vm| {
            let dog = describe_class(vm, "Dog").expect("Dog");
            assert_eq!(dog.name, "Dog");
            assert_eq!(dog.parent.as_deref(), Some("Animal"));
            let sels: Vec<&str> = dog
                .instance_methods
                .iter()
                .map(|m| m.selector.as_str())
                .collect();
            assert!(
                sels.contains(&"sound") && sels.contains(&"fetch:"),
                "{sels:?}"
            );

            let fetch = dog
                .instance_methods
                .iter()
                .find(|m| m.selector == "fetch:")
                .unwrap();
            assert_eq!(fetch.variants.len(), 1);
            assert_eq!(
                fetch.variants[0].param_types,
                vec![Some("String".to_string())]
            );
            assert!(!fetch.variants[0].native && !fetch.variants[0].guarded);

            assert_eq!(
                describe_class(vm, "Animal").unwrap().instance_vars,
                vec!["legs"]
            );
            assert!(describe_class(vm, "Nope").is_none());
        });
    }

    #[test]
    fn find_prefix_scans() {
        check(SRC, |_mc, vm| {
            assert!(find_globals(vm, "Do").contains(&"Dog".to_string()));
            assert!(find_globals(vm, "An").contains(&"Animal".to_string()));
            assert_eq!(
                find_selectors(vm, "Dog", "fe", false),
                vec!["fetch:".to_string()]
            );
            // `legs` is inherited from Animal — only with include_inherited.
            assert!(find_selectors(vm, "Dog", "le", false).is_empty());
            assert!(find_selectors(vm, "Dog", "le", true).contains(&"legs".to_string()));
        });
    }

    #[test]
    fn describe_value_lists_fields() {
        check(SRC, |_mc, vm| {
            let spot = {
                let g = vm.globals.borrow();
                g.iter()
                    .find(|(k, _)| k.to_string() == "Spot")
                    .map(|(_, v)| *v)
                    .expect("Spot global")
            };
            let info = describe_value(vm, spot);
            assert_eq!(info.class, "Animal");
            assert_eq!(info.fields, vec![("legs".to_string(), "Nil".to_string())]);
        });
    }

    #[test]
    fn session_locals_reads_repl_env() {
        check("1", |mc, vm| {
            let env = gcl!(mc, EnvFrame::new(None));
            let answer = vm.new_int(mc, 42);
            env.borrow_mut(mc).bind(Symbol::intern("answer"), answer);
            vm.repl_env = Some(env);
            let locals = session_locals(vm);
            assert!(
                locals
                    .iter()
                    .any(|b| b.name == "answer" && b.class == "Integer"),
                "{locals:?}"
            );
        });
    }

    #[test]
    fn signature_formatting() {
        let typed = |t: &str| MethodVariant {
            param_types: vec![Some(t.to_string())],
            ret_type: None,
            guarded: false,
            native: false,
            source: None,
            doc: None,
        };
        assert_eq!(
            signature(
                "sound",
                &MethodVariant {
                    param_types: vec![],
                    ret_type: None,
                    guarded: false,
                    native: false,
                    source: None,
                    doc: None,
                }
            ),
            "sound"
        );
        assert_eq!(signature("fetch:", &typed("String")), "fetch:String");
        assert_eq!(
            signature(
                "at:put:",
                &MethodVariant {
                    param_types: vec![Some("Integer".into()), None],
                    ret_type: None,
                    guarded: false,
                    native: false,
                    source: None,
                    doc: None,
                }
            ),
            "at:Integer put:"
        );
        assert_eq!(
            signature(
                "g:",
                &MethodVariant {
                    param_types: vec![None],
                    ret_type: None,
                    guarded: true,
                    native: false,
                    source: None,
                    doc: None,
                }
            ),
            "g: {…}"
        );
    }
}
