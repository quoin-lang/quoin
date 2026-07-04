// Mock cell + guard types (detection is name-based: `RefLock`/`RefCell` receivers
// producing `Ref<`/`RefMut<` guards) and a Yielder whose `suspend` is the yield point.
struct Ref<'a, T>(&'a T);
struct RefMut<'a, T>(&'a mut T);
struct RefLock<T>(T);
impl<T> RefLock<T> {
    fn borrow(&self) -> Ref<'_, T> {
        Ref(&self.0)
    }
    fn borrow_mut(&mut self) -> RefMut<'_, T> {
        RefMut(&mut self.0)
    }
}

struct Yielder;
impl Yielder {
    fn suspend(&self) {}
}

struct Class {
    init: Option<u32>,
}

// Yield-capability must propagate through the local call graph.
fn helper_that_yields(y: &Yielder) {
    y.suspend();
}

fn takes_and_yields(y: &Yielder, _v: Option<u32>) {
    y.suspend();
}

// BAD: named guard still alive at the suspend (dropped only afterwards).
fn named_guard_across_yield(y: &Yielder, cell: &RefLock<Class>) {
    let g = cell.borrow();
    y.suspend();
    drop(g);
}

// GOOD: guard explicitly dropped before the suspend.
fn named_guard_dropped_first(y: &Yielder, cell: &RefLock<Class>) {
    let g = cell.borrow();
    drop(g);
    y.suspend();
}

// GOOD: guard's whole scope ends before the suspend.
fn named_guard_scoped(y: &Yielder, cell: &RefLock<Class>) {
    {
        let g = cell.borrow();
        drop(g);
    }
    y.suspend();
}

// BAD: guard alive across a call that only *transitively* suspends.
fn named_guard_across_transitive_yield(y: &Yielder, cell: &RefLock<Class>) {
    let g = cell.borrow();
    helper_that_yields(y);
    drop(g);
}

// BAD: an `if let` scrutinee temporary lives through the success branch —
// the `finalize_instantiation` shape.
fn scrutinee_guard_if_let(y: &Yielder, cell: &RefLock<Class>) {
    if let Some(_v) = cell.borrow().0.init {
        y.suspend();
    }
}

// GOOD: a plain-bool condition's temporaries die before the branch runs.
fn plain_condition_is_fine(y: &Yielder, cell: &RefLock<Class>) {
    if cell.borrow().0.init.is_some() {
        y.suspend();
    }
}

// BAD: a `match` scrutinee temporary lives through every arm.
fn scrutinee_guard_match(y: &Yielder, cell: &RefLock<Class>) {
    match cell.borrow().0.init {
        Some(_) => y.suspend(),
        None => {}
    }
}

// GOOD: hoisting the lookup out of the scrutinee ends the temporary at the `;`.
fn hoisted_scrutinee_is_fine(y: &Yielder, cell: &RefLock<Class>) {
    let v = cell.borrow().0.init;
    if let Some(_v) = v {
        y.suspend();
    }
}

// BAD: guard temporary nested in a yield-capable call's arguments — alive for
// the whole call.
fn guard_inside_yielding_call(y: &Yielder, cell: &RefLock<Class>) {
    takes_and_yields(y, cell.borrow().0.init);
}

// GOOD: borrowing after the yield.
fn borrow_after_yield(y: &Yielder, cell: &RefLock<Class>) {
    y.suspend();
    let g = cell.borrow();
    drop(g);
}

// GOOD: mutable guards follow the same rules.
fn ref_mut_dropped_first(y: &Yielder, cell: &mut RefLock<Class>) {
    let g = cell.borrow_mut();
    drop(g);
    y.suspend();
}

fn main() {}
