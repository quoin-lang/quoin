// The fixture's `Gc` mirrors gc_arena's: the walk terminates at the ADT NAME,
// and everything else must reach it through owned structure.
struct Gc<'gc, T: ?Sized>(&'gc T);

struct Value<'gc>(Gc<'gc, ()>);

fn fresh_value() -> Value<'static> {
    Value(Gc(&()))
}

struct Yielder;
impl Yielder {
    fn suspend(&self) {}
}

// A value CREATED here (not a parameter copy) held across a suspend: the
// canonical hold — nothing roots it while the fiber is parked.
fn bad_func(yielder: &Yielder) {
    let local_val = fresh_value();
    yielder.suspend();
    let _ = local_val; // Use local_val after suspend!
}

fn good_func(yielder: &Yielder) {
    let local_val = fresh_value();
    let _ = local_val; // Use local_val before suspend!
    yielder.suspend();
}

// Parameters are the caller's values (rooted by the caller's conventions —
// active_native_args, callee frames); the hold is flagged at the caller
// instead. A bare COPY of a param, and a binding DESTRUCTURED from one, are
// the same rooted heap value under another name and inherit the exemption.
fn param_not_flagged<'gc>(yielder: &Yielder, val: Value<'gc>) {
    yielder.suspend();
    let _ = val;
}

fn param_copy_not_flagged<'gc>(yielder: &Yielder, val: Value<'gc>) {
    let local_val = val;
    yielder.suspend();
    let _ = local_val;
}

enum Recv<'gc> {
    Obj(Value<'gc>),
    #[allow(dead_code)]
    Nothing,
}

fn param_destructure_not_flagged<'gc>(yielder: &Yielder, recv: Recv<'gc>) {
    if let Recv::Obj(inner) = recv {
        yielder.suspend();
        let _ = inner; // rooted by the caller, same as `recv` itself
    }
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
// this is exactly how the S0 popped-frame segfault escaped the lint). The
// frame comes from a function (a popped frame), not a parameter.
struct Frame<'gc> {
    block: Gc<'gc, ()>,
    id: usize,
}

fn pop_frame() -> Frame<'static> {
    Frame {
        block: Gc(&()),
        id: 0,
    }
}

fn wrapper_struct_flagged(yielder: &Yielder) {
    let popped = pop_frame();
    yielder.suspend();
    let _ = popped.id; // popped's Gc fields may be dangling here
}

// Containers reach GC data through generic args (their internals hide the
// payload behind raw pointers, which the walk deliberately skips).
fn container_flagged(yielder: &Yielder) {
    let local_vals = vec![fresh_value()];
    yielder.suspend();
    let _ = local_vals.len();
}

fn reassignment_after_yield_not_flagged(yielder: &Yielder) {
    let mut current = fresh_value();
    yielder.suspend();
    current = fresh_value(); // a WRITE kills the old value; not a hold
    let _ = current;
}

// A binding BORN FROM a yielding call (if-let over its result) is not a hold
// across that yield.
fn yielding_source(yielder: &Yielder) -> Option<Value<'static>> {
    yielder.suspend();
    None
}

fn scrutinee_birth_not_flagged(yielder: &Yielder) {
    if let Some(v) = yielding_source(yielder) {
        let _ = v;
    }
}

// A local passed INTO the yielding call is rooted through it (callee
// frames/envs hold it for the duration); the use after the call is safe.
fn yield_with_arg(yielder: &Yielder, v: Value<'_>) {
    let _ = v;
    yielder.suspend();
}

fn arg_of_yield_not_flagged(yielder: &Yielder) {
    let conn = fresh_value();
    yield_with_arg(yielder, conn);
    let _ = conn; // rooted through the call it was an argument of
}

fn main() {}
