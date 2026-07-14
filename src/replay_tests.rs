//! Unit tests for the wake-log event format and replay cursor (`replay.rs`). The
//! end-to-end record→replay divergence test lives in `tests/wake_replay.rs`.

use super::*;

fn all_events() -> Vec<WakeEvent> {
    vec![
        WakeEvent::Pick { tid: 3 },
        WakeEvent::Rotate { preempt: true },
        WakeEvent::Rotate { preempt: false },
        WakeEvent::Io {
            tid: 0,
            aborted: false,
            hash: 0xDEAD_BEEF_0123_4567,
        },
        WakeEvent::Io {
            tid: 7,
            aborted: true,
            hash: 0,
        },
        WakeEvent::Deadline {
            tid: 2,
            target: 5,
            epoch: 99,
        },
    ]
}

#[test]
fn event_lines_round_trip() {
    for ev in all_events() {
        let line = ev.to_line();
        assert_eq!(WakeEvent::parse(&line), Some(ev), "line was {line:?}");
    }
}

#[test]
fn parse_rejects_junk() {
    for line in ["", "X 1", "P", "P x", "I 1 0", "P 1 2", "D 1 2"] {
        assert_eq!(WakeEvent::parse(line), None, "line was {line:?}");
    }
}

fn log_text(batch: u32, sections: &[&[WakeEvent]]) -> String {
    let mut out = format!("qn-wake-log v1 batch={batch} stress=off\n");
    for section in sections {
        out.push_str("RUN\n");
        for ev in *section {
            out.push_str(&ev.to_line());
            out.push('\n');
        }
    }
    out
}

#[test]
fn parse_log_splits_sections_in_order() {
    let events = all_events();
    let (a, b) = events.split_at(2);
    let sections = parse_log(&log_text(1, &[a, b]), 1).unwrap();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0], a.to_vec());
    assert_eq!(sections[1], b.to_vec());
}

#[test]
fn parse_log_rejects_batch_mismatch_and_bad_shapes() {
    let err = parse_log(&log_text(1, &[]), 256).unwrap_err();
    assert!(err.contains("batch=1"), "err was {err}");
    assert!(parse_log("not a log\n", 1).is_err());
    // An event before any RUN marker has no section to belong to.
    assert!(parse_log("qn-wake-log v1 batch=1\nP 0\n", 1).is_err());
}

#[test]
fn expect_helpers_report_divergence() {
    let mut ctx = ReplayCtx::default();
    ctx.replayer = Some(Replayer {
        events: all_events(),
        pos: 0,
    });
    assert_eq!(ctx.expect_pick(), Ok(3));
    assert_eq!(ctx.expect_rotate(), Ok(true));
    // Next is Rotate(false); asking for a Pick is a divergence naming the position.
    let err = ctx.expect_pick().unwrap_err();
    assert!(err.contains("divergence at event 2"), "err was {err}");
    // peek_delivery skips nothing: it answers None until the cursor sits on one.
    assert_eq!(ctx.peek_delivery(), None);
    ctx.expect_rotate().unwrap();
    assert!(ctx.peek_delivery().is_some());
}

#[test]
fn ring_is_bounded() {
    let mut ctx = ReplayCtx::default();
    ctx.ring = Some(std::collections::VecDeque::new());
    for i in 0..(RING_CAP + 10) {
        ctx.log(WakeEvent::Pick { tid: i });
    }
    let ring = ctx.ring.as_ref().unwrap();
    assert_eq!(ring.len(), RING_CAP);
    assert_eq!(ring.front(), Some(&WakeEvent::Pick { tid: 10 }));
}

#[test]
fn hash_debug_is_deterministic_and_discriminating() {
    assert_eq!(hash_debug(&vec![1, 2, 3]), hash_debug(&vec![1, 2, 3]));
    assert_ne!(hash_debug(&vec![1, 2, 3]), hash_debug(&vec![1, 2, 4]));
}
