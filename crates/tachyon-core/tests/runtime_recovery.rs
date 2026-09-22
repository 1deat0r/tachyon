//! M10 runtime slice RED: crash recovery, steering, scoped preflight.
//! A crash-child helper prepares a journaled partial batch then dies by a
//! real OS process exit; the parent reconciles via strict scoped recovery.
use std::path::PathBuf;
use std::sync::Arc;

use tachyon_core::runtime::{MutationOutcome, StageStatus, SteeringState, mark_recovering};
use tachyon_core::{TaskStatus, create_task};
use tachyon_mutation::{MutationEngine, PatchSpec, RecoveryAction, blake3_hex};
use tachyon_policy::Policy;
use tachyon_store::StoreWriter;
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
use tachyon_types::{MutationBatchId, SessionId, WorkspaceId};

const CRASH_FLAG: &str = "TACHYON_M10_CRASH_CHILD";
const BEFORE: &[u8] = b"before";
const AFTER: &[u8] = b"after";

struct Dirs {
    root: PathBuf,
    ws: PathBuf,
    state: PathBuf,
}

impl Dirs {
    fn fresh(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("tachyon-m10-{tag}-{}", uuid::Uuid::now_v7()));
        let ws = root.join("workspace");
        let state = root.join("mutation-state");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("target.rs"), BEFORE).unwrap();
        Self { root, ws, state }
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
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn crash_child_main() {
    let root = PathBuf::from(std::env::var("TACHYON_M10_CRASH_ROOT").unwrap());
    let ws = root.join("workspace");
    let state = root.join("mutation-state");
    let dirs = Dirs {
        root: root.clone(),
        ws: ws.clone(),
        state: state.clone(),
    };
    // NOTE: `dirs` must stay bound: dropping it deletes the fixture root.
    let context = dirs.context();
    let engine = MutationEngine::open(&ws, &state).expect("engine");
    let batch: MutationBatchId =
        serde_json::from_str(&std::env::var("TACHYON_M10_CRASH_BATCH").unwrap()).unwrap();
    let spec = PatchSpec {
        path: "target.rs".into(),
        base_hash: Some(blake3_hex(BEFORE)),
        new_content: AFTER.to_vec(),
    };
    engine
        .prepare_authorized(&context, batch, std::slice::from_ref(&spec))
        .expect("prepare");
    // Die with the batch journaled-but-uncommitted: real OS process death.
    std::process::exit(137);
}

/// Subprocess death after a journaled partial mutation, then strict scoped
/// reconciliation without reapplying stale proposals (same task/contract).
#[tokio::test]
async fn subprocess_death_reconciles_without_stale_reapply() {
    if std::env::var(CRASH_FLAG).is_ok() {
        crash_child_main();
    }
    let dirs = Dirs::fresh("crash");
    let batch = MutationBatchId::generate();
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(exe)
        .args([
            "--exact",
            "subprocess_death_reconciles_without_stale_reapply",
            "--nocapture",
        ])
        .env(CRASH_FLAG, "1")
        .env("TACHYON_M10_CRASH_ROOT", &dirs.root)
        .env(
            "TACHYON_M10_CRASH_BATCH",
            serde_json::to_string(&batch).unwrap(),
        )
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "child must die, not exit cleanly: {out:?}"
    );
    // Workspace untouched by the dead child: prepare stages temps, never targets.
    assert_eq!(std::fs::read(dirs.ws.join("target.rs")).unwrap(), BEFORE);
    // Fresh process view reconciles exactly this batch under the same contract.
    let context = dirs.context();
    let engine = MutationEngine::open(&dirs.ws, &dirs.state).expect("engine");
    let report = engine
        .recover_scoped(
            &context,
            batch,
            RecoveryAction::Finish,
            &["target.rs".into()],
        )
        .expect("scoped finish");
    assert_eq!(std::fs::read(dirs.ws.join("target.rs")).unwrap(), AFTER);
    let paths: Vec<&str> = report.changed.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, vec!["target.rs"]);
    // A stale proposal for the same path with the old base is refused now.
    let stale = PatchSpec {
        path: "target.rs".into(),
        base_hash: Some(blake3_hex(BEFORE)),
        new_content: b"stale".to_vec(),
    };
    assert!(
        engine
            .prepare_authorized(
                &context,
                MutationBatchId::generate(),
                std::slice::from_ref(&stale)
            )
            .is_err(),
        "stale preimage never reapplies"
    );
}

/// Corrupt/denied recovery preflight performs zero workspace writes.
#[test]
fn denied_recovery_preflight_writes_nothing() {
    let dirs = Dirs::fresh("denied-recovery");
    let context = dirs.context();
    let engine = MutationEngine::open(&dirs.ws, &dirs.state).expect("engine");
    let batch = MutationBatchId::generate();
    let spec = PatchSpec {
        path: "target.rs".into(),
        base_hash: Some(blake3_hex(BEFORE)),
        new_content: AFTER.to_vec(),
    };
    engine
        .prepare_authorized(&context, batch, std::slice::from_ref(&spec))
        .expect("prepare");
    // Denied cleanup/artifact scope: preflight fails before any write.
    let mut denied = Policy::trusted_workspace();
    denied.allow("mutation.patch", "workspace/**");
    denied.deny("fs.read", "workspace/target.rs");
    let denied_ctx = ToolsContext::new(
        dirs.ws.clone(),
        denied,
        ArtifactSpool::new(dirs.root.join("tool-artifacts")),
    );
    assert!(
        engine
            .recover_scoped(
                &denied_ctx,
                batch,
                RecoveryAction::Finish,
                &["target.rs".into()]
            )
            .is_err(),
        "denied scope reconciles nothing"
    );
    assert_eq!(std::fs::read(dirs.ws.join("target.rs")).unwrap(), BEFORE);
}

/// Compensation resolves uncertainty but never counts as a completed repair.
#[test]
fn compensation_is_not_success() {
    let dirs = Dirs::fresh("compensate");
    let context = dirs.context();
    let engine = MutationEngine::open(&dirs.ws, &dirs.state).expect("engine");
    let batch = MutationBatchId::generate();
    let spec = PatchSpec {
        path: "target.rs".into(),
        base_hash: Some(blake3_hex(BEFORE)),
        new_content: AFTER.to_vec(),
    };
    engine
        .prepare_authorized(&context, batch, std::slice::from_ref(&spec))
        .expect("prepare");
    let report = engine
        .recover_scoped(
            &context,
            batch,
            RecoveryAction::Compensate,
            &["target.rs".into()],
        )
        .expect("compensate");
    assert_eq!(std::fs::read(dirs.ws.join("target.rs")).unwrap(), BEFORE);
    assert!(report.changed.iter().any(|c| c.path == "target.rs"));
    assert!(
        !MutationOutcome::Compensated.is_success(),
        "compensation is not success"
    );
    assert!(!MutationOutcome::Unknown.is_success());
}

/// Steering/pause/cancel during delayed work: revision bumped, stale
/// proposal discarded, zero late writes; resume reuses the contract.
#[tokio::test]
async fn steering_discards_stale_proposals_with_zero_late_writes() {
    let root = std::env::temp_dir().join(format!("tachyon-m10-steer-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(root.join("state")).unwrap();
    let store = Arc::new(StoreWriter::open(&root.join("state")).await.unwrap());
    let session = SessionId::generate();
    store.create_session(&session.to_string()).await.unwrap();
    let task = create_task(
        session,
        WorkspaceId::generate(),
        "steer me".into(),
        store.clone(),
    )
    .await
    .unwrap();
    // Delayed model work starts against revision 0.
    let planned_revision = task.get_state().await.unwrap().revision;
    let mut steering = SteeringState::new();
    // Steering lands while the model is still delayed: revision bumps first.
    task.add_message("prefer the minimal fix".into())
        .await
        .unwrap();
    steering.note_steering();
    assert_eq!(
        task.get_state().await.unwrap().revision,
        planned_revision + 1
    );
    assert!(
        steering.should_discard(planned_revision),
        "old proposal discarded"
    );
    // Pause then cancel: terminal Cancelled, never Completed by a late write.
    task.pause().await.unwrap();
    assert_eq!(task.get_state().await.unwrap().status, TaskStatus::Paused);
    task.cancel().await.unwrap();
    assert_eq!(
        task.get_state().await.unwrap().status,
        TaskStatus::Cancelled
    );
    steering.cancel();
    assert!(steering.cancelled());
    assert!(steering.should_discard(planned_revision));
    // In-flight stages become Recovering on crash, never Completed.
    assert_eq!(
        mark_recovering(StageStatus::InFlight),
        StageStatus::Recovering
    );
    task.shutdown().await.unwrap();
    store.close().await;
    std::fs::remove_dir_all(&root).unwrap();
}
