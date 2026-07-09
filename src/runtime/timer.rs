use crate::value::NativeClassBuilder;

use std::time::Instant;

pub fn build_timer_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Timer", Some("Object"))
        .abstract_class()
        //
        .sdk_class_method("time:", |host, _receiver, args| {
            let start = Instant::now();
            // `execute_block` validates that args[0] is a block.
            host.execute_block(args[0], Vec::new(), None)?;
            let elapsed = start.elapsed().as_micros() as i64;
            Ok(host.new_int(elapsed))
        })
}
