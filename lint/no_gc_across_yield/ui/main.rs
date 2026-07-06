// The fixture's `Gc` mirrors gc_arena's: the walk terminates at the ADT NAME,
// and everything else must reach it through owned structure.
struct Gc<'gc, T: ?Sized>(&'gc T);

struct Value<'gc>(Gc<'gc, ()>);

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

// A plain-data type that merely mentions "Value" in its name owns no Gc
// anywhere; the field walk must not match it.
struct ValueInfo {
    class: String,
}

fn plain_data_not_flagged(yielder: &Yielder, info: ValueInfo) {
    let local_info = info;
    yielder.suspend();
    let _ = local_info.class;
}

// A WRAPPER struct owning Gc data through its fields — the shape the old
// name-substring test missed (`Frame<'gc>` prints without its field types;
// this is exactly how the S0 popped-frame segfault escaped the lint).
struct Frame<'gc> {
    block: Gc<'gc, ()>,
    id: usize,
}

fn wrapper_struct_flagged<'gc>(yielder: &Yielder, f: Frame<'gc>) {
    let popped = f;
    yielder.suspend();
    let _ = popped.id; // popped's Gc fields may be dangling here
}

// Containers reach GC data through generic args (their internals hide the
// payload behind raw pointers, which the walk deliberately skips).
fn container_flagged<'gc>(yielder: &Yielder, vals: Vec<Value<'gc>>) {
    let local_vals = vals;
    yielder.suspend();
    let _ = local_vals.len();
}

fn reassignment_after_yield_not_flagged<'gc>(yielder: &Yielder, a: Value<'gc>, b: Value<'gc>) {
    let mut current = a;
    yielder.suspend();
    current = b; // a WRITE kills the old value; not a hold across the yield
    let _ = current;
}

fn main() {}
