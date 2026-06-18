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

fn main() {}
