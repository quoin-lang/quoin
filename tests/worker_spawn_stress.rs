//! Randomized-timing stress for the parked process spawn (issue #147): race
//! `Async.timeout:` deadlines against a child stalled a random time before it
//! connects, and hold the invariants — every attempt answers `ok` or
//! `TimeoutError` (never a wedge, never the 10s-backstop string, never an
//! unclean exit), a generous-deadline spawn still succeeds AFTER the
//! randomized timeout churn (no corrupted spawn state), and no worker child
//! outlives the run (the cancel guard kills, the helper reaps). Half the
//! iterations run two attempts CONCURRENTLY (racing two helper threads and
//! two cancel guards), and half run under `QN_SCHED_STRESS` so preemption
//! randomizes underneath.
//!
//! Tunables: `QN_SPAWN_STRESS_ITERS` (default 8) scales a local soak;
//! `QN_SPAWN_STRESS_SEED` pins the run — every failure message carries the seed.

#![cfg(unix)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// SplitMix64 — no rand dev-dependency for one stream of test jitter.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn seed() -> u64 {
    env_u64(
        "QN_SPAWN_STRESS_SEED",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock after the epoch")
            .subsec_nanos() as u64
            | 1,
    )
}

/// One spawn attempt under a deadline, printing its verdict.
fn attempt(timeout_ms: u64) -> String {
    format!(
        "(({{ Async.timeout:{timeout_ms} do:{{ Worker.with:{{ Duration.seconds:1 }} \
         backing:'process' }}; 'ok' }}).catch:{{ |e| e.class.name }}).print;\n"
    )
}

#[test]
fn randomized_spawn_timeouts_hold_the_invariants() {
    let seed = seed();
    let iters = env_u64("QN_SPAWN_STRESS_ITERS", 8);
    let mut rng = Rng(seed);
    let dir = std::env::temp_dir().join(format!("qn_spawn_stress_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    for iter in 0..iters {
        let stall = rng.below(1200);
        let t1 = 50 + rng.below(1200);
        let t2 = 50 + rng.below(1200);
        let concurrent = rng.below(2) == 0;
        let sched_stress = rng.below(2) == 0;
        let ctx = format!(
            "iter {iter}/{iters} seed {seed} stall {stall} t1 {t1} t2 {t2} \
             concurrent {concurrent} sched_stress {sched_stress}\n\
             (replay: QN_SPAWN_STRESS_SEED={seed} QN_SPAWN_STRESS_ITERS={iters})"
        );

        // Two randomized-edge attempts, then a generous-deadline attempt that
        // MUST succeed — the machinery has to keep working after the churn.
        let script = if concurrent {
            format!(
                "var r = Async.gather:#(\n    {{ ({{ Async.timeout:{t1} do:{{ Worker.with:{{ \
                 Duration.seconds:1 }} backing:'process' }}; 'ok' }}).catch:{{ |e| e.class.name \
                 }} }}\n    {{ ({{ Async.timeout:{t2} do:{{ Worker.with:{{ Duration.seconds:1 }} \
                 backing:'process' }}; 'ok' }}).catch:{{ |e| e.class.name }} }}\n);\n(r.at:0).\
                 print;\n(r.at:1).print;\n{}",
                attempt(8000)
            )
        } else {
            format!("{}{}{}", attempt(t1), attempt(t2), attempt(8000))
        };
        let script_path = dir.join(format!("iter{iter}.qn"));
        std::fs::write(&script_path, &script).unwrap();

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
        cmd.arg(&script_path)
            .env("QN_WORKER_SERVE_STALL_MS", stall.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if sched_stress {
            cmd.env("QN_SCHED_STRESS", (seed ^ iter).to_string());
        }
        let mut child = cmd.spawn().expect("run qn");
        let qn_pid = child.id();

        // Bounded wait: a wedge must FAIL with its seed, never hang the suite.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() > deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("iteration wedged past 30s.\n{ctx}");
                }
                _ => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        let out = child.wait_with_output().expect("collect qn output");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "unclean exit {:?}.\n{ctx}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            out.status
        );

        let verdicts: Vec<&str> = stdout.lines().map(str::trim).collect();
        assert_eq!(
            verdicts.len(),
            3,
            "expected 3 verdicts.\n{ctx}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        for (i, (v, t)) in verdicts.iter().zip([t1, t2, 8000]).enumerate() {
            assert!(
                *v == "ok" || *v == "TimeoutError",
                "attempt {i} answered {v:?} (deadline {t}ms).\n{ctx}\nstderr:\n{stderr}"
            );
            // Plausibility, with wide jitter margins: a deadline comfortably
            // shorter than the stall cannot succeed (the child hasn't even
            // connected), and the generous final deadline must succeed.
            if t + 500 < stall {
                assert_eq!(
                    *v, "TimeoutError",
                    "attempt {i} (deadline {t}ms, stall {stall}ms) claims success \
                     before the child could connect.\n{ctx}"
                );
            }
        }
        assert_eq!(
            verdicts[2], "ok",
            "the generous-deadline spawn failed after the churn.\n{ctx}\nstderr:\n{stderr}"
        );

        // No worker child outlives the run: sock paths carry the parent's pid,
        // so any orphaned `worker-serve` still names it in its argv.
        let orphans = Command::new("pgrep")
            .args(["-f", &format!("quoin-worker-{qn_pid}-")])
            .output()
            .expect("run pgrep");
        let orphans = String::from_utf8_lossy(&orphans.stdout).trim().to_string();
        assert!(
            orphans.is_empty(),
            "worker children outlived the run: {orphans}\n{ctx}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
