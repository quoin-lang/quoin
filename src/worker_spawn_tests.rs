//! The done-lane typing (SUPERVISION.md slice 0): a body that RAN AND REPORTED is
//! `WorkerExit::Failed` — an ordinary catchable error — while a PANIC is
//! `WorkerExit::Died` with reason `Panicked`, the seam `join` raises
//! `PeerDiedError` from. E2e can't reach the panic arm: a panic in a worker
//! thread is by definition a bug no guest surface triggers, so the mapping is
//! pinned here.

use super::spawn_worker_with;
use crate::error::PeerDeathReason;
use crate::worker::WorkerExit;

#[test]
fn a_panicking_worker_body_is_a_death() {
    let ch = spawn_worker_with("t".to_string(), "worker", |_link| panic!("boom"));
    match ch.done_rx.recv_blocking() {
        Ok(Err(WorkerExit::Died { reason, detail })) => {
            assert_eq!(reason, PeerDeathReason::Panicked);
            assert!(detail.contains("worker panicked: boom"), "{detail}");
        }
        other => panic!("expected a Died report, got {other:?}"),
    }
}

#[test]
fn a_reporting_worker_body_is_a_failure_not_a_death() {
    let ch = spawn_worker_with("t".to_string(), "worker", |_link| {
        Err("unit refused".to_string())
    });
    match ch.done_rx.recv_blocking() {
        Ok(Err(WorkerExit::Failed(msg))) => assert_eq!(msg, "unit refused"),
        other => panic!("expected a Failed report, got {other:?}"),
    }
}
