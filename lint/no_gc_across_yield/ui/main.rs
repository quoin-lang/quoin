struct Value<'gc>(&'gc ());

struct Yielder;
impl Yielder {
    fn suspend(&self) {}
}

fn bad_func<'gc>(yielder: &Yielder, val: Value<'gc>) {
    let local_val = val;
    yielder.suspend();
    let _ = local_val; // Use local_val after suspend!
}

fn good_func<'gc>(yielder: &Yielder, val: Value<'gc>) {
    let local_val = val;
    let _ = local_val; // Use local_val before suspend!
    yielder.suspend();
}

// Parameters are the caller's values (rooted by the caller's conventions —
// active_native_args, callee frames); the hold is flagged at the caller instead.
fn param_not_flagged<'gc>(yielder: &Yielder, val: Value<'gc>) {
    yielder.suspend();
    let _ = val;
}

// A plain-data type that merely mentions "Value" in its name carries nothing
// collectable; the matcher requires a generic position (`Value<…>` / `Gc<…>`).
struct ValueInfo {
    class: String,
}

fn plain_data_not_flagged(yielder: &Yielder, info: ValueInfo) {
    let local_info = info;
    yielder.suspend();
    let _ = local_info.class;
}

fn main() {}
