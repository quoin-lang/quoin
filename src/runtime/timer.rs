use crate::value::NativeClassBuilder;

use crate::clock::Instant;

pub fn build_timer_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Timer", Some("Object"))
        .abstract_class()
        .class_doc(
            "A one-selector stopwatch: run a block, get its elapsed time.\n\n\
             For finer control (start now, read later, compare points), use `Instant` and \
             `Duration` directly.",
        )
        .sdk_class_method("time:", |host, _receiver, args| {
            let start = Instant::now();
            // `execute_block` validates that args[0] is a block.
            host.execute_block(args[0], Vec::new(), None)?;
            let elapsed = start.elapsed().as_micros() as i64;
            Ok(host.new_int(elapsed))
        })
        .doc(
            "Run the block and answer how long it took, in whole microseconds (an Integer). \
             Measured on the monotonic clock, so wall-clock adjustments cannot skew it.\n\n\
             ```\n\
             Timer.time:{ (1..1000).each:{ |i| i } }     \"* the elapsed microseconds\n\
             ```",
        )
}
