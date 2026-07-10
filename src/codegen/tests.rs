//! Differential tests: compiled methods vs `devirt_ops` / interpreter semantics.
//!
//! These drive the raw compiled fn directly with a huge fuel budget (the
//! checkpoint never fires, so no live VM is needed — its pointer is never
//! dereferenced) and a fresh depth counter. Scheduling/cancellation behavior is
//! exercised end-to-end at the `.qn` level (corpus + stress modes with
//! `QN_AOT=1`), not here.

use super::*;
use crate::compiler::Compiler;
use crate::devirt_ops;
use crate::parser::{NodeValue, try_parse_quoin_string_named};

/// Compile `src` (a program defining sealed classes) and register every AOT
/// candidate; returns `(selector, template_id)` for the registered ones.
fn compile_and_register(src: &str) -> Vec<(String, u32)> {
    let ast = try_parse_quoin_string_named(src, "<aot-test>").expect("parse");
    let NodeValue::Program(p) = &ast.value else {
        panic!("not a program");
    };
    let mut compiler = Compiler::new().with_template_ids().with_aot();
    compiler.compile_program(p).expect("compile");
    let cands = compiler.take_aot_candidates();
    let ids: Vec<(String, u32)> = cands
        .iter()
        .map(|c| (c.selector.clone(), c.block.template_id.unwrap()))
        .collect();
    let stats = compile_candidates(cands);
    assert!(
        stats.refused.is_empty(),
        "unexpected refusals: {:?}",
        stats.refused
    );
    ids
}

static EPOCH_FOR_TESTS: u64 = 1;
thread_local! {
    static SLOTS_FOR_TESTS: std::cell::Cell<*mut crate::value::SlotHead> =
        std::cell::Cell::new(Box::leak(Box::new(crate::value::SlotHead {
            ptr: std::ptr::null_mut(),
            len: 0,
        })));
}

fn entry_for(ids: &[(String, u32)], selector: &str) -> &'static AotEntry {
    let (_, tid) = ids
        .iter()
        .find(|(s, _)| s == selector)
        .unwrap_or_else(|| panic!("no candidate for {selector}"));
    lookup(*tid).unwrap_or_else(|| panic!("{selector} not registered"))
}

/// Invoke the raw compiled fn without a VM: fuel is set high enough that the
/// checkpoint (the only consumer of the vm pointer) can never fire.
fn run_raw(entry: &AotEntry, args: &[i64]) -> Result<i64, u8> {
    assert_eq!(args.len(), entry.params.len());
    let mut fuel: i64 = i64::MAX / 2;
    let mut depth: i64 = 0;
    let mut ret: i64 = 0;
    // vm/mc are never dereferenced on scalar-pure paths (the checkpoint is the
    // only consumer, and fuel never runs out here).
    let tag = unsafe {
        (entry.raw)(
            std::ptr::dangling_mut(),
            std::ptr::dangling(),
            &mut fuel,
            &mut depth,
            &EPOCH_FOR_TESTS,
            SLOTS_FOR_TESTS.with(|s| s.get()),
            0,
            args.as_ptr(),
            &mut ret,
        )
    };
    assert_eq!(depth, 0, "depth counter must balance");
    if tag == TAG_OK { Ok(ret) } else { Err(tag) }
}

#[test]
fn registry_starts_empty_for_unknown_ids() {
    assert!(lookup(u32::MAX).is_none());
}

#[test]
fn int_arithmetic_matches_devirt_ops() {
    let ids = compile_and_register(
        "M <- { .meta <-- {
            add:to: -> { |a: Integer b: Integer ^Integer| a + b };
            sub:from: -> { |a: Integer b: Integer ^Integer| a - b };
            mul:by: -> { |a: Integer b: Integer ^Integer| a * b };
            div:by: -> { |a: Integer b: Integer ^Integer| a / b };
            mod:by: -> { |a: Integer b: Integer ^Integer| a % b };
            lt:than: -> { |a: Integer b: Integer ^Boolean| a < b }
        }; .sealed! }",
    );
    // The contract (devirt_ops doc): wrapping i64 arithmetic, only a zero
    // divisor raises, `i64::MIN / -1` wraps. devirt_ops uses plain operators
    // (debug builds panic on overflow), so the wrapping expectation is computed
    // directly for all pairs, and cross-checked against devirt_ops wherever the
    // debug-checked reference can't panic.
    use crate::instruction::IntBinKind;
    let edge = [0i64, 1, -1, 2, -2, 7, -7, i64::MAX, i64::MIN, 1 << 40];
    for &a in &edge {
        for &b in &edge {
            for (sel, kind) in [
                ("add:to:", IntBinKind::Add),
                ("sub:from:", IntBinKind::Sub),
                ("mul:by:", IntBinKind::Mul),
                ("div:by:", IntBinKind::Div),
                ("mod:by:", IntBinKind::Mod),
                ("lt:than:", IntBinKind::Lt),
            ] {
                let got = run_raw(entry_for(&ids, sel), &[a, b]);
                // Checked semantics end to end: overflow bails with its own tag, a zero
                // divisor with the division tag; the one overflowing quotient (MIN / -1)
                // counts as overflow. int_bin is the reference for every non-error case,
                // with no carve-outs -- it can no longer panic on any input.
                let want: Result<i64, u8> = match kind {
                    IntBinKind::Add => a.checked_add(b).ok_or(TAG_INT_OVERFLOW),
                    IntBinKind::Sub => a.checked_sub(b).ok_or(TAG_INT_OVERFLOW),
                    IntBinKind::Mul => a.checked_mul(b).ok_or(TAG_INT_OVERFLOW),
                    IntBinKind::Div if b == 0 => Err(TAG_DIV_ZERO),
                    IntBinKind::Div => a.checked_div(b).ok_or(TAG_INT_OVERFLOW),
                    IntBinKind::Mod if b == 0 => Err(TAG_DIV_ZERO),
                    IntBinKind::Mod => Ok(a.wrapping_rem(b)), // MIN % -1 == 0, no overflow
                    IntBinKind::Lt => Ok((a < b) as i64),
                    _ => unreachable!(),
                };
                assert_eq!(got, want, "{sel} {a} {b}");
                match devirt_ops::int_bin(kind, a, b) {
                    Ok(devirt_ops::IntBinOut::Int(w)) => {
                        assert_eq!(got, Ok(w), "{sel} {a} {b} vs devirt_ops")
                    }
                    Ok(devirt_ops::IntBinOut::Bool(w)) => {
                        assert_eq!(got, Ok(w as i64), "{sel} {a} {b} vs devirt_ops")
                    }
                    Err(e) => {
                        let want_tag = if e.to_string().contains("overflow") {
                            TAG_INT_OVERFLOW
                        } else {
                            TAG_DIV_ZERO
                        };
                        assert_eq!(got, Err(want_tag), "{sel} {a} {b} vs devirt_ops err")
                    }
                }
            }
        }
    }
}

#[test]
fn double_arithmetic_matches_devirt_ops() {
    let ids = compile_and_register(
        "D <- { .meta <-- {
            add:to: -> { |a: Double b: Double ^Double| a + b };
            div:by: -> { |a: Double b: Double ^Double| a / b };
            mod:by: -> { |a: Double b: Double ^Double| a % b };
            le:than: -> { |a: Double b: Double ^Boolean| a <= b };
            eq:to: -> { |a: Double b: Double ^Boolean| a == b }
        }; .sealed! }",
    );
    let edge = [
        0.0f64,
        -0.0,
        1.5,
        -2.25,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        1e300,
    ];
    for &a in &edge {
        for &b in &edge {
            for (sel, kind) in [
                ("add:to:", crate::instruction::IntBinKind::Add),
                ("div:by:", crate::instruction::IntBinKind::Div),
                ("mod:by:", crate::instruction::IntBinKind::Mod),
                ("le:than:", crate::instruction::IntBinKind::Le),
                ("eq:to:", crate::instruction::IntBinKind::Eq),
            ] {
                let raw = [a.to_bits() as i64, b.to_bits() as i64];
                let got = run_raw(entry_for(&ids, sel), &raw).unwrap();
                match devirt_ops::double_bin(kind, a, b) {
                    devirt_ops::DoubleBinOut::Double(want) => {
                        let gotf = f64::from_bits(got as u64);
                        assert!(
                            gotf == want || (gotf.is_nan() && want.is_nan()),
                            "{sel} {a} {b}: got {gotf}, want {want}"
                        );
                    }
                    devirt_ops::DoubleBinOut::Bool(want) => {
                        assert_eq!(got, want as i64, "{sel} {a} {b}");
                    }
                }
            }
        }
    }
}

#[test]
fn control_flow_loops_and_locals() {
    let ids = compile_and_register(
        "L <- { .meta <-- {
            sumTo: -> { |n: Integer ^Integer|
                var total = 0;
                var i = 1;
                { i <= n }.whileDo:{ total = total + i; i = i + 1 };
                total
            }
        }; .sealed! }",
    );
    let e = entry_for(&ids, "sumTo:");
    assert_eq!(run_raw(e, &[0]), Ok(0));
    assert_eq!(run_raw(e, &[1]), Ok(1));
    assert_eq!(run_raw(e, &[100]), Ok(5050));
    assert_eq!(run_raw(e, &[10_000]), Ok(50_005_000));
}

#[test]
fn self_recursion_and_mutual_recursion() {
    let ids = compile_and_register(
        "R <- { .meta <-- {
            fib: -> { |n: Integer ^Integer|
                (n <= 1).if:{ ^n } else:{ ^(.fib:(n - 1)) + (.fib:(n - 2)) }
            };
            isEven: -> { |n: Integer ^Boolean|
                (n == 0).if:{ ^true } else:{ ^.isOdd:(n - 1) }
            };
            isOdd: -> { |n: Integer ^Boolean|
                (n == 0).if:{ ^false } else:{ ^.isEven:(n - 1) }
            }
        }; .sealed! }",
    );
    let fib = entry_for(&ids, "fib:");
    assert_eq!(run_raw(fib, &[0]), Ok(0));
    assert_eq!(run_raw(fib, &[1]), Ok(1));
    assert_eq!(run_raw(fib, &[10]), Ok(55));
    assert_eq!(run_raw(fib, &[25]), Ok(75_025));
    let even = entry_for(&ids, "isEven:");
    assert_eq!(run_raw(even, &[10]), Ok(1));
    assert_eq!(run_raw(even, &[11]), Ok(0));
}

#[test]
fn depth_guard_is_catchable_not_fatal() {
    let ids = compile_and_register(
        "Deep <- { .meta <-- {
            down: -> { |n: Integer ^Integer|
                (n == 0).if:{ ^0 } else:{ ^.down:(n - 1) }
            }
        }; .sealed! }",
    );
    let e = entry_for(&ids, "down:");
    assert_eq!(run_raw(e, &[100]), Ok(0));
    // Beyond the cap: the compiled prologue bails with the depth tag (and the
    // balanced-depth assert in run_raw checks the unwind decrements).
    assert_eq!(run_raw(e, &[AOT_MAX_CALL_DEPTH + 10]), Err(TAG_DEPTH));
}

#[test]
fn multimethod_variants_and_guards_are_not_candidates() {
    let ast = try_parse_quoin_string_named(
        "G <- { .meta <-- {
            f: -> { |a: Integer ^Integer| a };
            f: -> { |a: Double ^Double| a }
        }; .sealed! }",
        "<aot-test>",
    )
    .expect("parse");
    let NodeValue::Program(p) = &ast.value else {
        panic!("not a program");
    };
    let mut compiler = Compiler::new().with_template_ids().with_aot();
    compiler.compile_program(p).expect("compile");
    assert!(
        compiler.take_aot_candidates().is_empty(),
        "multi-variant selectors must not be candidates"
    );
}

#[test]
fn unsealed_class_is_an_open_owner_candidate() {
    // B2 (docs/BLOCK_AOT_ARCH.md §3): an OPEN owner's method may compile, marked
    // `open_owner` so the translator emits no direct sibling calls — every send
    // crosses a dispatch-equivalent seam, and a reopen simply dispatches to its
    // new template (per-dispatch minting; the stale entry stops being reachable).
    let ast = try_parse_quoin_string_named(
        "U <- { .meta <-- { f: -> { |a: Integer ^Integer| a } } }",
        "<aot-test>",
    )
    .expect("parse");
    let NodeValue::Program(p) = &ast.value else {
        panic!("not a program");
    };
    let mut compiler = Compiler::new().with_template_ids().with_aot();
    compiler.compile_program(p).expect("compile");
    let cands = compiler.take_aot_candidates();
    assert_eq!(cands.len(), 1, "open owners are candidates now");
    assert!(
        cands[0].open_owner,
        "…marked so direct calls are suppressed"
    );
}

#[test]
fn unannotated_method_is_a_speculative_candidate() {
    // S0 (docs/SPECULATIVE_AOT_ARCH.md): an unannotated param or an absent
    // return annotation no longer ends candidacy — the candidate collects as
    // SPECULATIVE (Obj placeholders, spec flags set) and waits on a runtime
    // kind profile instead of compiling at unit load.
    let ast = try_parse_quoin_string_named(
        "S <- { .meta <-- { f: -> { |a| a }; g: -> { |b: Integer ^Integer| b } } }",
        "<aot-test>",
    )
    .expect("parse");
    let NodeValue::Program(p) = &ast.value else {
        panic!("not a program");
    };
    let mut compiler = Compiler::new().with_template_ids().with_aot();
    compiler.compile_program(p).expect("compile");
    let cands = compiler.take_aot_candidates();
    let f = cands
        .iter()
        .find(|c| c.selector == "f:")
        .expect("f: collected");
    assert!(
        f.speculative(),
        "unannotated params make a speculative candidate"
    );
    assert_eq!(f.spec_params, vec![true]);
    assert!(f.spec_ret, "absent return annotation is speculated too");
    assert!(
        matches!(f.params[0], AotParam::Obj),
        "placeholder until profiled"
    );
    let g = cands
        .iter()
        .find(|c| c.selector == "g:")
        .expect("g: collected");
    assert!(!g.speculative(), "fully annotated candidates are classic");
}

#[test]
fn mixed_annotations_speculate_only_the_gaps() {
    // Annotations are dispatch GUARANTEES and win over observation; only the
    // unannotated slots ride as speculative.
    let ast = try_parse_quoin_string_named(
        "M <- { .meta <-- { f:g: -> { |a: Integer b ^Integer| a } } }",
        "<aot-test>",
    )
    .expect("parse");
    let NodeValue::Program(p) = &ast.value else {
        panic!("not a program");
    };
    let mut compiler = Compiler::new().with_template_ids().with_aot();
    compiler.compile_program(p).expect("compile");
    let cands = compiler.take_aot_candidates();
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].spec_params, vec![false, true]);
    assert!(!cands[0].spec_ret);
    assert!(
        matches!(cands[0].params[0], AotParam::Scalar(AotKind::Int)),
        "annotated slot keeps its kind"
    );
}

#[test]
fn spec_kind_lattice_merges() {
    use super::spec::{K_BOOL, K_DOUBLE, K_INT, K_OBJ, K_UNKNOWN, merge};
    assert_eq!(
        merge(K_UNKNOWN, K_INT),
        K_INT,
        "unknown rises to the first kind"
    );
    assert_eq!(merge(K_INT, K_INT), K_INT, "agreement is stable");
    assert_eq!(merge(K_INT, K_DOUBLE), K_OBJ, "conflict lands on Obj");
    assert_eq!(merge(K_BOOL, K_OBJ), K_OBJ, "Obj absorbs");
    assert_eq!(merge(K_OBJ, K_INT), K_OBJ, "Obj never narrows back");
}

#[test]
fn guarded_conditional_cold_span_keeps_untyped_fib_scalar_pure() {
    // The F1 strict-Boolean fix wraps every not-statically-Bool conditional in
    // a BranchIfNotBool guard whose COLD span re-materializes the arm blocks
    // and re-dispatches the real if:else: send. That span is dynamically dead
    // once the speculated compare folds the guard away — it must not evict the
    // method from the scalar-pure set. Shipping that blind spot cost untyped
    // fib 8x: eviction killed direct self-recursion, which made the speculated
    // scalar return unprovable, which demoted the entry to an Obj ret.
    let src = "Fib <- { .meta <-- { value: -> { |n| \
               (n <= 1).if:{ ^n } else:{ ^(.value:(n - 1)) + (.value:(n - 2)) } } } };";
    let ast = try_parse_quoin_string_named(src, "<aot-test>").expect("parse");
    let NodeValue::Program(p) = &ast.value else {
        panic!("not a program");
    };
    let mut compiler = Compiler::new().with_template_ids().with_aot();
    compiler.compile_program(p).expect("compile");
    let mut cands = compiler.take_aot_candidates();
    let cand = cands
        .iter_mut()
        .find(|c| c.selector == "value:")
        .expect("fib is a speculative candidate");
    assert!(cand.speculative(), "untyped fib must be speculative");
    // What spec_promote mints after a saturated all-Int profile (S1/S2):
    // scalar param + entry precondition + observed-scalar return.
    let tid = cand.block.template_id.unwrap();
    cand.params = vec![AotParam::Scalar(AotKind::Int)];
    cand.spec_preconditions = vec![Some(AotKind::Int)];
    cand.ret = AotRet::Scalar(AotKind::Int);
    let stats = compile_candidates(vec![cand.clone()]);
    assert!(stats.refused.is_empty(), "refused: {:?}", stats.refused);
    let entry = lookup(tid).expect("fib registered");
    assert_eq!(
        entry.ret,
        AotRet::Scalar(AotKind::Int),
        "the speculated scalar return must survive the guarded-conditional \
         shape (a RetDemote here means the cold span broke scalar purity)"
    );
}
