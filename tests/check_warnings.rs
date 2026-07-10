//! `qn check qnlib/warnings.qn` is the checker's demonstration gallery
//! (docs/TYPE_SYSTEM_ARCH.md): every "should warn" entry must fire and every "must emit NO
//! warning" entry must stay silent. Nothing ran it in CI, which is how two drifts slipped in
//! unnoticed (RELEASE_PREP Tier 4b): `badEachParam` fell silent when `each:` fusion started
//! bypassing block-param seeding, and `wellFormed:` gained a false positive when a literal's
//! bare `List` static type hit the width rule. This pins the gallery: the exact warning
//! count, the drift-prone messages, and the continued silence of the once-false-positive
//! line. A count change is deliberate friction — update it together with the gallery.

use std::process::Command;

#[test]
fn gallery_warnings_match_reality() {
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args(["check", "qnlib/warnings.qn"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn check");
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(1),
        "the gallery must exit 1 — it exists to have diagnostics\n{stderr}"
    );

    // The full census: a NEW warning is a false positive, a LOST one is a regressed check.
    let count = stderr.lines().filter(|l| l.contains(": warning:")).count();
    assert_eq!(count, 25, "gallery warning count drifted\n{stderr}");

    // The historically drift-prone checks, by message.
    for needle in [
        // badEachParam: the fused `each:` path must seed the block param's element type.
        "`List(String)` rejects a `Integer` element",
        // the multimethod arg check (`5.pow:'x'`).
        "no `pow:` variant on `Integer` accepts `String`",
        // a collection literal's statically-visible bad element.
        "`List(Integer)` rejects a `String` element",
        // compile-time MNU on a sealed class.
        "does not respond to `nopeMethod`",
        // return-type covariance.
        "override of `defined?` returns `String`",
    ] {
        assert!(
            stderr.contains(needle),
            "gallery lost a warning ({needle})\n{stderr}"
        );
    }

    // `wellFormed:` sits in a must-NOT-warn section and once drew a width false positive
    // (`expected List(Integer), found List` on its literal return) — no warning may cite
    // its line. Located by content so gallery edits don't silently retarget the assert.
    let gallery =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/qnlib/warnings.qn"))
            .expect("read the gallery");
    let well_formed_line = gallery
        .lines()
        .position(|l| l.contains("wellFormed:"))
        .expect("gallery defines wellFormed:")
        + 1;
    assert!(
        !stderr.contains(&format!("warnings.qn:{well_formed_line}:")),
        "must-not-warn `wellFormed:` (line {well_formed_line}) drew a warning\n{stderr}"
    );
}
