//! Regression tests: no RefCell/RefLock borrow may be held while arbitrary Quoin
//! runs, because that code can cooperatively yield — parking the whole task with
//! the borrow guard alive on its suspended Rust stack, where it collides with any
//! other borrow of the same cell ("RefCell already borrowed" panics). Three shapes
//! that used to panic:
//!
//! 1. `finalize_instantiation` held the CLASS borrow while a zero-arg `init` ran
//!    (an `if let` scrutinee temporary lives through the success branch — even in
//!    edition 2024, whose rescope only shortened the `else` path). An `init` that
//!    parks (fiber resume, I/O, sleep) left the borrow live; any other task
//!    instantiating the same class then hit `ensure_field_layout`'s `borrow_mut`.
//!    This is how `Iterator` (whose `init` resumes a fiber) blew up the web soak
//!    under QN_SCHED_STRESS.
//! 2. The same function held the `new:{}` config-block ENV borrow across the init
//!    chain, so an init calling a closure that assigns a config-block local
//!    panicked on the env's `borrow_mut` — no concurrency needed.
//! 3. `String %` with named (Map) arguments held the map's state borrow while
//!    calling `.s` on each value, so a value whose `s` touches the same map
//!    panicked on the map-state `borrow_mut`.

use std::process::Command;

#[test]
fn borrows_are_not_held_across_init_or_render_yields() {
    let script = r#"
var ok = true;

"* -- 1: concurrent instantiation of a class whose init parks ---------------
Napper <- { | @x |
    init -> { Async.sleep:30 }
};
var a = Task.spawn:{ Napper.new:{ var x = 1 }; 'done-a' };
var b = Task.spawn:{ Async.sleep:10; Napper.new:{ var x = 2 }; 'done-b' };
((a.join == 'done-a') && (b.join == 'done-b')).else:{ ok = false; 'FAIL concurrent init'.print };

"* Iterator's init resumes a fiber — the original trigger. Two tasks pulling
"* iterators over sources whose each: parks exercises the same window.
var t1 = Task.spawn:{ #( 1 2 3 4 ).drop:2 };
var t2 = Task.spawn:{ #( 5 6 7 8 ).drop:2 };
((t1.join == #( 3 4 )) && (t2.join == #( 7 8 ))).else:{ ok = false; 'FAIL iterators'.print };

"* -- 2: init runs a closure that assigns a config-block local ---------------
Counter <- { | @bump @count |
    init -> { @bump.value }
};
var c = Counter.new:{
    var count = 0;
    var bump = { count = count + 1 }
};
c.defined?.else:{ ok = false; 'FAIL env-writing init'.print };

"* -- 3: % map rendering where a value's s touches the same map ---------------
var shared = #{};
Sneaky <- {
    s -> { shared.at:'seen' put:true; 'v' }
};
shared.at:'k' put:(Sneaky.new);
var rendered = 'x=%k' % shared;
((rendered == 'x=v') && (shared.at:'seen')).else:{ ok = false; 'FAIL map render'.print };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;

    let dir = std::env::temp_dir();
    let path = dir.join("qn_borrow_across_yield_test.qn");
    std::fs::write(&path, script).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("PASS") && !stdout.contains("FAIL"),
        "script did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
