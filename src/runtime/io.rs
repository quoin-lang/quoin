use crate::arg;
use crate::error::BBError;
use crate::value::{NativeClassBuilder, OpaqueState, Value};

use std::fs::ReadDir;

pub struct IoFolder {
    pub path: String,
    pub iter: Option<ReadDir>,
}

pub fn build_io_folder_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[IO]Folder", Some("Object"))
        .class_method("open:", |vm, mc, args| {
            let path = arg!(args, String, 1);

            let state = OpaqueState::<IoFolder>(IoFolder {
                path: path.to_string(),
                iter: None,
            });

            let clz = arg!(args, Class, 0);
            let value = vm.new_native_state(mc, clz, state);
            Ok(value)
        })
        .instance_method("next", |vm, mc, args| {
            let r = args[0].with_native_state_mut(mc, |io: &mut IoFolder| {
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
                vm.new_string(mc, entry?.file_name().to_string_lossy().into_owned())
            } else {
                vm.new_nil(mc)
            });
        })
}
