//! The child-exit watch's ground truth (SUPERVISION.md slice 1), against real
//! children: the peek must not lie in either direction (a false "exited" turns
//! the watch into an instant false death event), the watch must fire on a real
//! exit, must NOT fire on a live child, and must never consume the exit status
//! the owner's reap is entitled to.

use super::{await_child_exit, child_already_exited};
use std::process::Command;
use std::time::Duration;

fn sleeper(secs: &str) -> std::process::Child {
    Command::new("sleep")
        .arg(secs)
        .spawn()
        .expect("spawn sleep")
}

#[test]
fn peek_says_running_for_a_live_child() {
    let mut child = sleeper("5");
    assert!(
        !child_already_exited(child.id()),
        "a live child peeked as exited"
    );
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn peek_sees_a_zombie_without_consuming_it() {
    let mut child = sleeper("60");
    let _ = child.kill();
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        child_already_exited(child.id()),
        "a zombie peeked as running"
    );
    // WNOWAIT left the status for the owner: the real reap still works.
    let status = child.wait().expect("reap after peek");
    assert!(!status.success());
}

#[test]
fn watch_fires_on_exit_and_not_before() {
    let mut child = sleeper("30");
    let pid = child.id();
    // Race the watch against a short timer: on a LIVE child the timer must win.
    let fired = futures_lite::future::block_on(futures_lite::future::or(
        async {
            await_child_exit(pid).await;
            true
        },
        async {
            async_io::Timer::after(Duration::from_millis(600)).await;
            false
        },
    ));
    assert!(!fired, "the watch fired on a live child (false positive)");
    let _ = child.kill();
    // Now it must fire promptly (bounded by the test harness timeout).
    futures_lite::future::block_on(await_child_exit(pid));
    let _ = child.wait();
}
