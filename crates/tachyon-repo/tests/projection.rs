//! M14 projection gate: warm queries read zero corpus bytes.
//!
//! Positive control first: a fresh projection must read from disk once and
//! then serve from memory. Then the index's own projection is warmed and
//! measured over 100 iterations of one `definition_use` plus one lexical
//! search; the byte counter must not move inside the measured window.
//!
//! A runtime counter can only count reads routed through the projection,
//! so the structural half of the property — no direct filesystem reads on
//! the query path — is pinned by `query_paths_perform_no_direct_io` below,
//! which greps the query functions' own source for `fs` tokens.
//!
//! Ignore-gated; the M14 ledger runs it with
//! `cargo test -p tachyon-repo --test projection --release -- --ignored --nocapture`.

use std::path::PathBuf;

use tachyon_repo::language::HeuristicBackend;
use tachyon_repo::{Inventory, SearchOptions, SymbolIndex, TextProjection, lexical_search};

const SAMPLES: usize = 100;
const SCAN_LIMIT: usize = 100_000;
/// A symbol that really exists in this workspace (`inventory.rs`).
const SYMBOL: &str = "Inventory";

#[test]
#[ignore = "M14 projection: release mode, run with --ignored"]
fn warm_queries_read_zero_corpus_bytes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let inventory = Inventory::scan(&root, SCAN_LIMIT).expect("inventory scans");
    assert!(
        inventory.files.len() > 100,
        "corpus too small: {} files",
        inventory.files.len()
    );
    let mut index = SymbolIndex::new(&root, HeuristicBackend);
    index.build(&inventory);

    let control = TextProjection::new();
    let record = inventory
        .get("crates/tachyon-repo/src/lib.rs")
        .expect("lib.rs is inventoried");
    assert!(
        control
            .get_or_read(&root, &record.rel, &record.hash)
            .is_some(),
        "positive control reads {}",
        record.rel
    );
    let control_after_first = control.query_bytes_read();
    assert!(
        control_after_first > 0,
        "positive control must read bytes on a cold projection"
    );
    assert!(
        control
            .get_or_read(&root, &record.rel, &record.hash)
            .is_some(),
        "positive control repeat serves from cache"
    );
    assert_eq!(
        control.query_bytes_read(),
        control_after_first,
        "positive control cache hit must not add bytes"
    );

    let options = SearchOptions::default();
    let pre_warm = index.projection().query_bytes_read();
    let _ = index.definition_use(SYMBOL);
    let _ = lexical_search(&root, &inventory, index.projection(), SYMBOL, &options);
    let measured_start = index.projection().query_bytes_read();

    for _ in 0..SAMPLES {
        let answer = index.definition_use(SYMBOL);
        assert!(!answer.definitions.is_empty(), "{SYMBOL} resolves");
        let hits = lexical_search(&root, &inventory, index.projection(), SYMBOL, &options);
        assert!(!hits.is_empty(), "{SYMBOL} lexical hits exist");
    }

    let delta = index.projection().query_bytes_read() - measured_start;
    assert_eq!(
        delta, 0,
        "warm queries read {delta} corpus bytes (pre-warm bytes {pre_warm})"
    );
    println!(
        "projection ok n={SAMPLES} files={} bytes_read={delta}",
        inventory.files.len()
    );
}

/// Structural guard for the projection: `references` and `search` must
/// perform no direct filesystem reads, so every query-time corpus byte
/// flows through the counted projection path. Runs in the default suite;
/// it only reads two source files.
#[test]
fn query_paths_perform_no_direct_io() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for (file, function) in [
        ("index.rs", "pub fn references"),
        ("search.rs", "pub fn search"),
    ] {
        let text = std::fs::read_to_string(src.join(file)).expect("own source readable");
        let (_, body) = text.split_once(function).expect("query fn present");
        let end = body.find("\n    pub fn ").unwrap_or(body.len());
        let until_next = &body[..end];
        for token in ["std::fs", "fs::", "File::"] {
            assert!(
                !until_next.contains(token),
                "{file}::{function} performs direct I/O ({token}) outside the projection"
            );
        }
    }
}
