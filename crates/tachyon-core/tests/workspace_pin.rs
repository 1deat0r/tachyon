//! M11 plan item 5 (core part): the canonical workspace root is pinned
//! into the task's DURABLE state through the supervisor's single-writer
//! journal path, exactly once, before any gateway-side policy boundary.
//!
//! The gateway validates/canonicalizes the root first, then calls
//! `pin_workspace_root`; the supervisor journals `workspace_pinned` so
//! recovery replays the pin even without a snapshot, refuses a second,
//! different root with a typed error, and never pins a terminal task.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tachyon_core::{CoreError, TaskStatus, create_task, recover_task};
use tachyon_store::StoreWriter;
use tachyon_types::{SessionId, WorkspaceId};

static COUNTER: AtomicU64 = AtomicU64::new(0);

async fn fixture() -> (
    Arc<StoreWriter>,
    std::path::PathBuf,
    tachyon_core::SupervisorHandle,
) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("tachyon-workspace-pin-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Arc::new(StoreWriter::open(&dir).await.unwrap());
    let session = SessionId::generate();
    store.create_session(&session.to_string()).await.unwrap();
    let handle = create_task(
        session,
        WorkspaceId::generate(),
        "pin the workspace".to_owned(),
        store.clone(),
    )
    .await
    .unwrap();
    (store, dir, handle)
}

#[tokio::test]
async fn pin_is_journalled_set_once_and_survives_recovery() {
    let (store, dir, handle) = fixture().await;
    let task_id = handle.task_id();

    assert_eq!(
        handle.get_state().await.unwrap().workspace_root,
        None,
        "a fresh task starts unpinned"
    );

    let pinned = handle
        .pin_workspace_root("/canonical/ws".to_owned())
        .await
        .unwrap();
    assert_eq!(pinned.workspace_root.as_deref(), Some("/canonical/ws"));
    assert_eq!(
        pinned.revision, 0,
        "pinning is not steering: no revision bump"
    );

    let events = store
        .load_events_since(&task_id.to_string(), -1)
        .await
        .unwrap();
    assert!(
        events.iter().any(|event| event.kind == "workspace_pinned"),
        "pin must be journalled for recovery-visible durability"
    );

    // Same root again is idempotent; a different root is a typed error.
    let again = handle
        .pin_workspace_root("/canonical/ws".to_owned())
        .await
        .unwrap();
    assert_eq!(again.workspace_root.as_deref(), Some("/canonical/ws"));
    let err = handle
        .pin_workspace_root("/canonical/other".to_owned())
        .await
        .unwrap_err();
    assert!(
        matches!(
            &err,
            CoreError::WorkspaceAlreadyPinned { pinned } if pinned == "/canonical/ws"
        ),
        "set-once refusal, got {err:?}"
    );

    // Recovery replays the pin from the journal tail.
    handle.shutdown().await.unwrap();
    let recovered = recover_task(task_id, store.clone()).await.unwrap();
    assert_eq!(
        recovered
            .get_state()
            .await
            .unwrap()
            .workspace_root
            .as_deref(),
        Some("/canonical/ws"),
        "pin must survive a restart without a snapshot"
    );
    recovered.shutdown().await.unwrap();
    store.close().await;
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn pin_is_refused_once_the_task_is_terminal() {
    let (store, dir, handle) = fixture().await;
    let cancelled = handle.cancel().await.unwrap();
    assert_eq!(cancelled.status, TaskStatus::Cancelled);

    let err = handle
        .pin_workspace_root("/canonical/ws".to_owned())
        .await
        .unwrap_err();
    assert!(
        matches!(err, CoreError::IllegalTransition { .. }),
        "a terminal task accepts no pin, got {err:?}"
    );
    assert_eq!(handle.get_state().await.unwrap().workspace_root, None);

    handle.shutdown().await.unwrap();
    store.close().await;
    std::fs::remove_dir_all(&dir).unwrap();
}
