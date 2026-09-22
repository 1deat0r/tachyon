//! M10: recovery never treats a marker filename as cleanup authority.

use std::path::{Path, PathBuf};

use tachyon_mutation::{
    MutationEngine, PatchSpec, PreparedBatch, RecoveryAction, RecoveryDisposition, blake3_hex,
};
use tachyon_policy::Policy;
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
use tachyon_types::MutationBatchId;

struct Fixture {
    root: PathBuf,
    ws: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tachyon-scoped-recovery-{}",
            MutationBatchId::generate()
        ));
        let ws = root.join("workspace");
        let state = root.join("state");
        std::fs::create_dir_all(&ws).expect("workspace");
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

    fn partial(&self) -> PreparedBatch {
        self.write("a.rs", b"before");
        self.write("sub/b.rs", b"before");
        let engine = self.engine();
        let prepared = engine
            .prepare(&[patch("a.rs"), patch("sub/b.rs")])
            .expect("prepare");
        assert!(
            !engine
                .commit_up_to(&prepared, 1)
                .expect("partial")
                .completed
        );
        prepared
    }

    fn write(&self, path: &str, content: &[u8]) {
        let path = self.ws.join(path);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("parents");
        std::fs::write(path, content).expect("fixture write");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).expect("remove owned fixture");
    }
}

fn patch(path: &str) -> PatchSpec {
    PatchSpec {
        path: path.to_owned(),
        base_hash: Some(blake3_hex(b"before")),
        new_content: b"after".to_vec(),
    }
}

fn bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("read fixture")
}

#[test]
fn legacy_recovery_preserves_another_tasks_prepared_temp_and_protected_marker() {
    let fixture = Fixture::new();
    fixture.write("b.rs", b"before");
    fixture.write("protected/.operator.tachyon-tmp-keep", b"operator data");
    let foreign = MutationEngine::open(&fixture.ws, &fixture.root.join("other-task"))
        .expect("other task engine");
    let prepared = foreign.prepare(&[patch("b.rs")]).expect("prepare foreign");
    let temp = fixture.ws.join(&prepared.files[0].temp_name);
    let recovery = fixture.engine().recover(true).expect("recover empty task");
    assert!(temp.exists(), "another task's journal owns this temp");
    assert_eq!(bytes(&temp), b"after");
    assert_eq!(
        bytes(&fixture.ws.join("protected/.operator.tachyon-tmp-keep")),
        b"operator data"
    );
    assert!(recovery.swept_tmps.is_empty());
}

fn snapshot(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut result = std::collections::BTreeMap::new();
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
                bytes(&path)
            };
            result.insert(key, value);
        }
    }
    result
}

fn append_record(fixture: &Fixture, value: &serde_json::Value) {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(fixture.state.join("mutation.log"))
        .expect("journal");
    writeln!(file, "{value}").expect("record");
}

#[test]
fn scoped_rejects_invalid_journal_shape_before_any_effect() {
    use tachyon_mutation::JournalRecord;
    use tachyon_types::Timestamp;
    let mut failures = Vec::new();
    for case in [
        "sibling",
        "unknown_receipt",
        "unknown_path",
        "duplicate_plan",
        "duplicate_receipt",
        "invalid_initial_state",
        "corrupt",
        "blank",
    ] {
        for action in [RecoveryAction::Finish, RecoveryAction::Compensate] {
            let fixture = Fixture::new();
            let mut prepared = fixture.partial();
            let engine = fixture.engine();
            let at = Timestamp::from_micros(0);
            let record = match case {
                "sibling" => {
                    fixture.write("other.rs", b"before");
                    engine.prepare(&[patch("other.rs")]).expect("sibling");
                    None
                }
                "unknown_receipt" => Some(JournalRecord::FileCommitted {
                    batch_id: MutationBatchId::generate(),
                    path: "a.rs".to_owned(),
                    at,
                }),
                "unknown_path" => Some(JournalRecord::FileCommitted {
                    batch_id: prepared.id,
                    path: "not-in-plan.rs".to_owned(),
                    at,
                }),
                "duplicate_plan" => Some(JournalRecord::BatchStarted {
                    batch_id: prepared.id,
                    files: prepared.files.clone(),
                    at,
                }),
                "duplicate_receipt" => Some(JournalRecord::FileCommitted {
                    batch_id: prepared.id,
                    path: "a.rs".to_owned(),
                    at,
                }),
                "invalid_initial_state" => {
                    prepared.files[0].state = tachyon_mutation::FileState::Committed;
                    std::fs::write(fixture.state.join("mutation.log"), b"").expect("reset fixture");
                    Some(JournalRecord::BatchStarted {
                        batch_id: prepared.id,
                        files: prepared.files,
                        at,
                    })
                }
                "corrupt" | "blank" => {
                    use std::io::Write as _;
                    let mut journal = std::fs::OpenOptions::new()
                        .append(true)
                        .open(fixture.state.join("mutation.log"))
                        .expect("journal");
                    journal
                        .write_all(if case == "corrupt" {
                            b"not-json\n"
                        } else {
                            b"\n"
                        })
                        .expect("inject");
                    None
                }
                _ => unreachable!(),
            };
            if let Some(record) = record {
                append_record(&fixture, &serde_json::to_value(record).expect("encode"));
            }
            let before = snapshot(&fixture.root);
            let result = engine.recover_scoped(&fixture.context(), prepared.id, action, &allowed());
            if result.is_ok() || snapshot(&fixture.root) != before {
                failures.push(format!("{case}/{action:?}: result={result:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "unsafe journal recovery: {failures:#?}"
    );
}

fn rewrite_plan(fixture: &Fixture, edit: impl FnOnce(&mut serde_json::Value)) {
    let journal = fixture.state.join("mutation.log");
    let text = std::fs::read_to_string(&journal).expect("journal read");
    let mut lines = text.lines();
    let mut plan: serde_json::Value =
        serde_json::from_str(lines.next().expect("plan")).expect("json");
    edit(&mut plan);
    let rest: Vec<_> = lines.collect();
    std::fs::write(journal, format!("{plan}\n{}\n", rest.join("\n"))).expect("inject plan");
}

#[test]
fn scoped_preflight_vetoes_entire_attempt_on_mismatch_or_denial() {
    let mut failures = Vec::new();
    for case in [
        "diverged",
        "temp_content",
        "temp_path",
        "artifact_path",
        "artifact_content",
        "artifact_missing",
        "preimage_descriptor",
        "target_alias",
        "not_allowed",
        "directory_grant",
        "workspace_mismatch",
        "deny_metadata",
        "deny_read",
        "deny_write",
        "deny_patch",
        "deny_temp_read",
        "deny_temp_write",
        "deny_cleanup",
        "deny_artifact_read",
    ] {
        for action in [RecoveryAction::Finish, RecoveryAction::Compensate] {
            let fixture = Fixture::new();
            let prepared = fixture.partial();
            let engine = fixture.engine();
            let mut context = fixture.context();
            let mut allowed_paths = allowed();
            let temp = format!("sub/{}", prepared.files[1].temp_name);
            let artifact = fixture
                .state
                .join("artifacts")
                .join(&prepared.files[1].post_artifact.0[..2])
                .join(&prepared.files[1].post_artifact.0);
            match case {
                "diverged" => fixture.write("sub/b.rs", b"operator edits"),
                "temp_content" => fixture.write(&temp, b"operator data"),
                "temp_path" => rewrite_plan(&fixture, |plan| {
                    plan["files"][1]["temp_name"] = "../.operator.tachyon-tmp-keep".into();
                }),
                "artifact_path" => rewrite_plan(&fixture, |plan| {
                    plan["files"][1]["post_artifact"] = "../../operator".into();
                }),
                "artifact_content" => {
                    std::fs::write(&artifact, b"wrong artifact content").expect("corrupt artifact");
                }
                "artifact_missing" => std::fs::remove_file(&artifact).expect("remove artifact"),
                "preimage_descriptor" => rewrite_plan(&fixture, |plan| {
                    plan["files"][1]["pre_artifact"] =
                        prepared.files[1].post_artifact.0.clone().into();
                }),
                "target_alias" => rewrite_plan(&fixture, |plan| {
                    plan["files"][1]["path"] = "sub/./b.rs".into();
                }),
                "not_allowed" => allowed_paths = vec!["a.rs".to_owned()],
                "directory_grant" => allowed_paths = vec!["a.rs".to_owned(), "sub/**".to_owned()],
                "workspace_mismatch" => {
                    context.workspace_root = fixture.root.join("different");
                    std::fs::create_dir(&context.workspace_root).expect("other workspace");
                }
                "deny_metadata" => context.policy.deny("fs.metadata", "workspace/sub/b.rs"),
                "deny_read" => context.policy.deny("fs.read", "workspace/sub/b.rs"),
                "deny_write" => context.policy.deny("fs.write", "workspace/sub/b.rs"),
                "deny_patch" => context.policy.deny("mutation.patch", "workspace/sub/b.rs"),
                "deny_temp_read" => context.policy.deny("fs.read", &format!("workspace/{temp}")),
                "deny_temp_write" => {
                    // A lost temp must be re-staged, not recreated without a grant.
                    std::fs::remove_file(fixture.ws.join(&temp)).expect("lose temp");
                    if action == RecoveryAction::Compensate {
                        context.policy.deny("fs.write", "workspace/**");
                    } else {
                        context
                            .policy
                            .deny("fs.write", &format!("workspace/{temp}"));
                    }
                }
                "deny_cleanup" => context
                    .policy
                    .deny("fs.delete", &format!("workspace/{temp}")),
                "deny_artifact_read" => context
                    .policy
                    .deny("fs.read", &format!("external:{}", artifact.display())),
                _ => unreachable!(),
            }
            fixture.write(".operator.tachyon-tmp-keep", b"protected");
            let before = snapshot(&fixture.root);
            let result = engine.recover_scoped(&context, prepared.id, action, &allowed_paths);
            if result.is_ok() || snapshot(&fixture.root) != before {
                failures.push(format!("{case}/{action:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "preflight must precede ALL effects: {failures:?}"
    );
}

#[test]
fn strict_recovery_preserves_torn_journal_evidence_across_reopens() {
    use std::io::Write as _;
    let fixture = Fixture::new();
    let prepared = fixture.partial();
    let journal = fixture.state.join("mutation.log");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&journal)
        .expect("journal")
        .write_all(b"{\"record\":\"batch_started\"")
        .expect("torn write");
    let before = snapshot(&fixture.root);
    for _ in 0..2 {
        let engine = fixture.engine();
        assert!(
            engine
                .recover_scoped(
                    &fixture.context(),
                    prepared.id,
                    RecoveryAction::Compensate,
                    &allowed()
                )
                .is_err()
        );
        assert_eq!(
            snapshot(&fixture.root),
            before,
            "opening/recovery must preserve corrupt evidence"
        );
    }
}

#[test]
fn legacy_corrupt_journal_compensation_is_read_only() {
    use std::io::Write as _;
    let fixture = Fixture::new();
    fixture.partial();
    let engine = fixture.engine();
    let journal = fixture.state.join("mutation.log");
    std::fs::OpenOptions::new()
        .append(true)
        .open(journal)
        .expect("journal")
        .write_all(b"not-json\n")
        .expect("corrupt line");
    let before = snapshot(&fixture.root);
    let report = engine.recover(false).expect("report gaps");
    assert_eq!(
        snapshot(&fixture.root),
        before,
        "a journal gap is not rollback authority"
    );
    assert_eq!(report.journal_gaps, vec![3]);
    assert!(report.changed.is_empty());
    assert!(report.swept_tmps.is_empty());
}

#[test]
fn strict_journal_rejects_untyped_fields_without_effects() {
    for location in ["record", "file"] {
        let fixture = Fixture::new();
        let prepared = fixture.partial();
        rewrite_plan(&fixture, |plan| {
            if location == "record" {
                plan["unexpected_authority"] = true.into();
            } else {
                plan["files"][1]["unexpected_authority"] = true.into();
            }
        });
        let before = snapshot(&fixture.root);
        let result = fixture.engine().recover_scoped(
            &fixture.context(),
            prepared.id,
            RecoveryAction::Compensate,
            &allowed(),
        );
        assert!(
            result.is_err() && snapshot(&fixture.root) == before,
            "untyped {location} fields are not a valid recovery plan"
        );
    }
}

#[test]
fn scoped_recovery_rejects_target_temp_aliases_before_effects() {
    use tachyon_mutation::{FileMutation, FileState};
    for action in [RecoveryAction::Finish, RecoveryAction::Compensate] {
        let fixture = Fixture::new();
        let prepared = fixture.partial();
        let alias = format!("sub/{}", prepared.files[1].temp_name);
        let name = Path::new(&alias)
            .file_name()
            .expect("name")
            .to_str()
            .expect("utf8");
        let post = ArtifactSpool::new(fixture.state.join("artifacts"))
            .store(b"third")
            .expect("post artifact");
        let file = FileMutation {
            path: alias.clone(),
            pre_hash: Some(blake3_hex(b"after")),
            pre_artifact: Some(prepared.files[1].post_artifact.clone()),
            post_hash: post.0.clone(),
            post_artifact: post,
            temp_name: format!(".{name}.tachyon-tmp-{}", prepared.id),
            state: FileState::Prepared,
        };
        fixture.write(&format!("sub/{}", file.temp_name), b"third");
        rewrite_plan(&fixture, |plan| {
            plan["files"]
                .as_array_mut()
                .expect("files")
                .push(serde_json::to_value(file).expect("file"));
        });
        let mut paths = allowed();
        paths.push(alias);
        let before = snapshot(&fixture.root);
        let result =
            fixture
                .engine()
                .recover_scoped(&fixture.context(), prepared.id, action, &paths);
        assert!(
            result.is_err() && snapshot(&fixture.root) == before,
            "target/temp alias must fail before {action:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn scoped_preflight_rejects_symlinks_even_when_content_hash_matches() {
    use std::os::unix::fs::symlink;
    for case in ["target", "dangling_target", "temp", "artifact", "journal"] {
        for action in [RecoveryAction::Finish, RecoveryAction::Compensate] {
            let fixture = Fixture::new();
            let prepared = fixture.partial();
            let engine = fixture.engine();
            let protected = fixture.root.join("operator-owned");
            let (path, content) = match case {
                "target" | "dangling_target" => (fixture.ws.join("sub/b.rs"), b"before".to_vec()),
                "temp" => (
                    fixture.ws.join("sub").join(&prepared.files[1].temp_name),
                    b"after".to_vec(),
                ),
                "artifact" => (
                    fixture
                        .state
                        .join("artifacts")
                        .join(&prepared.files[1].post_hash[..2])
                        .join(&prepared.files[1].post_hash),
                    b"after".to_vec(),
                ),
                "journal" => (
                    fixture.state.join("mutation.log"),
                    bytes(&fixture.state.join("mutation.log")),
                ),
                _ => unreachable!(),
            };
            std::fs::write(&protected, content).expect("protected file");
            std::fs::remove_file(&path).expect("replace with link");
            symlink(
                if case == "dangling_target" {
                    fixture.root.join("absent")
                } else {
                    protected
                },
                &path,
            )
            .expect("symlink");
            let before = snapshot(&fixture.root);
            let result = engine.recover_scoped(&fixture.context(), prepared.id, action, &allowed());
            assert!(
                result.is_err() && snapshot(&fixture.root) == before,
                "symlink {case}/{action:?} must not be followed"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn scoped_recovery_accepts_canonical_workspace_and_state_aliases() {
    let fixture = Fixture::new();
    let prepared = fixture.partial();
    let ws_alias = fixture.root.join("ws-alias");
    let state_alias = fixture.root.join("state-alias");
    std::os::unix::fs::symlink(&fixture.ws, &ws_alias).expect("workspace alias");
    std::os::unix::fs::symlink(&fixture.state, &state_alias).expect("state alias");
    let mut context = fixture.context();
    context.workspace_root = ws_alias.clone();
    let report = MutationEngine::open(&ws_alias, &state_alias)
        .expect("alias engine")
        .recover_scoped(&context, prepared.id, RecoveryAction::Finish, &allowed())
        .expect("finish through canonical aliases");
    assert_eq!(report.disposition, RecoveryDisposition::Committed);
    assert_eq!(bytes(&fixture.ws.join("sub/b.rs")), b"after");
}

#[test]
fn scoped_finish_restages_missing_temp_from_verified_compressed_artifact() {
    let fixture = Fixture::new();
    fixture.write("a.rs", b"before");
    let content = vec![b'x'; 70_000];
    let prepared = fixture
        .engine()
        .prepare(&[PatchSpec {
            path: "a.rs".to_owned(),
            base_hash: Some(blake3_hex(b"before")),
            new_content: content.clone(),
        }])
        .expect("prepare compressed");
    std::fs::remove_file(fixture.ws.join(&prepared.files[0].temp_name)).expect("lost temp");
    let report = fixture
        .engine()
        .recover_scoped(
            &fixture.context(),
            prepared.id,
            RecoveryAction::Finish,
            &["a.rs".to_owned()],
        )
        .expect("restage");
    assert_eq!(report.disposition, RecoveryDisposition::Committed);
    assert_eq!(bytes(&fixture.ws.join("a.rs")), content);
}

#[test]
fn scoped_created_file_compensation_requires_exact_deletion_permission() {
    let fixture = Fixture::new();
    fixture.write("sub/b.rs", b"before");
    let prepared = fixture
        .engine()
        .prepare(&[
            PatchSpec {
                path: "a.rs".to_owned(),
                base_hash: None,
                new_content: b"created".to_vec(),
            },
            patch("sub/b.rs"),
        ])
        .expect("create plan");
    fixture
        .engine()
        .commit_up_to(&prepared, 1)
        .expect("partial create");
    let mut context = fixture.context();
    context.policy.deny("fs.delete", "workspace/a.rs");
    let before = snapshot(&fixture.root);
    let error = fixture
        .engine()
        .recover_scoped(
            &context,
            prepared.id,
            RecoveryAction::Compensate,
            &allowed(),
        )
        .expect_err("denied deletion");
    assert!(!error.is_retryable());
    assert_eq!(snapshot(&fixture.root), before);
    let report = fixture
        .engine()
        .recover_scoped(
            &fixture.context(),
            prepared.id,
            RecoveryAction::Compensate,
            &allowed(),
        )
        .expect("authorized compensation");
    assert_eq!(report.disposition, RecoveryDisposition::Compensated);
    assert!(!fixture.ws.join("a.rs").exists());
    assert_eq!(bytes(&fixture.ws.join("sub/b.rs")), b"before");
    let receipts = tachyon_mutation::BatchJournal::open(&fixture.state)
        .expect("journal")
        .replay()
        .expect("receipts");
    assert!(!receipts[&prepared.id].completed);
    assert!(
        receipts[&prepared.id]
            .files
            .iter()
            .all(|file| file.state == tachyon_mutation::FileState::RolledBack)
    );
}

#[test]
fn scoped_finish_catches_up_rename_without_receipt() {
    let fixture = Fixture::new();
    let prepared = fixture.partial();
    std::fs::rename(
        fixture.ws.join("sub").join(&prepared.files[1].temp_name),
        fixture.ws.join("sub/b.rs"),
    )
    .expect("unreceipted rename");
    let report = fixture
        .engine()
        .recover_scoped(
            &fixture.context(),
            prepared.id,
            RecoveryAction::Finish,
            &allowed(),
        )
        .expect("catch up");
    assert_eq!(report.disposition, RecoveryDisposition::Committed);
    assert_eq!(
        report
            .changed
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["sub/b.rs"]
    );
}

#[test]
fn scoped_compensation_catches_up_restore_without_receipt() {
    let fixture = Fixture::new();
    let prepared = fixture.partial();
    fixture.write("a.rs", b"before"); // crash after restore, before rollback receipt
    let source_time = std::fs::metadata(fixture.ws.join("a.rs"))
        .expect("metadata")
        .modified()
        .expect("mtime");
    let report = fixture
        .engine()
        .recover_scoped(
            &fixture.context(),
            prepared.id,
            RecoveryAction::Compensate,
            &allowed(),
        )
        .expect("catch up rollback");
    assert_eq!(report.disposition, RecoveryDisposition::Compensated);
    assert_eq!(report.changed.len(), 2);
    assert_eq!(
        std::fs::metadata(fixture.ws.join("a.rs"))
            .expect("metadata")
            .modified()
            .expect("mtime"),
        source_time,
        "already restored source needs only a receipt"
    );
}

#[test]
fn scoped_terminal_receipts_require_fresh_images_before_cleanup() {
    for action in [RecoveryAction::Finish, RecoveryAction::Compensate] {
        let fixture = Fixture::new();
        let prepared = fixture.partial();
        let engine = fixture.engine();
        engine
            .recover_scoped(&fixture.context(), prepared.id, action, &allowed())
            .expect("initial recovery");
        // The opposite known image is still divergence from a terminal receipt.
        fixture.write(
            "a.rs",
            if action == RecoveryAction::Finish {
                b"before"
            } else {
                b"after"
            },
        );
        fixture.write(&format!("sub/{}", prepared.files[1].temp_name), b"after");
        let before = snapshot(&fixture.root);
        let result = engine.recover_scoped(&fixture.context(), prepared.id, action, &allowed());
        assert!(result.is_err() && snapshot(&fixture.root) == before);
    }
}

#[test]
fn scoped_unknown_batch_has_no_effects() {
    let fixture = Fixture::new();
    fixture.partial();
    let before = snapshot(&fixture.root);
    let result = fixture.engine().recover_scoped(
        &fixture.context(),
        MutationBatchId::generate(),
        RecoveryAction::Compensate,
        &allowed(),
    );
    assert!(result.is_err() && snapshot(&fixture.root) == before);
}

#[test]
fn legacy_cleanup_retains_changed_owned_temp_and_unjournaled_restore_orphan() {
    let fixture = Fixture::new();
    let prepared = fixture.partial();
    let temp = format!("sub/{}", prepared.files[1].temp_name);
    fixture.write(&temp, b"operator replaced owned temp");
    fixture.write(".a.rs.tachyon-restore-unrecorded", b"before");
    let report = fixture.engine().recover(false).expect("legacy compensate");
    assert!(report.swept_tmps.is_empty());
    assert_eq!(
        bytes(&fixture.ws.join(temp)),
        b"operator replaced owned temp"
    );
    assert_eq!(
        bytes(&fixture.ws.join(".a.rs.tachyon-restore-unrecorded")),
        b"before"
    );
}

#[test]
fn scoped_reconciliation_preserves_foreign_task_temps() {
    for action in [RecoveryAction::Finish, RecoveryAction::Compensate] {
        let fixture = Fixture::new();
        let prepared = fixture.partial();
        let foreign_state = fixture.root.join("another-task");
        let foreign = MutationEngine::open(&fixture.ws, &foreign_state)
            .expect("foreign engine")
            .prepare(&[patch("sub/b.rs")])
            .expect("foreign prepared batch");
        let foreign_temp = fixture.ws.join("sub").join(&foreign.files[0].temp_name);
        let journal = bytes(&foreign_state.join("mutation.log"));
        fixture
            .engine()
            .recover_scoped(&fixture.context(), prepared.id, action, &allowed())
            .expect("own reconciliation");
        assert_eq!(bytes(&foreign_temp), b"after");
        assert_eq!(bytes(&foreign_state.join("mutation.log")), journal);
    }
}

fn allowed() -> Vec<String> {
    vec!["a.rs".to_owned(), "sub/b.rs".to_owned()]
}

#[test]
fn scoped_finish_reconciles_partial_batch_and_repeated_receipts() {
    let fixture = Fixture::new();
    let prepared = fixture.partial();
    let engine = fixture.engine();
    let context = fixture.context();
    let report = engine
        .recover_scoped(&context, prepared.id, RecoveryAction::Finish, &allowed())
        .expect("finish");
    assert_eq!(report.batch_id, prepared.id);
    assert_eq!(report.disposition, RecoveryDisposition::Committed);
    assert_eq!(report.changed.len(), 1);
    assert_eq!(bytes(&fixture.ws.join("a.rs")), b"after");
    assert_eq!(bytes(&fixture.ws.join("sub/b.rs")), b"after");
    let again = engine
        .recover_scoped(&context, prepared.id, RecoveryAction::Finish, &allowed())
        .expect("repeat");
    assert_eq!(again.disposition, RecoveryDisposition::Committed);
    assert!(again.changed.is_empty());
    assert!(again.cleaned.is_empty());
}

#[test]
fn scoped_compensation_proves_preimages_cleans_owned_temp_and_is_repeatable() {
    let fixture = Fixture::new();
    let prepared = fixture.partial();
    let temp = fixture.ws.join("sub").join(&prepared.files[1].temp_name);
    fixture.write(".operator.tachyon-tmp-keep", b"keep");
    let engine = fixture.engine();
    let context = fixture.context();
    let report = engine
        .recover_scoped(
            &context,
            prepared.id,
            RecoveryAction::Compensate,
            &allowed(),
        )
        .expect("compensate");
    assert_eq!(report.disposition, RecoveryDisposition::Compensated);
    assert_eq!(report.changed.len(), 2);
    assert_eq!(report.cleaned, vec![temp]);
    assert_eq!(bytes(&fixture.ws.join("a.rs")), b"before");
    assert_eq!(bytes(&fixture.ws.join("sub/b.rs")), b"before");
    assert_eq!(
        bytes(&fixture.ws.join(".operator.tachyon-tmp-keep")),
        b"keep"
    );
    let again = engine
        .recover_scoped(
            &context,
            prepared.id,
            RecoveryAction::Compensate,
            &allowed(),
        )
        .expect("repeat compensation");
    assert_eq!(again.disposition, RecoveryDisposition::Compensated);
    assert!(again.changed.is_empty());
    assert!(again.cleaned.is_empty());
    assert!(
        engine
            .recover_scoped(&context, prepared.id, RecoveryAction::Finish, &allowed())
            .is_err(),
        "never resurrect compensation"
    );
}
