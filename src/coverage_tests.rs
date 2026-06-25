use super::*;

#[test]
fn record_line_accumulates_per_file_and_line() {
    let mut cov = CoverageState::new();
    cov.record_line("a.qn", 1);
    cov.record_line("a.qn", 1);
    cov.record_line("a.qn", 2);
    cov.record_line("b.qn", 5);

    assert_eq!(cov.hit_count("a.qn", 1), 2);
    assert_eq!(cov.hit_count("a.qn", 2), 1);
    assert_eq!(cov.hit_count("b.qn", 5), 1);
    assert_eq!(cov.hit_count("a.qn", 9), 0); // line never recorded
    assert_eq!(cov.hit_count("c.qn", 1), 0); // file never recorded
}

#[test]
fn line_totals_counts_found_and_hit() {
    let mut report = CoverageReport::default();
    let f = report.files.entry("x.qn".to_string()).or_default();
    f.lines.insert(1, 5);
    f.lines.insert(2, 0);
    f.lines.insert(3, 1);
    assert_eq!(report.line_totals(), (3, 2));
}

#[test]
fn to_lcov_emits_records_in_deterministic_order() {
    let mut report = CoverageReport::default();
    let f = report.files.entry("foo.qn".to_string()).or_default();
    // Insert out of order; BTreeMaps make the output deterministic regardless.
    f.funcs
        .insert("Foo#baz".to_string(), FnReport { line: 6, hits: 0 });
    f.funcs
        .insert("Foo#bar".to_string(), FnReport { line: 2, hits: 3 });
    f.lines.insert(6, 0);
    f.lines.insert(2, 3);
    f.lines.insert(3, 3);

    let lcov = to_lcov(&report);
    let expected = "\
TN:
SF:foo.qn
FN:2,Foo#bar
FN:6,Foo#baz
FNDA:3,Foo#bar
FNDA:0,Foo#baz
FNF:2
FNH:1
DA:2,3
DA:3,3
DA:6,0
LF:3
LH:2
end_of_record
";
    assert_eq!(lcov, expected);
}

#[test]
fn to_lcov_handles_multiple_files() {
    let mut report = CoverageReport::default();
    report
        .files
        .entry("b.qn".to_string())
        .or_default()
        .lines
        .insert(1, 1);
    report
        .files
        .entry("a.qn".to_string())
        .or_default()
        .lines
        .insert(1, 0);

    let lcov = to_lcov(&report);
    // Files are emitted in sorted order, each as its own SF…end_of_record record.
    let a_pos = lcov.find("SF:a.qn").unwrap();
    let b_pos = lcov.find("SF:b.qn").unwrap();
    assert!(a_pos < b_pos);
    assert_eq!(lcov.matches("end_of_record").count(), 2);
}
