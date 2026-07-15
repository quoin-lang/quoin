//! Per-object claims and connection lanes (docs/internal/ACTOR_OBJECTS.md §5,
//! discipline frozen as §5.1): the state machine behind hosted-object
//! mailboxes. One `PeerClaims` per worker/peer, shared `Rc` across its
//! proxies, registered in `vm.io.claim_peers` for `VM.claims` and the cycle
//! walk.
//!
//! The module is deliberately scheduler-agnostic — every method is a pure
//! state transition returning DECISIONS (grant this task, with this lane) that
//! the caller maps onto parks and wakes. That keeps the deadlock-critical
//! logic unit-testable without a VM. All state is parent-side plain data
//! mutated between yields, which is what makes the two §5.1 superpowers
//! possible:
//!
//! - **Atomic joint acquisition** (rule 2): a top-level send takes (object,
//!   lane) in one step or parks wanting both — never holding one kind while
//!   waiting for the other, so cross-kind cycles cannot form. A freed object
//!   is RESERVED for its FIFO head (no barging); freed lanes go to reserved
//!   heads in per-peer FIFO order — so a hot object's queue never pins a lane
//!   and never starves the peer's other objects.
//! - **Complete cycle detection** (rule 6): host-op callbacks run on the
//!   caller's own fiber, so every wait in a re-entrant call web is a task in
//!   some peer's `parked_on` index. `would_deadlock` walks task → owner →
//!   task across ALL registered peers before a caller parks; a closed walk
//!   raises catchably at the task that closes the cycle instead of parking.
//!
//! Nested sends (rule 3) ride their conversation's bound lane and only ever
//! wait for object claims — a nested `try_acquire` never touches the lane
//! pool. Same-owner re-entry nests depth-capped (rule 4).

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::Instant;

/// The registry of every peer's claim state in one VM (`vm.io.claim_peers`):
/// the substrate for the cross-peer waits-for walk and for `VM.claims`.
/// Entries deliberately outlive their peer — a dead service's counters are
/// the post-mortem (the `ext_stats` precedent).
pub type ClaimRegistry = Rc<RefCell<Vec<Rc<RefCell<PeerClaims>>>>>;

/// The most deeply one task may re-enter a single object's claim (a hosted
/// method's callback sending to the object it is already inside). Mirrors the
/// extension connection cap.
const MAX_OBJECT_DEPTH: u32 = 16;

/// Defensive bound on the waits-for walk (a real cycle closes in a handful of
/// steps; hitting this means corrupted state, not a longer deadlock).
const MAX_WALK: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitKind {
    /// Wants (object, lane) jointly — holds nothing while waiting.
    TopLevel,
    /// Holds a bound lane (an open conversation); wants the object only.
    Nested,
}

#[derive(Debug)]
struct Waiter {
    task: usize,
    epoch: u64,
    kind: WaitKind,
    queued_at: Instant,
}

#[derive(Debug, Default)]
struct ObjectClaim {
    /// `(task, park_epoch at claim)` of the current holder.
    owner: Option<(usize, u64)>,
    /// Same-owner re-entry depth (rule 4); the claim releases at 0.
    depth: u32,
    /// Which lane the holder's conversation occupies — `None` for a holder
    /// granted nested (riding an outer conversation's lane).
    owner_lane: Option<u32>,
    /// "Class#id" for reports and cycle messages (set at first touch).
    label: String,
    waiters: VecDeque<Waiter>,
    /// Freed while a TOP-LEVEL head waiter had no lane: held for that head
    /// (no barging) until a lane frees (`ready_heads`).
    reserved: bool,
}

/// One decision from `try_acquire`. `WouldQueue` commits nothing — the caller
/// runs the cycle walk and then either `enqueue`s and parks, or raises.
#[derive(Debug, PartialEq, Eq)]
pub enum Acquire {
    /// Object claimed; `lane` is `Some` for a top-level grant (joint), `None`
    /// for a nested grant (rides the bound lane).
    Granted { lane: Option<u32> },
    /// Same-owner re-entry; depth bumped. Ride the bound lane.
    Reentrant,
    /// Re-entry past `MAX_OBJECT_DEPTH`.
    TooDeep,
    /// Another task holds (or is reserved for) the object; `blocker` is the
    /// task a waits-for edge would point at (`None` when the object is merely
    /// reserved — the head holds nothing, so no cycle can pass through it).
    WouldQueue { blocker: Option<usize> },
}

/// One wake decision returned by a release: hand `Wake::ServiceClaim` to
/// `task` (ownership already transferred). `lane` is `Some` for a top-level
/// grant, `None` for a nested grant or a drain wake.
#[derive(Debug, PartialEq, Eq)]
pub struct Grant {
    pub task: usize,
    pub epoch: u64,
    pub lane: Option<u32>,
}

/// Accumulated per-peer counters (survive the peer; rendered by
/// `VM.claims` / `VM.claimsReport`).
#[derive(Debug, Default)]
pub struct ClaimStats {
    pub acquisitions: u64,
    pub contended: u64,
    pub total_wait_micros: u64,
    pub max_wait_micros: u64,
    pub queue_high_water: usize,
    pub max_depth: u32,
    pub deadlocks: u64,
}

/// A live row for `VM.claims`.
#[derive(Debug)]
pub struct ObjectRow {
    pub object: u64,
    pub label: String,
    pub owner: Option<usize>,
    pub depth: u32,
    pub reserved: bool,
    /// (task, kind, waited µs) per queued waiter, FIFO order.
    pub waiters: Vec<(usize, WaitKind, u64)>,
}

#[derive(Debug)]
pub struct PeerClaims {
    /// Peer label for reports ("svc:pool.qn").
    pub label: String,
    objects: HashMap<u64, ObjectClaim>,
    lane_count: u32,
    free_lanes: Vec<u32>,
    /// Objects reserved for their head waiter, FIFO by reservation time —
    /// the next freed lane goes to the front.
    ready_heads: VecDeque<u64>,
    /// task → (object, park epoch) it is claim-parked on: the waits-for
    /// index the cycle walk reads. Epochs let the walk skip entries whose
    /// task was cancelled and hasn't retracted yet.
    parked_on: HashMap<usize, (u64, u64)>,
    /// Tasks parked in `serviceStop` waiting for all lanes to come home.
    drain_waiters: Vec<(usize, u64)>,
    pub stats: ClaimStats,
}

impl PeerClaims {
    pub fn new(label: String, lanes: u32) -> Self {
        let lanes = lanes.max(1);
        PeerClaims {
            label,
            objects: HashMap::new(),
            lane_count: lanes,
            free_lanes: (0..lanes).rev().collect(),
            ready_heads: VecDeque::new(),
            parked_on: HashMap::new(),
            drain_waiters: Vec::new(),
            stats: ClaimStats::default(),
        }
    }

    /// (total, free).
    pub fn lanes(&self) -> (u32, u32) {
        (self.lane_count, self.free_lanes.len() as u32)
    }

    fn touch_label(&mut self, object_id: u64, label: &str) {
        let o = self.objects.entry(object_id).or_default();
        if o.label.is_empty() {
            o.label = format!("{label}#{object_id}");
        }
    }

    /// Rule 2 (top-level, `nested: false`): grant (object, lane) jointly or
    /// report who blocks. Rules 3/4 (`nested: true`): grant the object alone,
    /// re-enter, or report the blocker — the lane pool is never consulted.
    /// Commits nothing on `WouldQueue` (no park separates the caller's cycle
    /// walk from its `enqueue`, so the decision cannot go stale).
    pub fn try_acquire(
        &mut self,
        task: usize,
        epoch: u64,
        object_id: u64,
        label: &str,
        nested: bool,
    ) -> Acquire {
        self.stats.acquisitions += 1;
        self.touch_label(object_id, label);
        enum Step {
            Reentrant(u32),
            TooDeep,
            Blocked(usize),
            Reserved,
            Free,
        }
        let step = {
            let o = self.objects.get_mut(&object_id).expect("touched above");
            match o.owner {
                Some((owner, _)) if owner == task => {
                    if o.depth >= MAX_OBJECT_DEPTH {
                        Step::TooDeep
                    } else {
                        o.depth += 1;
                        Step::Reentrant(o.depth)
                    }
                }
                Some((owner, _)) => Step::Blocked(owner),
                None if o.reserved => Step::Reserved,
                None => Step::Free,
            }
        };
        match step {
            Step::TooDeep => Acquire::TooDeep,
            Step::Reentrant(d) => {
                if d > self.stats.max_depth {
                    self.stats.max_depth = d;
                }
                Acquire::Reentrant
            }
            Step::Blocked(owner) => Acquire::WouldQueue {
                blocker: Some(owner),
            },
            Step::Reserved => Acquire::WouldQueue { blocker: None },
            Step::Free => {
                if nested {
                    let o = self.objects.get_mut(&object_id).expect("present");
                    o.owner = Some((task, epoch));
                    o.depth = 1;
                    o.owner_lane = None;
                    return Acquire::Granted { lane: None };
                }
                match self.free_lanes.pop() {
                    Some(lane) => {
                        let o = self.objects.get_mut(&object_id).expect("present");
                        o.owner = Some((task, epoch));
                        o.depth = 1;
                        o.owner_lane = Some(lane);
                        Acquire::Granted { lane: Some(lane) }
                    }
                    // Object free, no lane: the caller becomes the reserved
                    // head when it enqueues.
                    None => Acquire::WouldQueue { blocker: None },
                }
            }
        }
    }

    /// Queue after a `WouldQueue` decision.
    pub fn enqueue(
        &mut self,
        task: usize,
        epoch: u64,
        object_id: u64,
        label: &str,
        kind: WaitKind,
    ) {
        self.stats.contended += 1;
        self.touch_label(object_id, label);
        self.parked_on.insert(task, (object_id, epoch));
        let (queue_len, needs_reserve) = {
            let o = self.objects.get_mut(&object_id).expect("touched above");
            o.waiters.push_back(Waiter {
                task,
                epoch,
                kind,
                queued_at: Instant::now(),
            });
            (o.waiters.len(), o.owner.is_none() && !o.reserved)
        };
        if queue_len > self.stats.queue_high_water {
            self.stats.queue_high_water = queue_len;
        }
        if needs_reserve {
            self.objects.get_mut(&object_id).expect("present").reserved = true;
            self.ready_heads.push_back(object_id);
        }
    }

    /// Withdraw a queued waiter (cancellation unwound before any grant): drop
    /// its queue entry and waits-for edge so no stale edge or ghost survives.
    pub fn retract(&mut self, task: usize, object_id: u64) {
        self.parked_on.remove(&task);
        if let Some(o) = self.objects.get_mut(&object_id) {
            o.waiters.retain(|w| w.task != task);
        }
        self.gc_object(object_id);
    }

    /// Drop a fully idle object entry — no owner, no waiters, not reserved.
    /// Keeps the map bounded when object ids are minted per call (extension
    /// class-side pseudo-objects); a live id just re-enters via `touch_label`.
    fn gc_object(&mut self, object_id: u64) {
        if self
            .objects
            .get(&object_id)
            .is_some_and(|o| o.owner.is_none() && o.waiters.is_empty() && !o.reserved)
        {
            self.objects.remove(&object_id);
        }
    }

    /// End one call on an object: the outermost release frees the claim and,
    /// for a top-level holder, its lane. Returns the wakes to deliver, in
    /// order. `live` answers whether a queued `(task, epoch)` still waits
    /// (park-epoch identity — cancelled ghosts are skipped, the channel.rs
    /// rule).
    pub fn end_call(
        &mut self,
        task: usize,
        object_id: u64,
        live: &mut dyn FnMut(usize, u64) -> bool,
    ) -> Vec<Grant> {
        let lane = {
            let Some(o) = self.objects.get_mut(&object_id) else {
                return Vec::new();
            };
            debug_assert_eq!(o.owner.map(|(t, _)| t), Some(task), "release by non-owner");
            o.depth = o.depth.saturating_sub(1);
            if o.depth > 0 {
                return Vec::new();
            }
            o.owner = None;
            o.owner_lane.take()
        };
        let mut grants = Vec::new();
        self.promote_head(object_id, live, &mut grants);
        if let Some(lane) = lane {
            self.release_lane(lane, live, &mut grants);
        }
        self.gc_object(object_id);
        grants
    }

    /// The object just lost its owner: hand it to the front LIVE waiter — a
    /// nested head takes it immediately (no lane involved); a top-level head
    /// takes a free lane with it, or reserves the object and queues for one.
    fn promote_head(
        &mut self,
        object_id: u64,
        live: &mut dyn FnMut(usize, u64) -> bool,
        grants: &mut Vec<Grant>,
    ) {
        loop {
            let popped = {
                let Some(o) = self.objects.get_mut(&object_id) else {
                    return;
                };
                match o.waiters.pop_front() {
                    Some(w) => w,
                    None => {
                        o.reserved = false;
                        return;
                    }
                }
            };
            if !live(popped.task, popped.epoch) {
                self.parked_on.remove(&popped.task);
                continue;
            }
            match popped.kind {
                WaitKind::Nested => {
                    {
                        let o = self.objects.get_mut(&object_id).expect("present");
                        o.owner = Some((popped.task, popped.epoch));
                        o.depth = 1;
                        o.owner_lane = None;
                        o.reserved = false;
                    }
                    self.grant(popped, None, grants);
                    return;
                }
                WaitKind::TopLevel => {
                    if let Some(lane) = self.free_lanes.pop() {
                        {
                            let o = self.objects.get_mut(&object_id).expect("present");
                            o.owner = Some((popped.task, popped.epoch));
                            o.depth = 1;
                            o.owner_lane = Some(lane);
                            o.reserved = false;
                        }
                        self.grant(popped, Some(lane), grants);
                        return;
                    }
                    // No lane: the head keeps its front slot and its edge;
                    // the object joins the lane queue.
                    let needs_ready = {
                        let o = self.objects.get_mut(&object_id).expect("present");
                        o.waiters.push_front(popped);
                        !std::mem::replace(&mut o.reserved, true)
                    };
                    if needs_ready {
                        self.ready_heads.push_back(object_id);
                    }
                    return;
                }
            }
        }
    }

    /// A lane came home: hand it to the oldest reserved head (FIFO), or bank
    /// it — and once every lane is home, wake the drain waiters
    /// (`serviceStop`).
    fn release_lane(
        &mut self,
        lane: u32,
        live: &mut dyn FnMut(usize, u64) -> bool,
        grants: &mut Vec<Grant>,
    ) {
        let mut lane = Some(lane);
        'homes: while let Some(l) = lane.take() {
            let Some(object_id) = self.ready_heads.pop_front() else {
                self.free_lanes.push(l);
                break;
            };
            loop {
                let popped = {
                    let Some(o) = self.objects.get_mut(&object_id) else {
                        lane = Some(l);
                        continue 'homes;
                    };
                    if !o.reserved {
                        lane = Some(l);
                        continue 'homes;
                    }
                    match o.waiters.pop_front() {
                        Some(w) => w,
                        None => {
                            o.reserved = false;
                            lane = Some(l);
                            continue 'homes;
                        }
                    }
                };
                if !live(popped.task, popped.epoch) {
                    self.parked_on.remove(&popped.task);
                    continue;
                }
                match popped.kind {
                    WaitKind::Nested => {
                        // A ghost head left a nested waiter in front: it takes
                        // the object without the lane; the lane keeps looking.
                        {
                            let o = self.objects.get_mut(&object_id).expect("present");
                            o.owner = Some((popped.task, popped.epoch));
                            o.depth = 1;
                            o.owner_lane = None;
                            o.reserved = false;
                        }
                        self.grant(popped, None, grants);
                        lane = Some(l);
                        continue 'homes;
                    }
                    WaitKind::TopLevel => {
                        {
                            let o = self.objects.get_mut(&object_id).expect("present");
                            o.owner = Some((popped.task, popped.epoch));
                            o.depth = 1;
                            o.owner_lane = Some(l);
                            o.reserved = false;
                        }
                        self.grant(popped, Some(l), grants);
                        continue 'homes;
                    }
                }
            }
        }
        if self.free_lanes.len() as u32 == self.lane_count {
            for (task, epoch) in self.drain_waiters.drain(..) {
                grants.push(Grant {
                    task,
                    epoch,
                    lane: None,
                });
            }
        }
    }

    fn grant(&mut self, w: Waiter, lane: Option<u32>, grants: &mut Vec<Grant>) {
        self.parked_on.remove(&w.task);
        let micros = w.queued_at.elapsed().as_micros() as u64;
        self.stats.total_wait_micros += micros;
        if micros > self.stats.max_wait_micros {
            self.stats.max_wait_micros = micros;
        }
        grants.push(Grant {
            task: w.task,
            epoch: w.epoch,
            lane,
        });
    }

    /// `serviceStop`'s drain: `true` = already drained (every lane home);
    /// otherwise the task is queued for a wake when the last lane returns.
    pub fn request_drain(&mut self, task: usize, epoch: u64) -> bool {
        if self.free_lanes.len() as u32 == self.lane_count {
            return true;
        }
        self.drain_waiters.push((task, epoch));
        false
    }

    /// The waits-for edge of a task parked on THIS peer, if any: the owner of
    /// the object it waits for. `None` when it waits behind a reservation or
    /// for a lane (nothing to cycle through — the §5.1 completeness
    /// argument), or when the entry is stale (`live` fails: cancelled, not
    /// yet retracted).
    fn waits_for(
        &self,
        task: usize,
        live: &mut dyn FnMut(usize, u64) -> bool,
    ) -> Option<(usize, String)> {
        let (object_id, epoch) = *self.parked_on.get(&task)?;
        if !live(task, epoch) {
            return None;
        }
        let o = self.objects.get(&object_id)?;
        let (owner, _) = o.owner?;
        Some((owner, o.label.clone()))
    }

    /// All live waits-for edges of this peer, for `VM.claims`/`claimsReport`:
    /// `(waiter task, object label, owner task if the object is held)`.
    pub fn edges(&self) -> Vec<(usize, String, Option<usize>)> {
        self.parked_on
            .iter()
            .map(|(task, (obj, _))| {
                let o = self.objects.get(obj);
                (
                    *task,
                    o.map(|o| o.label.clone()).unwrap_or_default(),
                    o.and_then(|o| o.owner.map(|(t, _)| t)),
                )
            })
            .collect()
    }

    /// Live rows for `VM.claims` (objects with an owner, waiters, or a
    /// reservation), sorted by object id.
    pub fn object_rows(&self) -> Vec<ObjectRow> {
        let mut rows: Vec<ObjectRow> = self
            .objects
            .iter()
            .filter(|(_, o)| o.owner.is_some() || !o.waiters.is_empty() || o.reserved)
            .map(|(id, o)| ObjectRow {
                object: *id,
                label: o.label.clone(),
                owner: o.owner.map(|(t, _)| t),
                depth: o.depth,
                reserved: o.reserved,
                waiters: o
                    .waiters
                    .iter()
                    .map(|w| (w.task, w.kind, w.queued_at.elapsed().as_micros() as u64))
                    .collect(),
            })
            .collect();
        rows.sort_by_key(|r| r.object);
        rows
    }
}

/// The §5.1 rule-6 walk, run by a caller holding NO borrow on any peer:
/// follow `task → owner(parked-on object) → …` across every registered peer.
/// Returns the rendered cycle when the walk closes on `me` — the caller
/// raises instead of parking. `start` is the blocking owner reported by
/// `try_acquire`; `first_label` names the object being acquired; `live`
/// filters stale (cancelled, unretracted) edges so they can't fabricate a
/// cycle.
pub fn would_deadlock(
    registry: &ClaimRegistry,
    me: usize,
    start: usize,
    first_label: &str,
    live: &mut dyn FnMut(usize, u64) -> bool,
) -> Option<String> {
    let mut path = vec![format!(
        "task {me} waits for {first_label} (held by task {start})"
    )];
    let mut current = start;
    for _ in 0..MAX_WALK {
        if current == me {
            return Some(format!(
                "deadlock: the calls wait on each other in a cycle — {}",
                path.join(", which ")
            ));
        }
        let mut edge: Option<(usize, String)> = None;
        for peer in registry.borrow().iter() {
            if let Some(e) = peer.borrow().waits_for(current, live) {
                edge = Some(e);
                break;
            }
        }
        let (next, label) = edge?;
        path.push(format!(
            "task {current} waits for {label} (held by task {next})"
        ));
        current = next;
    }
    None
}

#[cfg(test)]
#[path = "claims_tests.rs"]
mod claims_tests;
