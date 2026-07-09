use crate::error::QuoinError;
use crate::io_backend::{IoError, IoRequest, IoResult};
use crate::value::{NativeClassBuilder, ObjectPayload, OpaqueState, Value};
use crate::vm::{StdStream, VmState};
use crate::{ansi_colorizer, arg};

use gc_arena::{Gc, Mutation};
use std::ffi::OsString;
use std::fs::{File, Metadata, ReadDir, metadata, read_dir};
use std::io::{Stderr, Stdin, Stdout, Write, stderr, stdin, stdout};
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
        .class_method("open:", |vm, mc, _receiver, args| {
            let path = arg!(args, String, 0);
            new_native_io_folder(vm, mc, path)
        })
        .instance_method("path", |vm, mc, receiver, _args| {
            receiver
                .with_native_state(|io: &NativeIoFolder| {
                    vm.new_string(mc, io.path.to_string_lossy().into_owned())
                })
                .map_err(|e| QuoinError::Other(e.to_string()))
        })
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

            return Ok(if let Some(entry) = r {
                let ent = entry?;
                let os_string = ent.path().into_os_string();
                new_native_io_file(vm, mc, os_string, ent.metadata().map_err(fs_err)?)
            } else {
                vm.new_nil(mc)
            });
        })
        .instance_method("reset", |vm, mc, receiver, _args| {
            receiver.with_native_state_mut(mc, |io: &mut NativeIoFolder| {
                io.iter = None;
            })?;
            Ok(vm.new_nil(mc))
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_path = receiver.with_native_state(|io: &NativeIoFolder| io.path.clone())?;
            let rhs_path = args[0].with_native_state(|io: &NativeIoFolder| io.path.clone());
            match rhs_path {
                Ok(rhs_path) => Ok(vm.new_bool(mc, lhs_path == rhs_path)),
                Err(_) => Ok(vm.new_bool(mc, false)),
            }
        })
}

fn new_native_io_folder<'a>(
    vm: &mut VmState<'a>,
    mc: &Mutation<'a>,
    path: Gc<'a, String>,
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

pub fn build_io_file_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[IO]File", Some("Object"))
        .construct_with("use [IO]File.open:")
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
        .instance_method("fullpath", |vm, mc, receiver, _args| {
            receiver
                .with_native_state(|io: &NativeIoFile| {
                    vm.new_string(mc, io.path.to_string_lossy().into_owned())
                })
                .map_err(|e| QuoinError::Other(e.to_string()))
        })
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
        .instance_method("ext", |vm, mc, receiver, _args| {
            let ext = receiver.with_native_state(|io: &NativeIoFile| {
                PathBuf::from(&io.path)
                    .extension()
                    .map(|s| s.to_os_string())
                    .unwrap_or_else(|| OsString::from(""))
            })?;
            Ok(vm.new_string(mc, ext.to_string_lossy().to_string()))
        })
        .instance_method("s", |vm, mc, receiver, _args| {
            Ok(vm.new_string(
                mc,
                receiver
                    .with_native_state(|io: &NativeIoFile| {
                        io.path.to_string_lossy().to_owned().to_string()
                    })
                    .map_err(|e| QuoinError::Other(e.to_string()))?,
            ))
        })
        .instance_method("is_file?", |vm, mc, receiver, _args| {
            receiver
                .with_native_state(|io: &NativeIoFile| io.metadata.is_file())
                .map_err(|e| QuoinError::Other(e.to_string()))
                .map(|v| vm.new_bool(mc, v))
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_path = receiver.with_native_state(|io: &NativeIoFile| io.path.clone())?;
            let rhs_path = args[0].with_native_state(|io: &NativeIoFile| io.path.clone());
            match rhs_path {
                Ok(rhs_path) => Ok(vm.new_bool(mc, lhs_path == rhs_path)),
                Err(_) => Ok(vm.new_bool(mc, false)),
            }
        })
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
}

pub enum NativeIoHandleWrapper {
    Stdout(Stdout),
    Stderr(Stderr),
    Stdin(Stdin),
    File(File),
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

fn get_io_string<'gc>(
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
            if let Value::Object(o) = string_val {
                if let ObjectPayload::String(st) = &o.borrow().payload {
                    return Ok(ansi_colorizer::colorize(st));
                }
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
/// adapter can capture them as `output` events instead of corrupting the protocol stream); a
/// file-backed handle writes directly; stdin errors.
fn handle_write<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    handle: Value<'gc>,
    bytes: &[u8],
) -> Result<(), QuoinError> {
    enum Kind {
        Out,
        Err,
        Stdin,
        File,
    }
    let kind = handle
        .with_native_state(|h: &NativeIoHandle| match &h.wrapper {
            NativeIoHandleWrapper::Stdout(_) => Kind::Out,
            NativeIoHandleWrapper::Stderr(_) => Kind::Err,
            NativeIoHandleWrapper::Stdin(_) => Kind::Stdin,
            NativeIoHandleWrapper::File(_) => Kind::File,
        })
        .map_err(QuoinError::Other)?;
    match kind {
        Kind::Out => vm
            .write_std(StdStream::Out, bytes)
            .map_err(|e| QuoinError::Other(e.to_string())),
        Kind::Err => vm
            .write_std(StdStream::Err, bytes)
            .map_err(|e| QuoinError::Other(e.to_string())),
        // A typed `IoError`, not a bare String: `catch:{|e:Error|}` must be able to see it.
        // (Mirrors `[IO]Handle.stringStream` refusing the write handles.)
        Kind::Stdin => Err(QuoinError::io(
            crate::error::IoErrorKind::InvalidInput,
            "[IO]Handle.stdin is read-only (stdin cannot be written)",
        )),
        Kind::File => handle.with_native_state_mut(mc, |h: &mut NativeIoHandle| {
            if let NativeIoHandleWrapper::File(f) = &mut h.wrapper {
                f.write_all(bytes)
                    .map_err(|e| QuoinError::Other(e.to_string()))
            } else {
                Ok(())
            }
        })?,
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
        .class_method("stream", |vm, mc, _r, _args| {
            stdin_stream(vm, mc, "stringStream", "[IO]Stdin.stream")
        })
        .returns("StringStream")
        .class_method("byteStream", |vm, mc, _r, _args| {
            stdin_stream(vm, mc, "byteStream", "[IO]Stdin.byteStream")
        })
        .returns("ByteStream")
}

pub fn build_io_handle_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[IO]Handle", Some("Object"))
        .construct_with("use [IO]Handle.stdin / stdout / stderr")
        .class_method("stdout", |vm, mc, _receiver, _args| {
            Ok(new_native_io_handle_with_wrapper(
                vm,
                mc,
                NativeIoHandleWrapper::Stdout(stdout()),
            ))
        })
        .class_method("stderr", |vm, mc, _receiver, _args| {
            Ok(new_native_io_handle_with_wrapper(
                vm,
                mc,
                NativeIoHandleWrapper::Stderr(stderr()),
            ))
        })
        .class_method("stdin", |vm, mc, _receiver, _args| {
            Ok(new_native_io_handle_with_wrapper(
                vm,
                mc,
                NativeIoHandleWrapper::Stdin(stdin()),
            ))
        })
        .instance_method("s", |vm, mc, receiver, _args| {
            let s = receiver.with_native_state(|h: &NativeIoHandle| match &h.wrapper {
                NativeIoHandleWrapper::Stdout(_) => "[IO]Handle.stdout",
                NativeIoHandleWrapper::Stderr(_) => "[IO]Handle.stderr",
                NativeIoHandleWrapper::Stdin(_) => "[IO]Handle.stdin",
                NativeIoHandleWrapper::File(_) => "[IO]Handle.file",
            })?;
            Ok(vm.new_string(mc, s.to_string()))
        })
        // stringStream / byteStream: read the handle through the same stream stack as a socket
        // or a file, so `readLine` / `eachLine:` / `readAll` come for free and every read PARKS
        // the task rather than freezing the single-threaded scheduler. Only stdin is readable;
        // the write handles say so rather than handing back a stream that fails on first read.
        .instance_method("stringStream", |vm, mc, receiver, _args| {
            open_stdin_stream(vm, mc, receiver, "stringStream")
        })
        .instance_method("byteStream", |vm, mc, receiver, _args| {
            open_stdin_stream(vm, mc, receiver, "byteStream")
        })
        .instance_method("write:", |vm, mc, _receiver, args| {
            let mut s = get_io_string(vm, mc, args[0])?;
            let active_receiver = vm.active_native_args.last().unwrap().receiver;
            if is_std_stream(active_receiver)? && !vm.options.supports_color {
                s = ansi_colorizer::decolorize(&s);
            }
            handle_write(vm, mc, active_receiver, s.as_bytes())?;
            Ok(vm.new_nil(mc))
        })
        .instance_method("writeln:", |vm, mc, _receiver, args| {
            let mut s = get_io_string(vm, mc, args[0])?;
            let active_receiver = vm.active_native_args.last().unwrap().receiver;
            if is_std_stream(active_receiver)? && !vm.options.supports_color {
                s = ansi_colorizer::decolorize(&s);
            }
            s.push('\n');
            handle_write(vm, mc, active_receiver, s.as_bytes())?;
            Ok(vm.new_nil(mc))
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_val = receiver.with_native_state(|h: &NativeIoHandle| match &h.wrapper {
                NativeIoHandleWrapper::Stdout(_) => Some(0),
                NativeIoHandleWrapper::Stderr(_) => Some(1),
                NativeIoHandleWrapper::Stdin(_) => Some(2),
                NativeIoHandleWrapper::File(_) => None,
            })?;
            let rhs_val = args[0].with_native_state(|h: &NativeIoHandle| match &h.wrapper {
                NativeIoHandleWrapper::Stdout(_) => Some(0),
                NativeIoHandleWrapper::Stderr(_) => Some(1),
                NativeIoHandleWrapper::Stdin(_) => Some(2),
                NativeIoHandleWrapper::File(_) => None,
            });
            match rhs_val {
                Ok(Some(r)) if lhs_val == Some(r) => Ok(vm.new_bool(mc, true)),
                _ => Ok(vm.new_bool(mc, false)),
            }
        })
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
                    Ok(vm.new_string(mc, "$bw[bold text$]".to_string()))
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

            // colorized version of "$bw[bold text$]" starts with "\x1b[" and ends with reset code
            assert!(s.contains("bold text"));
            assert!(s.contains("\x1b["));
        });
    }

    #[test]
    fn test_handle_write_to_file() {
        use std::io::Read as _;

        // A file-backed [IO]Handle is not constructible from Quoin (the class only mints
        // stdout/stderr/stdin), so the File write/writeln arms are exercised here directly.
        let path = std::env::temp_dir().join(format!("quoin_io_handle_{}.txt", std::process::id()));
        let arena_path = path.clone();

        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());
            vm.register_native_class(mc, object::build_object_class());
            vm.register_native_class(mc, class::build_class_class());
            vm.register_native_class(mc, string::build_string_class());
            vm.register_native_class(mc, build_io_handle_class());
            vm
        });

        arena.mutate_root(|mc, vm| {
            let file = File::create(&arena_path).unwrap();
            let handle =
                new_native_io_handle_with_wrapper(vm, mc, NativeIoHandleWrapper::File(file));

            // s and ==: also have File arms that Quoin can't reach (no file-backed ctor).
            let s = vm.call_method(mc, handle, "s", vec![]).unwrap();
            match s {
                Value::Object(o) => match &o.borrow().payload {
                    ObjectPayload::String(st) => assert_eq!(st.as_str(), "[IO]Handle.file"),
                    _ => panic!("s did not return a string"),
                },
                _ => panic!("s did not return an object"),
            }
            // Two file-backed handles never compare equal (File -> None).
            let eq = vm.call_method(mc, handle, "==:", vec![handle]).unwrap();
            assert!(eq.is_false());

            // write: and writeln: dispatch through call_method, which sets up
            // active_native_args (the receiver the write arms read back).
            let hello = vm.new_string(mc, "hello".to_string());
            let r = vm.call_method(mc, handle, "write:", vec![hello]).unwrap();
            assert!(r.is_nil());

            let line = vm.new_string(mc, "line".to_string());
            let r = vm.call_method(mc, handle, "writeln:", vec![line]).unwrap();
            assert!(r.is_nil());
        });

        // The arena (and the File it owns) is still alive, but std::fs::File is unbuffered,
        // so the bytes are already visible to a second handle on the same path.
        let mut contents = String::new();
        File::open(&path)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(contents, "helloline\n");
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
