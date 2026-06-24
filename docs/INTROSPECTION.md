# VM introspection — design

A read-only API for inspecting a running Quoin VM, returning **plain owned Rust structs** with
no `'gc` lifetime. It lives in `src/introspect.rs` and owns all the VM-internal walking
(`Class` layout, the multimethod method chain, `globals`, `repl_env`) so consumers — the REPL's
`$`-commands, tab completion, and a future Quoin reflection API — stay ignorant of internals.

## Principle

Introspection is **surface metadata, pure read.** No Quoin code runs, no `Mutation` is needed
(`&VmState` is enough). The returned structs are `'static` (owned `String`/`Vec`), so a caller
pulls them straight out of the arena borrow:

```rust
let info = arena.mutate_root(|_mc, vm| introspect::describe_class(vm, "Foo"));
// `info: Option<ClassInfo>` — owned, usable outside mutate_root, no Gc, no internals.
```

Anything heavier — a value's `.s` repr, a method's source body, the real `Value` — the caller
fetches itself (the REPL already has `render_value`); the API only hands back names, signatures,
and flags.

## Structs (`#[derive(Debug, Clone)]`, all `'static`)

```rust
struct GlobalInfo  { name: String, kind: GlobalKind }
enum   GlobalKind  { Class, Value { class: String } }       // a class vs a constant of some class

struct ClassInfo {
    name: String,
    parent: Option<String>,
    mixins: Vec<String>,
    instance_vars: Vec<String>,
    instance_methods: Vec<MethodInfo>,   // *own* methods (not inherited)
    class_methods: Vec<MethodInfo>,      // class-side / metaclass methods
    is_sealed: bool,
    is_abstract: bool,
}

struct MethodInfo    { selector: String, variants: Vec<MethodVariant> }   // a multimethod
struct MethodVariant { param_types: Vec<Option<String>>, guarded: bool, native: bool, source: Option<SourceLoc> }
struct SourceLoc     { file: String, line: usize, column: usize }

struct BindingInfo   { name: String, class: String }        // a repl_env local: name + value's class
struct ValueInfo     { class: String, fields: Vec<(String, String)> }     // object: class + (field, field's class)
```

`MethodInfo` carries **variants** because Quoin dispatch is multimethod: a selector on a class is
a chain of typed/guarded overloads. `param_types` is `Vec<Option<String>>` — `None` is an untyped
param (the VM stores untyped as `"Object"`; the API normalizes that to `None`). `guarded` marks a
variant with a `{…}` guard; `native` marks a Rust-backed method; `source` is the Quoin definition
site (absent for native).

## Function surface

**Exact lookup / full enumeration:**

```rust
fn globals(vm) -> Vec<GlobalInfo>                 // every global ($globals)
fn describe_class(vm, name) -> Option<ClassInfo>  // one class, by exact name
fn describe_value(vm, value: Value) -> ValueInfo  // one value (called within mutate_root)
fn session_locals(vm) -> Vec<BindingInfo>         // the persistent repl_env bindings
```

**Prefix finds (completion-oriented; names only, lightweight):**

```rust
fn find_globals(vm, prefix) -> Vec<String>                       // bare-word completion
fn find_namespaces(vm, prefix) -> Vec<String>                    // inside `[ … ]`
fn find_selectors(vm, class, prefix, include_inherited) -> Vec<String>   // after `.`
```

The `find_` prefix is the convention for "prefix scan, may match many" — distinct from the exact
`describe_*` / full `globals` / `session_locals`. The completion driver picks one by lexical
context (`.` → `find_selectors`, an unclosed `[` → `find_namespaces`, else `find_globals` + the
session locals + keywords) and never reaches into the VM.

## Data sources

| field / fn | from |
|---|---|
| `globals` / `find_globals` | `vm.globals` keys (`NamespacedName`, rendered via `Display`); `GlobalKind` = `Value::Class` ? Class : `Value{class}` |
| `find_namespaces` | the `path` component of each `NamespacedName` key, joined (`[IO]…` → `"IO"`), de-duped, prefix-filtered |
| `describe_class` | the `Class` behind a `Value::Class` global — `name`/`parent`/`mixin_classes`/`instance_vars`/`is_sealed`/`is_abstract`, and the two method maps |
| `MethodInfo` / variants | each `instance_methods`/`class_methods` value is the head of a chain; walk `get_next_method_in_chain`, read `get_block_from_method` → `block.param_types`/`decl_block`/`source_info`, else `native_method_param_types` |
| `find_selectors` | the class's `instance_methods` keys (+ parent/mixin keys when `include_inherited`), prefix-filtered |
| `session_locals` | `vm.repl_env` → `EnvFrame.vars` (`Symbol` name + each value's `class_name()`) |
| `describe_value` | a `Value::Object`'s `class` (→ name) and `fields` mapped through `Class::field_slots` (slot → ivar name), each field's `class_name()` |

Eigenclasses (`Class::is_eigenclass`) are excluded from class listings — they're transient per-
instance singletons, not user-facing types.

## Future: a Quoin `Mirror` API

The plan (not v1) is a native reflection class in Quoin — Bracha-style **mirrors**, so reflection
lives in separate mirror objects rather than methods bolted onto every class. The Rust structs are
the bridge: a `Mirror.of: Foo` (or `[Reflect]Class.named:`) native method calls `describe_class`
and converts the `ClassInfo` 1:1 into a Quoin `ClassMirror` object (`.name`, `.parent`,
`.methods`, …). Keeping the structs clean and Quoin-mappable is the only thing v1 owes the future.

## Scope

**v1:** globals, namespaces, classes, methods (with variants), repl session locals, value/object
inspection — i.e. everything the `$`-commands and tab completion need.

**Out of v1:** live call-stack / frame introspection (a debugger concern — not useful until you can
break mid-stack); method *bodies* / decompilation; the Quoin `Mirror` wrapper (a later layer over
this one).
