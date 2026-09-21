//! Milestone 8 gate: prepare/commit roundtrip, stale refusal at both
//! gates, crash injection between every file commit, rollback,
//! divergence freeze, and Vertical Slice C.

use tachyon_mutation::{MutationEngine, MutationError, PatchSpec, Transition, blake3_hex};
use tachyon_retrieval::{EvidenceItem, EvidenceKind, EvidencePackage, Provenance};

use std::path::PathBuf;

fn fixture(name: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "tachyon-m8-{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let ws = root.join("ws");
    let state = root.join("state");
    std::fs::create_dir_all(&ws).expect("ws");
    (ws, state)
}

fn write(ws: &std::path::Path, rel: &str, content: &str) {
    let path = ws.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parents");
    }
    std::fs::write(&path, content).expect("write");
}

fn read(ws: &std::path::Path, rel: &str) -> String {
    std::fs::read_to_string(ws.join(rel)).expect("read")
}

fn spec(ws: &std::path::Path, rel: &str, new_content: &str) -> PatchSpec {
    let current = std::fs::read(ws.join(rel)).ok();
    PatchSpec {
        path: rel.to_owned(),
        base_hash: current.as_deref().map(blake3_hex),
        new_content: new_content.as_bytes().to_vec(),
    }
}

#[test]
fn prepare_commit_roundtrip_with_events() {
    let (ws, state) = fixture("roundtrip");
    write(&ws, "a.rs", "old a");
    write(&ws, "sub/b.rs", "old b");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    let prepared = engine
        .prepare(&[spec(&ws, "a.rs", "new a"), spec(&ws, "sub/b.rs", "new b")])
        .expect("prepare");
    assert_eq!(prepared.files.len(), 2);
    let report = engine.commit(&prepared).expect("commit");
    assert!(report.completed);
    assert_eq!(report.committed.len(), 2);
    assert!(
        report
            .committed
            .iter()
            .all(|changed| changed.transition == Transition::Committed)
    );
    assert_eq!(read(&ws, "a.rs"), "new a");
    assert_eq!(read(&ws, "sub/b.rs"), "new b");
    // Journal shows completion: a fresh engine sees nothing to recover.
    let engine2 = MutationEngine::open(&ws, &state).expect("reopen");
    let recovery = engine2.recover(true).expect("recover");
    assert!(recovery.finished.is_empty());
    assert!(recovery.diverged.is_empty());
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

#[test]
fn stale_base_refuses_at_prepare() {
    let (ws, state) = fixture("stale-prepare");
    write(&ws, "a.rs", "v1");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    let bad = PatchSpec {
        path: "a.rs".to_owned(),
        base_hash: Some("deadbeef".to_owned()),
        new_content: b"v2".to_vec(),
    };
    let error = engine.prepare(&[bad]).expect_err("stale");
    assert!(matches!(error, MutationError::StalePreimage { .. }));
    assert!(!error.is_retryable());
    // Nothing written, nothing journaled.
    assert_eq!(read(&ws, "a.rs"), "v1");
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

#[test]
fn external_change_between_prepare_and_commit_aborts() {
    let (ws, state) = fixture("stale-commit");
    write(&ws, "a.rs", "v1");
    write(&ws, "b.rs", "v1");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    let prepared = engine
        .prepare(&[spec(&ws, "a.rs", "v2"), spec(&ws, "b.rs", "v2")])
        .expect("prepare");
    // External hands touch b.rs after prepare.
    write(&ws, "b.rs", "intruder");
    let report = engine.commit(&prepared).expect_err("stale at commit");
    assert!(matches!(report, MutationError::StalePreimage { .. }));
    // a.rs committed (valid), b.rs untouched by us.
    assert_eq!(read(&ws, "a.rs"), "v2");
    assert_eq!(read(&ws, "b.rs"), "intruder");
    // Recovery documents the coherent state: a committed, b diverged.
    let engine2 = MutationEngine::open(&ws, &state).expect("reopen");
    let recovery = engine2.recover(true).expect("recover");
    assert!(recovery.diverged.contains(&"b.rs".to_owned()));
    assert_eq!(read(&ws, "a.rs"), "v2");
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

#[test]
fn crash_between_every_commit_recovers_coherent() {
    const FILES: [(&str, &str, &str); 3] = [
        ("one.rs", "old 1", "new 1"),
        ("two.rs", "old 2", "new 2"),
        ("three.rs", "old 3", "new 3"),
    ];
    for crash_after in 0..=FILES.len() {
        let (ws, state) = fixture(&format!("crash-{crash_after}"));
        let mut specs = Vec::new();
        for (rel, old, new) in FILES {
            write(&ws, rel, old);
            specs.push(spec(&ws, rel, new));
        }
        let engine = MutationEngine::open(&ws, &state).expect("open");
        let prepared = engine.prepare(&specs).expect("prepare");
        // Simulated crash: commit a prefix, then drop the engine without
        // completing. Recovery must finish the rest.
        let partial = engine
            .commit_up_to(&prepared, crash_after)
            .expect("partial");
        assert_eq!(partial.committed.len(), crash_after);
        drop(engine);
        drop(prepared);
        let engine2 = MutationEngine::open(&ws, &state).expect("reopen");
        let recovery = engine2.recover(true).expect("recover");
        assert!(recovery.diverged.is_empty(), "k={crash_after}");
        for (rel, _, new) in FILES {
            assert_eq!(read(&ws, rel), new, "k={crash_after} {rel}");
        }
        std::fs::remove_dir_all(ws.parent().expect("root")).ok();
    }
}

#[test]
fn rollback_restores_preimages() {
    let (ws, state) = fixture("rollback");
    write(&ws, "a.rs", "orig a");
    write(&ws, "b.rs", "orig b");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    let prepared = engine
        .prepare(&[
            spec(&ws, "a.rs", "changed a"),
            spec(&ws, "b.rs", "changed b"),
        ])
        .expect("prepare");
    let partial = engine.commit_up_to(&prepared, 1).expect("partial");
    assert_eq!(partial.committed.len(), 1);
    drop(engine);
    // Operator abandons the batch: compensate instead of finishing.
    let engine2 = MutationEngine::open(&ws, &state).expect("reopen");
    let recovery = engine2.recover(false).expect("rollback");
    assert_eq!(recovery.finished.len(), 0);
    assert_eq!(recovery.rolled_back.len(), 1);
    assert_eq!(read(&ws, "a.rs"), "orig a");
    assert_eq!(read(&ws, "b.rs"), "orig b");
    assert!(
        recovery
            .changed
            .iter()
            .any(|changed| changed.transition == Transition::RolledBack)
    );
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

/// Vertical Slice C: fix the incorrect implementation. Repo evidence feeds
/// a scripted-correct replacement (reasoning quality is M6's job; this
/// slice proves the mutation mechanics end to end).
#[test]
fn slice_c_fixes_the_incorrect_implementation() {
    let (ws, state) = fixture("slicec");
    let buggy = "pub fn refresh_token_legacy(s: &S) -> Token {\n    if s.presented == s.expected { issue(s) } else { reject() }\n}";
    let fixed = "pub fn refresh_token_legacy(s: &S) -> Token {\n    if constant_time_eq(&s.presented, &s.expected) { issue(s) } else { reject() }\n}";
    write(&ws, "src/legacy.rs", buggy);
    // Evidence names the file and the defect (M4/M6 machinery).
    let evidence = EvidencePackage {
        question: "Fix the incorrect implementation.".to_owned(),
        findings: vec![EvidenceItem::new(
            EvidenceKind::SymbolDefinition,
            buggy,
            Provenance::repo("repo.symbol.search", "src/legacy.rs"),
        )],
        contradictions: vec![],
        gaps: vec![],
    };
    assert_eq!(
        evidence.findings[0].provenance.path.as_deref(),
        Some("src/legacy.rs")
    );

    let engine = MutationEngine::open(&ws, &state).expect("open");
    let path = evidence.findings[0]
        .provenance
        .path
        .clone()
        .expect("provenance path");
    let prepared = engine
        .prepare(&[PatchSpec {
            path,
            base_hash: Some(blake3_hex(buggy.as_bytes())),
            new_content: fixed.as_bytes().to_vec(),
        }])
        .expect("prepare");
    let report = engine.commit(&prepared).expect("commit");
    assert!(report.completed);
    assert_eq!(report.committed.len(), 1);
    assert_eq!(report.committed[0].transition, Transition::Committed);
    assert_eq!(read(&ws, "src/legacy.rs"), fixed);
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

#[test]
fn empty_batch_and_aliasing_paths_refused() {
    let (ws, state) = fixture("shapes");
    write(&ws, "sub/f.rs", "x");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    let error = engine.prepare(&[]).expect_err("empty");
    assert!(matches!(error, MutationError::InvalidPath(_)));
    let dup = engine
        .prepare(&[
            spec(&ws, "sub/f.rs", "y"),
            PatchSpec {
                path: "sub/./f.rs".to_owned(),
                base_hash: Some(blake3_hex(b"x")),
                new_content: b"y".to_vec(),
            },
        ])
        .expect_err("aliasing duplicate");
    assert!(matches!(dup, MutationError::InvalidPath(_)));
    let abs = engine
        .prepare(&[PatchSpec {
            path: "/abs/f.rs".to_owned(),
            base_hash: None,
            new_content: b"y".to_vec(),
        }])
        .expect_err("absolute");
    assert!(matches!(abs, MutationError::InvalidPath(_)));
    assert_eq!(read(&ws, "sub/f.rs"), "x");
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

#[test]
fn double_commit_refused_and_recover_is_idempotent() {
    let (ws, state) = fixture("idempotent");
    write(&ws, "a.rs", "v1");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    let prepared = engine.prepare(&[spec(&ws, "a.rs", "v2")]).expect("prepare");
    let report = engine.commit(&prepared).expect("commit");
    assert!(report.completed);
    let again = engine.commit(&prepared).expect_err("double");
    assert!(matches!(again, MutationError::AlreadyCompleted(_)));
    // Rollback twice: the second run changes nothing.
    let prepared2 = engine
        .prepare(&[spec(&ws, "a.rs", "v3")])
        .expect("prepare 2");
    drop(prepared2);
    drop(engine);
    let engine2 = MutationEngine::open(&ws, &state).expect("reopen");
    let first = engine2.recover(false).expect("recover");
    assert_eq!(first.rolled_back.len(), 1);
    let second = engine2.recover(false).expect("recover again");
    assert!(second.changed.is_empty());
    assert!(second.diverged.is_empty());
    assert_eq!(read(&ws, "a.rs"), "v2");
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

#[test]
fn create_file_rolls_back_by_removal() {
    let (ws, state) = fixture("create");
    write(&ws, "a.rs", "v1");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    let prepared = engine
        .prepare(&[
            PatchSpec {
                path: "fresh/new.rs".to_owned(),
                base_hash: None,
                new_content: b"hello".to_vec(),
            },
            spec(&ws, "a.rs", "v2"),
        ])
        .expect("prepare");
    // Crash between the first commit and batch completion: the created
    // file is committed but the batch is still open.
    let partial = engine.commit_up_to(&prepared, 1).expect("partial");
    assert!(!partial.completed);
    assert_eq!(read(&ws, "fresh/new.rs"), "hello");
    drop(engine);
    let engine2 = MutationEngine::open(&ws, &state).expect("reopen");
    let recovery = engine2.recover(false).expect("rollback");
    assert_eq!(recovery.rolled_back.len(), 1);
    assert!(!ws.join("fresh/new.rs").exists());
    assert_eq!(read(&ws, "a.rs"), "v1");
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

#[test]
fn same_instant_batches_stage_distinct_tmps() {
    let (ws, state) = fixture("distinct");
    write(&ws, "a.rs", "v1");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    // Two batches back to back (same millisecond under UUIDv7): temps
    // must still differ, one commit per batch, no divergence.
    let batch_a = engine
        .prepare(&[spec(&ws, "a.rs", "v2")])
        .expect("prepare a");
    let batch_b = engine
        .prepare(&[spec(&ws, "a.rs", "v2")])
        .expect("prepare b");
    assert_ne!(
        batch_a.files[0].temp_name, batch_b.files[0].temp_name,
        "same-ms batches share no temp"
    );
    let report_a = engine.commit(&batch_a).expect("commit a");
    assert!(report_a.completed);
    // B raced the same file and lost: its base is stale, nothing written.
    let raced = engine.commit(&batch_b).expect_err("commit b");
    assert!(matches!(raced, MutationError::StalePreimage { .. }));
    assert_eq!(read(&ws, "a.rs"), "v2");
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

#[test]
fn finish_never_resurrects_compensated_batches() {
    let (ws, state) = fixture("noresurrect");
    write(&ws, "a.rs", "v1");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    let prepared = engine.prepare(&[spec(&ws, "a.rs", "v2")]).expect("prepare");
    drop(prepared);
    drop(engine);
    let engine2 = MutationEngine::open(&ws, &state).expect("reopen");
    let compensated = engine2.recover(false).expect("rollback");
    assert_eq!(compensated.rolled_back.len(), 1);
    let resumed = engine2.recover(true).expect("finish attempt");
    assert!(resumed.finished.is_empty());
    assert!(resumed.changed.is_empty());
    assert_eq!(read(&ws, "a.rs"), "v1");
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

#[test]
fn stale_descriptor_commit_after_compensate_refused() {
    let (ws, state) = fixture("stale");
    write(&ws, "a.rs", "v1");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    let prepared = engine.prepare(&[spec(&ws, "a.rs", "v2")]).expect("prepare");
    drop(engine);
    let engine2 = MutationEngine::open(&ws, &state).expect("reopen");
    let compensated = engine2.recover(false).expect("rollback");
    assert_eq!(compensated.rolled_back.len(), 1);
    // The pre-compensation descriptor is dead: recommitting neither
    // applies nor seals the batch against future recovery.
    let dead = engine2.commit(&prepared).expect_err("stale commit");
    assert!(matches!(dead, MutationError::Compensated(_)));
    assert!(!dead.is_retryable());
    assert_eq!(read(&ws, "a.rs"), "v1");
    let again = engine2.recover(false).expect("recover again");
    assert!(again.changed.is_empty());
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

#[test]
fn sibling_batches_isolated_from_recovery_failure() {
    let (ws, state) = fixture("isolate");
    write(&ws, "a.rs", "v1");
    write(&ws, "a2.rs", "v1");
    write(&ws, "b.rs", "w1");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    let batch_a = engine
        .prepare(&[spec(&ws, "a.rs", "v2"), spec(&ws, "a2.rs", "v2")])
        .expect("prepare a");
    // Commit one file: batch A stays open with a committed file on disk.
    let partial = engine.commit_up_to(&batch_a, 1).expect("commit a");
    assert!(!partial.completed);
    let _batch_b = engine
        .prepare(&[spec(&ws, "b.rs", "w2")])
        .expect("prepare b");
    drop(engine);
    // Wipe the artifact spool: batch A can no longer restore its
    // committed file, but batch B must still roll back.
    std::fs::remove_dir_all(state.join("artifacts")).expect("wipe spool");
    let engine2 = MutationEngine::open(&ws, &state).expect("reopen");
    let recovery = engine2.recover(false).expect("recover");
    assert_eq!(recovery.batch_errors.len(), 1);
    assert_eq!(recovery.batch_errors[0].0, batch_a.id);
    assert_eq!(recovery.rolled_back.len(), 1);
    assert_eq!(read(&ws, "b.rs"), "w1");
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}

#[test]
fn sweep_removes_only_engine_tmps() {
    let (ws, state) = fixture("sweep");
    write(&ws, "a.rs", "v1");
    write(&ws, "notes.tachyon-tmp-backup", "user data");
    #[cfg(unix)]
    std::os::unix::fs::symlink("..", ws.join("loop")).expect("symlink");
    let engine = MutationEngine::open(&ws, &state).expect("open");
    let prepared = engine.prepare(&[spec(&ws, "a.rs", "v2")]).expect("prepare");
    drop(prepared);
    drop(engine);
    // Crash before commit: temp staged, plan journaled. Recovery finishes,
    // keeps user files, and terminates despite the symlink loop.
    let engine2 = MutationEngine::open(&ws, &state).expect("reopen");
    let recovery = engine2.recover(true).expect("recover");
    assert_eq!(read(&ws, "a.rs"), "v2");
    assert_eq!(read(&ws, "notes.tachyon-tmp-backup"), "user data");
    assert!(recovery.batch_errors.is_empty());
    std::fs::remove_dir_all(ws.parent().expect("root")).ok();
}
