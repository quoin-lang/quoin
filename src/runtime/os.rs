//! The `[OS]` namespace: process-level facilities that are neither I/O handles nor data types.
//!
//! `[OS]Path` — purely *lexical* path manipulation over Strings. It never touches the
//! filesystem: nothing here stats, resolves symlinks, or requires a path to exist, which is
//! what makes it safe to call on a path you are about to create. Filesystem access lives on
//! `[IO]File` / `[IO]Folder`.
//!
//! `[OS]Env` — **read-only** access to the process environment. Mutation is deliberately absent:
//! since edition 2024 `std::env::set_var` is `unsafe`, because the C environment is global state
//! that another thread may be reading concurrently — and this VM runs worker threads and a
//! blocking I/O pool. The usual reason to set a variable is to configure a child process, and
//! subprocess spawning is itself deferred (`QUOIN_TODO.md`), so the mutation half would buy a
//! soundness hazard for nothing.

use crate::arg;
use crate::error::QuoinError;
use crate::runtime::list::NativeListState;
use crate::value::{NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
use indexmap::IndexMap;
use std::path::{Component, Path, PathBuf};

/// Elements of a Quoin `List` argument, or a `TypeError` naming the caller.
fn list_elems<'gc>(value: Value<'gc>, who: &str) -> Result<Vec<Value<'gc>>, QuoinError> {
    value
        .with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
        .map_err(|_| QuoinError::TypeError {
            expected: "List".to_string(),
            got: value.type_name().to_string(),
            msg: format!("{who} expects a List of Strings"),
        })
}

/// A `Value` that must be a String, as a `String`.
fn as_str<'gc>(value: Value<'gc>, who: &str) -> Result<String, QuoinError> {
    match value {
        Value::Object(obj) => match &obj.borrow().payload {
            crate::value::ObjectPayload::String(s) => Ok((**s).clone()),
            _ => Err(QuoinError::TypeError {
                expected: "String".to_string(),
                got: value.type_name().to_string(),
                msg: format!("{who} expects Strings"),
            }),
        },
        _ => Err(QuoinError::TypeError {
            expected: "String".to_string(),
            got: value.type_name().to_string(),
            msg: format!("{who} expects Strings"),
        }),
    }
}

/// `PathBuf` -> Quoin String. Paths built from Quoin Strings are already UTF-8; `to_string_lossy`
/// is the total function that says so without an unwrap.
fn path_string<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, p: &Path) -> Value<'gc> {
    vm.new_string(mc, p.to_string_lossy().into_owned())
}

/// Join the segments of a `List` of Strings, left to right. An absolute segment resets the
/// result, matching `PathBuf::push` (and every shell): `join:#('/a' '/b')` is `/b`.
fn join_list<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    args: &[Value<'gc>],
    who: &str,
) -> Result<Value<'gc>, QuoinError> {
    let mut out = PathBuf::new();
    for seg in list_elems(args[0], who)? {
        out.push(as_str(seg, who)?);
    }
    Ok(path_string(vm, mc, &out))
}

/// Lexical normalization: collapse `.`, resolve `..` against the preceding segment, and squash
/// repeated separators. Purely textual — `..` past the root is dropped, and `..` at the start of
/// a *relative* path is kept, because there is nothing to resolve it against without touching the
/// filesystem. `normalize:''` and a path that cancels to nothing both give `.`.
fn normalize(path: &str) -> String {
    let mut out: Vec<Component> = Vec::new();
    for comp in Path::new(path).components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                // `a/..` cancels; `../..` and `/..` do not.
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                _ => out.push(comp),
            },
            other => out.push(other),
        }
    }
    if out.is_empty() {
        return ".".to_string();
    }
    out.iter()
        .collect::<PathBuf>()
        .to_string_lossy()
        .into_owned()
}

pub fn build_os_path_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[OS]Path", Some("Object"))
        .abstract_class()
        // `[OS]Path.join:'usr' join:'local'` — the repeated keyword folds into one List at
        // compile time (the `+` variadic form), so this is the same method as `join:`.
        .typed_class_method("join+:", &["List"], |vm, mc, _r, args| {
            join_list(vm, mc, &args, "[OS]Path.join:")
        })
        // `[OS]Path.join:#('usr' 'local')` — the explicit List form.
        .typed_class_method("join:", &["List"], |vm, mc, _r, args| {
            join_list(vm, mc, &args, "[OS]Path.join:")
        })
        .returns("String")
        // Everything up to the last separator. Total, never nil, following POSIX / Python /
        // Node: `.` for a bare name, and *idempotent at the root* (`dirname:'/'` is `/`), so the
        // walk-upward idiom `{ p != [OS]Path.dirname:p }.whileDo:{ … }` terminates.
        .typed_class_method("dirname:", &["String"], |vm, mc, _r, args| {
            let p = arg!(args, String, 0);
            Ok(match Path::new(&*p).parent() {
                Some(parent) if parent.as_os_str().is_empty() => vm.new_string(mc, ".".to_string()),
                Some(parent) => path_string(vm, mc, parent),
                // No parent: a root, or the empty path. Both are their own dirname.
                None => vm.new_string(mc, (*p).clone()),
            })
        })
        .returns("String")
        // The final component; `''` where there is none (`/`, `.`, `..`, `''`). A trailing
        // separator is ignored, so `basename:'a/'` is `a`.
        .typed_class_method("basename:", &["String"], |vm, mc, _r, args| {
            let p = arg!(args, String, 0);
            let name = Path::new(&*p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(vm.new_string(mc, name))
        })
        .returns("String")
        // The extension *without* its dot (`report.tar.gz` -> `gz`), or `''` when there is none.
        // A dotfile has no extension: `.bashrc` is a name, so `extension:'.bashrc'` is `''`.
        .typed_class_method("extension:", &["String"], |vm, mc, _r, args| {
            let p = arg!(args, String, 0);
            let ext = Path::new(&*p)
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(vm.new_string(mc, ext))
        })
        .returns("String")
        // The final component with its extension removed (`report.tar.gz` -> `report.tar`).
        .typed_class_method("stem:", &["String"], |vm, mc, _r, args| {
            let p = arg!(args, String, 0);
            let stem = Path::new(&*p)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(vm.new_string(mc, stem))
        })
        .returns("String")
        .typed_class_method("normalize:", &["String"], |vm, mc, _r, args| {
            let p = arg!(args, String, 0);
            Ok(vm.new_string(mc, normalize(&p)))
        })
        .returns("String")
        .typed_class_method("absolute?:", &["String"], |vm, mc, _r, args| {
            let p = arg!(args, String, 0);
            Ok(vm.new_bool(mc, Path::new(&*p).is_absolute()))
        })
        .returns("Boolean")
}

/// The environment as `(name, value)` pairs, sorted by name, skipping any entry whose name or
/// value is not valid UTF-8.
///
/// Sorted because `std::env::vars_os` yields an unspecified order, and a stdlib that returns a
/// different `keys` list run-to-run is a trap for tests and for anything that hashes its output.
/// Non-UTF-8 entries are skipped rather than lossily mangled: a Quoin String is UTF-8, and a
/// silently-corrupted name would be worse than an absent one.
fn env_pairs() -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
}

pub fn build_os_env_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[OS]Env", Some("Object"))
        .abstract_class()
        // The value of `name`, or `nil` when unset. An empty value is `''`, not `nil` — `FOO=`
        // is set. A name or value that is not valid UTF-8 reads as `nil`.
        .typed_class_method("at:", &["String"], |vm, mc, _r, args| {
            let name = arg!(args, String, 0);
            Ok(
                match std::env::var_os(&*name).and_then(|v| v.into_string().ok()) {
                    Some(value) => vm.new_string(mc, value),
                    None => vm.new_nil(mc),
                },
            )
        })
        .returns("String?")
        // Whether `name` is set at all, empty value included.
        .typed_class_method("contains?:", &["String"], |vm, mc, _r, args| {
            let name = arg!(args, String, 0);
            Ok(vm.new_bool(mc, std::env::var_os(&*name).is_some()))
        })
        .returns("Boolean")
        // Every variable name, sorted.
        .class_method("keys", |vm, mc, _r, _args| {
            let names = env_pairs()
                .into_iter()
                .map(|(k, _)| vm.new_string(mc, k))
                .collect::<Vec<_>>();
            Ok(vm.new_list(mc, names))
        })
        .returns("List")
        // The whole environment as a Map, sorted by name. This is also how you get the `Iterate`
        // combinators (`select:`, `collect:`, …) — a namespace class has no instances to mix into.
        .class_method("asMap", |vm, mc, _r, _args| {
            let mut map = IndexMap::new();
            for (k, v) in env_pairs() {
                let value = vm.new_string(mc, v);
                map.insert(k, value);
            }
            Ok(vm.new_map(mc, map))
        })
        .returns("Map")
}

#[cfg(test)]
#[path = "os_tests.rs"]
mod os_tests;
