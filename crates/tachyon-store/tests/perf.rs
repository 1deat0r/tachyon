//! M13 `comp[persistence]`: durable journal/task commit latency (report-only).
//!
//! The §43 targets do not name a persistence number; this baseline feeds
//! the M13 report's critical-path breakdown. Measures the real single-writer
//! SQLite path under `journal_mode=WAL` + `synchronous=FULL`:
//! `create_task` (row + created event in one commit) and `append_event`
//! (journal commit). Ignore-gated; the M13 ledger runs it with
//! `cargo test --release … -- --ignored --nocapture`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tachyon_store::StoreWriter;
use tachyon_types::TaskId;

const CREATE_SAMPLES: usize = 100;
const APPEND_SAMPLES: usize = 200;

fn percentiles(mut samples: Vec<Duration>) -> (Duration, Duration) {
    samples.sort();
    let n = samples.len();
    let p50 = samples[n * 50 / 100];
    let p95 = samples[(n * 95 / 100).min(n - 1)];
    (p50, p95)
}

fn test_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("tachyon-m13-store-{}-{nanos}", std::process::id()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "M13 perf component: release mode, run with --ignored"]
async fn comp_persistence_commit_latency() {
    let dir = test_dir();
    std::fs::create_dir_all(&dir).expect("data dir");
    let store = StoreWriter::open(&dir).await.expect("store opens");
    let session_id = tachyon_types::SessionId::generate().to_string();
    store.create_session(&session_id).await.expect("session");

    // Warmup: page cache, WAL file creation, connection pool.
    for _ in 0..10 {
        store
            .create_task(
                &TaskId::generate().to_string(),
                &session_id,
                "ws",
                "warmup",
                "created",
                "{}",
                "{}",
            )
            .await
            .expect("warmup create_task");
    }

    let mut create_latencies = Vec::with_capacity(CREATE_SAMPLES);
    for i in 0..CREATE_SAMPLES {
        let start = Instant::now();
        store
            .create_task(
                &TaskId::generate().to_string(),
                &session_id,
                "ws",
                &format!("task {i}"),
                "created",
                "{}",
                "{}",
            )
            .await
            .expect("create_task commits");
        create_latencies.push(start.elapsed());
    }

    let task_id = TaskId::generate().to_string();
    store
        .create_task(
            &task_id,
            &session_id,
            "ws",
            "append host",
            "created",
            "{}",
            "{}",
        )
        .await
        .expect("append host task");
    let mut append_latencies = Vec::with_capacity(APPEND_SAMPLES);
    for i in 0..APPEND_SAMPLES {
        let start = Instant::now();
        store
            .append_event(&task_id, "perf.synthetic", &format!("{{\"i\":{i}}}"))
            .await
            .expect("append_event commits");
        append_latencies.push(start.elapsed());
    }

    let (create_p50, create_p95) = percentiles(create_latencies);
    let (append_p50, append_p95) = percentiles(append_latencies);
    println!(
        "comp[persistence.create_task] n={CREATE_SAMPLES} p50={create_p50:?} p95={create_p95:?}"
    );
    println!(
        "comp[persistence.append_event] n={APPEND_SAMPLES} p50={append_p50:?} p95={append_p95:?}"
    );

    store.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}
