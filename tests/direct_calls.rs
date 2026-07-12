//! D3a — null retranslation (docs/internal/DIRECT_CALLS_ARCH.md §3.5): with
//! QN_DIRECT_WARM set, warm AOT-IC sites queue their callers, the driver
//! drains the queue between steps, and the registry entry is OVERWRITTEN
//! with identically-generic code. Behavior must not change; VM.stats
//! reports the retranslation count.

use std::process::Command;

const W0: &str = r#"
Adder <- {
    add:to: -> { |a: Integer b: Integer ^Integer| ^^ a + b }
};
Driver <- {
    step:with: -> { |x: Integer adder ^Integer|
        var a = adder.add:x to:1;
        ^^ a
    }
};
var d = Driver.new;
var a = Adder.new;
var total = 0;
(0..200000).each:{ |r| total = total + (d.step:r with:a) };
('total=' + total.s).print;
var st = VM.stats.at:'aot';
('retranslated=' + (st.at:'retranslated').s + ' directSites=' + (st.at:'directSites').s).print;
"#;

const HOT: &str = r#"
D3aProbe <- {
    add:to: -> { |a: Integer b: Integer ^Integer| ^^ a + b };
    work: -> { |n: Integer ^Integer|
        var s = 0;
        var i = 0;
        { i < n }.whileDo:{ s = .add:i to:s; i = i + 1 };
        ^^ s
    }
};
var m = D3aProbe.new;
('sum=' + (m.work:200000).s).print;
('retranslated=' + ((VM.stats.at:'aot').at:'retranslated').s).print;
"#;

fn run_src(name: &str, src: &str, envs: &[(&str, &str)]) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("qn_direct_calls_{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hot.qn");
    std::fs::write(&path, src).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
    cmd.arg(&path);
    cmd.env_remove("QN_DIRECT_WARM");
    cmd.env_remove("QN_DIRECT_NULL");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run qn");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn run(warm: Option<&str>) -> (String, String) {
    match warm {
        // The D3a null contract needs the test hook now: production skips
        // retranslations with no baked sites.
        Some(v) => run_src("a", HOT, &[("QN_DIRECT_WARM", v), ("QN_DIRECT_NULL", "1")]),
        None => run_src("a", HOT, &[]),
    }
}

#[test]
fn null_retranslation_preserves_behavior_and_counts() {
    // Tier off (default): correct sum, zero retranslations.
    let (off, err_off) = run(None);
    assert!(
        off.contains("sum=19999900000"),
        "default-off wrong sum:\n{off}\n{err_off}"
    );
    assert!(off.contains("retranslated=0"), "tier ran while off:\n{off}");

    // Forced (=1): identical result, retranslations counted.
    let (on, err_on) = run(Some("1"));
    assert!(
        on.contains("sum=19999900000"),
        "forced-warm wrong sum:\n{on}\n{err_on}"
    );
    let count: i64 = on
        .lines()
        .find_map(|l| l.strip_prefix("retranslated="))
        .expect("stats line")
        .trim()
        .parse()
        .expect("count");
    assert!(
        count >= 1,
        "no retranslations under QN_DIRECT_WARM=1:\n{on}"
    );
}

#[test]
fn w0_direct_edge_bakes_and_preserves_behavior() {
    let (out, err) = run_src("w0", W0, &[("QN_DIRECT_WARM", "64")]);
    assert!(
        out.contains("total=20000100000"),
        "wrong sum:\n{out}\n{err}"
    );
    let direct: i64 = out
        .split("directSites=")
        .nth(1)
        .expect("stats line")
        .trim()
        .parse()
        .unwrap();
    assert!(direct >= 1, "no direct edge baked:\n{out}");

    // Tier off: same result, no machinery.
    let (off, _) = run_src("w0off", W0, &[]);
    assert!(off.contains("total=20000100000"), "off-path sum:\n{off}");
    assert!(off.contains("directSites=0"));
}
