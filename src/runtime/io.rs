use crate::error::QuoinError;
use crate::io_backend::{IoError, IoRequest, IoResult};
use crate::value::{NativeClassBuilder, ObjectPayload, OpaqueState, Value};
use crate::vm::{StdStream, VmState};
use crate::{ansi_colorizer, arg};

use gc_arena::Mutation;
use std::ffi::OsString;
use std::fs::{File, Metadata, ReadDir, metadata, read_dir};
use std::io::{Stderr, Stdin, Stdout, stderr, stdin, stdout};
use std::path::PathBuf;

pub struct NativeIoFolder {
    pub path: OsString,
    pub iter: Option<ReadDir>,
}

/// A filesystem failure as a catchable Quoin `IoError` (kind + message), never a
/// panic — a missing or unreadable directory is ordinary program input.
fn fs_err(e: std::io::Error) -> QuoinError {
    let ioe: IoError = e.into();
    QuoinError::from_io_error(&ioe)
}

pub fn build_io_folder_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[IO]Folder", Some("Object"))
        .construct_with("use [IO]Folder.open:")
        .class_doc(
            "A directory on the local filesystem, listed entry by entry.\n\n\
             `open:` starts the listing; each `next` answers one entry as an `[IO]File` (nil \
             when exhausted), and `reset` starts over. The class side manages directories \
             themselves: `create:` (like `mkdir -p`) and `delete:` (empty directories \
             only).\n\n\
             ```\n\
             var d = [IO]Folder.open:'/tmp'\n\
             var entry = d.next     \"* an [IO]File, or nil once exhausted\n\
             ```",
        )
        .class_method("open:", |vm, mc, _receiver, args| {
            let path = arg!(args, String, 0);
            new_native_io_folder(vm, mc, path)
        })
        .doc(
            "Open a directory for listing. A missing or unreadable directory throws a \
             catchable IoError.",
        )
        // create:path -> create the directory and any missing parents (idempotent, like
        // `mkdir -p`: an existing directory is not an error).
        .class_method("create:", |vm, mc, _receiver, args| {
            let path = arg!(args, String, 0);
            std::fs::create_dir_all(path.as_str())
                .map_err(|e| QuoinError::from_io_error(&e.into()))?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Create the directory and any missing parents — idempotent, like `mkdir -p`: an \
             existing directory is not an error. Returns nil.",
        )
        // delete:path -> remove an *empty* directory. Refusing to recurse is deliberate:
        // a one-selector recursive delete is too easy to call by accident.
        .class_method("delete:", |vm, mc, _receiver, args| {
            let path = arg!(args, String, 0);
            std::fs::remove_dir(path.as_str()).map_err(|e| QuoinError::from_io_error(&e.into()))?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Remove an *empty* directory (a non-empty one throws an IoError). Refusing to \
             recurse is deliberate: a one-selector recursive delete is too easy to call by \
             accident. Returns nil.",
        )
        .instance_method("path", |vm, mc, receiver, _args| {
            receiver
                .with_native_state(|io: &NativeIoFolder| {
                    vm.new_string(mc, io.path.to_string_lossy().into_owned())
                })
                .map_err(|e| QuoinError::Other(e.to_string()))
        })
        .doc("The directory's path, as the String it was opened with.")
        .instance_method("next", |vm, mc, receiver, _args| {
            // Lazily (re)open after `reset`, outside the state borrow, so a vanished
            // directory is a catchable IoError rather than a panic.
            let needs_iter = receiver.with_native_state(|io: &NativeIoFolder| io.iter.is_none())?;
            if needs_iter {
                let path = receiver.with_native_state(|io: &NativeIoFolder| io.path.clone())?;
                let iter = read_dir(&path).map_err(fs_err)?;
                receiver.with_native_state_mut(mc, |io: &mut NativeIoFolder| {
                    io.iter = Some(iter);
                })?;
            }
            let r = receiver.with_native_state_mut(mc, |io: &mut NativeIoFolder| {
                io.iter
                    .as_mut()
                    .expect("iter installed above")
                    .next()
                    .map(|r| r.map_err(fs_err))
            })?;

            Ok(if let Some(entry) = r {
                let ent = entry?;
                let os_string = ent.path().into_os_string();
                new_native_io_file(vm, mc, os_string, ent.metadata().map_err(fs_err)?)
            } else {
                vm.new_nil(mc)
            })
        })
        .doc(
            "The next directory entry as an `[IO]File` (its full path plus metadata), or nil \
             once the listing is exhausted. Order is whatever the OS yields — not sorted; \
             `.` and `..` are not included.",
        )
        .instance_method("reset", |vm, mc, receiver, _args| {
            receiver.with_native_state_mut(mc, |io: &mut NativeIoFolder| {
                io.iter = None;
            })?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Restart the listing from the beginning: the next `next` re-reads the directory \
             (picking up any changes since). Returns nil.",
        )
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_path = receiver.with_native_state(|io: &NativeIoFolder| io.path.clone())?;
            let rhs_path = args[0].with_native_state(|io: &NativeIoFolder| io.path.clone());
            match rhs_path {
                Ok(rhs_path) => Ok(vm.new_bool(mc, lhs_path == rhs_path)),
                Err(_) => Ok(vm.new_bool(mc, false)),
            }
        })
        .doc(
            "Whether the argument is an [IO]Folder with the same path string (no \
             canonicalization — `/tmp` and `/tmp/` differ). A non-Folder argument is simply \
             unequal.",
        )
}

fn new_native_io_folder<'a>(
    vm: &mut VmState<'a>,
    mc: &Mutation<'a>,
    path: crate::value::Str<'a>,
) -> Result<Value<'a>, QuoinError> {
    let state = OpaqueState(NativeIoFolder {
        path: OsString::from(path.as_str()),
        iter: Some(read_dir(path.as_str()).map_err(fs_err)?),
    });

    Ok(vm.new_native_state(mc, vm.get_builtin_class("[IO]Folder"), state))
}

pub struct NativeIoFile {
    pub path: OsString,
    pub metadata: Metadata,
    pub file: Option<File>,
}

fn new_native_io_file<'a>(
    vm: &mut VmState<'a>,
    mc: &Mutation<'a>,
    path: OsString,
    metadata: Metadata,
) -> Value<'a> {
    let state = OpaqueState(NativeIoFile {
        path,
        metadata,
        file: None,
    });

    vm.new_native_state(mc, vm.get_builtin_class("[IO]File"), state)
}

/// Open `args[0]` for writing and hand back a buffered `ByteStream`, registered for the
/// end-of-program flush. `append` picks truncate-or-append.
fn open_write_stream<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: &[Value<'gc>],
    append: bool,
) -> Result<Value<'gc>, QuoinError> {
    let path = arg!(args, String, 0);
    let path = OsString::from(path.as_str());
    match vm.await_io(IoRequest::OpenFileWrite { path, append })? {
        IoResult::Connected(id) => {
            let stream = crate::runtime::streams::make_write_byte_stream(vm, mc, id);
            vm.track_write_stream(stream);
            Ok(stream)
        }
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(QuoinError::Other(format!(
            "[IO]File.create:: unexpected I/O result {other:?}"
        ))),
    }
}

pub fn build_io_file_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[IO]File", Some("Object"))
        .construct_with("use [IO]File.open:")
        .class_doc(
            "A file on the local filesystem.\n\n\
             The class side does the everyday work: `create:` / `append:` answer buffered \
             write streams, and `delete:` / `rename:to:` / `exists?:` are one-shot metadata \
             ops. `open:` answers an [IO]File *value* — a path plus a metadata snapshot — \
             whose `byteStream` / `stringStream` open it for reading. Reads and writes park \
             the task, not the scheduler.\n\n\
             ```\n\
             var out = [IO]File.create:'/tmp/notes.txt'\n\
             out.writeAll:'hi'.asBytes\n\
             out.close\n\
             ([IO]File.open:'/tmp/notes.txt').stringStream.readAll     \"* -> 'hi'\n\
             [IO]File.delete:'/tmp/notes.txt'\n\
             ```",
        )
        .class_method("open:", |vm, mc, _receiver, args| {
            let path = arg!(args, String, 0);
            let os_string = OsString::from(path.as_str());
            Ok(new_native_io_file(
                vm,
                mc,
                os_string.clone(),
                metadata(os_string).map_err(|e| QuoinError::from_io_error(&e.into()))?,
            ))
        })
        .doc(
            "An [IO]File over `path`, reading its metadata now — a missing file throws a \
             catchable IoError (probe with `exists?:` first to avoid that). Opening holds no \
             file descriptor; `byteStream` / `stringStream` open one when reading starts.",
        )
        // create:path -> a writable ByteStream over a new (or truncated) file. Buffered:
        // writes accumulate and drain every IO_BUFFER_BYTES, and `close` flushes. A stream
        // never closed is flushed when the program ends (`VmState::open_write_streams`).
        .class_method("create:", |vm, mc, _receiver, args| {
            open_write_stream(vm, mc, &args, false)
        })
        .doc(
            "A writable ByteStream over a new file at `path` (an existing file is \
             truncated). Writes are buffered — they accumulate and drain in 16 KiB chunks — \
             and `close` flushes; a stream never closed is flushed when the program ends. \
             Wrap in `.stringStream` for text writing.\n\n\
             ```\n\
             var out = ([IO]File.create:'/tmp/log.txt').stringStream\n\
             out.writeln:'first line'\n\
             out.close\n\
             [IO]File.delete:'/tmp/log.txt'\n\
             ```",
        )
        // append:path -> the same, positioned at the end of an existing file (created if absent).
        .class_method("append:", |vm, mc, _receiver, args| {
            open_write_stream(vm, mc, &args, true)
        })
        .doc(
            "Like `create:`, but positioned at the end of an existing file (created if \
             absent) — the log-file mode. Answers the same buffered, flush-on-close \
             ByteStream.",
        )
        // delete:path -> remove the file. Synchronous, like `open:`'s metadata read: the
        // filesystem *stream* ops park, these small metadata ops do not.
        .class_method("delete:", |vm, mc, _receiver, args| {
            let path = arg!(args, String, 0);
            std::fs::remove_file(path.as_str())
                .map_err(|e| QuoinError::from_io_error(&e.into()))?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Remove the file at `path` (a missing file throws an IoError). For directories \
             use `[IO]Folder.delete:`. Returns nil.",
        )
        // rename:to: -> rename (or move) a file. Overwrites the destination, as POSIX does.
        .class_method("rename:to:", |vm, mc, _receiver, args| {
            let from = arg!(args, String, 0);
            let to = arg!(args, String, 1);
            std::fs::rename(from.as_str(), to.as_str())
                .map_err(|e| QuoinError::from_io_error(&e.into()))?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Rename (or move) a file. An existing destination is overwritten, as POSIX \
             rename does. Returns nil.",
        )
        // exists?:path -> whether anything exists at `path`. Answers without opening, so it
        // cannot raise for an absent file the way `open:` does.
        .class_method("exists?:", |vm, mc, _receiver, args| {
            let path = arg!(args, String, 0);
            Ok(vm.new_bool(mc, std::path::Path::new(path.as_str()).exists()))
        })
        .doc(
            "Whether anything (file or directory) exists at `path`. Answers without opening, \
             so it cannot throw for an absent file the way `open:` does.",
        )
        .instance_method("fullpath", |vm, mc, receiver, _args| {
            receiver
                .with_native_state(|io: &NativeIoFile| {
                    vm.new_string(mc, io.path.to_string_lossy().into_owned())
                })
                .map_err(|e| QuoinError::Other(e.to_string()))
        })
        .doc(
            "The file's whole path, as the String it was opened with (no \
             canonicalization).",
        )
        .instance_method("name", |vm, mc, receiver, _args| {
            receiver
                .with_native_state(|io: &NativeIoFile| {
                    vm.new_string(
                        mc,
                        PathBuf::from(&io.path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or("".to_string()),
                    )
                })
                .map_err(|e| QuoinError::Other(e.to_string()))
        })
        .doc(
            "The final path component — the file's name with its extension \
             (`'/tmp/notes.txt'` → `'notes.txt'`).",
        )
        .instance_method("ext", |vm, mc, receiver, _args| {
            let ext = receiver.with_native_state(|io: &NativeIoFile| {
                PathBuf::from(&io.path)
                    .extension()
                    .map(|s| s.to_os_string())
                    .unwrap_or_else(|| OsString::from(""))
            })?;
            Ok(vm.new_string(mc, ext.to_string_lossy().to_string()))
        })
        .doc(
            "The extension without its dot (`'/tmp/notes.txt'` → `'txt'`), or `''` when \
             there is none.",
        )
        .instance_method("s", |vm, mc, receiver, _args| {
            Ok(vm.new_string(
                mc,
                receiver
                    .with_native_state(|io: &NativeIoFile| io.path.to_string_lossy().into_owned())
                    .map_err(|e| QuoinError::Other(e.to_string()))?,
            ))
        })
        .doc("The same String as `fullpath`.")
        .instance_method("is_file?", |vm, mc, receiver, _args| {
            receiver
                .with_native_state(|io: &NativeIoFile| io.metadata.is_file())
                .map_err(|e| QuoinError::Other(e.to_string()))
                .map(|v| vm.new_bool(mc, v))
        })
        .doc(
            "Whether this is a regular file — false for a directory (what an \
             `[IO]Folder.next` walk uses to tell the two apart). From the metadata snapshot \
             taken when the [IO]File was created.",
        )
        .instance_method("size", |vm, mc, receiver, _args| {
            receiver
                .with_native_state(|io: &NativeIoFile| io.metadata.len())
                .map_err(|e| QuoinError::Other(e.to_string()))
                .map(|n| vm.new_int(mc, n as i64))
        })
        .doc(
            "The file's length in bytes, from the metadata snapshot taken when the \
             [IO]File was created — what a tar header (which precedes the content) or a \
             Content-Length needs before reading.",
        )
        .sdk_instance_method("modified", |host, receiver, _args| {
            let mtime = receiver
                .with_native_state(|io: &NativeIoFile| io.metadata.modified())
                .map_err(|e| QuoinError::Other(e.to_string()))?
                .map_err(|e| QuoinError::from_io_error(&e.into()))?;
            let ts = jiff::Timestamp::try_from(mtime).map_err(|e| {
                QuoinError::ValueError(format!("[IO]File#modified: out of range: {e}"))
            })?;
            Ok(crate::runtime::timestamp::make_timestamp(host, ts))
        })
        .doc(
            "The file's last-modification time as a Timestamp, from the metadata snapshot \
             taken when the [IO]File was created.",
        )
        // randomAccess -> open the file for positioned reads (pread-style; see the
        // RandomAccessFile class). Like byteStream, a fresh fd per call; the size
        // comes from an open-time stat, not this [IO]File's snapshot.
        .instance_method("randomAccess", |vm, mc, receiver, _args| {
            let path = receiver
                .with_native_state(|io: &NativeIoFile| io.path.clone())
                .map_err(QuoinError::Other)?;
            match vm.await_io(IoRequest::OpenFileRandom { path })? {
                IoResult::Opened { id, size } => Ok(
                    crate::runtime::random_access::make_random_access(vm, mc, id, size),
                ),
                IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
                other => Err(QuoinError::Other(format!(
                    "randomAccess: unexpected I/O result {other:?}"
                ))),
            }
        })
        .doc(
            "Open the file for positioned reads and answer a RandomAccessFile — \
             `readAt:offset count:` anywhere in the file, no cursor. What a \
             random-access format (zip) reads through; a fresh fd per call.",
        )
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_path = receiver.with_native_state(|io: &NativeIoFile| io.path.clone())?;
            let rhs_path = args[0].with_native_state(|io: &NativeIoFile| io.path.clone());
            match rhs_path {
                Ok(rhs_path) => Ok(vm.new_bool(mc, lhs_path == rhs_path)),
                Err(_) => Ok(vm.new_bool(mc, false)),
            }
        })
        .doc(
            "Whether the argument is an [IO]File with the same path string (no \
             canonicalization, so distinct spellings of one file differ). A non-File \
             argument is simply unequal.",
        )
        // byteStream -> open the file (read-only) and return a buffered ByteStream over it,
        // the same stream class a socket yields. The file's path stays an OsString through
        // the open; a fresh fd is opened each call (the [IO]File is not consumed).
        .instance_method("byteStream", |vm, mc, receiver, _args| {
            let path = receiver
                .with_native_state(|io: &NativeIoFile| io.path.clone())
                .map_err(QuoinError::Other)?;
            match vm.await_io(IoRequest::OpenFile { path })? {
                IoResult::Connected(id) => {
                    Ok(crate::runtime::streams::make_byte_stream(vm, mc, id))
                }
                IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
                other => Err(QuoinError::Other(format!(
                    "[IO]File.byteStream: unexpected I/O result {other:?}"
                ))),
            }
        })
        .doc(
            "Open the file for reading and answer a buffered ByteStream over it — the same \
             stream class a socket yields, so `read:` / `readUntil:` / `readAll` work \
             identically. Each call opens a fresh descriptor; the [IO]File itself is not \
             consumed.",
        )
        // stringStream -> open the file (read-only) and return a text StringStream over it.
        .instance_method("stringStream", |vm, mc, receiver, _args| {
            let path = receiver
                .with_native_state(|io: &NativeIoFile| io.path.clone())
                .map_err(QuoinError::Other)?;
            match vm.await_io(IoRequest::OpenFile { path })? {
                IoResult::Connected(id) => Ok(crate::runtime::streams::make_string_stream(
                    vm,
                    mc,
                    id,
                    Vec::new(),
                )),
                IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
                other => Err(QuoinError::Other(format!(
                    "[IO]File.stringStream: unexpected I/O result {other:?}"
                ))),
            }
        })
        .doc(
            "Open the file for reading and answer a text StringStream over it — `readLine` / \
             `eachLine:` / `readAll` are the everyday surface.\n\n\
             ```\n\
             var out = [IO]File.create:'/tmp/notes.txt'\n\
             out.writeAll:'hi'.asBytes\n\
             out.close\n\
             ([IO]File.open:'/tmp/notes.txt').stringStream.readAll     \"* -> 'hi'\n\
             [IO]File.delete:'/tmp/notes.txt'\n\
             ```",
        )
}

pub enum NativeIoHandleWrapper {
    Stdout(Stdout),
    Stderr(Stderr),
    Stdin(Stdin),
}

pub struct NativeIoHandle {
    pub wrapper: NativeIoHandleWrapper,
}

fn new_native_io_handle_with_wrapper<'a>(
    vm: &mut VmState<'a>,
    mc: &Mutation<'a>,
    wrapper: NativeIoHandleWrapper,
) -> Value<'a> {
    vm.new_native_state(
        mc,
        vm.get_builtin_class("[IO]Handle"),
        OpaqueState(NativeIoHandle { wrapper }),
    )
}

pub(crate) fn get_io_string<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    val: Value<'gc>,
) -> Result<String, QuoinError> {
    let val_type_name = val.type_name().to_string();
    let val_debug = format!("{:?}", val);

    if let Value::Object(obj) = val {
        let payload = obj.borrow().payload;
        if let ObjectPayload::String(s) = payload {
            return Ok(s.to_string());
        }
        let is_ansi = obj.borrow().class.borrow().name.name == "ANSI";
        if is_ansi {
            let string_val = vm.call_method(mc, val, "string", vec![])?;
            if let Value::Object(o) = string_val
                && let ObjectPayload::String(st) = &o.borrow().payload
            {
                return Ok(ansi_colorizer::colorize(st));
            }
            return Err(QuoinError::TypeError {
                expected: "String".to_string(),
                got: string_val.type_name().to_string(),
                msg: "Expected string return from ANSI#string".to_string(),
            });
        }
    }
    Err(QuoinError::TypeError {
        expected: "String or ANSI".to_string(),
        got: val_type_name,
        msg: format!("Expected String or ANSI (got {})", val_debug),
    })
}

/// Whether an `[IO]Handle` wraps stdout/stderr (the streams the color/decolor and DAP-capture
/// rules apply to) — as opposed to a file or stdin.
fn is_std_stream(handle: Value<'_>) -> Result<bool, QuoinError> {
    handle
        .with_native_state(|h: &NativeIoHandle| {
            matches!(
                h.wrapper,
                NativeIoHandleWrapper::Stdout(_) | NativeIoHandleWrapper::Stderr(_)
            )
        })
        .map_err(QuoinError::Other)
}

/// Write `bytes` to the handle's sink. Stdout/stderr go through `vm.write_std` (so the DAP
/// adapter can capture them as `output` events instead of corrupting the protocol stream);
/// stdin errors. Files are not handles: they are streams (`[IO]File.create:`), because a
/// blocking `write_all` here would freeze the scheduler.
fn handle_write<'gc>(
    vm: &mut VmState<'gc>,
    handle: Value<'gc>,
    bytes: &[u8],
) -> Result<(), QuoinError> {
    enum Kind {
        Out,
        Err,
        Stdin,
    }
    let kind = handle
        .with_native_state(|h: &NativeIoHandle| match &h.wrapper {
            NativeIoHandleWrapper::Stdout(_) => Kind::Out,
            NativeIoHandleWrapper::Stderr(_) => Kind::Err,
            NativeIoHandleWrapper::Stdin(_) => Kind::Stdin,
        })
        .map_err(QuoinError::Other)?;
    match kind {
        Kind::Out => vm.write_std_guest(StdStream::Out, bytes),
        Kind::Err => vm.write_std_guest(StdStream::Err, bytes),
        // A typed `IoError`, not a bare String: `catch:{|e:Error|}` must be able to see it.
        // (Mirrors `[IO]Handle.stringStream` refusing the write handles.)
        Kind::Stdin => Err(QuoinError::io(
            crate::error::IoErrorKind::InvalidInput,
            "[IO]Handle.stdin is read-only (stdin cannot be written)",
        )),
    }
}

/// The one stdin stream, created on first use (`vm.stdin_stream`).
///
/// Memoized rather than freshly opened per call because a stream *buffers*: two streams over
/// fd 0 would each hold bytes the other never sees, so reading a line through one and then the
/// other would silently drop input. `kind` therefore also fixes the flavour — asking for the
/// byte view after the text view (or vice versa) is a mistake, not a conversion.
fn stdin_stream<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    kind: &str,
    who: &str,
) -> Result<Value<'gc>, QuoinError> {
    let want = if kind == "byteStream" {
        "ByteStream"
    } else {
        "StringStream"
    };
    if let Some(existing) = vm.stdin_stream {
        // `type_name()` says `Object` for a native-state value; the class is what identifies it.
        let have = match existing {
            Value::Object(obj) => obj.borrow().class_name(),
            other => other.type_name().to_string(),
        };
        if have != want {
            return Err(QuoinError::io(
                crate::error::IoErrorKind::InvalidInput,
                format!(
                    "{who}: stdin is already open as a {have}; it buffers, so it cannot also be read as a {want}"
                ),
            ));
        }
        return Ok(existing);
    }
    let stream = match vm.await_io(IoRequest::OpenStdin)? {
        IoResult::Connected(id) => {
            if kind == "byteStream" {
                crate::runtime::streams::make_byte_stream(vm, mc, id)
            } else {
                crate::runtime::streams::make_string_stream(vm, mc, id, Vec::new())
            }
        }
        IoResult::Err(e) => return Err(QuoinError::from_io_error(&e)),
        other => {
            return Err(QuoinError::Other(format!(
                "{who}: unexpected I/O result {other:?}"
            )));
        }
    };
    vm.stdin_stream = Some(stream);
    Ok(stream)
}

/// Back `[IO]Handle#stringStream` / `#byteStream`. The write handles are refused here rather
/// than at first read, so the mistake surfaces where it was made.
fn open_stdin_stream<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
    kind: &str,
) -> Result<Value<'gc>, QuoinError> {
    let readable = receiver.with_native_state(|h: &NativeIoHandle| {
        matches!(h.wrapper, NativeIoHandleWrapper::Stdin(_))
    })?;
    if !readable {
        return Err(QuoinError::io(
            crate::error::IoErrorKind::InvalidInput,
            format!("[IO]Handle.{kind}: only stdin is readable (stdout/stderr are write-only)"),
        ));
    }
    stdin_stream(vm, mc, kind, &format!("[IO]Handle.{kind}"))
}

/// `[IO]Stdin` — the readable half of the standard streams, mirroring the `[IO]Stdout` /
/// `[IO]Stderr` constants. A *class*, not a prelude constant, because opening stdin is an
/// `await_io` and the prelude also runs under `qn benchmark`, which has no scheduler to park on;
/// the stream is therefore created on first read. `readLine` / `eachLine:` / `readAll` are Quoin
/// delegators over `.stream` (`qnlib/core/06-io.qn`).
pub fn build_io_stdin_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[IO]Stdin", Some("Object"))
        .abstract_class()
        .class_doc(
            "The process's standard input.\n\nReads park the task rather than blocking the \
             scheduler, so a program can serve sockets while it waits on a line. The stream is \
             created on the first read and reused: it buffers, so there can only be one. \
             `readLine` / `eachLine:` / `readAll` are the everyday surface; `stream` and \
             `byteStream` expose the stream itself.",
        )
        .class_method("stream", |vm, mc, _r, _args| {
            stdin_stream(vm, mc, "stringStream", "[IO]Stdin.stream")
        })
        .returns("StringStream")
        .doc(
            "The one shared StringStream over standard input, created on first use. Asking \
             for the byte view after the text view (or vice versa) is an error: a stream \
             buffers, so two views would each hold bytes the other never sees.",
        )
        .class_method("byteStream", |vm, mc, _r, _args| {
            stdin_stream(vm, mc, "byteStream", "[IO]Stdin.byteStream")
        })
        .returns("ByteStream")
        .doc("The one shared ByteStream over standard input, created on first use.")
}

pub fn build_io_handle_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[IO]Handle", Some("Object"))
        .construct_with("use [IO]Handle.stdin / stdout / stderr")
        .class_doc(
            "One of the process's three standard streams: stdout, stderr, or stdin.\n\n\
             The prelude binds the write handles as the constants `[IO]Stdout` and \
             `[IO]Stderr` (`[IO]Stdout.writeln:'hi'` is the everyday print); reading belongs \
             to `[IO]Stdin`. Writes to stdout/stderr are routed through the VM, so ANSI \
             color is stripped when the terminal doesn't support it and a debugger can \
             capture program output cleanly.",
        )
        .class_method("stdout", |vm, mc, _receiver, _args| {
            Ok(new_native_io_handle_with_wrapper(
                vm,
                mc,
                NativeIoHandleWrapper::Stdout(stdout()),
            ))
        })
        .doc(
            "The standard-output handle — normally reached through the prelude constant \
             `[IO]Stdout`.",
        )
        .class_method("stderr", |vm, mc, _receiver, _args| {
            Ok(new_native_io_handle_with_wrapper(
                vm,
                mc,
                NativeIoHandleWrapper::Stderr(stderr()),
            ))
        })
        .doc(
            "The standard-error handle — normally reached through the prelude constant \
             `[IO]Stderr`.",
        )
        .class_method("stdin", |vm, mc, _receiver, _args| {
            Ok(new_native_io_handle_with_wrapper(
                vm,
                mc,
                NativeIoHandleWrapper::Stdin(stdin()),
            ))
        })
        .doc(
            "The standard-input handle. It is read-only: `write:` / `writeln:` throw, and \
             reading goes through `stringStream` / `byteStream` — or, more directly, the \
             `[IO]Stdin` class.",
        )
        .instance_method("s", |vm, mc, receiver, _args| {
            let s = receiver.with_native_state(|h: &NativeIoHandle| match &h.wrapper {
                NativeIoHandleWrapper::Stdout(_) => "[IO]Handle.stdout",
                NativeIoHandleWrapper::Stderr(_) => "[IO]Handle.stderr",
                NativeIoHandleWrapper::Stdin(_) => "[IO]Handle.stdin",
            })?;
            Ok(vm.new_string(mc, s.to_string()))
        })
        .doc(
            "Which handle this is, as its constructor expression.\n\n\
             ```\n\
             [IO]Stdout.s     \"* -> '[IO]Handle.stdout'\n\
             ```",
        )
        // stringStream / byteStream: read the handle through the same stream stack as a socket
        // or a file, so `readLine` / `eachLine:` / `readAll` come for free and every read PARKS
        // the task rather than freezing the single-threaded scheduler. Only stdin is readable;
        // the write handles say so rather than handing back a stream that fails on first read.
        .instance_method("stringStream", |vm, mc, receiver, _args| {
            open_stdin_stream(vm, mc, receiver, "stringStream")
        })
        .doc(
            "The one shared StringStream over standard input (stdin only — the write handles \
             throw an IoError). Reads park the task, not the scheduler; the stream is \
             created on first use and reused, exactly as `[IO]Stdin.stream` describes.",
        )
        .instance_method("byteStream", |vm, mc, receiver, _args| {
            open_stdin_stream(vm, mc, receiver, "byteStream")
        })
        .doc(
            "The one shared ByteStream over standard input (stdin only — the write handles \
             throw an IoError). Mutually exclusive with the text view: stdin buffers, so it \
             is one stream or the other for the whole program.",
        )
        .instance_method("write:", |vm, mc, _receiver, args| {
            let mut s = get_io_string(vm, mc, args[0])?;
            let active_receiver = vm.active_native_args.last().unwrap().receiver;
            if is_std_stream(active_receiver)? && !vm.options.supports_color {
                s = ansi_colorizer::decolorize(&s);
            }
            handle_write(vm, active_receiver, s.as_bytes())?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Write a String (or an ANSI color expression) to the handle, without a trailing \
             newline. ANSI codes are stripped when the output doesn't support color; writing \
             to the stdin handle throws an IoError. Returns nil.",
        )
        .instance_method("writeln:", |vm, mc, _receiver, args| {
            let mut s = get_io_string(vm, mc, args[0])?;
            let active_receiver = vm.active_native_args.last().unwrap().receiver;
            if is_std_stream(active_receiver)? && !vm.options.supports_color {
                s = ansi_colorizer::decolorize(&s);
            }
            s.push('\n');
            handle_write(vm, active_receiver, s.as_bytes())?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "`write:` plus a trailing newline — `[IO]Stdout.writeln:'hello'` is the everyday \
             print statement. Returns nil.",
        )
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_val = receiver.with_native_state(|h: &NativeIoHandle| match &h.wrapper {
                NativeIoHandleWrapper::Stdout(_) => Some(0),
                NativeIoHandleWrapper::Stderr(_) => Some(1),
                NativeIoHandleWrapper::Stdin(_) => Some(2),
            })?;
            let rhs_val = args[0].with_native_state(|h: &NativeIoHandle| match &h.wrapper {
                NativeIoHandleWrapper::Stdout(_) => Some(0),
                NativeIoHandleWrapper::Stderr(_) => Some(1),
                NativeIoHandleWrapper::Stdin(_) => Some(2),
            });
            match rhs_val {
                Ok(Some(r)) if lhs_val == Some(r) => Ok(vm.new_bool(mc, true)),
                _ => Ok(vm.new_bool(mc, false)),
            }
        })
        .doc(
            "Whether the argument is a handle on the same standard stream (any two stdout \
             handles are equal). A non-handle argument is simply unequal.\n\n\
             ```\n\
             [IO]Stdout == [IO]Handle.stdout     \"* -> true\n\
             ```",
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{class, object, string};
    use crate::value::Value;
    use crate::vm::{VmOptions, VmState};

    use gc_arena::{Arena, Rootable};

    #[test]
    fn test_get_io_string_ansi() {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());
            vm.register_native_class(mc, object::build_object_class());
            vm.register_native_class(mc, class::build_class_class());
            vm.register_native_class(mc, string::build_string_class());

            // Build and register the ANSI class
            let ansi_builder = NativeClassBuilder::new("ANSI", Some("Object"))
                .instance_method("string", |vm, mc, _receiver, _args| {
                    Ok(vm.new_string(mc, "[bold]bold text[/]".to_string()))
                });
            vm.register_native_class(mc, ansi_builder);
            vm
        });

        arena.mutate_root(|mc, vm| {
            // Test 1: Quoin String
            let string_val = vm.new_string(mc, "hello".to_string());
            let s = get_io_string(vm, mc, string_val).unwrap();
            assert_eq!(s, "hello");

            // Test 2: ANSI Instance
            let ansi_class = vm.get_builtin_class("ANSI");
            let ansi_instance = Value::Object(vm.new_object(mc, ansi_class));
            let s = get_io_string(vm, mc, ansi_instance).unwrap();

            // colorized version of "[bold]bold text[/]" starts with "\x1b[" and ends with reset code
            assert!(s.contains("bold text"));
            assert!(s.contains("\x1b["));
        });
    }

    // With `capture_output` armed (the DAP mode), stdout/stderr `[IO]Handle` writes buffer into
    // `program_output` (tagged by stream) instead of hitting the real fds, and drain cleanly.
    #[test]
    fn test_handle_capture_routes_std_writes_to_buffer() {
        use crate::vm::StdStream;

        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());
            vm.register_native_class(mc, object::build_object_class());
            vm.register_native_class(mc, class::build_class_class());
            vm.register_native_class(mc, string::build_string_class());
            vm.register_native_class(mc, build_io_handle_class());
            vm
        });

        arena.mutate_root(|mc, vm| {
            vm.output.capture = true;
            let out_handle =
                new_native_io_handle_with_wrapper(vm, mc, NativeIoHandleWrapper::Stdout(stdout()));
            let err_handle =
                new_native_io_handle_with_wrapper(vm, mc, NativeIoHandleWrapper::Stderr(stderr()));

            let hello = vm.new_string(mc, "hello".to_string());
            vm.call_method(mc, out_handle, "write:", vec![hello])
                .unwrap();
            let line = vm.new_string(mc, "line".to_string());
            vm.call_method(mc, out_handle, "writeln:", vec![line])
                .unwrap();
            let warn = vm.new_string(mc, "warn".to_string());
            vm.call_method(mc, err_handle, "write:", vec![warn])
                .unwrap();

            let chunks = vm.take_program_output();
            assert_eq!(chunks.len(), 3);
            assert_eq!(chunks[0].stream, StdStream::Out);
            assert_eq!(chunks[0].bytes, b"hello".to_vec());
            assert_eq!(chunks[1].stream, StdStream::Out);
            assert_eq!(chunks[1].bytes, b"line\n".to_vec());
            assert_eq!(chunks[2].stream, StdStream::Err);
            assert_eq!(chunks[2].bytes, b"warn".to_vec());
            // Draining empties the buffer.
            assert!(vm.take_program_output().is_empty());
        });
    }
}
