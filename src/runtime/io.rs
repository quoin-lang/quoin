use crate::arg;
use crate::error::BBError;
use crate::value::{NativeClassBuilder, OpaqueState, Value};
use crate::vm::VmState;

use gc_arena::{Gc, Mutation};
use std::ffi::OsString;
use std::fs::{metadata, File, Metadata, ReadDir};
use std::io::{stderr, stdin, stdout, Stderr, Stdin, Stdout, Write};
use std::path::PathBuf;

pub struct NativeIoFolder {
    pub path: OsString,
    pub iter: Option<ReadDir>,
}

pub fn build_io_folder_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[IO]Folder", Some("Object"))
        .class_method("open:", |vm, mc, args| {
            let path = arg!(args, String, 1);
            Ok(new_native_io_folder(vm, mc, path))
        })
        .instance_method("path", |vm, mc, args| {
            args[0]
                .with_native_state(|io: &NativeIoFolder| {
                    vm.new_string(mc, io.path.to_string_lossy().into_owned())
                })
                .map_err(|e| BBError::Other(e.to_string()))
        })
        .instance_method("next", |vm, mc, args| {
            let r = args[0].with_native_state_mut(mc, |io: &mut NativeIoFolder| {
                if io.iter.is_none() {
                    io.iter = Some(std::fs::read_dir(&io.path).unwrap());
                }
                io.iter
                    .as_mut()
                    .unwrap()
                    .next()
                    .map(|r| r.map_err(|e| BBError::Other(e.to_string())))
            })?;

            return Ok(if let Some(entry) = r {
                let ent = entry?;
                let os_string = ent.path().into_os_string();
                new_native_io_file(
                    vm,
                    mc,
                    os_string,
                    ent.metadata().map_err(|e| BBError::Other(e.to_string()))?,
                )
            } else {
                vm.new_nil(mc)
            });
        })
        .instance_method("reset", |vm, mc, args| {
            args[0].with_native_state_mut(mc, |io: &mut NativeIoFolder| {
                io.iter = None;
            })?;
            Ok(vm.new_nil(mc))
        })
}

fn new_native_io_folder<'a>(
    vm: &mut VmState<'a>,
    mc: &Mutation<'a>,
    path: Gc<'a, String>,
) -> Value<'a> {
    let state = OpaqueState(NativeIoFolder {
        path: OsString::from(path.as_str()),
        iter: Some(std::fs::read_dir(path.as_str()).unwrap()),
    });

    vm.new_native_state(mc, vm.get_builtin_class("[IO]Folder"), state)
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
        .class_method("open:", |vm, mc, args| {
            let path = arg!(args, String, 1);
            let os_string = OsString::from(path.as_str());
            Ok(new_native_io_file(
                vm,
                mc,
                os_string.clone(),
                metadata(os_string).map_err(|e| BBError::Other(e.to_string()))?,
            ))
        })
        .instance_method("fullpath", |vm, mc, args| {
            args[0]
                .with_native_state(|io: &NativeIoFile| {
                    vm.new_string(mc, io.path.to_string_lossy().into_owned())
                })
                .map_err(|e| BBError::Other(e.to_string()))
        })
        .instance_method("name", |vm, mc, args| {
            args[0]
                .with_native_state(|io: &NativeIoFile| {
                    vm.new_string(
                        mc,
                        PathBuf::from(&io.path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or("".to_string()),
                    )
                })
                .map_err(|e| BBError::Other(e.to_string()))
        })
        .instance_method("ext", |vm, mc, args| {
            let ext = args[0].with_native_state(|io: &NativeIoFile| {
                PathBuf::from(&io.path)
                    .extension()
                    .map(|s| s.to_os_string())
                    .unwrap_or_else(|| OsString::from(""))
            })?;
            Ok(vm.new_string(mc, ext.to_string_lossy().to_string()))
        })
        .instance_method("s", |vm, mc, args| {
            Ok(vm.new_string(
                mc,
                args[0]
                    .with_native_state(|io: &NativeIoFile| {
                        io.path.to_string_lossy().to_owned().to_string()
                    })
                    .map_err(|e| BBError::Other(e.to_string()))?,
            ))
        })
        .instance_method("is_file?", |vm, mc, args| {
            args[0]
                .with_native_state(|io: &NativeIoFile| io.metadata.is_file())
                .map_err(|e| BBError::Other(e.to_string()))
                .map(|v| vm.new_bool(mc, v))
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

pub fn build_io_handle_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[IO]Handle", Some("Object"))
        .class_method("stdout", |vm, mc, _args| {
            Ok(new_native_io_handle_with_wrapper(
                vm,
                mc,
                NativeIoHandleWrapper::Stdout(stdout()),
            ))
        })
        .class_method("stderr", |vm, mc, _args| {
            Ok(new_native_io_handle_with_wrapper(
                vm,
                mc,
                NativeIoHandleWrapper::Stderr(stderr()),
            ))
        })
        .class_method("stdin", |vm, mc, _args| {
            Ok(new_native_io_handle_with_wrapper(
                vm,
                mc,
                NativeIoHandleWrapper::Stdin(stdin()),
            ))
        })
        .instance_method("s", |vm, mc, args| {
            let s = args[0].with_native_state(|h: &NativeIoHandle| match &h.wrapper {
                NativeIoHandleWrapper::Stdout(_) => "[IO]Handle.stdout",
                NativeIoHandleWrapper::Stderr(_) => "[IO]Handle.stderr",
                NativeIoHandleWrapper::Stdin(_) => "[IO]Handle.stdin",
                NativeIoHandleWrapper::File(_) => "[IO]Handle.file",
            })?;
            Ok(vm.new_string(mc, s.to_string()))
        })
        .instance_method("write:", |vm, mc, args| {
            let s = arg!(args, String, 1);
            let bytes = s.as_bytes().to_vec();

            args[0].with_native_state_mut(mc, |h: &mut NativeIoHandle| {
                match &mut h.wrapper {
                    NativeIoHandleWrapper::Stdout(out) => {
                        out.write(&bytes)
                            .map_err(|e| BBError::Other(e.to_string()))?;
                        Ok(())
                    }
                    NativeIoHandleWrapper::Stderr(err) => {
                        err.write(&bytes)
                            .map_err(|e| BBError::Other(e.to_string()))?;
                        Ok(())
                    }
                    NativeIoHandleWrapper::Stdin(_) => {
                        Err(BBError::Other("can't write to stdin!".to_string()))
                    }
                    NativeIoHandleWrapper::File(f) => {
                        f.write(&bytes).map_err(|e| BBError::Other(e.to_string()))?;
                        Ok(())
                    }
                }
            })??;

            Ok(vm.new_nil(mc))
        })
        .instance_method("writeln:", |vm, mc, args| {
            let s = arg!(args, String, 1);
            let bytes = format!("{}\n", s).into_bytes();

            args[0].with_native_state_mut(mc, |h: &mut NativeIoHandle| {
                match &mut h.wrapper {
                    NativeIoHandleWrapper::Stdout(out) => {
                        out.write(&bytes)
                            .map_err(|e| BBError::Other(e.to_string()))?;
                        Ok(())
                    }
                    NativeIoHandleWrapper::Stderr(err) => {
                        err.write(&bytes)
                            .map_err(|e| BBError::Other(e.to_string()))?;
                        Ok(())
                    }
                    NativeIoHandleWrapper::Stdin(_) => {
                        Err(BBError::Other("can't write to stdin!".to_string()))
                    }
                    NativeIoHandleWrapper::File(f) => {
                        f.write(&bytes).map_err(|e| BBError::Other(e.to_string()))?;
                        Ok(())
                    }
                }
            })??;

            Ok(vm.new_nil(mc))
        })
}
