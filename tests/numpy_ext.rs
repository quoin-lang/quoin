//! Integration tests for the `numpy` extension package (`quoin_packages/numpy`) — NumPy-backed
//! n-dimensional arrays as `[NumPy]Array`, over the Python SDK (Phase 3 extension-backed classes).
//!
//! Gated on a `python3` that can import `flatbuffers` (the SDK runtime) *and* `numpy`; skips
//! cleanly otherwise (e.g. CI without Python set up), like the polyglot tests in `extension.rs`.

use std::process::Command;

/// Run a `.qn` script through the `qn` binary and assert it printed `PASS`. Retries a few times:
/// these tests spawn a `qn` subprocess that itself spawns a Python subprocess, and under the full
/// suite's process load the child can occasionally be killed before it runs (see `extension.rs`).
fn assert_script_passes(name: &str, script: &str) {
    const ATTEMPTS: u32 = 4;
    let mut last_diag = String::new();
    for attempt in 1..=ATTEMPTS {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, script).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_qn"))
            .arg(&path)
            .output()
            .expect("run qn");
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("PASS") {
            return;
        }
        last_diag = format!(
            "status: {:?}\nstdout:\n{stdout}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if attempt < ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(100 * attempt as u64));
        }
    }
    panic!("numpy script did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

/// True if `python3` can import both `flatbuffers` and `numpy` — the package's dependencies.
fn numpy_fixture_runnable() -> bool {
    Command::new("python3")
        .args(["-c", "import flatbuffers, numpy"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The whole slice-1 surface: creation, introspection, element access (scalar for 1-D, a row
/// instance for n-D), the materialization exit ramps (`toList` / `toArray` -> host `Array`), and
/// a catchable error that leaves the extension alive.
#[test]
fn numpy_array_skeleton() {
    if !numpy_fixture_runnable() {
        eprintln!(
            "skipping numpy_array_skeleton: python3 with `flatbuffers` + `numpy` unavailable"
        );
        return;
    }
    let pkg = concat!(env!("CARGO_MANIFEST_DIR"), "/quoin_packages/numpy");
    let script = format!(
        r#"
var ok = true;
var e = Extension.loadPackage:'{pkg}';

"* creation + introspection
var z = [NumPy]Array.zeros:#( 2 3 );
(z.shape == #( 2 3 )).else:{{ ok = false }};
(z.size == 6).else:{{ ok = false }};
(z.ndim == 2).else:{{ ok = false }};
(z.dtype == 'float64').else:{{ ok = false }};
((([NumPy]Array.ones:4).shape) == #( 4 )).else:{{ ok = false }};

"* fromList infers dtype (all-int -> int64, floats -> float64)
var v = [NumPy]Array.fromList:#( 1.0 2.0 3.0 );
(v.dtype == 'float64').else:{{ ok = false }};
((v.at:1) == 2.0).else:{{ ok = false }};
(v.toList == #( 1.0 2.0 3.0 )).else:{{ ok = false }};
(v.s == 'Array(float64 3) [1. 2. 3.]').else:{{ ok = false }};

var ints = [NumPy]Array.arange:5;
(ints.dtype == 'int64').else:{{ ok = false }};
((ints.at:4) == 4).else:{{ ok = false }};

((([NumPy]Array.linspace:0.0 to:1.0 count:5).at:2) == 0.5).else:{{ ok = false }};
(([NumPy]Array.random:#( 4 )).size == 4).else:{{ ok = false }};

"* n-D: `at:` on a 2-D array returns a row as a new [NumPy]Array instance
var m = [NumPy]Array.fromList:#( #( 1 2 ) #( 3 4 ) );
(m.ndim == 2).else:{{ ok = false }};
(m.dtype == 'int64').else:{{ ok = false }};
((m.at:1).toList == #( 3 4 )).else:{{ ok = false }};

"* the bulk exit ramp: toArray -> a host `Array` (data plane, ext -> host)
var host = v.toArray;
(host.length == 3).else:{{ ok = false }};
(host.sum == 6.0).else:{{ ok = false }};

"* a numpy error is a catchable Quoin error, and the extension SURVIVES it
var caught = {{ v.at:99 }}.catch:{{ |ex| 'caught' }};
(caught == 'caught').else:{{ ok = false }};
((v.at:0) == 1.0).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_numpy_skeleton_test.qn", &script);
}

/// The lazy expression layer (init.qn): operators build a host-side DAG with no socket traffic;
/// a force point (a reduction / toList / eval) ships the whole graph in ONE `evalGraph:` send.
/// Covers: broadcasting with scalars, chained arith, diamonds (shared subexpressions), memoized
/// `eval` results re-entering later graphs, NumPy promotion (int / float), the multi-base selector
/// ladder, and the >8-distinct-arrays error.
#[test]
fn numpy_lazy_expressions() {
    if !numpy_fixture_runnable() {
        eprintln!(
            "skipping numpy_lazy_expressions: python3 with `flatbuffers` + `numpy` unavailable"
        );
        return;
    }
    let pkg = concat!(env!("CARGO_MANIFEST_DIR"), "/quoin_packages/numpy");
    let script = format!(
        r#"
var ok = true;
var e = Extension.loadPackage:'{pkg}';

var a = [NumPy]Array.fromList:#( 1.0 2.0 3.0 );
var b = [NumPy]Array.fromList:#( 4.0 5.0 6.0 );

"* chained elementwise ops + scalar broadcasting -> one round trip at toList
(((a + b) * 2.0).toList == #( 10.0 14.0 18.0 )).else:{{ ok = false }};
((a / 2.0).toList == #( 0.5 1.0 1.5 )).else:{{ ok = false }};
(a.neg.toList == #( (-1.0) (-2.0) (-3.0) )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 4.0 9.0 )).sqrt).toList == #( 2.0 3.0 )).else:{{ ok = false }};

"* reductions collapse the whole chain to a scalar in one send
((a * a).sum == 14.0).else:{{ ok = false }};
(((a - b) * (a - b)).mean == 9.0).else:{{ ok = false }};
((a.pow:2.0).sum == 14.0).else:{{ ok = false }};

"* eager reductions directly on a resident array
(a.sum == 6.0).else:{{ ok = false }};
(a.mean == 2.0).else:{{ ok = false }};
(a.max == 3.0).else:{{ ok = false }};

"* a diamond: d is referenced twice but serialized/evaluated once
var d = a + b;
(((d * d).sum) == 155.0).else:{{ ok = false }};

"* eval materializes + memoizes; the result mixes back into new expressions as a base
var m = (a + b).eval;
(m.toList == #( 5.0 7.0 9.0 )).else:{{ ok = false }};
((d + 1.0).toList == #( 6.0 8.0 10.0 )).else:{{ ok = false }};

"* NumPy promotion: int64 / float -> float64
((([NumPy]Array.arange:4) / 2.0).toList == #( 0.0 0.5 1.0 1.5 )).else:{{ ok = false }};

"* the selector ladder carries up to 8 distinct arrays; more is a clear, catchable error
var c1 = [NumPy]Array.fromList:#( 1.0 );
var c2 = [NumPy]Array.fromList:#( 2.0 );
var c3 = [NumPy]Array.fromList:#( 3.0 );
var c4 = [NumPy]Array.fromList:#( 4.0 );
var c5 = [NumPy]Array.fromList:#( 5.0 );
var c6 = [NumPy]Array.fromList:#( 6.0 );
((((((((a + b) + c1) + c2) + c3) + c4) + c5) * 1.0).toList == #( 20.0 22.0 24.0 ))
    .else:{{ ok = false }};
var c7 = [NumPy]Array.fromList:#( 7.0 );
var wide = (((((((a + b) + c1) + c2) + c3) + c4) + c5) + c6) + c7;
var caught = {{ wide.toList; 'no-throw' }}.catch:{{ |ex| 'caught' }};
(caught == 'caught').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_numpy_lazy_test.qn", &script);
}

/// `use numpy:*` resolves the package folder from the default search root (`./quoin_packages/`),
/// relative to the VM's cwd — which under `cargo test` is the workspace root.
#[test]
fn numpy_package_via_use() {
    if !numpy_fixture_runnable() {
        eprintln!(
            "skipping numpy_package_via_use: python3 with `flatbuffers` + `numpy` unavailable"
        );
        return;
    }
    let script = r#"
use numpy:*;
var ok = ([NumPy]Array.arange:10).size == 10;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_script_passes("qn_numpy_use_test.qn", script);
}
