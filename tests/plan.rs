//! Plan (the join graph, §13.5) — the pieces qnlib tests can't pin:
//! REAL process-leaf termination under cancelRest (a would-be-60s child
//! must die and the await must return fast), and the `valueWithSelfOrArg:`
//! self-binding fix the arc surfaced (fields inside parameterized
//! combinator blocks resolve LEXICALLY, before and after a park).

use std::process::Command;

fn run_script(name: &str, script: &str, units: &[(&str, &str)]) -> String {
    let dir = std::env::temp_dir().join(format!("qn_plan_{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut script = script.to_string();
    for (unit_name, source) in units {
        let path = dir.join(unit_name);
        std::fs::write(&path, source).unwrap();
        script = script.replace(&format!("@{unit_name}@"), path.to_str().unwrap());
    }
    let main_path = dir.join("main.qn");
    std::fs::write(&main_path, &script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&main_path)
        .output()
        .expect("run qn");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    text
}

#[test]
fn cancel_rest_terminates_process_leaves() {
    // The child would sleep 60s; the whole await must come back in seconds
    // (test timeout is the backstop) with the process leaf killed.
    let script = r#"
var r = {
    (Plan.all:#(
        (Plan.process:'@slow.qn@' label:'victim')
        (Plan.task:{ Async.sleep:50; 'boom'.throw })
    )).await; 'no-error'
}.catch:{ |e| e.s };
Async.sleep:300;
var row = (VM.ps.at:'workers').at:0;
('err=' + (r.contains?:'boom').s
    + ' dead=' + ((row.at:'state') == 'exited').s
    + ' label=' + (row.at:'label')).print;
"#;
    let out = run_script(
        "terminate",
        script,
        &[("slow.qn", "Async.sleep:60000;\n'survived'\n")],
    );
    assert!(
        out.contains("err=true dead=true label=victim"),
        "cancelRest did not terminate the process leaf:\n{out}"
    );
}

#[test]
fn self_or_arg_keeps_lexical_self_for_parameterized_blocks() {
    // The latent VM bug this arc surfaced: `valueWithSelfOrArg:`'s
    // interpreted path bound self to the ITEM even when the block took a
    // parameter, so `@field` inside `each:`/`collect:` blocks read the
    // item's (nonexistent) fields — visible whenever the AOT/devirt paths
    // didn't run, and forced by any PARK inside the block. Both forms, both
    // sides of a park:
    let script = r#"
T <- { |@f|
    init -> { @f = 'set' };
    go -> {
        var pre = nil;
        var post = nil;
        (0..1).each:{ |j|
            pre = @f;
            Async.sleep:5;
            post = @f
        };
        ('pre=' + pre.s + ' post=' + post.s).print
    }
};
T.new.go;
"* the parameterless implicit-self form still binds the item
P <- { |@n| init: -> { |v| @n = v }; n -> { @n } };
var picked = #( (P.new:{ var v = 10 }) (P.new:{ var v = 20 }) ).collect:{ .n };
('implicit=' + picked.s).print;
"#;
    let out = run_script("selforarg", script, &[]);
    assert!(
        out.contains("pre=set post=set"),
        "lexical self lost in parameterized block:\n{out}"
    );
    assert!(
        out.contains("implicit=#(10 20)"),
        "implicit-self form broken:\n{out}"
    );
}
