use crate::arg;
use crate::value::{NativeClassBuilder, Value};

use std::time::Instant;

pub fn build_timer_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Timer", Some("Object"))
        //
        .class_method("time:", |vm, mc, _receiver, args| {
            let block = arg!(args, Block, 0);
            let start = Instant::now();
            vm.execute_block(mc, block, Vec::new(), None)?;
            let elapsed = start.elapsed().as_millis() as i64;
            Ok(vm.new_int(mc, elapsed))
        })
}
