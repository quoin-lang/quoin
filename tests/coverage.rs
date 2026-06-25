//! End-to-end test for `qn <file> --coverage`: a fixture with one method that is called
//! and one that is not, proving the LCOV output distinguishes them — i.e. the denominator
//! includes a compiled-but-never-called method (FNDA:0), and defining a method does not
//! count its body as executed (the block-attribution fix).

use std::process::Command;

#[test]
fn coverage_distinguishes_called_and_uncalled_methods() {
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let src = dir.join(format!("quoin_cov_it_{pid}.qn"));
    let out = dir.join(format!("quoin_cov_it_{pid}.lcov"));

    // `used` is called once; `unused` never is. Both bodies sit on their own line.
    std::fs::write(
        &src,
        "Thing <- {\n    used -> { 42 };\n    unused -> { 99 };\n};\n(Thing.new).used;\n",
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&src)
        .arg("--coverage")
        .arg(format!("--coverage-out={}", out.display()))
        .status()
        .expect("run qn");
    assert!(status.success(), "qn exited with failure");

    let lcov = std::fs::read_to_string(&out).expect("coverage output written");
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&out);

    // Isolate the fixture's own SF…end_of_record record (the report also covers the
    // stdlib loaded as the prelude).
    let sf = format!("SF:{}", src.display());
    let record = lcov
        .split("end_of_record")
        .find(|r| r.contains(&sf))
        .unwrap_or_else(|| panic!("no SF record for the fixture in:\n{lcov}"));

    // `used` called once, `unused` never — function and line coverage agree.
    assert!(
        record.contains("FNDA:1,Thing#used"),
        "expected used called once:\n{record}"
    );
    assert!(
        record.contains("FNDA:0,Thing#unused"),
        "expected unused never called:\n{record}"
    );
    assert!(
        record.contains("DA:2,1"),
        "used's body line should be hit once:\n{record}"
    );
    assert!(
        record.contains("DA:3,0"),
        "unused's body line should be unhit:\n{record}"
    );
}
