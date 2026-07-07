//! `VM` — runtime self-introspection. v1 ships the AOT coverage counters the
//! codegen module has kept process-wide since the structural pass:
//!
//! - `VM.stats` -> a Map of sections (only `'aot'` today, shaped so `'gc'` /
//!   `'dispatch'` can join later without breaking callers):
//!   `#{ 'aot': #{ 'compiled': n 'refused': n 'skipped': n 'reasons': #{ kind: count } } }`
//! - `VM.aotRefusals` -> the drill-down: a List of
//!   `#{ 'selector': s 'kind': k 'reason': why }`, one per distinct refusal/skip.
//!
//! Semantics: `compiled` counts translation EVENTS (`codegen::compile_totals` —
//! speculative members recompile); `refused`/`skipped` count DISTINCT members
//! from the deduplicated refusal log (`codegen::refusal_snapshot`), split by
//! translator refusal vs candidacy pre-check. The log is bounded
//! (`REFUSAL_LOG_CAP`), so a very long session may undercount — counters, not
//! ledgers. Block-template skips appear under the pseudo-selector
//! `block@<template-id>` (a block literal has no user-facing name).

use indexmap::IndexMap;

use crate::codegen;
use crate::value::{NativeClassBuilder, Value};
use crate::vm::VmState;

/// The `'aot'` section of `VM.stats`.
fn aot_section<'gc>(vm: &VmState<'gc>, mc: &gc_arena::Mutation<'gc>) -> Value<'gc> {
    let (compiled, _refused_events) = codegen::compile_totals();
    let records = codegen::refusal_snapshot();

    let mut refused = 0i64;
    let mut skipped = 0i64;
    let mut reasons: IndexMap<String, i64> = IndexMap::new();
    for r in &records {
        if r.kind.is_precheck() {
            skipped += 1;
        } else {
            refused += 1;
        }
        *reasons.entry(r.kind.name().to_string()).or_insert(0) += 1;
    }

    let mut reasons_map = IndexMap::new();
    for (k, n) in reasons {
        reasons_map.insert(k, vm.new_int(mc, n));
    }

    let mut aot = IndexMap::new();
    aot.insert("compiled".to_string(), vm.new_int(mc, compiled as i64));
    aot.insert("refused".to_string(), vm.new_int(mc, refused));
    aot.insert("skipped".to_string(), vm.new_int(mc, skipped));
    aot.insert("reasons".to_string(), vm.new_map(mc, reasons_map));
    vm.new_map(mc, aot)
}

pub fn build_vm_stats_class() -> NativeClassBuilder {
    NativeClassBuilder::new("VM", Some("Object"))
        // `VM.stats` -> the section Map (see the module doc for the shape and
        // the events-vs-distinct-members counting semantics).
        .class_method("stats", |vm, mc, _receiver, _args| {
            let mut sections = IndexMap::new();
            sections.insert("aot".to_string(), aot_section(vm, mc));
            Ok(vm.new_map(mc, sections))
        })
        // `VM.aotRefusals` -> one Map per distinct refusal/skip, for finding
        // which of YOUR members stayed interpreted and why.
        .class_method("aotRefusals", |vm, mc, _receiver, _args| {
            let records = codegen::refusal_snapshot();
            let mut out = Vec::with_capacity(records.len());
            for r in records {
                let mut m = IndexMap::new();
                m.insert("selector".to_string(), vm.new_string(mc, r.selector));
                m.insert(
                    "kind".to_string(),
                    vm.new_string(mc, r.kind.name().to_string()),
                );
                m.insert("reason".to_string(), vm.new_string(mc, r.why));
                out.push(vm.new_map(mc, m));
            }
            Ok(vm.new_list(mc, out))
        })
}
