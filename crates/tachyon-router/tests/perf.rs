//! M13 T1: deterministic router path latency (spec §43: < 2 ms p95).
//!
//! Ignore-gated and meant for release mode; the M13 ledger runs it with
//! `cargo test --release … -- --ignored --nocapture`. Measures the full
//! `Router::route` path — classify + plan + telemetry record — over a
//! corpus covering every route class. No model, no judge, no I/O.

use std::time::{Duration, Instant};
use tachyon_router::Router;

/// Representative requests spanning all five route classes.
const CORPUS: &[&str] = &[
    "Where is refreshToken defined?",
    "Find references to SessionStore.",
    "Show git status.",
    "Run the tests.",
    "grep for TODO in the scheduler",
    "Why is this test failing?",
    "What changed recently around the gateway?",
    "How does the grace window work?",
    "Find the root cause of this intermittent race and fix it.",
    "Redesign the approval flow to be idempotent.",
    "Fix the flaky watcher settle budget test.",
    "Implement support for Windows named pipes.",
    "compare the router plan with the serial reference",
    "why is the redesign broken",
    "where is TaskId used?",
    "git log for the mutation engine",
    "look for unbounded queues in the gateway",
    "Explain why these two implementations behave differently",
    "What changed in the verify runner last week?",
    "run tests for tachyon-store",
];

const WARMUP: usize = 500;
const SAMPLES: usize = 5_000;
const TARGET_P95: Duration = Duration::from_millis(2);

fn percentiles(mut samples: Vec<Duration>) -> (Duration, Duration) {
    samples.sort();
    let n = samples.len();
    let p50 = samples[n * 50 / 100];
    let p95 = samples[(n * 95 / 100).min(n - 1)];
    (p50, p95)
}

#[test]
#[ignore = "M13 perf target: release mode, run with --ignored"]
fn t1_router_path_p95_under_2ms() {
    let mut router = Router::new();
    for i in 0..WARMUP {
        let plan = router.route(CORPUS[i % CORPUS.len()]);
        assert!(
            !plan.evidence.is_empty(),
            "every corpus route plans evidence"
        );
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let request = CORPUS[i % CORPUS.len()];
        let start = Instant::now();
        let plan = router.route(request);
        samples.push(start.elapsed());
        assert!(
            plan.predicted_evidence_ms >= 0.0,
            "plan carries a sane evidence estimate"
        );
    }

    let (p50, p95) = percentiles(samples);
    println!(
        "perf[T1] n={SAMPLES} p50={p50:?} p95={p95:?} target p95<2ms {}",
        if p95 < TARGET_P95 { "PASS" } else { "MISS" }
    );
    assert!(p95 < TARGET_P95, "T1 miss: p95={p95:?} >= {TARGET_P95:?}");
    println!("perf[T1] PASS");
}
