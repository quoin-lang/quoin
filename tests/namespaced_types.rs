//! Integration test for namespaced class names in type annotations. All four type
//! positions (`|x:[Ns]T|`, `var x: [Ns]T`, `^[Ns]T`, block-local `- x:[Ns]T`) accept an
//! optionally namespaced `type_ref`, and the annotation resolves exactly like an
//! expression-position global: a bare name means the root namespace (never a leaf-name
//! match against some `[X]Name`), `[Ns]Name` means that namespace, and `[/]Name` is the
//! explicit root. Also pins the dispatch-scoring rule that an exactly-typed variant
//! beats the untyped catch-all (`|x|` ⇒ `:Object`) for user-class arguments.

use std::process::Command;

#[test]
fn namespaced_type_annotations_dispatch_and_catch() {
    let script = r#"
var ok = true;

[Web]Thing <- { poke -> { 'web' } };
Thing <- { poke -> { 'root' } };
[Deep/Nest]Gadget <- {};
[Iso]Only <- {};
Error <- [Web]Halt <- {};

Probe <- {
    describe: -> { |x:[Web]Thing| 'web thing' };
    describe: --> { |x:Thing| 'root thing' };
    describe: --> { |x| 'other' };

    maker: -> { |n ^[Web]Thing| [Web]Thing.new };

    "* Bare `Only` names the ROOT class only; [Iso]Only must not leaf-match it.
    pick: -> { |x:Only| 'bare-matched' };
    pick: --> { |x| 'fallback' };

    "* Explicit root: [/]Thing is the same annotation as bare Thing.
    grab: -> { |x:[/]Thing| 'explicit-root' };
    grab: --> { |x| 'no' }
};

var p = Probe.new;

"* Multimethod dispatch distinguishes [Web]Thing / root Thing / everything else.
((p.describe:([Web]Thing.new)) == 'web thing').else:{ ok = false };
((p.describe:(Thing.new)) == 'root thing').else:{ ok = false };
((p.describe:42) == 'other').else:{ ok = false };
((p.describe:([Deep/Nest]Gadget.new)) == 'other').else:{ ok = false };

"* A subclass instance matches the parent-typed variant by distance, not the catch-all.
Thing <- SubThing <- {};
((p.describe:(SubThing.new)) == 'root thing').else:{ ok = false };

"* Typed catch on a namespaced Error subclass; a non-matching handler re-raises.
var c1 = { [Web]Halt.throw:'stop' }.catch:{ |e:[Web]Halt| 'caught:' + e.message };
(c1 == 'caught:stop').else:{ ok = false };
var c2 = { { ValueError.throw:'nope' }.catch:{ |e:[Web]Halt| 'wrong' } }.catch:{ |e| 'outer' };
(c2 == 'outer').else:{ ok = false };

"* Typed declaration, block return type, and typed block-local all execute.
var t: [Web]Thing = [Web]Thing.new;
(t.poke == 'web').else:{ ok = false };
((p.maker:1).poke == 'web').else:{ ok = false };
var blk = { |a - g:[Deep/Nest]Gadget| g = [Deep/Nest]Gadget.new; g };
([Deep/Nest]Gadget ~ (blk.value:0)).else:{ ok = false };

"* Bare hint = root namespace only (no [Iso]Only leaf match; root Only is undefined).
((p.pick:([Iso]Only.new)) == 'fallback').else:{ ok = false };

"* [/]Thing behaves exactly like bare Thing.
((p.grab:(Thing.new)) == 'explicit-root').else:{ ok = false };
((p.grab:([Web]Thing.new)) == 'no').else:{ ok = false };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;

    let dir = std::env::temp_dir();
    let path = dir.join("qn_namespaced_types_test.qn");
    std::fs::write(&path, script).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("PASS"),
        "script did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
