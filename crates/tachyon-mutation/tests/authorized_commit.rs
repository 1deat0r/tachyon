//! M10: authorized per-file commit boundaries on the real filesystem.
//!
//! The trusted runtime holds the workspace lease, derives the effect boundary
//! from the journal, and asks for one file at a time so it can recheck
//! cancellation, revision and policy between real renames.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use tachyon_mutation::{
    BatchJournal, CommitReport, FileState, MutationEngine, MutationError, PatchSpec, PreparedBatch,
    RecoveryAction, RecoveryDisposition, blake3_hex,
};
use tachyon_policy::{DefaultPosture, Policy};
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
use tachyon_types::MutationBatchId;

const BEFORE: &[u8] = b"before";
const AFTER: &[u8] = b"after";
const SPEC_PATHS: [&str; 2] = ["a.rs", "sub/b.rs"];

struct Fixture {
    root: PathBuf,
    ws: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tachyon-authorized-commit-{}",
            MutationBatchId::generate()
        ));
        std::fs::create_dir_all(root.join("workspace").join("sub")).expect("workspace");
        // Canonicalize once so canonical engine paths and policy scopes
        // built from display strings agree (macOS /var → /private/var).
        let root = std::fs::canonicalize(&root).expect("canonical root");
        let ws = root.join("workspace");
        let state = root.join("state");
        std::fs::write(ws.join("a.rs"), BEFORE).expect("source");
        std::fs::write(ws.join("sub/b.rs"), BEFORE).expect("source");
        Self { root, ws, state }
    }

    fn engine(&self) -> MutationEngine {
        MutationEngine::open(&self.ws, &self.state).expect("engine")
    }

    fn context(&self) -> ToolsContext {
        let mut policy = Policy::trusted_workspace();
        policy.allow("mutation.patch", "workspace/**");
        policy.allow("fs.delete", "workspace/**");
        for capability in ["fs.read", "fs.metadata"] {
            policy.allow(
                capability,
                &format!("external:{}/artifacts/**", self.state.display()),
            );
        }
        ToolsContext::new(
            self.ws.clone(),
            policy,
            ArtifactSpool::new(self.root.join("tool-artifacts")),
        )
    }

    fn write(&self, path: &str, content: &[u8]) {
        std::fs::write(self.ws.join(path), content).expect("fixture write");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).expect("remove owned fixture");
    }
}

fn specs() -> [PatchSpec; 2] {
    SPEC_PATHS.map(|path| PatchSpec {
        path: path.to_owned(),
        base_hash: Some(blake3_hex(BEFORE)),
        new_content: AFTER.to_vec(),
    })
}

fn prepare_batch(engine: &MutationEngine, context: &ToolsContext) -> PreparedBatch {
    engine
        .prepare_authorized(context, MutationBatchId::generate(), &specs())
        .expect("authorized preparation")
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).expect("snapshot dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let key = path.strip_prefix(root).expect("relative").to_path_buf();
            let kind = entry.file_type().expect("type");
            let value = if kind.is_symlink() {
                std::fs::read_link(&path)
                    .expect("link")
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec()
            } else if kind.is_dir() {
                dirs.push(path.clone());
                b"<directory>".to_vec()
            } else {
                std::fs::read(&path).expect("read fixture")
            };
            result.insert(key, value);
        }
    }
    result
}

fn committed_paths(report: &CommitReport) -> Vec<&str> {
    report
        .committed
        .iter()
        .map(|changed| changed.path.as_str())
        .collect()
}

#[test]
fn one_file_boundary_skips_prior_files_and_returns_the_completion_receipt() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let context = fixture.context();
    let prepared = prepare_batch(&engine, &context);

    let first = engine
        .commit_authorized_up_to(&context, &prepared, 1)
        .expect("first boundary");
    assert_eq!(committed_paths(&first), ["a.rs"]);
    assert!(
        !first.completed,
        "one file cannot complete a two-file batch"
    );
    assert_eq!(
        std::fs::read(fixture.ws.join("a.rs")).expect("source"),
        AFTER
    );
    assert_eq!(
        std::fs::read(fixture.ws.join("sub/b.rs")).expect("source"),
        BEFORE,
        "the pending file stays untouched"
    );
    let pending_temp = fixture.ws.join("sub").join(&prepared.files[1].temp_name);
    assert_eq!(
        std::fs::read(&pending_temp).expect("pending temp"),
        AFTER,
        "the pending temp survives the boundary"
    );

    // The original descriptor is reused verbatim: prior files come from the
    // journal, not from stale `Prepared` flags on the caller's copy.
    let second = engine
        .commit_authorized_up_to(&context, &prepared, 1)
        .expect("second boundary");
    assert_eq!(
        committed_paths(&second),
        ["sub/b.rs"],
        "an already committed file is skipped, never recommitted"
    );
    assert!(second.completed, "the last boundary completes the batch");
    assert!(!pending_temp.exists(), "the consumed temp is gone");
    assert_eq!(
        std::fs::read(fixture.ws.join("sub/b.rs")).expect("source"),
        AFTER
    );

    // Completion is journaled truth, not a returned flag.
    let error = engine
        .commit_up_to(&prepared, 1)
        .expect_err("completion is journaled");
    assert!(
        matches!(error, MutationError::AlreadyCompleted(_)),
        "{error}"
    );
    let error = engine
        .commit_authorized_up_to(&context, &prepared, 1)
        .expect_err("completion is journaled for the authorized boundary too");
    assert!(
        matches!(error, MutationError::AlreadyCompleted(_)),
        "{error}"
    );
}

#[test]
fn boundary_denial_changes_nothing_behind_broad_grants() {
    for (capability, on_temp) in [
        ("fs.metadata", false),
        ("fs.read", false),
        ("mutation.patch", false),
        ("fs.write", false),
        ("fs.metadata", true),
        ("fs.read", true),
        ("fs.write", true),
        ("fs.delete", true),
    ] {
        let fixture = Fixture::new();
        let engine = fixture.engine();
        let prepared = prepare_batch(&engine, &fixture.context());
        let mut context = fixture.context();
        let scope = if on_temp {
            format!("workspace/sub/{}", prepared.files[1].temp_name)
        } else {
            "workspace/sub/b.rs".to_owned()
        };
        // The exact denial must win even with trusted-workspace broad grants
        // and the sibling file's grants in place.
        context.policy.deny(capability, &scope);
        let before = snapshot(&fixture.root);
        let error = engine
            .commit_authorized_up_to(&context, &prepared, usize::MAX)
            .expect_err("exact permission denied");
        assert!(error.to_string().contains(capability), "{error}");
        assert_eq!(snapshot(&fixture.root), before, "{capability}/{scope}");

        // Nothing was receipted: the untouched batch still commits in full.
        let report = engine
            .commit_authorized_up_to(&fixture.context(), &prepared, usize::MAX)
            .expect("clean retry");
        assert_eq!(report.committed.len(), 2, "{capability}/{scope}");
        assert!(report.completed, "{capability}/{scope}");
    }
}

#[test]
fn unresolved_ask_cannot_self_approve_a_boundary() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let prepared = prepare_batch(&engine, &fixture.context());
    let mut context = fixture.context();
    let mut policy = Policy::new(DefaultPosture::Ask);
    for capability in ["fs.metadata", "fs.read", "fs.write", "fs.delete"] {
        policy.allow(capability, "workspace/**");
    }
    policy.allow("mutation.patch", "workspace/a.rs");
    context.policy = policy;
    let before = snapshot(&fixture.root);
    let error = engine
        .commit_authorized_up_to(&context, &prepared, usize::MAX)
        .expect_err("unresolved ask");
    assert!(
        error.to_string().contains("approval required"),
        "the ask stays unresolved instead of self-approving: {error}"
    );
    assert_eq!(
        snapshot(&fixture.root),
        before,
        "an unresolved ask cannot write"
    );
}

#[test]
fn commit_authorizations_follow_journal_progress() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let context = fixture.context();
    let prepared = prepare_batch(&engine, &context);

    let ops = engine
        .commit_authorizations(&prepared, 1)
        .expect("authorizations");
    assert_eq!(
        ops.iter()
            .map(|op| op.capability.as_str())
            .collect::<Vec<_>>(),
        [
            "fs.metadata",
            "fs.read",
            "mutation.patch",
            "fs.write",
            "fs.metadata",
            "fs.read",
            "fs.write",
            "fs.delete",
        ],
        "four target and four derived-temp capabilities for the first file"
    );
    assert_eq!(ops[0].scope, "workspace/a.rs");
    assert_eq!(
        ops[4].scope,
        format!("workspace/{}", prepared.files[0].temp_name)
    );
    for op in &ops {
        assert_eq!(op.operation["batch_id"], serde_json::json!(prepared.id));
        assert_eq!(op.operation["action"], serde_json::json!("commit"));
        assert_eq!(op.operation["limit"], serde_json::json!(1));
        assert_eq!(op.operation["path"], serde_json::json!("a.rs"));
        assert_eq!(op.operation["scope"], serde_json::json!(op.scope));
    }

    // The boundary is one file, and the next call derives it from the journal.
    engine
        .commit_authorized_up_to(&context, &prepared, 1)
        .expect("first boundary");
    let next = engine
        .commit_authorizations(&prepared, 1)
        .expect("next authorizations");
    assert_eq!(next[0].scope, "workspace/sub/b.rs");
    assert_eq!(
        next[4].scope,
        format!("workspace/sub/{}", prepared.files[1].temp_name)
    );
    assert_eq!(next[0].operation["path"], serde_json::json!("sub/b.rs"));

    engine
        .commit_authorized_up_to(&context, &prepared, 1)
        .expect("second boundary");
    let error = engine
        .commit_authorizations(&prepared, 1)
        .expect_err("completed batch has nothing to authorize");
    assert!(
        matches!(error, MutationError::AlreadyCompleted(_)),
        "{error}"
    );
}

#[test]
fn foreign_temp_content_refuses_the_boundary() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let context = fixture.context();
    let prepared = prepare_batch(&engine, &context);
    let temp_a = fixture.ws.join(&prepared.files[0].temp_name);
    std::fs::write(&temp_a, b"tampered").expect("tamper the owned temp");
    let error = engine
        .commit_authorized_up_to(&context, &prepared, usize::MAX)
        .expect_err("foreign temp content");
    assert!(error.to_string().contains("foreign content"), "{error}");
    assert_eq!(
        std::fs::read(fixture.ws.join("a.rs")).expect("source"),
        BEFORE,
        "a foreign temp is never renamed over the target"
    );
    assert_eq!(std::fs::read(&temp_a).expect("temp"), b"tampered");
}

#[test]
fn lost_temp_is_restaged_from_the_retained_postimage_under_exact_grants() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let context = fixture.context();
    let prepared = prepare_batch(&engine, &context);
    let temp_a = fixture.ws.join(&prepared.files[0].temp_name);
    std::fs::remove_file(&temp_a).expect("simulate a swept owned temp");

    // Without the external artifact read grants a lost temp cannot be rebuilt.
    let mut withheld = fixture.context();
    for capability in ["fs.read", "fs.metadata"] {
        withheld.policy.deny(
            capability,
            &format!("external:{}/artifacts/**", fixture.state.display()),
        );
    }
    let before = snapshot(&fixture.root);
    let error = engine
        .commit_authorized_up_to(&withheld, &prepared, 1)
        .expect_err("artifact read denied");
    assert!(error.to_string().contains("external:"), "{error}");
    assert_eq!(
        snapshot(&fixture.root),
        before,
        "nothing is staged or renamed without the artifact grant"
    );

    // With the grants, the verified postimage is re-staged and committed.
    let report = engine
        .commit_authorized_up_to(&context, &prepared, 1)
        .expect("re-staged boundary");
    assert_eq!(committed_paths(&report), ["a.rs"]);
    assert!(!report.completed);
    assert_eq!(
        std::fs::read(fixture.ws.join("a.rs")).expect("source"),
        AFTER
    );
    assert!(
        !temp_a.exists(),
        "the re-staged temp is consumed by the rename"
    );
}

#[test]
fn stale_source_refuses_the_boundary_without_touching_the_batch() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let context = fixture.context();
    let prepared = prepare_batch(&engine, &context);
    fixture.write("a.rs", b"foreign");
    let temp_a = fixture.ws.join(&prepared.files[0].temp_name);
    let error = engine
        .commit_authorized_up_to(&context, &prepared, usize::MAX)
        .expect_err("stale source");
    assert!(
        matches!(error, MutationError::StalePreimage { .. }),
        "{error}"
    );
    assert_eq!(
        std::fs::read(fixture.ws.join("a.rs")).expect("source"),
        b"foreign",
        "the foreign image is never overwritten"
    );
    assert_eq!(
        std::fs::read(fixture.ws.join("sub/b.rs")).expect("source"),
        BEFORE,
        "no later file is committed after the refusal"
    );
    assert_eq!(
        std::fs::read(&temp_a).expect("owned temp"),
        AFTER,
        "the owned temp is not consumed"
    );
    let journaled = BatchJournal::open(&fixture.state)
        .expect("journal")
        .replay()
        .expect("replay");
    let batch = &journaled[&prepared.id];
    assert!(!batch.completed, "no completion receipt is written");
    assert!(
        batch
            .files
            .iter()
            .all(|file| file.state == FileState::Prepared),
        "the batch stays exactly as prepared"
    );
}

#[test]
fn stopped_after_one_boundary_stays_partial_for_explicit_scoped_recovery() {
    for (action, disposition, image) in [
        (
            RecoveryAction::Finish,
            RecoveryDisposition::Committed,
            AFTER,
        ),
        (
            RecoveryAction::Compensate,
            RecoveryDisposition::Compensated,
            BEFORE,
        ),
    ] {
        let fixture = Fixture::new();
        let engine = fixture.engine();
        let context = fixture.context();
        let prepared = prepare_batch(&engine, &context);
        let partial = engine
            .commit_authorized_up_to(&context, &prepared, 1)
            .expect("first boundary");
        assert_eq!(committed_paths(&partial), ["a.rs"]);
        assert!(!partial.completed, "a true partial batch survives the stop");

        let allowed: Vec<String> = SPEC_PATHS.map(str::to_owned).to_vec();
        let report = engine
            .recover_scoped(&context, prepared.id, action, &allowed)
            .expect("explicit scoped recovery");
        assert_eq!(report.batch_id, prepared.id);
        assert_eq!(report.disposition, disposition);
        assert_eq!(
            std::fs::read(fixture.ws.join("a.rs")).expect("source"),
            image
        );
        assert_eq!(
            std::fs::read(fixture.ws.join("sub/b.rs")).expect("source"),
            image
        );
    }
}

#[test]
fn unknown_forged_sibling_and_malformed_identity_cannot_authorize_a_commit() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let prepared = prepare_batch(&engine, &fixture.context());

    // Unknown identity: a journal with no record for this batch.
    let empty =
        MutationEngine::open(&fixture.ws, &fixture.root.join("empty-state")).expect("engine");
    let before = snapshot(&fixture.root);
    let error = empty
        .commit_authorized_up_to(&fixture.context(), &prepared, 1)
        .expect_err("unknown batch");
    assert!(matches!(error, MutationError::UnknownBatch(_)), "{error}");
    assert_eq!(snapshot(&fixture.root), before);

    // Forged descriptor: same identity, divergent plan.
    let mut forged = prepared.clone();
    forged.files[0].post_hash = blake3_hex(b"evil");
    let error = engine
        .commit_authorized_up_to(&fixture.context(), &forged, 1)
        .expect_err("forged descriptor");
    assert!(matches!(error, MutationError::UnknownBatch(_)), "{error}");
    assert_eq!(snapshot(&fixture.root), before);

    // Sibling batch in the same attempt journal: never adopted.
    fixture.write("other.rs", BEFORE);
    engine
        .prepare(&[PatchSpec {
            path: "other.rs".to_owned(),
            base_hash: Some(blake3_hex(BEFORE)),
            new_content: AFTER.to_vec(),
        }])
        .expect("trusted sibling preparation");
    let with_sibling = snapshot(&fixture.root);
    let error = engine
        .commit_authorized_up_to(&fixture.context(), &prepared, 1)
        .expect_err("sibling record");
    assert!(matches!(error, MutationError::JournalCorrupt(_)), "{error}");
    assert_eq!(snapshot(&fixture.root), with_sibling);

    // Malformed journal: no write, and new preparation is refused too.
    let broken = Fixture::new();
    let broken_engine = broken.engine();
    let broken_context = broken.context();
    let broken_prepared = prepare_batch(&broken_engine, &broken_context);
    let mut journal = std::fs::OpenOptions::new()
        .append(true)
        .open(broken.state.join("mutation.log"))
        .expect("journal");
    writeln!(journal, "{{\"record\": \"batch_complet").expect("torn record");
    drop(journal);
    let before = snapshot(&broken.root);
    let error = broken_engine
        .commit_authorized_up_to(&broken_context, &broken_prepared, 1)
        .expect_err("malformed journal");
    assert!(matches!(error, MutationError::JournalCorrupt(_)), "{error}");
    let error = broken_engine
        .prepare_authorized(&broken_context, MutationBatchId::generate(), &specs())
        .expect_err("malformed journal blocks preparation");
    assert!(matches!(error, MutationError::JournalCorrupt(_)), "{error}");
    assert_eq!(snapshot(&broken.root), before);
}
