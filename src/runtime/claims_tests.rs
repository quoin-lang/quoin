//! The §5.1 deadlock list, driven directly against the claim state machine
//! (no VM): joint atomicity, reservation/no-barging, FIFO fairness, nested
//! lanelessness, hot-object non-starvation, cycles (2-party, ring,
//! cross-peer), ghost skipping, depth caps, retraction, and drain.

use std::cell::RefCell;
use std::rc::Rc;

use super::{Acquire, ClaimRegistry, PeerClaims, WaitKind, would_deadlock};

fn all_live() -> impl FnMut(usize, u64) -> bool {
    |_, _| true
}

fn peer(lanes: u32) -> PeerClaims {
    PeerClaims::new("svc:test".into(), lanes)
}

/// Acquire or panic — for test steps that must succeed.
fn take(p: &mut PeerClaims, task: usize, object: u64, nested: bool) -> Option<u32> {
    match p.try_acquire(task, task as u64, object, "Obj", nested) {
        Acquire::Granted { lane } => lane,
        other => panic!("task {task} expected grant on {object}, got {other:?}"),
    }
}

/// Queue or panic — for test steps that must contend.
fn queue(p: &mut PeerClaims, task: usize, object: u64, kind: WaitKind) -> Option<usize> {
    let nested = kind == WaitKind::Nested;
    match p.try_acquire(task, task as u64, object, "Obj", nested) {
        Acquire::WouldQueue { blocker } => {
            p.enqueue(task, task as u64, object, "Obj", kind);
            blocker
        }
        other => panic!("task {task} expected to queue on {object}, got {other:?}"),
    }
}

#[test]
fn uncontended_joint_grant_and_release() {
    let mut p = peer(2);
    let lane = take(&mut p, 1, 10, false);
    assert!(lane.is_some());
    let grants = p.end_call(1, 10, &mut all_live());
    assert!(grants.is_empty());
    assert_eq!(p.lanes(), (2, 2));
}

#[test]
fn joint_atomicity_object_free_but_no_lane() {
    // One lane: t1 takes O1+lane; t2 wants O2 (free) but no lane exists —
    // t2 must hold NOTHING while it waits (rule 2), and be granted both
    // atomically when the lane comes home.
    let mut p = peer(1);
    take(&mut p, 1, 1, false);
    assert_eq!(queue(&mut p, 2, 2, WaitKind::TopLevel), None);
    // O2 is reserved for t2: a third caller queues BEHIND, not past it.
    assert_eq!(queue(&mut p, 3, 2, WaitKind::TopLevel), None);
    let grants = p.end_call(1, 1, &mut all_live());
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].task, 2);
    assert!(grants[0].lane.is_some());
    // t3 becomes the new reserved head only after t2 finishes.
    let lane = grants[0].lane;
    let grants = p.end_call(2, 2, &mut all_live());
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].task, 3);
    assert_eq!(grants[0].lane, lane);
}

#[test]
fn fifo_per_object_no_barging() {
    let mut p = peer(4);
    take(&mut p, 1, 7, false);
    queue(&mut p, 2, 7, WaitKind::TopLevel);
    queue(&mut p, 3, 7, WaitKind::TopLevel);
    let grants = p.end_call(1, 7, &mut all_live());
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].task, 2, "head of the FIFO, not a later waiter");
    // A newcomer queues behind t3 even though it arrived after the handoff.
    assert!(matches!(
        p.try_acquire(4, 4, 7, "Obj", false),
        Acquire::WouldQueue { blocker: Some(2) }
    ));
}

#[test]
fn nested_never_touches_lanes() {
    // Rule 3: with zero free lanes, a nested acquire of a free object is
    // granted immediately (it rides its bound lane).
    let mut p = peer(1);
    take(&mut p, 1, 1, false);
    assert_eq!(p.lanes(), (1, 0));
    let lane = take(&mut p, 1, 2, true);
    assert_eq!(lane, None);
    // Its release frees no lane.
    let grants = p.end_call(1, 2, &mut all_live());
    assert!(grants.is_empty());
    assert_eq!(p.lanes(), (1, 0));
}

#[test]
fn reentry_depth_and_cap() {
    let mut p = peer(1);
    take(&mut p, 1, 1, false);
    for _ in 0..15 {
        assert_eq!(p.try_acquire(1, 1, 1, "Obj", true), Acquire::Reentrant);
    }
    assert_eq!(p.try_acquire(1, 1, 1, "Obj", true), Acquire::TooDeep);
    // Unwind: 15 nested ends release nothing; the outermost frees the lane.
    for _ in 0..15 {
        assert!(p.end_call(1, 1, &mut all_live()).is_empty());
    }
    assert_eq!(p.lanes(), (1, 0));
    p.end_call(1, 1, &mut all_live());
    assert_eq!(p.lanes(), (1, 1));
}

#[test]
fn hot_object_queue_never_pins_a_lane() {
    // The anti-head-of-line test that kills the lane-first discipline: a
    // saturated hot object leaves other objects reachable at lane speed.
    let mut p = peer(2);
    take(&mut p, 1, 1, false); // hot object, lane 0 or 1
    queue(&mut p, 2, 1, WaitKind::TopLevel);
    queue(&mut p, 3, 1, WaitKind::TopLevel);
    queue(&mut p, 4, 1, WaitKind::TopLevel);
    // The queue holds no lane: a call to a second object gets one instantly.
    assert!(take(&mut p, 5, 2, false).is_some());
}

#[test]
fn shape_one_regression_lane_exhaustion_with_nested_sends() {
    // §5.1's shape-1: all lanes busy; the busy calls' callbacks nested-send
    // to an object a further caller waits on. Everything must drain.
    let mut p = peer(2);
    take(&mut p, 1, 1, false); // lane
    take(&mut p, 2, 2, false); // lane — pool exhausted
    // t3 wants object 3 top-level: no lane, becomes reserved head.
    queue(&mut p, 3, 3, WaitKind::TopLevel);
    // t1's callback nested-sends to object 2 (owned by t2): queues laneless.
    assert_eq!(queue(&mut p, 1, 2, WaitKind::Nested), Some(2));
    // t2 finishes: its object goes to t1 (nested, no lane), its lane to t3.
    let grants = p.end_call(2, 2, &mut all_live());
    assert_eq!(grants.len(), 2);
    assert_eq!((grants[0].task, grants[0].lane), (1, None));
    assert_eq!(grants[1].task, 3);
    assert!(grants[1].lane.is_some());
    // t1 unwinds: nested object, then its own call.
    assert!(p.end_call(1, 2, &mut all_live()).is_empty());
    p.end_call(1, 1, &mut all_live());
    p.end_call(3, 3, &mut all_live());
    assert_eq!(p.lanes(), (2, 2));
}

fn registry_of(peers: Vec<Rc<RefCell<PeerClaims>>>) -> ClaimRegistry {
    Rc::new(RefCell::new(peers))
}

#[test]
fn two_party_cycle_detected_at_the_closer() {
    let p = Rc::new(RefCell::new(peer(4)));
    let reg = registry_of(vec![p.clone()]);
    take(&mut p.borrow_mut(), 1, 1, false);
    take(&mut p.borrow_mut(), 2, 2, false);
    // t1 (inside a callback) nested-waits on O2 (held by t2): no cycle yet.
    let blocker = queue(&mut p.borrow_mut(), 1, 2, WaitKind::Nested).unwrap();
    assert!(would_deadlock(&reg, 1, blocker, "Obj#2", &mut all_live()).is_none());
    // t2 nested-tries O1 (held by t1): t1 waits on t2 — the walk closes.
    let d = match p.borrow_mut().try_acquire(2, 2, 1, "Obj", true) {
        Acquire::WouldQueue { blocker: Some(b) } => b,
        other => panic!("expected queue, got {other:?}"),
    };
    let cycle = would_deadlock(&reg, 2, d, "Obj#1", &mut all_live());
    let cycle = cycle.expect("cycle must be detected");
    assert!(cycle.contains("deadlock"), "{cycle}");
    assert!(
        cycle.contains("Obj#1") && cycle.contains("Obj#2"),
        "{cycle}"
    );
}

#[test]
fn three_party_ring_detected() {
    let p = Rc::new(RefCell::new(peer(8)));
    let reg = registry_of(vec![p.clone()]);
    take(&mut p.borrow_mut(), 1, 1, false);
    take(&mut p.borrow_mut(), 2, 2, false);
    take(&mut p.borrow_mut(), 3, 3, false);
    queue(&mut p.borrow_mut(), 1, 2, WaitKind::Nested); // t1 → t2
    queue(&mut p.borrow_mut(), 2, 3, WaitKind::Nested); // t2 → t3
    let b = match p.borrow_mut().try_acquire(3, 3, 1, "Obj", true) {
        Acquire::WouldQueue { blocker: Some(b) } => b,
        other => panic!("expected queue, got {other:?}"),
    };
    assert!(would_deadlock(&reg, 3, b, "Obj#1", &mut all_live()).is_some());
}

#[test]
fn cross_peer_cycle_detected() {
    // t1 holds A.O1 and nested-waits on B.O9; t2 holds B.O9 and nested-tries
    // A.O1 — the walk crosses peers through the shared registry.
    let a = Rc::new(RefCell::new(PeerClaims::new("svc:a".into(), 4)));
    let b = Rc::new(RefCell::new(PeerClaims::new("svc:b".into(), 4)));
    let reg = registry_of(vec![a.clone(), b.clone()]);
    take(&mut a.borrow_mut(), 1, 1, false);
    take(&mut b.borrow_mut(), 2, 9, false);
    queue(&mut b.borrow_mut(), 1, 9, WaitKind::Nested); // t1 → t2 via B
    let blocker = match a.borrow_mut().try_acquire(2, 2, 1, "Obj", true) {
        Acquire::WouldQueue { blocker: Some(x) } => x,
        other => panic!("expected queue, got {other:?}"),
    };
    let cycle = would_deadlock(&reg, 2, blocker, "Obj#1", &mut all_live());
    assert!(cycle.is_some());
    // And without t1's wait, no cycle: retract and re-check.
    b.borrow_mut().retract(1, 9);
    assert!(would_deadlock(&reg, 2, blocker, "Obj#1", &mut all_live()).is_none());
}

#[test]
fn ghosts_are_skipped_and_stale_edges_ignored() {
    let p = Rc::new(RefCell::new(peer(1)));
    let reg = registry_of(vec![p.clone()]);
    take(&mut p.borrow_mut(), 1, 1, false);
    queue(&mut p.borrow_mut(), 2, 1, WaitKind::TopLevel);
    queue(&mut p.borrow_mut(), 3, 1, WaitKind::TopLevel);
    // t2 was cancelled (not live): the release skips it, grants t3.
    let mut live = |t: usize, _e: u64| t != 2;
    let grants = p.borrow_mut().end_call(1, 1, &mut live);
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].task, 3);
    // A stale edge (cancelled, unretracted) can't fabricate a cycle: t4
    // queues on O1; a walk passing through the ghost t2 dies at the filter.
    let b = match p.borrow_mut().try_acquire(4, 4, 1, "Obj", true) {
        Acquire::WouldQueue { blocker: Some(b) } => b,
        other => panic!("expected queue, got {other:?}"),
    };
    assert!(would_deadlock(&reg, 4, b, "Obj#1", &mut live).is_none());
}

#[test]
fn retract_clears_queue_and_edge() {
    let mut p = peer(1);
    take(&mut p, 1, 1, false);
    queue(&mut p, 2, 1, WaitKind::TopLevel);
    p.retract(2, 1);
    let grants = p.end_call(1, 1, &mut all_live());
    assert!(grants.is_empty(), "retracted waiter must not be granted");
    assert_eq!(p.lanes(), (1, 1));
}

#[test]
fn drain_waits_for_every_lane() {
    let mut p = peer(2);
    take(&mut p, 1, 1, false);
    take(&mut p, 2, 2, false);
    assert!(!p.request_drain(9, 9));
    let g = p.end_call(1, 1, &mut all_live());
    assert!(g.is_empty(), "one lane still out — no drain wake yet");
    let g = p.end_call(2, 2, &mut all_live());
    assert_eq!(g.len(), 1);
    assert_eq!((g[0].task, g[0].lane), (9, None));
    // Idempotent once drained.
    assert!(p.request_drain(9, 9));
}

#[test]
fn stats_track_contention() {
    let mut p = peer(1);
    take(&mut p, 1, 1, false);
    queue(&mut p, 2, 1, WaitKind::TopLevel);
    queue(&mut p, 3, 1, WaitKind::TopLevel);
    p.end_call(1, 1, &mut all_live());
    assert_eq!(p.stats.acquisitions, 3);
    assert_eq!(p.stats.contended, 2);
    assert_eq!(p.stats.queue_high_water, 2);
    let rows = p.object_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].owner, Some(2));
    assert_eq!(rows[0].waiters.len(), 1);
}
