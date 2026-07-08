//! D3a — null retranslation (docs/DIRECT_CALLS_ARCH.md §3.5): with
//! QN_DIRECT_WARM set, warm AOT-IC sites queue their callers, the driver
//! drains the queue between steps, and the registry entry is OVERWRITTEN
//! with identically-generic code. Behavior must not change; VM.stats
//! reports the retranslation count.

use std::process::Command;

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

fn run(warm: Option<&str>) -> (String, String) {
    let dir = std::env::temp_dir().join("qn_direct_calls_a");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hot.qn");
    std::fs::write(&path, HOT).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
    cmd.arg(&path);
    match warm {
        Some(v) => cmd.env("QN_DIRECT_WARM", v),
        None => cmd.env_remove("QN_DIRECT_WARM"),
    };
    let out = cmd.output().expect("run qn");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
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
