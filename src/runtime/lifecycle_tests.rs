//! The sink's two laws: FIRST TERMINAL WINS (a death observed by the lazy path
//! and the exit watch emits once; `terminate`'s quiet stop beats the reader's
//! died), and overflow DROPS-AND-COUNTS rather than growing or parking.

use super::{LifeSink, LifeStatus};
use crate::error::PeerDeathReason;
use crate::worker::WorkerMsg;
use quoin_ext_proto::DataValue as WireData;

fn kinds(sink: &LifeSink) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(WorkerMsg::Data(WireData::Map(fields))) = sink.rx.try_recv() {
        if let Some((_, WireData::Str(k))) = fields.iter().find(|(n, _)| n == "kind") {
            out.push(k.clone());
        }
    }
    out
}

#[test]
fn first_terminal_wins() {
    let sink = LifeSink::new("p".into(), "worker", "process", None);
    sink.emit_stopped("terminated");
    sink.emit_died(PeerDeathReason::Exited, "worker process exited");
    assert_eq!(kinds(&sink), ["spawned", "stopped"]);
    assert!(matches!(sink.status(), LifeStatus::Stopped(m) if m == "terminated"));
    // The staging closed at the terminal: the pump's next read ends the stream.
    assert!(sink.rx.try_recv().is_err());
}

#[test]
fn a_second_death_observation_is_dropped_whole() {
    let sink = LifeSink::new("p".into(), "extension", "process", Some(42));
    sink.emit_died(PeerDeathReason::Exited, "exited with status 7");
    sink.emit_died(PeerDeathReason::Exited, "observed by the exit watch");
    assert_eq!(kinds(&sink), ["spawned", "died"]);
    assert!(
        matches!(sink.status(), LifeStatus::Died { detail, .. } if detail.contains("status 7"))
    );
}

#[test]
fn overflow_drops_and_counts() {
    let sink = LifeSink::new("p".into(), "worker", "thread", None);
    for _ in 0..200 {
        sink.push(sink.record("x", None, ""));
    }
    assert!(sink.dropped() > 0);
    assert!(kinds(&sink).len() <= 64, "the staging lane is bounded");
}
