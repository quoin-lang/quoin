# The window arena: native slot stores (the crossing-removal arc)

*Status: DESIGN (recon done; no slices implemented). Successor to the
direct-calls arc (docs/DIRECT_CALLS_ARCH.md, PR #75 + perf/window-hoist).
Written at `70030c5`.*

## 1. Why: the triple proof

Three independent experiments on the direct-calls arc converged:

1. **D2.5 interior riders** (env-swap skip, marshaling plans): flat.
2. **W0 method edges** (guards + call_indirect + slot_set): net-negative.
3. **Window-hoisted block edges** (guard + 2-3 slot_sets + call_indirect
   + result copy): net-zero — combinators warm-on 140.6ms vs off 139.7ms.

On this VM, **redistributing Cranelift→extern boundary crossings never
pays; only removing them does**. Every dormant edge family (W1 methods,
hoisted blocks) is blocked on exactly one capability: compiled code
writing and reading `vm.stack` slots with PLAIN NATIVE STORES/LOADS.

## 2. The two prerequisites

### 2.1 `Value` gets a fixed layout

Native code cannot construct a Rust enum with unspecified repr. Today
(pinned by `value_layout_facts`): 16 bytes, align 8, niche-packed
`Option<Value>`.

- `#[repr(C, u64)]` (or `u64` + explicit discriminants): tag qword at
  offset 0, payload qword at offset 8. Same 16-byte size.
- Discriminants pinned in declaration order: Int=0, Double=1, Bool=2,
  Nil=3, Object=4, Class=5, ClassMeta=6. The first three COINCIDE with
  the helper lane kinds (`KIND_INT/DOUBLE/BOOL`) — a scalar lane→Value
  native store is tag=kind, payload=bits verbatim. `KIND_NIL`(4)↔Nil(3)
  and `KIND_SLOT`(3, not a Value tag) need a 2-entry fixup — or renumber
  the KIND constants to match (preferred: KIND_SLOT is helper-internal).
- COST ACCEPTED: `Option<Value>` loses its niche (16→24B). Audited: only
  small per-task/per-handle fields and short Vecs (resume_stack, gather
  results, handle_table slots) — no bulk storage.
- Collect derive is repr-agnostic (tracing is by match, not layout).
- Payload semantics for native stores: Int/Double/Bool/Nil payloads are
  plain bits (Double via to_bits; Bool 0/1; Nil payload ignored-but-
  zeroed). Object/Class payloads are Gc pointers — native code only ever
  COPIES those whole (slot-to-slot 16-byte copy), never fabricates them.

### 2.2 `vm.stack` becomes `SlotStack`

A dedicated `#[repr(C)]` container replacing `Vec<Value>`:

    #[repr(C)]
    pub struct SlotStack<'gc> {
        ptr: *mut Value<'gc>,   // offset 0 — read by compiled code
        len: usize,             // offset 8 — read by compiled code
        cap: usize,
        _marker: ...,
    }

- Same API surface as the Vec usage today (push/truncate/len/get/
  get_mut/index/resize/last/iter…) — the swap is mechanical.
- `Collect`: trace `ptr[0..len]`.
- The struct's ADDRESS is stable for the VM's life (a VmState field;
  gc_arena is non-moving) — passed through the raw ABI (`slots: *mut
  SlotStack`) beside fuel/depth/epoch. Compiled code re-loads (ptr, len)
  from it per access, so Vec-style reallocation on grow stays legal —
  the pointer is never cached across a call that could grow.
- Growth stays in Rust (helper path); native code never pushes — it only
  reads/writes EXISTING slots below `len` (bounds-checked, branch to the
  invariant path).

## 3. What gets rewired (the payoff)

- The window-hoist block edge (already emitted, arena-ready): its 3
  slot_set crossings become native stores → per element = guard_block +
  call_indirect + native stores ≈ the designed ~25ns/element for
  collect:-shapes (from 43).
- `slot_peek`/`slot_set` call sites inside compiled code generally —
  every Dyn read/write in translated bodies drops a crossing.
- W1 method edges: the window push (receiver+args) becomes native
  stores into reserved caller scratch — the btrees/richards shell
  (field-heavy callees) finally addressable.
- `guard_recv`'s decode can go native later (read tag+payload directly)
  — recorded, not v1.

## 4. Slices (each gated)

- **A1 — Value repr + layout pin** (no behavior change): the repr, the
  discriminant/KIND alignment, `value_layout_facts` extended to pin tag
  offsets via a debug assertion on transmuted probes. GATE: corpus
  green; bench suite FLAT (interleaved 15-run; same-binary shim on any
  ±2% mover — repr changes are exactly the layout-noise shape).
- **A2 — SlotStack swap** (no behavior change): the container + Collect
  + mechanical call-site migration. GATE: corpus green ×5 incl. GC
  stress (tracing is the risk surface); bench FLAT.
- **A3 — native slot load/store emission + rewire the block edge**: the
  ABI slots pointer; emit load/bounds/store sequences; replace the
  hoisted edge's slot_sets. GATE: combinators ≥5% vs tier-off with
  QN_DIRECT_WARM on; suites across warm/stress matrix.
- **A4 — W1 method edges** (native window push at baked method sites).
  GATE: btrees ≥3%.
- **A5 — hardening**: the D3d-style sweep (GC stress on native-written
  slots is the new invariant surface: every native store must leave the
  slot a VALID traceable Value at every instant — tag written before or
  atomically-with payload; decide store order and pin it).

## 5. Soundness invariants

1. A native store never leaves a slot in a state the tracer cannot
   walk: write payload first, tag second (a stale tag with a new payload
   is a valid-if-wrong Value; a new OBJECT tag with a stale payload is a
   wild pointer — ORDER IS LOAD-BEARING and asserted in A5).
   [Interim simpler rule for A3: native stores write only scalar tags
   (Int/Double/Bool/Nil) and whole-16-byte copies of existing slots;
   fabricated object stores stay in helpers.]
2. Native code never grows the stack; `len` is the hard bound.
3. The SlotStack address is per-VM; compiled code receives it per call
   (workers each pass their own).
4. All existing helper index semantics unchanged (same indices, same
   storage — one address space).

## 6. Non-goals

- No arena-separate-from-stack (the doc's original option C ring): the
  single-address-space SlotStack achieves native access without the
  two-space migration; revisit only if Vec-grow semantics measurably
  hurt.
- No NaN-boxing / Value shrink — different arc, different risk.
- No interpreter changes: SlotStack's Rust API keeps the interpreter's
  code shape byte-for-byte.
