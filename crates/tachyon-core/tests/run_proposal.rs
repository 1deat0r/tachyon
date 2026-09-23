//! M11 item 6 (core part): `SupervisorCommand::StartRun` executes through
//! the M10 plan §2 proposal/ack pattern — the worker proposes a private,
//! run-ID + task-ID + revision-bound message and the supervisor
//! acknowledges and journals it. Stale, foreign and unknown proposals are
//! typed errors; a replayed `StartRun` for the same run is idempotent.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tachyon_core::{
    RunProposal, RunRecord, StageRecord, SupervisorHandle, create_task, recover_task,
};
use tachyon_store::StoreWriter;
use tachyon_types::{SessionId, WorkspaceId};

static COUNTER: AtomicU64 = AtomicU64::new(0);

async fn open_test_store() -> (Arc<StoreWriter>, std::path::PathBuf) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("tachyon-run-proposal-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Arc::new(StoreWriter::open(&dir).await.unwrap());
    (store, dir)
}

async fn fresh_task(store: &Arc<StoreWriter>) -> SupervisorHandle {
    let session = SessionId::generate();
    store.create_session(&session.to_string()).await.unwrap();
    create_task(
        session,
        WorkspaceId::generate(),
        "run the thing".to_owned(),
        store.clone(),
    )
    .await
    .unwrap()
}

fn proposal(
    handle: &SupervisorHandle,
    run_id: &str,
    revision: u64,
    record: RunRecord,
) -> RunProposal {
    RunProposal {
        run_id: run_id.to_owned(),
        task_id: handle.task_id(),
        revision,
        record,
    }
}

#[tokio::test]
async fn start_run_and_stage_records_flow_through_the_proposal_ack_path() {
    let (store, dir) = open_test_store().await;
    let handle = fresh_task(&store).await;

    // Worker proposes the start bound to run-id + task-id + revision;
    // supervisor acknowledges and journals a `stage` record.
    let state = handle.get_state().await.unwrap();
    let started = handle
        .start_run("run-1".to_owned(), state.revision)
        .await
        .unwrap();
    assert_eq!(
        started.stages,
        vec![StageRecord {
            stage: "run".to_owned(),
            detail: "started".to_owned(),
        }]
    );

    // Ack required before scheduling each stage (M10 plan §2).
    let acked = handle
        .propose(proposal(
            &handle,
            "run-1",
            started.revision,
            RunRecord::Stage {
                stage: "evidence".to_owned(),
                detail: "scheduled".to_owned(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(acked.stages.len(), 2);
    assert_eq!(acked.revision, started.revision);

    // Durable: replay through a fresh recovery leaves identical records.
    let task_id = handle.task_id();
    handle.shutdown().await.unwrap();
    let recovered = recover_task(task_id, store.clone()).await.unwrap();
    let state = recovered.get_state().await.unwrap();
    assert_eq!(state.stages, acked.stages);
    let journal = store
        .load_events_since(&task_id.to_string(), -1)
        .await
        .unwrap();
    assert!(
        journal.iter().any(|e| e.kind == "stage"),
        "journalled stage events"
    );
    recovered.shutdown().await.unwrap();
    store.close().await;
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn stale_foreign_and_unknown_proposals_are_typed_errors_with_zero_writes() {
    let (store, dir) = open_test_store().await;
    let handle = fresh_task(&store).await;
    let task_id = handle.task_id();

    // Proposals before any acknowledged StartRun: unknown run.
    let err = handle
        .propose(proposal(
            &handle,
            "run-1",
            0,
            RunRecord::AgentMessage {
                message: "sneaky".to_owned(),
            },
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(err, tachyon_core::CoreError::UnknownRun { .. }),
        "got {err:?}"
    );

    handle.start_run("run-1".to_owned(), 0).await.unwrap();

    // Steering bumps the revision; the old proposal is stale, refused.
    handle.add_message("pivot".to_owned()).await.unwrap();
    let err = handle
        .propose(proposal(
            &handle,
            "run-1",
            0,
            RunRecord::AgentMessage {
                message: "stale".to_owned(),
            },
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            tachyon_core::CoreError::StaleRunProposal {
                expected: 1,
                got: 0
            }
        ),
        "got {err:?}"
    );

    // A StartRun bound to the old revision is stale too.
    let err = handle.start_run("run-2".to_owned(), 0).await.unwrap_err();
    assert!(matches!(
        err,
        tachyon_core::CoreError::StaleRunProposal { .. }
    ));

    // Foreign task id: never writes to this task.
    let foreign = RunProposal {
        run_id: "run-1".to_owned(),
        task_id: tachyon_types::TaskId::generate(),
        revision: 1,
        record: RunRecord::Stage {
            stage: "evidence".to_owned(),
            detail: "foreign".to_owned(),
        },
    };
    let err = handle.propose(foreign).await.unwrap_err();
    assert!(matches!(
        err,
        tachyon_core::CoreError::ForeignProposal { .. }
    ));

    // A second, different run while one is active: typed conflict.
    let err = handle.start_run("run-3".to_owned(), 1).await.unwrap_err();
    assert!(matches!(
        err,
        tachyon_core::CoreError::RunAlreadyActive { .. }
    ));

    // Zero writes: no new journal events from any refused proposal.
    let after = store
        .load_events_since(&task_id.to_string(), -1)
        .await
        .unwrap()
        .len();
    // created + start_run stage + steering message = 3 legitimate events.
    assert_eq!(after, 3, "refused proposals must journal nothing");

    handle.shutdown().await.unwrap();
    store.close().await;
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn start_run_replay_is_idempotent_and_one_shot_per_run() {
    let (store, dir) = open_test_store().await;
    let handle = fresh_task(&store).await;
    let started = handle.start_run("run-1".to_owned(), 0).await.unwrap();
    // Worker crashed after the ack and replays the identical proposal.
    let replayed = handle.start_run("run-1".to_owned(), 0).await.unwrap();
    assert_eq!(
        replayed.stages, started.stages,
        "a replayed StartRun must not duplicate the journal record"
    );
    assert_eq!(replayed.stages.len(), 1);

    handle.shutdown().await.unwrap();
    store.close().await;
    std::fs::remove_dir_all(&dir).unwrap();
}
