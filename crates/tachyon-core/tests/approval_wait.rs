//! M11 item 8 / D4 + G6 core primitives: a supervisor-owned job that hits
//! `ToolError::ApprovalRequired` parks the task, journals
//! `approval_request`, and inserts a pending row in the existing
//! 5-column `approvals` table (`decision='pending'`, `decided_at=0`).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tachyon_core::{TaskStatus, create_task, recover_task};
use tachyon_policy::ApprovalRequest;
use tachyon_policy::Policy;
use tachyon_store::StoreWriter;
use tachyon_tools::ToolsContext;
use tachyon_tools::artifact::ArtifactSpool;
use tachyon_types::{ApprovalId, CapabilityId, SessionId, WorkspaceId};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    store: Arc<StoreWriter>,
    dir: std::path::PathBuf,
    handle: tachyon_core::SupervisorHandle,
    context: Arc<ToolsContext>,
}

async fn fixture() -> Fixture {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("tachyon-approval-wait-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Arc::new(StoreWriter::open(&dir).await.unwrap());
    let session = SessionId::generate();
    store.create_session(&session.to_string()).await.unwrap();
    let handle = create_task(
        session,
        WorkspaceId::generate(),
        "needs an approval".to_owned(),
        store.clone(),
    )
    .await
    .unwrap();
    let context = Arc::new(ToolsContext::new(
        dir.clone(),
        Policy::trusted_workspace(),
        ArtifactSpool::new(dir.join("artifacts")),
    ));
    Fixture {
        store,
        dir,
        handle,
        context,
    }
}

fn ask(summary: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: ApprovalId::generate(),
        capability: CapabilityId("mutation.patch".to_owned()),
        scope: "workspace/src/lib.rs".to_owned(),
        operation_hash: format!("hash-of-{summary}"),
        summary: summary.to_owned(),
    }
}

#[tokio::test]
async fn park_transitions_journals_and_inserts_the_pending_row() {
    let fx = fixture().await;
    let request = ask("patch");

    let _waiter = fx
        .handle
        .park_approval(fx.context.clone(), request.clone())
        .await
        .unwrap();

    let state = fx.handle.get_state().await.unwrap();
    assert_eq!(state.status, TaskStatus::WaitingApproval);
    assert_eq!(state.approval_requests, vec![request.clone()]);
    assert_eq!(state.revision, 0, "parking is not steering");

    let row = fx
        .store
        .load_by_id(&request.id.to_string())
        .await
        .unwrap()
        .expect("pending row must exist");
    assert_eq!(row.task_id, fx.handle.task_id().to_string());
    assert_eq!(row.operation_hash, request.operation_hash);
    assert_eq!(row.decision, "pending");
    assert_eq!(row.decided_at, 0, "D4: decided_at is 0 while pending");

    let pending = fx
        .store
        .load_pending_for_task(&fx.handle.task_id().to_string())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);

    let journal = fx
        .store
        .load_events_since(&fx.handle.task_id().to_string(), -1)
        .await
        .unwrap();
    assert!(
        journal.iter().any(|e| e.kind == "approval_request"),
        "the parked ask must be journalled"
    );

    fx.handle.shutdown().await.unwrap();
    fx.store.close().await;
    std::fs::remove_dir_all(&fx.dir).unwrap();
}

/// Ask-policy context sharing the SAME registry the supervisor arms on a
/// grant, so the parked operation's re-run goes through real
/// `tachyon_tools::authorize`.
fn ask_context(dir: &std::path::Path) -> Arc<ToolsContext> {
    Arc::new(ToolsContext::new(
        dir.to_path_buf(),
        Policy::new(tachyon_policy::DefaultPosture::Ask),
        ArtifactSpool::new(dir.join("artifacts-ask")),
    ))
}

fn read_operation() -> serde_json::Value {
    serde_json::json!({
        "capability": "fs.read",
        "scope": "workspace/data.txt",
        "path": "data.txt"
    })
}

fn ask_err(context: &ToolsContext) -> tachyon_tools::ToolError {
    tachyon_tools::authorize(
        &context.policy,
        &context.approvals,
        "fs.read",
        "workspace/data.txt",
        &read_operation(),
        "read data.txt",
    )
    .expect_err("ask posture must refuse before a decision")
}

/// G6 primitive: grant -> row `applied` before execution + exactly one
/// re-run under the same operation hash, then the grant is consumed.
#[tokio::test]
async fn grant_marks_applied_before_the_single_authorized_rerun() {
    let fx = fixture().await;
    let context = ask_context(&fx.dir);
    let tachyon_tools::ToolError::ApprovalRequired { request, .. } = ask_err(&context) else {
        panic!("expected ApprovalRequired before any decision");
    };
    let waiter = fx
        .handle
        .park_approval(context.clone(), *request.clone())
        .await
        .unwrap();

    fx.handle
        .decide_approval(request.id, true, "human said go".to_owned())
        .await
        .unwrap();

    // The waiter resolves only after the durable applied marker.
    assert_eq!(
        waiter.wait().await.unwrap(),
        tachyon_core::ApprovalResolution::Granted
    );
    let row = fx
        .store
        .load_by_id(&request.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.decision, "applied", "applied is durable before re-run");
    assert!(row.decided_at > 0);

    // Exactly one re-run succeeds under the same operation hash...
    tachyon_tools::authorize(
        &context.policy,
        &context.approvals,
        "fs.read",
        "workspace/data.txt",
        &read_operation(),
        "read data.txt",
    )
    .expect("the granted re-run must be authorized exactly once");

    // ...and is consumed: the same operation must Ask again under a FRESH
    // approval id (the model can never mint or replay its own grant).
    let tachyon_tools::ToolError::ApprovalRequired {
        request: second, ..
    } = ask_err(&context)
    else {
        panic!("second use must re-ask: grant was not consumed");
    };
    assert_ne!(second.id, request.id, "fresh approval id on re-ask");

    let state = fx.handle.get_state().await.unwrap();
    assert_eq!(
        state.status,
        TaskStatus::Executing,
        "wait resolved: execution resumes"
    );
    let journal = fx
        .store
        .load_events_since(&fx.handle.task_id().to_string(), -1)
        .await
        .unwrap();
    assert!(journal.iter().any(|e| e.kind == "approval"));

    fx.handle.shutdown().await.unwrap();
    fx.store.close().await;
    std::fs::remove_dir_all(&fx.dir).unwrap();
}

/// G6 primitive: double decide is a typed error; the first decision stands.
#[tokio::test]
async fn double_decide_is_a_typed_error() {
    let fx = fixture().await;
    let request = ask("once");
    let _waiter = fx
        .handle
        .park_approval(fx.context.clone(), request.clone())
        .await
        .unwrap();
    fx.handle
        .decide_approval(request.id, true, String::new())
        .await
        .unwrap();
    let second = fx
        .handle
        .decide_approval(request.id, false, "overturned".to_owned())
        .await
        .unwrap_err();
    assert!(
        matches!(
            second,
            tachyon_core::CoreError::ApprovalNotPending { ref decision, .. } if decision == "applied"
        ),
        "got {second:?}"
    );
    let row = fx
        .store
        .load_by_id(&request.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.decision, "applied", "first decision must stand");

    // A never-parked id is a different typed error.
    let stranger = ApprovalId::generate();
    let missing = fx
        .handle
        .decide_approval(stranger, true, String::new())
        .await
        .unwrap_err();
    assert!(matches!(
        missing,
        tachyon_core::CoreError::ApprovalMissing { .. }
    ));

    fx.handle.shutdown().await.unwrap();
    fx.store.close().await;
    std::fs::remove_dir_all(&fx.dir).unwrap();
}

/// G6 primitive: deny delivers the recorded failure to the waiter.
#[tokio::test]
async fn denial_delivers_the_recorded_failure_to_the_waiter() {
    let fx = fixture().await;
    let request = ask("deny me");
    let waiter = fx
        .handle
        .park_approval(fx.context.clone(), request.clone())
        .await
        .unwrap();
    fx.handle
        .decide_approval(request.id, false, "touches a migration".to_owned())
        .await
        .unwrap();
    assert_eq!(
        waiter.wait().await.unwrap(),
        tachyon_core::ApprovalResolution::Denied {
            reason: "touches a migration".to_owned()
        }
    );
    let row = fx
        .store
        .load_by_id(&request.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.decision, "denied");
    assert!(row.decided_at > 0);
    let state = fx.handle.get_state().await.unwrap();
    assert_eq!(state.status, TaskStatus::Executing, "the wait is over");

    fx.handle.shutdown().await.unwrap();
    fx.store.close().await;
    std::fs::remove_dir_all(&fx.dir).unwrap();
}

/// G6 primitive: cancel wins — terminal `Cancelled`, pending row expired,
/// parked operation never runs, later Approve is a typed error.
#[tokio::test]
async fn cancel_during_wait_is_terminal_expires_the_row_and_never_reruns() {
    let fx = fixture().await;
    let context = ask_context(&fx.dir);
    let tachyon_tools::ToolError::ApprovalRequired { request, .. } = ask_err(&context) else {
        panic!("expected ApprovalRequired");
    };
    let waiter = fx
        .handle
        .park_approval(context.clone(), *request.clone())
        .await
        .unwrap();

    let state = fx.handle.cancel().await.unwrap();
    assert_eq!(state.status, tachyon_core::TaskStatus::Cancelled);

    // Row expired (checked before awaiting so a RED fails instead of hangs).
    let row = fx
        .store
        .load_by_id(&request.id.to_string())
        .await
        .unwrap()
        .expect("row must still exist");
    assert_eq!(row.decision, "expired", "cancel expires the pending row");
    assert!(
        fx.store
            .load_pending_for_task(&fx.handle.task_id().to_string())
            .await
            .unwrap()
            .is_empty()
    );

    // The waiter is told the wait died; it is never a grant.
    assert_eq!(
        waiter.wait().await.unwrap(),
        tachyon_core::ApprovalResolution::Cancelled
    );

    // Later Approve on the expired id: typed error.
    let late = fx
        .handle
        .decide_approval(request.id, true, String::new())
        .await
        .unwrap_err();
    assert!(
        matches!(
            late,
            tachyon_core::CoreError::ApprovalNotPending { ref decision, .. } if decision == "expired"
        ),
        "got {late:?}"
    );

    // The parked operation never ran and never got a registry grant: the
    // same operation is still an unsatisfied ask under a fresh id.
    let tachyon_tools::ToolError::ApprovalRequired { request: fresh, .. } = ask_err(&context)
    else {
        panic!("the cancelled operation must never be authorized");
    };
    assert_ne!(fresh.id, request.id);

    fx.handle.shutdown().await.unwrap();
    fx.store.close().await;
    std::fs::remove_dir_all(&fx.dir).unwrap();
}

/// G6 primitive: a task journalled in `WaitingApproval` recovers cleanly
/// (never `Corrupt`), lands in spec §41 `Recovering` per the plan's
/// adjudicated restart variant, has its stale pending row expired, and a
/// later decision on that id is a typed error — the decision is required
/// again under a fresh request.
#[tokio::test]
async fn waiting_approval_journal_recovers_into_recovering_with_expired_row() {
    let fx = fixture().await;
    let request = ask("across restart");
    let _waiter = fx
        .handle
        .park_approval(fx.context.clone(), request.clone())
        .await
        .unwrap();
    let task_id = fx.handle.task_id();
    fx.handle.shutdown().await.unwrap();

    let recovered = recover_task(task_id, fx.store.clone()).await.unwrap();
    let state = recovered.get_state().await.unwrap();
    assert_eq!(
        state.status,
        tachyon_core::TaskStatus::Recovering,
        "restart lands in Recovering, never silently back in the wait"
    );
    // The approval_request event replays cleanly as a typed record.
    assert_eq!(state.approval_requests, vec![request.clone()]);

    let row = fx
        .store
        .load_by_id(&request.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.decision, "expired",
        "stale pending rows expire on restart"
    );
    assert!(
        fx.store
            .load_pending_for_task(&task_id.to_string())
            .await
            .unwrap()
            .is_empty()
    );

    // A decision on the pre-restart id is a typed error.
    let err = recovered
        .decide_approval(request.id, true, String::new())
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            tachyon_core::CoreError::ApprovalNotPending { ref decision, .. } if decision == "expired"
        ),
        "got {err:?}"
    );

    recovered.shutdown().await.unwrap();
    fx.store.close().await;
    std::fs::remove_dir_all(&fx.dir).unwrap();
}

/// Seat2-B3 (R1): a crash between `decide(granted)` and `mark_applied`
/// leaves a `granted`-never-`applied` row. Nothing could have executed
/// (execution needs the waiter resolved after `applied`), so recovery must
/// expire it like a stale pending row instead of leaving it wedged forever.
#[tokio::test]
async fn granted_without_applied_expires_on_recovery() {
    let fx = fixture().await;
    let request = ask("granted then crashed");
    let _waiter = fx
        .handle
        .park_approval(fx.context.clone(), request.clone())
        .await
        .unwrap();
    let task_id = fx.handle.task_id();
    // Simulate the crash window: durable decision recorded, `applied`
    // never written, re-run never started.
    fx.store
        .decide(
            &request.id.to_string(),
            tachyon_store::ApprovalOutcome::Granted,
        )
        .await
        .unwrap();
    fx.handle.shutdown().await.unwrap();

    let recovered = recover_task(task_id, fx.store.clone()).await.unwrap();
    let row = fx
        .store
        .load_by_id(&request.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.decision, "expired",
        "granted-without-applied rows expire on restart"
    );
    // Journal/row agree in the SAFE direction (R1 board B4): the crash
    // happened before any decision event existed, so the journal must
    // show the request only — never a grant the store never recorded.
    let kinds: Vec<String> = fx
        .store
        .load_events_since(&task_id.to_string(), -1)
        .await
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect();
    assert!(
        kinds.iter().any(|kind| kind == "approval_request"),
        "the parked request is journalled: {kinds:?}"
    );
    assert!(
        !kinds.iter().any(|kind| kind == "approval"),
        "no forged decision event exists: {kinds:?}"
    );

    recovered.shutdown().await.unwrap();
    fx.store.close().await;
    std::fs::remove_dir_all(&fx.dir).unwrap();
}

/// R1 board B4/B2 (amended M11 scope): a crash AFTER `applied` leaves the
/// durable row as history — recovery must NOT expire or re-arm it, and a
/// later decision on the consumed id is a typed refusal. The continuation
/// re-ask rides M12's driver re-entry; M11 asserts never-a-silent-auto-
/// re-run (spec §19: recovery re-arms nothing).
#[tokio::test]
async fn applied_row_survives_recovery_undecidable_and_unjournaled() {
    let fx = fixture().await;
    let request = ask("applied then crashed");
    let _waiter = fx
        .handle
        .park_approval(fx.context.clone(), request.clone())
        .await
        .unwrap();
    let task_id = fx.handle.task_id();
    fx.store
        .decide(
            &request.id.to_string(),
            tachyon_store::ApprovalOutcome::Granted,
        )
        .await
        .unwrap();
    fx.store
        .mark_applied(&request.id.to_string())
        .await
        .unwrap();
    fx.handle.shutdown().await.unwrap();

    let recovered = recover_task(task_id, fx.store.clone()).await.unwrap();
    let row = fx
        .store
        .load_by_id(&request.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.decision, "applied",
        "applied rows are history: recovery neither expires nor re-arms them"
    );
    // The consumed id stays undecidable: no replayed grant, no second use.
    let err = recovered
        .decide_approval(request.id, true, String::new())
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            tachyon_core::CoreError::ApprovalNotPending { ref decision, .. } if decision == "applied"
        ),
        "double decide on a consumed id is a typed refusal: {err:?}"
    );
    // Crash lost the audit event (journal lags row by design); recovery
    // does not invent one.
    let kinds: Vec<String> = fx
        .store
        .load_events_since(&task_id.to_string(), -1)
        .await
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect();
    assert!(
        !kinds.iter().any(|kind| kind == "approval"),
        "recovery never journalls a decision: {kinds:?}"
    );

    recovered.shutdown().await.unwrap();
    fx.store.close().await;
    std::fs::remove_dir_all(&fx.dir).unwrap();
}
