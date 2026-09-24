//! G6 restart leg (plan item 8, restart-during-wait semantics): park an
//! approval, drop the gateway, restart on the same store dir, and assert
//! the observable contract —
//!
//!   * the non-terminal task recovers to `Recovering` (spec §41) via the
//!     EXISTING `recover_incomplete` -> `recover_task` path (this test
//!     confirms that landed logic; no gateway-side duplicate exists),
//!   * the stale `pending` row is `expired`,
//!   * a later `Approve` on the expired id is a TYPED `approval_*` error,
//!   * the attempt to continue through `Resume` does not manufacture a
//!     fresh `approval_request` (decision stays required after restart).
//!
//! The park itself uses the supervisor API directly (the gateway's
//! default trusted-workspace policy never Asks, so no protocol-driven
//! run parks); the durable state it leaves — `WaitingApproval` + a
//! pending row — is exactly what a crashed gateway leaves behind, and
//! the first `start` over that store dir runs the real recovery code
//! path a process kill would.

use std::path::Path;
use std::sync::Arc;

use tachyon_gateway::start;
use tachyon_policy::{ApprovalRequest, Policy};
use tachyon_protocol::Command;
use tachyon_store::StoreWriter;
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
use tachyon_types::{ApprovalId, CapabilityId, SessionId, TaskId, WorkspaceId};

mod common;
use common::{code_of, err, new_task, ok, send, test_dir};

/// Parks one approval into `dir`'s store and shuts everything down:
/// the exact durable shape a gateway crash during an approval wait leaves.
async fn park_before_gateway(dir: &Path) -> (TaskId, ApprovalId) {
    std::fs::create_dir_all(dir).unwrap();
    let store = Arc::new(StoreWriter::open(dir).await.unwrap());
    let session = SessionId::generate();
    store.create_session(&session.to_string()).await.unwrap();
    let handle = tachyon_core::create_task(
        session,
        WorkspaceId::generate(),
        "restart during approval wait".to_owned(),
        store.clone(),
    )
    .await
    .unwrap();
    let context = Arc::new(ToolsContext::new(
        dir.join("workspace-never-touched"),
        Policy::trusted_workspace(),
        ArtifactSpool::new(dir.join("artifacts")),
    ));
    let request = ApprovalRequest {
        id: ApprovalId::generate(),
        capability: CapabilityId("mutation.patch".to_owned()),
        scope: "workspace/src/lib.rs".to_owned(),
        operation_hash: "hash-parked-at-restart".to_owned(),
        summary: "parked operation".to_owned(),
    };
    handle
        .park_approval(context, request.clone())
        .await
        .expect("park is durable");
    let task_id = handle.task_id();
    handle.shutdown().await.unwrap();
    store.close().await;
    (task_id, request.id)
}

async fn approval_request_count(socket: &Path, task_id: TaskId) -> usize {
    let got = ok(socket, Command::GetTask { task_id }).await;
    got["task"]["approval_requests"]
        .as_array()
        .map_or(0, Vec::len)
}

#[tokio::test]
async fn restart_during_approval_wait_recovers_expires_and_refuses_typed() {
    let dir = test_dir();
    let (task_id, approval) = park_before_gateway(&dir).await;

    // --- leg 1: the first start over the parked store runs recovery ---
    let gateway = start(&dir).await.unwrap();
    let socket = gateway.address().to_owned();

    let state = ok(&socket, Command::GetTask { task_id }).await;
    assert_eq!(
        state["task"]["status"], "Recovering",
        "restart lands in spec 41 Recovering, never back in the wait"
    );

    let row = gateway
        .store()
        .load_by_id(&approval.to_string())
        .await
        .unwrap()
        .expect("the parked row exists");
    assert_eq!(
        row.decision, "expired",
        "the stale pending row is expired by the recovery path"
    );
    assert!(
        gateway
            .store()
            .load_pending_for_task(&task_id.to_string())
            .await
            .unwrap()
            .is_empty(),
        "no pending row survives recovery"
    );

    let got = err(
        &socket,
        Command::Approve {
            task_id,
            approval_id: approval,
        },
    )
    .await;
    assert_eq!(
        code_of(&got),
        "approval_not_pending",
        "Approve on an expired id is a typed approval_* error: {got}"
    );

    // Drop the gateway for real: drain, release the runtime dir, close
    // the store — the same shape a process kill leaves on disk.
    gateway.shutdown().await;

    // --- leg 2: restart on the SAME store dir ---
    let gateway = start(&dir).await.unwrap();
    assert_eq!(gateway.address(), socket.as_path());

    let state = ok(&socket, Command::GetTask { task_id }).await;
    assert_eq!(state["task"]["status"], "Recovering");
    let row = gateway
        .store()
        .load_by_id(&approval.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.decision, "expired");
    let got = err(
        &socket,
        Command::Approve {
            task_id,
            approval_id: approval,
        },
    )
    .await;
    assert_eq!(code_of(&got), "approval_not_pending", "{got}");

    // --- continuation: no workspace pin means no run to re-enter, so
    //     `Resume` lands `Recovering → Paused` (ADR 0002). It must not
    //     manufacture a fresh `approval_request` or silently grant. The
    //     fresh-id + driver-re-entry leg lives in `reentry.rs` (run case). ---
    let before = approval_request_count(&socket, task_id).await;
    let resume = send(&socket, Command::ResumeTask { task_id }).await;
    assert_eq!(
        resume.0, 200,
        "M12 Resume on Recovering with no run lands Paused: {:?}",
        resume.2
    );
    let after_state = ok(&socket, Command::GetTask { task_id }).await;
    assert_eq!(
        after_state["task"]["status"], "Paused",
        "Recovering with no pin transitions to Paused"
    );
    let after = approval_request_count(&socket, task_id).await;
    assert_eq!(
        before, after,
        "no fresh approval_request appears without a real re-run"
    );
    let row = gateway
        .store()
        .load_by_id(&approval.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.decision, "expired", "nothing was granted by resuming");

    gateway.shutdown().await;
    let _ = new_task; // keep the shared helper imported for symmetry
}
