//! M13 T5 and `comp[index_cold]` (spec §43).
//!
//! - T5: warmed simple symbol/reference request — one `definition_use`
//!   (definitions + references) plus one lexical search over an already
//!   built index of the Tachyon workspace itself (the representative
//!   repository, `target/`/`.git` pruned) — must stay under 250 ms p50
//!   and 500 ms p95. Routing adds the T1-measured cost on top; this test
//!   isolates the repository half of the path.
//! - `comp[index_cold]`: one-off cold inventory scan + index build timings
//!   (report-only; the §43 target is for the warmed request).
//!
//! Ignore-gated; the M13 ledger runs it with
//! `cargo test --release … -- --ignored --nocapture`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tachyon_repo::language::HeuristicBackend;
use tachyon_repo::{Inventory, SearchOptions, SymbolIndex, lexical_search};

/// Symbols that really exist in this workspace, rotated per sample.
const SYMBOLS: &[&str] = &[
    "TaskId",
    "RoutePlan",
    "ExecutionNode",
    "GatewayEvent",
    "StoreWriter",
    "SymbolIndex",
    "refreshToken",
    "VerificationPlan",
];

const WARMUP: usize = 10;
const SAMPLES: usize = 100;
const SCAN_LIMIT: usize = 100_000;
const TARGET_P50: Duration = Duration::from_millis(250);
const TARGET_P95: Duration = Duration::from_millis(500);

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root canonicalizes")
}

fn percentiles(mut samples: Vec<Duration>) -> (Duration, Duration) {
    samples.sort();
    let n = samples.len();
    let p50 = samples[n * 50 / 100];
    let p95 = samples[(n * 95 / 100).min(n - 1)];
    (p50, p95)
}

#[test]
#[ignore = "M13 perf target: release mode, run with --ignored"]
fn t5_warm_symbol_reference_p50_under_250ms() {
    let root = workspace_root();

    // Two passes: the first may pay a genuine cold start (empty page cache,
    // idle drive) and is reported as such; the repeat is the steady-state
    // figure. Indexing runs on the repeat.
    let first_start = Instant::now();
    let first_scan = Inventory::scan(&root, SCAN_LIMIT).expect("inventory scans");
    let first_elapsed = first_start.elapsed();
    let scan_start = Instant::now();
    let inventory = Inventory::scan(&root, SCAN_LIMIT).expect("inventory rescans");
    let scan_elapsed = scan_start.elapsed();
    drop(first_scan);

    let mut index = SymbolIndex::new(&root, HeuristicBackend);
    let build_start = Instant::now();
    index.build(&inventory);
    let build_elapsed = build_start.elapsed();
    println!(
        "comp[index_cold] files={} first_scan={first_elapsed:?} repeat_scan={scan_elapsed:?} build={build_elapsed:?} (report-only; first_scan is cold only when the page cache is)",
        inventory.files.len()
    );
    assert!(
        inventory.files.len() >= 100,
        "representative corpus too small: {} files",
        inventory.files.len()
    );

    let options = SearchOptions {
        limit: 50,
        ..SearchOptions::default()
    };
    for symbol in SYMBOLS {
        let found = index.definition_use(symbol);
        assert!(
            !found.definitions.is_empty() || !found.references.is_empty(),
            "warmup: {symbol} exists in this workspace"
        );
    }

    for i in 0..WARMUP {
        let symbol = SYMBOLS[i % SYMBOLS.len()];
        let _ = index.definition_use(symbol);
        let _ = lexical_search(&root, &inventory, index.projection(), symbol, &options);
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    let mut lookup_samples = Vec::with_capacity(SAMPLES);
    let mut search_samples = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let symbol = SYMBOLS[i % SYMBOLS.len()];
        let start = Instant::now();
        let found = index.definition_use(symbol);
        let after_lookup = Instant::now();
        let hits = lexical_search(&root, &inventory, index.projection(), symbol, &options);
        let end = Instant::now();
        lookup_samples.push(after_lookup.duration_since(start));
        search_samples.push(end.duration_since(after_lookup));
        samples.push(end.duration_since(start));
        assert!(
            !found.definitions.is_empty() || !found.references.is_empty(),
            "{symbol} resolves"
        );
        assert!(!hits.is_empty(), "{symbol} lexical hits exist");
    }

    let (lookup_p50, lookup_p95) = percentiles(lookup_samples);
    let (search_p50, search_p95) = percentiles(search_samples);
    println!(
        "perf[T5.split] lookup p50={lookup_p50:?} p95={lookup_p95:?} | lexical_search p50={search_p50:?} p95={search_p95:?} (indexing profile)"
    );

    let (p50, p95) = percentiles(samples);
    println!(
        "perf[T5] n={SAMPLES} p50={p50:?} p95={p95:?} target p50<250ms p95<500ms {}",
        if p50 < TARGET_P50 && p95 < TARGET_P95 {
            "PASS"
        } else {
            "MISS"
        }
    );
    assert!(p50 < TARGET_P50, "T5 miss: p50={p50:?} >= {TARGET_P50:?}");
    assert!(p95 < TARGET_P95, "T5 miss: p95={p95:?} >= {TARGET_P95:?}");
    println!("perf[T5] PASS");
}
