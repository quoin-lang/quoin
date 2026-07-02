//! Tier-1 unboxed-VM ceiling screen — see `docs/FUTURE_ARCH.md`.
//!
//! A minimal *bytecode interpreter* over RAW `i64`: no `Value` enum, no tag
//! checks, no method lookup. Kernels (`fib`, `sieve`) are hand-assembled; calls
//! resolve to a direct function index (devirtualized). This measures the
//! UPPER-BOUND ceiling for an unboxed + devirtualized Quoin tier.
//!
//! It is deliberately an *upper bound*: it keeps a real dispatch loop and real
//! call frames (return address + frame-based locals), but drops overhead a true
//! integrated tier would still pay (GC rooting, the typed/untyped boundary
//! guards, richer frames). Array access is modelled as dedicated `Array*`
//! opcodes over an `i64` backing store — direct indexed access, the *best case*;
//! a real devirtualized `at:`/`at:put:` would still be a native call with a
//! bounds check. If even this optimistic number isn't compelling, stop.
//!
//! Targets (interpreter-only, from profiling/lang-comparison):
//!   | bench      | Quoin today | Ruby 2.6 | Python 3.9 |
//!   | fib(20)    | ~18 ms      | 0.35 ms  | 1.04 ms    |
//!   | sieve(1e4) | ~44 ms      | 0.72 ms  | 1.12 ms    |
//!
//! Usage: `unboxed-proto [fib|sieve]` (default: both).

use std::cell::RefCell;
use std::env;
use std::rc::Rc;
use std::time::Instant;

/// Unboxed instruction set — same shape as a real bytecode VM (an enum matched
/// in a dispatch loop), but every operand/stack/local slot is a raw `i64`.
#[derive(Clone, Copy)]
enum Op {
    PushInt(i64),
    LoadLocal(u16),
    StoreLocal(u16),
    IAdd,
    ISub,
    IMul,
    ILt, // <
    ILe, // <=
    IEq, // ==
    Jump(u32),
    JumpIfFalse(u32),
    /// Direct call to `func` (devirtualized — no method lookup), consuming
    /// `argc` operands as the callee's leading locals.
    Call { func: u16, argc: u16 },
    Ret,
    // --- array ops: unboxed i64 elements, direct indexed access (the ceiling) ---
    NewArray,   // -> push new empty array handle
    ArrayPush,  // [handle, value] -> arr[handle].push(value)
    ArrayLoad,  // [handle, index] -> push arr[handle][index]
    ArrayStore, // [handle, index, value] -> arr[handle][index] = value
    ArrayLen,   // [handle] -> push arr[handle].len()
    // --- 3-field struct nodes (unboxed): bump-allocated in an arena, handle = index ---
    NewNode,   // [item, left, right] -> push node handle
    NodeItem,  // [handle] -> push node.item
    NodeLeft,  // [handle] -> push node.left
    NodeRight, // [handle] -> push node.right
}

/// A flat, unboxed 3-field struct node (the `TreeNode` shape). Children are
/// i64 handles into the arena; -1 = nil.
struct Node3 {
    item: i64,
    left: i64,
    right: i64,
}

struct Func {
    code: Vec<Op>,
    /// Total local slots (params fill the leading `n_params`; the rest zero-init).
    n_locals: usize,
}

#[derive(Clone, Copy)]
struct Frame {
    func: usize,
    ip: usize,
    base: usize,
}

/// Run `funcs[entry]` with a single `i64` argument, returning its `i64` result.
///
/// Hot state (`func`/`ip`/`base`) lives in locals, saved/restored only across a
/// Call/Ret — the standard "top frame in registers" interpreter trick.
fn run(funcs: &[Func], entry: usize, arg: i64) -> i64 {
    let mut operands: Vec<i64> = Vec::with_capacity(256);
    let mut locals: Vec<i64> = Vec::with_capacity(256);
    let mut frames: Vec<Frame> = Vec::with_capacity(256);
    // Fresh per run — sieve allocates its arrays here, so their alloc cost is
    // included in the number (not elided).
    let mut arrays: Vec<Vec<i64>> = Vec::new();
    // Unboxed node arena: bump-allocated for the whole run, bulk-freed when this
    // call returns (a region). Models unboxed struct nodes — no per-node malloc.
    let mut nodes: Vec<Node3> = Vec::new();

    let mut func = entry;
    let mut ip = 0usize;
    let mut base = 0usize;
    locals.push(arg); // slot 0 = the entry arg
    for _ in 1..funcs[entry].n_locals {
        locals.push(0);
    }

    loop {
        let op = funcs[func].code[ip];
        ip += 1;
        match op {
            Op::PushInt(v) => operands.push(v),
            Op::LoadLocal(slot) => operands.push(locals[base + slot as usize]),
            Op::StoreLocal(slot) => {
                let v = operands.pop().unwrap();
                locals[base + slot as usize] = v;
            }
            Op::IAdd => {
                let b = operands.pop().unwrap();
                let a = operands.pop().unwrap();
                operands.push(a + b);
            }
            Op::ISub => {
                let b = operands.pop().unwrap();
                let a = operands.pop().unwrap();
                operands.push(a - b);
            }
            Op::IMul => {
                let b = operands.pop().unwrap();
                let a = operands.pop().unwrap();
                operands.push(a * b);
            }
            Op::ILt => {
                let b = operands.pop().unwrap();
                let a = operands.pop().unwrap();
                operands.push((a < b) as i64);
            }
            Op::ILe => {
                let b = operands.pop().unwrap();
                let a = operands.pop().unwrap();
                operands.push((a <= b) as i64);
            }
            Op::IEq => {
                let b = operands.pop().unwrap();
                let a = operands.pop().unwrap();
                operands.push((a == b) as i64);
            }
            Op::Jump(target) => ip = target as usize,
            Op::JumpIfFalse(target) => {
                if operands.pop().unwrap() == 0 {
                    ip = target as usize;
                }
            }
            Op::Call { func: callee, argc } => {
                let new_base = locals.len();
                let start = operands.len() - argc as usize;
                locals.extend_from_slice(&operands[start..]);
                operands.truncate(start);
                for _ in (argc as usize)..funcs[callee as usize].n_locals {
                    locals.push(0);
                }
                frames.push(Frame { func, ip, base });
                func = callee as usize;
                ip = 0;
                base = new_base;
            }
            Op::Ret => {
                let ret = operands.pop().unwrap();
                locals.truncate(base);
                match frames.pop() {
                    Some(fr) => {
                        func = fr.func;
                        ip = fr.ip;
                        base = fr.base;
                        operands.push(ret);
                    }
                    None => return ret,
                }
            }
            Op::NewArray => {
                arrays.push(Vec::new());
                operands.push((arrays.len() - 1) as i64);
            }
            Op::ArrayPush => {
                let v = operands.pop().unwrap();
                let h = operands.pop().unwrap();
                arrays[h as usize].push(v);
            }
            Op::ArrayLoad => {
                let idx = operands.pop().unwrap();
                let h = operands.pop().unwrap();
                operands.push(arrays[h as usize][idx as usize]);
            }
            Op::ArrayStore => {
                let v = operands.pop().unwrap();
                let idx = operands.pop().unwrap();
                let h = operands.pop().unwrap();
                arrays[h as usize][idx as usize] = v;
            }
            Op::ArrayLen => {
                let h = operands.pop().unwrap();
                operands.push(arrays[h as usize].len() as i64);
            }
            Op::NewNode => {
                let right = operands.pop().unwrap();
                let left = operands.pop().unwrap();
                let item = operands.pop().unwrap();
                nodes.push(Node3 { item, left, right });
                operands.push((nodes.len() - 1) as i64);
            }
            Op::NodeItem => {
                let h = operands.pop().unwrap();
                operands.push(nodes[h as usize].item);
            }
            Op::NodeLeft => {
                let h = operands.pop().unwrap();
                operands.push(nodes[h as usize].left);
            }
            Op::NodeRight => {
                let h = operands.pop().unwrap();
                operands.push(nodes[h as usize].right);
            }
        }
    }
}

/// Tiny label-patching assembler so jump targets aren't hand-indexed.
struct Asm {
    code: Vec<Op>,
    labels: Vec<(&'static str, usize)>,
    fixups: Vec<(usize, &'static str)>,
}
impl Asm {
    fn new() -> Self {
        Asm { code: Vec::new(), labels: Vec::new(), fixups: Vec::new() }
    }
    fn emit(&mut self, op: Op) {
        self.code.push(op);
    }
    fn label(&mut self, name: &'static str) {
        self.labels.push((name, self.code.len()));
    }
    /// Emit a jump to `name` (patched in `finish`). The variant carries the kind.
    fn jump(&mut self, op: Op, name: &'static str) {
        self.fixups.push((self.code.len(), name));
        self.code.push(op);
    }
    fn finish(mut self) -> Vec<Op> {
        for (at, name) in &self.fixups {
            let target = self.labels.iter().find(|(n, _)| n == name).unwrap().1 as u32;
            match &mut self.code[*at] {
                Op::Jump(t) | Op::JumpIfFalse(t) => *t = target,
                _ => unreachable!("fixup on non-jump"),
            }
        }
        self.code
    }
}

/// fib(n): if n < 2 { n } else { fib(n-1) + fib(n-2) }.  slot 0 = n
fn build_fib() -> Vec<Func> {
    use Op::*;
    let code = vec![
        LoadLocal(0),
        PushInt(2),
        ILt,
        JumpIfFalse(6),
        LoadLocal(0),
        Ret,
        LoadLocal(0),
        PushInt(1),
        ISub,
        Call { func: 0, argc: 1 },
        LoadLocal(0),
        PushInt(2),
        ISub,
        Call { func: 0, argc: 1 },
        IAdd,
        Ret,
    ];
    vec![Func { code, n_locals: 1 }]
}

/// Faithful port of `Sieve.primesUpTo:` from qnlib/benchmark.qn.
/// Returns the number of primes ≤ limit (checksum; 1229 for limit=10000).
/// locals: 0=limit, 1=is_prime(handle), 2=i, 3=p, 4=primes(handle)
fn build_sieve() -> Vec<Func> {
    use Op::*;
    let mut a = Asm::new();

    // is_prime = #()
    a.emit(NewArray);
    a.emit(StoreLocal(1));
    // i = 0
    a.emit(PushInt(0));
    a.emit(StoreLocal(2));
    // while i <= limit { is_prime.add:true; i = i+1 }
    a.label("build");
    a.emit(LoadLocal(2));
    a.emit(LoadLocal(0));
    a.emit(ILe);
    a.jump(JumpIfFalse(0), "build_end");
    a.emit(LoadLocal(1));
    a.emit(PushInt(1));
    a.emit(ArrayPush);
    a.emit(LoadLocal(2));
    a.emit(PushInt(1));
    a.emit(IAdd);
    a.emit(StoreLocal(2));
    a.jump(Jump(0), "build");
    a.label("build_end");

    // is_prime.at:0 put:false ; is_prime.at:1 put:false
    a.emit(LoadLocal(1));
    a.emit(PushInt(0));
    a.emit(PushInt(0));
    a.emit(ArrayStore);
    a.emit(LoadLocal(1));
    a.emit(PushInt(1));
    a.emit(PushInt(0));
    a.emit(ArrayStore);

    // p = 2
    a.emit(PushInt(2));
    a.emit(StoreLocal(3));
    // while p*p <= limit
    a.label("sieve");
    a.emit(LoadLocal(3));
    a.emit(LoadLocal(3));
    a.emit(IMul);
    a.emit(LoadLocal(0));
    a.emit(ILe);
    a.jump(JumpIfFalse(0), "sieve_end");
    // if is_prime.at:p
    a.emit(LoadLocal(1));
    a.emit(LoadLocal(3));
    a.emit(ArrayLoad);
    a.jump(JumpIfFalse(0), "not_prime");
    // i = p*p
    a.emit(LoadLocal(3));
    a.emit(LoadLocal(3));
    a.emit(IMul);
    a.emit(StoreLocal(2));
    // while i <= limit { is_prime.at:i put:false; i = i+p }
    a.label("mark");
    a.emit(LoadLocal(2));
    a.emit(LoadLocal(0));
    a.emit(ILe);
    a.jump(JumpIfFalse(0), "mark_end");
    a.emit(LoadLocal(1));
    a.emit(LoadLocal(2));
    a.emit(PushInt(0));
    a.emit(ArrayStore);
    a.emit(LoadLocal(2));
    a.emit(LoadLocal(3));
    a.emit(IAdd);
    a.emit(StoreLocal(2));
    a.jump(Jump(0), "mark");
    a.label("mark_end");
    a.label("not_prime");
    // p = p+1
    a.emit(LoadLocal(3));
    a.emit(PushInt(1));
    a.emit(IAdd);
    a.emit(StoreLocal(3));
    a.jump(Jump(0), "sieve");
    a.label("sieve_end");

    // primes = #() ; i = 2
    a.emit(NewArray);
    a.emit(StoreLocal(4));
    a.emit(PushInt(2));
    a.emit(StoreLocal(2));
    // while i <= limit { if is_prime.at:i { primes.add:i }; i = i+1 }
    a.label("collect");
    a.emit(LoadLocal(2));
    a.emit(LoadLocal(0));
    a.emit(ILe);
    a.jump(JumpIfFalse(0), "collect_end");
    a.emit(LoadLocal(1));
    a.emit(LoadLocal(2));
    a.emit(ArrayLoad);
    a.jump(JumpIfFalse(0), "collect_skip");
    a.emit(LoadLocal(4));
    a.emit(LoadLocal(2));
    a.emit(ArrayPush);
    a.label("collect_skip");
    a.emit(LoadLocal(2));
    a.emit(PushInt(1));
    a.emit(IAdd);
    a.emit(StoreLocal(2));
    a.jump(Jump(0), "collect");
    a.label("collect_end");

    // ^primes.length
    a.emit(LoadLocal(4));
    a.emit(ArrayLen);
    a.emit(Ret);

    vec![Func { code: a.finish(), n_locals: 5 }]
}

/// Native compiled recursive fib — the "ceiling of the ceiling" (machine code,
/// no interpreter loop), for context on how much the dispatch loop costs.
fn native_fib(n: i64) -> i64 {
    if n < 2 { n } else { native_fib(n - 1) + native_fib(n - 2) }
}

fn main() {
    let which = env::args().nth(1).unwrap_or_default();
    let run_fib = which.is_empty() || which == "fib";
    let run_sieve = which.is_empty() || which == "sieve";
    let run_tree = which.is_empty() || which == "tree";

    if run_fib {
        bench_fib();
    }
    if run_sieve {
        bench_sieve();
    }
    if run_tree {
        bench_tree();
    }
}

fn bench_fib() {
    let funcs = build_fib();
    assert_eq!(run(&funcs, 0, 10), 55);
    assert_eq!(run(&funcs, 0, 20), 6765);
    assert_eq!(run(&funcs, 0, 30), 832040);

    const QUOIN: f64 = 18.0;
    const RUBY: f64 = 0.35;
    const PYTHON: f64 = 1.04;

    let iters = 50_000u64;
    let mut acc = 0i64;
    let t = Instant::now();
    for _ in 0..iters {
        acc = acc.wrapping_add(run(&funcs, 0, 20));
    }
    let per_ms = (t.elapsed().as_nanos() as f64 / iters as f64) / 1e6;
    let calls = (2 * 10946 - 1) as f64; // 2*fib(21)-1
    println!("=== fib(20) — unboxed interpreter ({} iters, checksum {}) ===", iters, acc);
    println!("  {:.5} ms/run   ({:.1} ns/vm-call)", per_ms, per_ms * 1e6 / calls);
    report(per_ms, QUOIN, RUBY, PYTHON);

    // heavy low-noise cross-check
    let t = Instant::now();
    let r = run(&funcs, 0, 35);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let calls35 = (2 * 14930352 - 1) as f64;
    println!("  fib(35)={} in {:.1} ms ({:.1} ns/vm-call)  |  native fib(35): {:.1} ms\n",
        r, ms, ms * 1e6 / calls35, {
            let t = Instant::now();
            let _ = native_fib(35);
            t.elapsed().as_secs_f64() * 1e3
        });
}

fn bench_sieve() {
    let funcs = build_sieve();
    assert_eq!(run(&funcs, 0, 100), 25); // 25 primes ≤ 100
    assert_eq!(run(&funcs, 0, 10000), 1229); // 1229 primes ≤ 10000

    const QUOIN: f64 = 44.0;
    const RUBY: f64 = 0.72;
    const PYTHON: f64 = 1.12;

    let iters = 5_000u64;
    let mut acc = 0i64;
    let t = Instant::now();
    for _ in 0..iters {
        acc = acc.wrapping_add(run(&funcs, 0, 10000));
    }
    let per_ms = (t.elapsed().as_nanos() as f64 / iters as f64) / 1e6;
    println!("=== sieve(10000) — unboxed interpreter ({} iters, {} primes) ===", iters, acc / iters as i64);
    println!("  {:.5} ms/run", per_ms);
    report(per_ms, QUOIN, RUBY, PYTHON);
    println!();
}

// ===================== Binary Trees =====================
// Faithful port of `TreeBenchmark.run:` from qnlib/benchmark.qn, under three node
// allocation strategies to isolate the "unboxed struct types" lever. This is
// native Rust (not the bytecode interpreter): the tree's bottleneck is node
// *allocation*, not dispatch, so this is an even looser upper bound — but it's
// the clean, low-risk way to size the allocation lever. Rc<RefCell> has no real
// tracing GC, so it reads faster than Quoin's actual 661 ms (a floor, not a
// replica). makeTree/check/powerOfTwo mirror the .qn exactly.

const TREE_MIN_DEPTH: i64 = 4;

fn power_of_two(p: i64) -> i64 {
    1i64 << p
}

// --- backend 1: arena / unboxed struct (the ceiling) ---
// Flat POD nodes in a bump `Vec`; children are i32 indices (-1 = nil). Temp trees
// reuse a cleared scratch arena → bump-allocate + bulk-free, no per-node malloc,
// no GC. This is the unboxed-struct upper bound.
struct ANode {
    item: i64,
    left: i32,
    right: i32,
}
struct Arena {
    nodes: Vec<ANode>,
}
impl Arena {
    fn make(&mut self, item: i64, depth: i64) -> i32 {
        let (left, right) = if depth > 0 {
            (self.make(2 * item - 1, depth - 1), self.make(2 * item, depth - 1))
        } else {
            (-1, -1)
        };
        let idx = self.nodes.len() as i32;
        self.nodes.push(ANode { item, left, right });
        idx
    }
    fn check(&self, idx: i32) -> i64 {
        let n = &self.nodes[idx as usize];
        if n.left != -1 {
            n.item + self.check(n.left) - self.check(n.right)
        } else {
            n.item
        }
    }
}
fn run_arena(max_depth: i64) -> i64 {
    let mut checksum = 0i64;
    let mut long = Arena { nodes: Vec::new() };
    let long_root = long.make(0, max_depth); // kept alive the whole run
    let mut scratch = Arena { nodes: Vec::new() };
    let mut depth = TREE_MIN_DEPTH;
    while depth <= max_depth {
        let iterations = power_of_two(max_depth - depth + TREE_MIN_DEPTH);
        let mut i = 1;
        while i <= iterations {
            scratch.nodes.clear();
            let t = scratch.make(i, depth);
            checksum += scratch.check(t);
            scratch.nodes.clear();
            let t = scratch.make(-i, depth);
            checksum += scratch.check(t);
            i += 1;
        }
        depth += 2;
    }
    checksum + long.check(long_root)
}

// --- backend 2: Box per node (raw malloc/free per node, no GC) ---
struct BNode {
    item: i64,
    left: Option<Box<BNode>>,
    right: Option<Box<BNode>>,
}
fn make_box(item: i64, depth: i64) -> Box<BNode> {
    if depth > 0 {
        Box::new(BNode {
            item,
            left: Some(make_box(2 * item - 1, depth - 1)),
            right: Some(make_box(2 * item, depth - 1)),
        })
    } else {
        Box::new(BNode { item, left: None, right: None })
    }
}
fn check_box(n: &BNode) -> i64 {
    match &n.left {
        Some(l) => n.item + check_box(l) - check_box(n.right.as_ref().unwrap()),
        None => n.item,
    }
}
fn run_box(max_depth: i64) -> i64 {
    let mut checksum = 0i64;
    let long = make_box(0, max_depth);
    let mut depth = TREE_MIN_DEPTH;
    while depth <= max_depth {
        let iterations = power_of_two(max_depth - depth + TREE_MIN_DEPTH);
        let mut i = 1;
        while i <= iterations {
            let t = make_box(i, depth);
            checksum += check_box(&t); // t freed at end of iteration
            let t = make_box(-i, depth);
            checksum += check_box(&t);
            i += 1;
        }
        depth += 2;
    }
    checksum + check_box(&long)
}

// --- backend 3: Rc<RefCell> per node (closest standalone analog to Gc<RefLock>) ---
type RLink = Option<Rc<RefCell<RNode>>>;
struct RNode {
    item: i64,
    left: RLink,
    right: RLink,
}
fn make_rc(item: i64, depth: i64) -> Rc<RefCell<RNode>> {
    if depth > 0 {
        Rc::new(RefCell::new(RNode {
            item,
            left: Some(make_rc(2 * item - 1, depth - 1)),
            right: Some(make_rc(2 * item, depth - 1)),
        }))
    } else {
        Rc::new(RefCell::new(RNode { item, left: None, right: None }))
    }
}
fn check_rc(n: &Rc<RefCell<RNode>>) -> i64 {
    let b = n.borrow(); // model the RefLock borrow on every field access
    match &b.left {
        Some(l) => b.item + check_rc(l) - check_rc(b.right.as_ref().unwrap()),
        None => b.item,
    }
}
fn run_rc(max_depth: i64) -> i64 {
    let mut checksum = 0i64;
    let long = make_rc(0, max_depth);
    let mut depth = TREE_MIN_DEPTH;
    while depth <= max_depth {
        let iterations = power_of_two(max_depth - depth + TREE_MIN_DEPTH);
        let mut i = 1;
        while i <= iterations {
            let t = make_rc(i, depth);
            checksum += check_rc(&t);
            let t = make_rc(-i, depth);
            checksum += check_rc(&t);
            i += 1;
        }
        depth += 2;
    }
    checksum + check_rc(&long)
}

/// The tree benchmark hand-assembled as bytecode for the interpreter — the
/// honest ceiling WITH per-node dispatch. Four functions:
/// 0=run, 1=makeTree, 2=check, 3=powerOfTwo. Nodes bump-allocate into the
/// interpreter's arena (unboxed structs). Mirrors qnlib/benchmark.qn exactly.
fn build_tree_program() -> Vec<Func> {
    use Op::*;
    const RUN: u16 = 0;
    const MAKE: u16 = 1;
    const CHECK: u16 = 2;
    const POW: u16 = 3;
    let _ = RUN;
    const NIL: i64 = -1;

    // run(max_depth): 0=max_depth 1=checksum 2=depth 3=iterations 4=i 5=longRoot 6=t
    let run_fn = {
        let mut a = Asm::new();
        a.emit(PushInt(0));
        a.emit(StoreLocal(1)); // checksum = 0
        // longRoot = makeTree(0, max_depth)
        a.emit(PushInt(0));
        a.emit(LoadLocal(0));
        a.emit(Call { func: MAKE, argc: 2 });
        a.emit(StoreLocal(5));
        a.emit(PushInt(4));
        a.emit(StoreLocal(2)); // depth = 4
        a.label("depth_loop");
        a.emit(LoadLocal(2));
        a.emit(LoadLocal(0));
        a.emit(ILe);
        a.jump(JumpIfFalse(0), "depth_end"); // depth <= max_depth
        // iterations = powerOfTwo(max_depth - depth + 4)
        a.emit(LoadLocal(0));
        a.emit(LoadLocal(2));
        a.emit(ISub);
        a.emit(PushInt(4));
        a.emit(IAdd);
        a.emit(Call { func: POW, argc: 1 });
        a.emit(StoreLocal(3));
        a.emit(PushInt(1));
        a.emit(StoreLocal(4)); // i = 1
        a.label("iter_loop");
        a.emit(LoadLocal(4));
        a.emit(LoadLocal(3));
        a.emit(ILe);
        a.jump(JumpIfFalse(0), "iter_end"); // i <= iterations
        // t = makeTree(i, depth); checksum += t.check
        a.emit(LoadLocal(4));
        a.emit(LoadLocal(2));
        a.emit(Call { func: MAKE, argc: 2 });
        a.emit(StoreLocal(6));
        a.emit(LoadLocal(1));
        a.emit(LoadLocal(6));
        a.emit(Call { func: CHECK, argc: 1 });
        a.emit(IAdd);
        a.emit(StoreLocal(1));
        // t = makeTree(-i, depth); checksum += t.check   (-i = 0 - i)
        a.emit(PushInt(0));
        a.emit(LoadLocal(4));
        a.emit(ISub);
        a.emit(LoadLocal(2));
        a.emit(Call { func: MAKE, argc: 2 });
        a.emit(StoreLocal(6));
        a.emit(LoadLocal(1));
        a.emit(LoadLocal(6));
        a.emit(Call { func: CHECK, argc: 1 });
        a.emit(IAdd);
        a.emit(StoreLocal(1));
        // i += 1
        a.emit(LoadLocal(4));
        a.emit(PushInt(1));
        a.emit(IAdd);
        a.emit(StoreLocal(4));
        a.jump(Jump(0), "iter_loop");
        a.label("iter_end");
        a.emit(LoadLocal(2));
        a.emit(PushInt(2));
        a.emit(IAdd);
        a.emit(StoreLocal(2)); // depth += 2
        a.jump(Jump(0), "depth_loop");
        a.label("depth_end");
        // checksum += longRoot.check
        a.emit(LoadLocal(1));
        a.emit(LoadLocal(5));
        a.emit(Call { func: CHECK, argc: 1 });
        a.emit(IAdd);
        a.emit(StoreLocal(1));
        a.emit(LoadLocal(1));
        a.emit(Ret);
        Func { code: a.finish(), n_locals: 7 }
    };

    // makeTree(item, depth): 0=item 1=depth 2=left 3=right
    let make_fn = {
        let mut a = Asm::new();
        a.emit(PushInt(0));
        a.emit(LoadLocal(1));
        a.emit(ILt); // 0 < depth  (depth > 0)
        a.jump(JumpIfFalse(0), "leaf");
        // left = makeTree(2*item - 1, depth - 1)
        a.emit(PushInt(2));
        a.emit(LoadLocal(0));
        a.emit(IMul);
        a.emit(PushInt(1));
        a.emit(ISub);
        a.emit(LoadLocal(1));
        a.emit(PushInt(1));
        a.emit(ISub);
        a.emit(Call { func: MAKE, argc: 2 });
        a.emit(StoreLocal(2));
        // right = makeTree(2*item, depth - 1)
        a.emit(PushInt(2));
        a.emit(LoadLocal(0));
        a.emit(IMul);
        a.emit(LoadLocal(1));
        a.emit(PushInt(1));
        a.emit(ISub);
        a.emit(Call { func: MAKE, argc: 2 });
        a.emit(StoreLocal(3));
        // return NewNode(item, left, right)
        a.emit(LoadLocal(0));
        a.emit(LoadLocal(2));
        a.emit(LoadLocal(3));
        a.emit(NewNode);
        a.emit(Ret);
        a.label("leaf");
        a.emit(LoadLocal(0));
        a.emit(PushInt(NIL));
        a.emit(PushInt(NIL));
        a.emit(NewNode);
        a.emit(Ret);
        Func { code: a.finish(), n_locals: 4 }
    };

    // check(node): 0 = node handle.  if left==nil { item } else { item + left.check - right.check }
    let check_fn = {
        let mut a = Asm::new();
        a.emit(LoadLocal(0));
        a.emit(NodeLeft);
        a.emit(PushInt(NIL));
        a.emit(IEq); // isLeaf = (left == nil)
        a.jump(JumpIfFalse(0), "internal"); // not leaf -> recurse
        a.emit(LoadLocal(0));
        a.emit(NodeItem);
        a.emit(Ret); // leaf: return item
        a.label("internal");
        a.emit(LoadLocal(0));
        a.emit(NodeItem);
        a.emit(LoadLocal(0));
        a.emit(NodeLeft);
        a.emit(Call { func: CHECK, argc: 1 });
        a.emit(IAdd); // item + left.check
        a.emit(LoadLocal(0));
        a.emit(NodeRight);
        a.emit(Call { func: CHECK, argc: 1 });
        a.emit(ISub); // (item + left.check) - right.check
        a.emit(Ret);
        Func { code: a.finish(), n_locals: 1 }
    };

    // powerOfTwo(p): 0=p 1=res 2=i  => 2^p via loop-multiply (as in the .qn)
    let pow_fn = {
        let mut a = Asm::new();
        a.emit(PushInt(1));
        a.emit(StoreLocal(1)); // res = 1
        a.emit(PushInt(0));
        a.emit(StoreLocal(2)); // i = 0
        a.label("loop");
        a.emit(LoadLocal(2));
        a.emit(LoadLocal(0));
        a.emit(ILt);
        a.jump(JumpIfFalse(0), "end"); // i < p
        a.emit(LoadLocal(1));
        a.emit(PushInt(2));
        a.emit(IMul);
        a.emit(StoreLocal(1)); // res *= 2
        a.emit(LoadLocal(2));
        a.emit(PushInt(1));
        a.emit(IAdd);
        a.emit(StoreLocal(2)); // i += 1
        a.jump(Jump(0), "loop");
        a.label("end");
        a.emit(LoadLocal(1));
        a.emit(Ret);
        Func { code: a.finish(), n_locals: 3 }
    };

    vec![run_fn, make_fn, check_fn, pow_fn]
}

fn bench_tree() {
    const MAX_DEPTH: i64 = 10;
    const QUOIN: f64 = 661.0;
    const RUBY: f64 = 32.5;
    const PYTHON: f64 = 83.0;

    // Cross-backend correctness: all three must produce the identical checksum.
    let cs = run_arena(MAX_DEPTH);
    assert_eq!(run_box(MAX_DEPTH), cs, "Box backend checksum mismatch");
    assert_eq!(run_rc(MAX_DEPTH), cs, "Rc backend checksum mismatch");

    let iters = 30u32;
    let time = |f: fn(i64) -> i64| -> f64 {
        let _ = f(MAX_DEPTH); // warmup
        let t = Instant::now();
        let mut acc = 0i64;
        for _ in 0..iters {
            acc = acc.wrapping_add(f(MAX_DEPTH));
        }
        std::hint::black_box(acc);
        (t.elapsed().as_secs_f64() * 1e3) / iters as f64
    };
    let arena_ms = time(run_arena);
    let box_ms = time(run_box);
    let rc_ms = time(run_rc);

    println!("=== binary trees(10) — native port, {} iters, checksum {} ===", iters, cs);
    println!("  arena-unboxed : {:.3} ms/run   <- the unboxed-struct ceiling", arena_ms);
    report(arena_ms, QUOIN, RUBY, PYTHON);
    println!("  Box-per-node  : {:.3} ms/run   ({:.1}x arena — cost of per-node malloc)", box_ms, box_ms / arena_ms);
    println!("  Rc<RefCell>   : {:.3} ms/run   ({:.1}x arena — ~today's alloc model, no real GC)", rc_ms, rc_ms / arena_ms);

    // The honest ceiling: unboxed nodes WITH real per-node interpreter dispatch
    // (makeTree/check as bytecode). This is the apples-to-apples number vs the
    // interpreter fib/sieve figures and vs Ruby/Python (also interpreters).
    let funcs = build_tree_program();
    assert_eq!(run(&funcs, 0, MAX_DEPTH), cs, "interpreter tree checksum mismatch");
    let _ = run(&funcs, 0, MAX_DEPTH); // warmup
    let t = Instant::now();
    let mut acc = 0i64;
    for _ in 0..iters {
        acc = acc.wrapping_add(run(&funcs, 0, MAX_DEPTH));
    }
    std::hint::black_box(acc);
    let interp_ms = (t.elapsed().as_secs_f64() * 1e3) / iters as f64;
    println!("\n  interpreter (unboxed nodes + per-node dispatch) — THE HONEST CEILING:");
    println!("  {:.3} ms/run   ({:.1}x the native arena — the interpreter tax)", interp_ms, interp_ms / arena_ms);
    report(interp_ms, QUOIN, RUBY, PYTHON);
    println!();
}

fn report(per_ms: f64, quoin: f64, ruby: f64, python: f64) {
    let dir = |target: f64| {
        if per_ms <= target {
            format!("{:.2}x FASTER than", target / per_ms)
        } else {
            format!("{:.2}x slower than", per_ms / target)
        }
    };
    println!("  vs Quoin  ~{:>5.1} ms  ->  {:.1}x faster (ceiling)", quoin, quoin / per_ms);
    println!("  vs Ruby   ~{:>5.2} ms  ->  {}", ruby, dir(ruby));
    println!("  vs Python ~{:>5.2} ms  ->  {}", python, dir(python));
}
