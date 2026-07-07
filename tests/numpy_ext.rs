//! Integration tests for the `numpy` extension package (`quoin_packages/numpy`) — NumPy-backed
//! n-dimensional arrays as `[NumPy]Array`, over the Python SDK (Phase 3 extension-backed classes).
//!
//! Gated on a `python3` that can import `msgpack` (the SDK's wire codec) *and* `numpy`; skips
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

/// True if `python3` can import both `msgpack` and `numpy` — the package's dependencies.
fn numpy_fixture_runnable() -> bool {
    Command::new("python3")
        .args(["-c", "import msgpack, numpy"])
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
        eprintln!("skipping numpy_array_skeleton: python3 with `msgpack` + `numpy` unavailable");
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
/// `eval` results re-entering later graphs, NumPy promotion (int / float), and a 9-distinct-base
/// graph in one send (base nodes carry live-instance references; no arity ceiling).
#[test]
fn numpy_lazy_expressions() {
    if !numpy_fixture_runnable() {
        eprintln!("skipping numpy_lazy_expressions: python3 with `msgpack` + `numpy` unavailable");
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

"* base nodes carry live-instance references, so one send spans ANY number of distinct
"* arrays (the old 8-slot selector ladder is gone — 9 bases here, one round trip)
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
(wide.toList == #( 33.0 35.0 37.0 )).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_numpy_lazy_test.qn", &script);
}

/// Slice-3 vocabulary: `matMul:` (matrix/vector/dot), axis reductions (which return arrays and
/// so STAY lazy, composing into the same graph), the scalar reductions (argMin/argMax/std/prod),
/// the widened unary set, `mod:`, and dtype promotion through int ops.
#[test]
fn numpy_vocabulary() {
    if !numpy_fixture_runnable() {
        eprintln!("skipping numpy_vocabulary: python3 with `msgpack` + `numpy` unavailable");
        return;
    }
    let pkg = concat!(env!("CARGO_MANIFEST_DIR"), "/quoin_packages/numpy");
    let script = format!(
        r#"
var ok = true;
var e = Extension.loadPackage:'{pkg}';

"* matMul: matrix x matrix, matrix x vector, and 1-D dot (a scalar)
var m = [NumPy]Array.fromList:#( #( 1.0 2.0 ) #( 3.0 4.0 ) );
var m2 = [NumPy]Array.fromList:#( #( 5.0 6.0 ) #( 7.0 8.0 ) );
((m.matMul:m2).toList == #( #( 19.0 22.0 ) #( 43.0 50.0 ) )).else:{{ ok = false }};
var v = [NumPy]Array.fromList:#( 1.0 1.0 );
((m.matMul:v).toList == #( 3.0 7.0 )).else:{{ ok = false }};
"* a 1-D dot yields a scalar only at force time — matMul: itself is lazy, so eval it
(((v.matMul:v).eval) == 2.0).else:{{ ok = false }};

"* matmul composes lazily with elementwise ops — still one send
(((m.matMul:v) + 1.0).toList == #( 4.0 8.0 )).else:{{ ok = false }};

"* axis reductions return arrays and stay in the graph
((m.sum:0).toList == #( 4.0 6.0 )).else:{{ ok = false }};
((m.sum:1).toList == #( 3.0 7.0 )).else:{{ ok = false }};
((m.mean:0).toList == #( 2.0 3.0 )).else:{{ ok = false }};
(((m.sum:0) * 10.0).toList == #( 40.0 60.0 )).else:{{ ok = false }};

"* scalar reductions force
var a = [NumPy]Array.fromList:#( 3.0 1.0 4.0 1.0 5.0 );
(a.argMax == 4).else:{{ ok = false }};
(a.argMin == 1).else:{{ ok = false }};
(a.prod == 60.0).else:{{ ok = false }};
(a.std == 1.6).else:{{ ok = false }};

"* widened elementwise set
(([NumPy]Array.fromList:#( 0.0 )).cos.toList == #( 1.0 )).else:{{ ok = false }};
(([NumPy]Array.fromList:#( 1.4 2.6 )).floor.toList == #( 1.0 2.0 )).else:{{ ok = false }};
(([NumPy]Array.fromList:#( 1.4 2.6 )).ceil.toList == #( 2.0 3.0 )).else:{{ ok = false }};
(([NumPy]Array.fromList:#( 1.4 2.6 )).round.toList == #( 1.0 3.0 )).else:{{ ok = false }};
(([NumPy]Array.fromList:#( (-3.0) 0.0 5.0 )).sign.toList == #( (-1.0) 0.0 1.0 ))
    .else:{{ ok = false }};
((([NumPy]Array.fromList:#( 7 8 9 )).mod:3).toList == #( 1 2 0 )).else:{{ ok = false }};

"* promotion: int ops keep int64; mean promotes to float
var ints = [NumPy]Array.arange:5;
((ints * 2).dtype == 'int64').else:{{ ok = false }};
(ints.mean == 2.0).else:{{ ok = false }};

"* centering: the forced scalar re-enters the next graph as a constant
var c = [NumPy]Array.fromList:#( 1.0 2.0 3.0 );
((c - c.mean).toList == #( (-1.0) 0.0 1.0 )).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_numpy_vocab_test.qn", &script);
}

/// Slice-4 shape ops and slicing — all lazy nodes: transpose/flatten/reshape:, from:to: (first
/// axis), row:/col:. Shape errors (e.g. a broadcast mismatch) surface AT FORCE TIME as catchable
/// errors carrying numpy's message, and the extension survives them. (Host-side shape inference
/// was deliberately cut: base shapes would cost a round trip each without a host-side cache.)
#[test]
fn numpy_shapes_and_slicing() {
    if !numpy_fixture_runnable() {
        eprintln!(
            "skipping numpy_shapes_and_slicing: python3 with `msgpack` + `numpy` unavailable"
        );
        return;
    }
    let pkg = concat!(env!("CARGO_MANIFEST_DIR"), "/quoin_packages/numpy");
    let script = format!(
        r#"
var ok = true;
var e = Extension.loadPackage:'{pkg}';

var m = [NumPy]Array.fromList:#( #( 1.0 2.0 ) #( 3.0 4.0 ) );
(m.transpose.toList == #( #( 1.0 3.0 ) #( 2.0 4.0 ) )).else:{{ ok = false }};
((([NumPy]Array.arange:6).reshape:#( 2 3 )).toList == #( #( 0 1 2 ) #( 3 4 5 ) ))
    .else:{{ ok = false }};
((([NumPy]Array.arange:6).reshape:#( 2 3 )).flatten.toList == #( 0 1 2 3 4 5 ))
    .else:{{ ok = false }};

var a = [NumPy]Array.arange:10;
((a.from:2 to:5).toList == #( 2 3 4 )).else:{{ ok = false }};
((m.row:1).toList == #( 3.0 4.0 )).else:{{ ok = false }};
((m.col:0).toList == #( 1.0 3.0 )).else:{{ ok = false }};

"* shape ops compose lazily with the rest of the graph — still one send per force
((m.transpose.matMul:m).toList == #( #( 10.0 14.0 ) #( 14.0 20.0 ) )).else:{{ ok = false }};
(((a.from:0 to:3) + (a.from:3 to:6)).toList == #( 3 5 7 )).else:{{ ok = false }};
(((([NumPy]Array.arange:6).reshape:#( 2 3 )).sum:1).toList == #( 3 12 )).else:{{ ok = false }};

"* a broadcast mismatch errors at force, catchably, and the extension survives
var b3 = [NumPy]Array.fromList:#( 1.0 2.0 3.0 );
var b2 = [NumPy]Array.fromList:#( 1.0 2.0 );
var caught = {{ (b3 + b2).toList; 'no-throw' }}.catch:{{ |ex| 'caught' }};
(caught == 'caught').else:{{ ok = false }};
(b3.sum == 6.0).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_numpy_shapes_test.qn", &script);
}

/// Slice-5 masks: comparisons on arrays/exprs are ELEMENTWISE (NumPy semantics) and build lazy
/// bool-mask nodes; and:/or:/not combine them; select: does boolean indexing; where:else: is the
/// functional conditional; any/all/sum reduce them. Masks materialize as Booleans via toList and
/// as int64 0/1 via toArray (the wire has no bool dtype).
#[test]
fn numpy_masks() {
    if !numpy_fixture_runnable() {
        eprintln!("skipping numpy_masks: python3 with `msgpack` + `numpy` unavailable");
        return;
    }
    let pkg = concat!(env!("CARGO_MANIFEST_DIR"), "/quoin_packages/numpy");
    let script = format!(
        r#"
var ok = true;
var e = Extension.loadPackage:'{pkg}';

var a = [NumPy]Array.fromList:#( 3.0 1.0 4.0 1.0 5.0 );

"* comparisons build masks
((a > 2.0).toList == #( true false true false true )).else:{{ ok = false }};
((a > 2.0).dtype == 'bool').else:{{ ok = false }};
((a == 1.0).toList == #( false true false true false )).else:{{ ok = false }};

"* mask reductions: count via sum, any, all
((a > 2.0).sum == 3).else:{{ ok = false }};
((a > 10.0).any == false).else:{{ ok = false }};
((a > 0.0).all == true).else:{{ ok = false }};

"* combinators
(((a > 1.0).and:(a < 5.0)).toList == #( true false true false false )).else:{{ ok = false }};
(((a == 1.0).or:(a == 5.0)).toList == #( false true false true true )).else:{{ ok = false }};
(((a > 2.0).not).toList == #( false true false true false )).else:{{ ok = false }};

"* boolean selection + the functional conditional
((a.select:(a > 2.0)).toList == #( 3.0 4.0 5.0 )).else:{{ ok = false }};
(((a > 2.0).where:a else:0.0).toList == #( 3.0 0.0 4.0 0.0 5.0 )).else:{{ ok = false }};
(((a > 2.0).where:1.0 else:(-1.0)).toList == #( 1.0 (-1.0) 1.0 (-1.0) 1.0 ))
    .else:{{ ok = false }};

"* masks cross the border: toArray -> a host int64 0/1 column
var mask = (a > 2.0).eval;
(mask.toArray.sum == 3).else:{{ ok = false }};

"* composed: mean of the elements above the mean (mean forces, select+mean is one send)
((a.select:(a > a.mean)).mean == 4.0).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_numpy_masks_test.qn", &script);
}

/// Regression: repeated references to the SAME resident array inside one graph must dedup to one
/// base slot (keyed by `Extension.resourceIdOf:`), not exhaust the 8-array selector ladder. The
/// Mandelbrot demo's iteration loop references one array ~35 times per row and exposed this.
#[test]
fn numpy_repeated_base_dedup() {
    if !numpy_fixture_runnable() {
        eprintln!(
            "skipping numpy_repeated_base_dedup: python3 with `msgpack` + `numpy` unavailable"
        );
        return;
    }
    let pkg = concat!(env!("CARGO_MANIFEST_DIR"), "/quoin_packages/numpy");
    let script = format!(
        r#"
var ok = true;
var e = Extension.loadPackage:'{pkg}';

"* one array referenced 20+ times across an iteration-style chain: one base slot, one graph
var a = [NumPy]Array.fromList:#( 1.0 2.0 3.0 );
var acc = a * 0.0;
(0..20).each:{{ |i| acc = (acc + a) * 1.0 }};
(acc.toList == #( 20.0 40.0 60.0 )).else:{{ ok = false }};

"* mixing repeated refs to several distinct arrays still fits the ladder
var b = [NumPy]Array.fromList:#( 8.0 16.0 32.0 );
var m = ((a + b) + (a * b)) + ((b - a) + (a / b));
"* at 0: (1+8) + (1*8) + (8-1) + (1/8) = 24.125 (all exact in binary)
(((m.at:0) - 24.125).abs == 0.0).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_numpy_base_dedup_test.qn", &script);
}

/// `use numpy:*` resolves the package folder from the default search root (`./quoin_packages/`),
/// relative to the VM's cwd — which under `cargo test` is the workspace root.
#[test]
fn numpy_package_via_use() {
    if !numpy_fixture_runnable() {
        eprintln!("skipping numpy_package_via_use: python3 with `msgpack` + `numpy` unavailable");
        return;
    }
    let script = r#"
use numpy:*;
var ok = ([NumPy]Array.arange:10).size == 10;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_script_passes("qn_numpy_use_test.qn", script);
}

/// The data plane both ways plus live instances inside structured returns: `fromArray:` builds a
/// resident array from a host bulk `Array` (an `Array` method argument on the wire — no
/// per-element exploding), and `split:` returns a List of live `[NumPy]Array` instances
/// (resource references inside a data value), each usable in new lazy expressions.
#[test]
fn numpy_array_args_and_instance_lists() {
    if !numpy_fixture_runnable() {
        eprintln!(
            "skipping numpy_array_args_and_instance_lists: python3 with `msgpack` + `numpy` unavailable"
        );
        return;
    }
    let pkg = concat!(env!("CARGO_MANIFEST_DIR"), "/quoin_packages/numpy");
    let script = format!(
        r#"
var ok = true;
var e = Extension.loadPackage:'{pkg}';

"* fromArray: — a host bulk Array crosses whole-buffer and becomes a resident ndarray
var v = [NumPy]Array.fromArray:(Array.ofFloats:#( 1.5 2.5 3.5 ));
(v.toList == #( 1.5 2.5 3.5 )).else:{{ ok = false }};
((v + 1.0).toList == #( 2.5 3.5 4.5 )).else:{{ ok = false }};

"* an int column keeps its dtype
var iv = [NumPy]Array.fromArray:(Array.ofInts:#( 1 2 3 ));
(iv.dtype == 'int64').else:{{ ok = false }};

"* split: — a List of live instances returned inside one structured value
var parts = ([NumPy]Array.arange:7).split:3;
(parts.count == 3).else:{{ ok = false }};
((parts.at:0).toList == #( 0 1 2 )).else:{{ ok = false }};
((parts.at:2).toList == #( 5 6 )).else:{{ ok = false }};
"* each part is a real resident array: it joins new lazy expressions
(((parts.at:1) + 1).toList == #( 4 5 )).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_numpy_array_args_test.qn", &script);
}

/// P1 parity round-out: the widened elementwise set (hyperbolics, inverse trig, extra logs,
/// cbrt, maximum/minimum/arcTan2/floorDiv/hypot, the 3-operand clip), the float-inspection
/// masks (isNan/isInf/isFinite), and the new reductions (variance/ptp/median/countNonZero as
/// forcing scalars + lazy axis forms) plus the lazy cumulative forms (cumSum/cumProd).
#[test]
fn numpy_elementwise_roundout() {
    if !numpy_fixture_runnable() {
        eprintln!(
            "skipping numpy_elementwise_roundout: python3 with `msgpack` + `numpy` unavailable"
        );
        return;
    }
    let pkg = concat!(env!("CARGO_MANIFEST_DIR"), "/quoin_packages/numpy");
    let script = format!(
        r#"
var ok = true;
var e = Extension.loadPackage:'{pkg}';

"* new unaries, probed at exact-value points
((([NumPy]Array.fromList:#( 8.0 )).log2).toList == #( 3.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 100.0 )).log10).toList == #( 2.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 0.0 )).log1p).toList == #( 0.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 0.0 )).expm1).toList == #( 0.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 27.0 )).cbrt).toList == #( 3.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 0.0 )).tanh).toList == #( 0.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 0.0 )).sinh).toList == #( 0.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 0.0 )).cosh).toList == #( 1.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 0.0 )).arcSin).toList == #( 0.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 1.0 )).arcCos).toList == #( 0.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 0.0 )).arcTan).toList == #( 0.0 )).else:{{ ok = false }};

"* new binaries (elementwise, broadcasting like the rest)
var x = [NumPy]Array.fromList:#( 1.0 5.0 );
var y = [NumPy]Array.fromList:#( 4.0 2.0 );
((x.maximum:y).toList == #( 4.0 5.0 )).else:{{ ok = false }};
((x.minimum:y).toList == #( 1.0 2.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 3.0 )).hypot:4.0).toList == #( 5.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 7.0 )).floorDiv:2.0).toList == #( 3.0 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 0.0 )).arcTan2:1.0).toList == #( 0.0 )).else:{{ ok = false }};

"* clip is a 3-operand graph node (scalar bounds become const leaves)
((([NumPy]Array.fromList:#( (-1.0) 5.0 10.0 )).clip:0.0 to:6.0).toList == #( 0.0 5.0 6.0 ))
    .else:{{ ok = false }};

"* float-inspection masks
var weird = ([NumPy]Array.fromList:#( (-1.0) 1.0 )).sqrt;
(weird.isNan.toList == #( true false )).else:{{ ok = false }};
(weird.isFinite.toList == #( false true )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 1.0 )) / 0.0).isInf.toList == #( true )).else:{{ ok = false }};

"* new reductions: whole-array forms force to scalars...
var a = [NumPy]Array.fromList:#( 1.0 3.0 );
(a.variance == 1.0).else:{{ ok = false }};
(a.ptp == 2.0).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 1.0 9.0 3.0 )).median) == 3.0).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 0 3 0 5 )).countNonZero) == 2).else:{{ ok = false }};

"* ...axis forms stay lazy and compose back into graphs
var m = [NumPy]Array.fromList:#( #( 1.0 3.0 ) #( 5.0 9.0 ) );
(((m.variance:1) * 1.0).toList == #( 1.0 4.0 )).else:{{ ok = false }};
((m.ptp:0).toList == #( 4.0 6.0 )).else:{{ ok = false }};

"* cumulative forms are array-shaped and stay in the graph
((([NumPy]Array.fromList:#( 1 2 3 )).cumSum).toList == #( 1 3 6 )).else:{{ ok = false }};
((([NumPy]Array.fromList:#( 1 2 3 )).cumProd).toList == #( 1 2 6 )).else:{{ ok = false }};
(((m.cumSum:1).row:1).toList == #( 5.0 14.0 )).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_numpy_roundout_test.qn", &script);
}

/// P2 parity: sorting & searching — lazy sort/argSort (NumPy's last-axis default + axis forms),
/// unique, searchSorted:, fancy indexing via takeAt:, and nonZero (a List of index arrays, one
/// per dimension — live instances on the wire).
#[test]
fn numpy_sort_search() {
    if !numpy_fixture_runnable() {
        eprintln!("skipping numpy_sort_search: python3 with `msgpack` + `numpy` unavailable");
        return;
    }
    let pkg = concat!(env!("CARGO_MANIFEST_DIR"), "/quoin_packages/numpy");
    let script = format!(
        r#"
var ok = true;
var e = Extension.loadPackage:'{pkg}';

var v = [NumPy]Array.fromList:#( 3.0 1.0 2.0 );
(v.sort.toList == #( 1.0 2.0 3.0 )).else:{{ ok = false }};
(v.argSort.toList == #( 1 2 0 )).else:{{ ok = false }};

"* argSort composes with takeAt: (fancy indexing) inside one graph
((v.takeAt:(v.argSort)).toList == #( 1.0 2.0 3.0 )).else:{{ ok = false }};
(((v * 10.0).takeAt:([NumPy]Array.fromList:#( 2 0 ))).toList == #( 20.0 30.0 ))
    .else:{{ ok = false }};

((([NumPy]Array.fromList:#( 1 2 2 3 1 )).unique).toList == #( 1 2 3 )).else:{{ ok = false }};

"* searchSorted: — array probe stays lazy; scalar probe forces to an index
var srt = [NumPy]Array.fromList:#( 1.0 3.0 5.0 );
((srt.searchSorted:([NumPy]Array.fromList:#( 0.0 4.0 6.0 ))).toList == #( 0 2 3 ))
    .else:{{ ok = false }};
(((srt.searchSorted:4.0).eval) == 2).else:{{ ok = false }};

"* axis-form sort on 2-D (columns here), composing back into the graph
var m = [NumPy]Array.fromList:#( #( 3.0 1.0 ) #( 2.0 4.0 ) );
(((m.sort:0).col:0).toList == #( 2.0 3.0 )).else:{{ ok = false }};

"* nonZero: one index array per dimension, each a live [NumPy]Array
var nz = ([NumPy]Array.fromList:#( 0 7 0 9 )).nonZero;
(nz.count == 1).else:{{ ok = false }};
((nz.at:0).toList == #( 1 3 )).else:{{ ok = false }};
var nz2 = ([NumPy]Array.fromList:#( #( 1 0 ) #( 0 2 ) )).nonZero;
(nz2.count == 2).else:{{ ok = false }};
((nz2.at:0).toList == #( 0 1 )).else:{{ ok = false }};
((nz2.at:1).toList == #( 0 1 )).else:{{ ok = false }};

"* nonZero on a lazy mask expression forces first (an Expr delegate)
(((v > 1.5).nonZero.at:0).toList == #( 0 2 )).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_numpy_sort_search_test.qn", &script);
}
