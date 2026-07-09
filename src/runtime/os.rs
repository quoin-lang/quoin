//! The `[OS]` namespace: process-level facilities that are neither I/O handles nor data types.
//!
//! `[OS]Path` — purely *lexical* path manipulation over Strings. It never touches the
//! filesystem: nothing here stats, resolves symlinks, or requires a path to exist, which is
//! what makes it safe to call on a path you are about to create. Filesystem access lives on
//! `[IO]File` / `[IO]Folder`.

use crate::arg;
use crate::error::QuoinError;
use crate::runtime::list::NativeListState;
use crate::value::{NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
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

#[cfg(test)]
#[path = "os_tests.rs"]
mod os_tests;
