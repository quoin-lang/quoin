use crate::arg;
use crate::error::BBError;
use crate::value::{Class, NativeClassBuilder, OpaqueState, Value};
use crate::vm::VmState;

use gc_arena::{Gc, Mutation, RefLock};
use std::ffi::OsString;
use std::fs::{metadata, File, Metadata, ReadDir};
use std::path::PathBuf;

pub struct NativeIoFolder {
    pub path: OsString,
    pub iter: Option<ReadDir>,
}

pub fn build_io_folder_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[IO]Folder", Some("Object"))
        .class_method("open:", |vm, mc, args| {
            let path = arg!(args, String, 1);
            let clz = arg!(args, Class, 0);
            new_native_io_folder(vm, mc, clz, path)
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

            let clz = vm.get_builtin_class("[IO]File");
            return Ok(if let Some(entry) = r {
                let ent = entry?;
                let os_string = ent.path().into_os_string();
                new_native_io_file(vm, mc, clz, os_string, ent.metadata().unwrap())?
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
    clz: Gc<'a, RefLock<Class<'a>>>,
    path: Gc<'a, String>,
) -> Result<Value<'a>, BBError> {
    let state = OpaqueState(NativeIoFolder {
        path: OsString::from(path.as_str()),
        iter: Some(std::fs::read_dir(path.as_str()).unwrap()),
    });

    Ok(vm.new_native_state(mc, clz, state))
}

pub struct NativeIoFile {
    pub path: OsString,
    pub metadata: Metadata,
    pub file: Option<File>,
}

fn new_native_io_file<'a>(
    vm: &mut VmState<'a>,
    mc: &Mutation<'a>,
    clz: Gc<'a, RefLock<Class<'a>>>,
    path: OsString,
    metadata: Metadata,
) -> Result<Value<'a>, BBError> {
    let state = OpaqueState(NativeIoFile {
        path,
        metadata,
        file: None,
    });

    Ok(vm.new_native_state(mc, clz, state))
}

pub fn build_io_file_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[IO]File", Some("Object"))
        .class_method("open:", |vm, mc, args| {
            let path = arg!(args, String, 1);
            let clz = arg!(args, Class, 0);
            let os_string = OsString::from(path.as_str());
            Ok(new_native_io_file(
                vm,
                mc,
                clz,
                os_string.clone(),
                metadata(os_string).map_err(|e| BBError::Other(e.to_string()))?,
            )?)
        })
        .instance_method("path", |vm, mc, args| {
            args[0]
                .with_native_state(|io: &NativeIoFile| {
                    vm.new_string(mc, io.path.to_string_lossy().into_owned())
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
