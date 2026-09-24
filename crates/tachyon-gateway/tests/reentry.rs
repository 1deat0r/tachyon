//! M12 driver re-entry / fresh-id gates (ADR 0002).
//!
//! Recovering with a durable workspace pin re-enters the run through
//! `Resume` → `start_run`; Recovering without a pin lands `Paused`.
//! A continuation approval after restart always carries a fresh id.

use std::path::Path;
use std::sync::Arc;

use tachyon_gateway::{GatewayRuntime, start_with};
use tachyon_policy::{ApprovalRequest, Policy};
use tachyon_protocol::Command;
use tachyon_store::StoreWriter;
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
use tachyon_types::{ApprovalId, CapabilityId, SessionId, TaskId, WorkspaceId};

mod common;
use common::{armed_runtime, fake, ok, send, test_dir};

async fn approval_request_count(socket: &Path, task_id: TaskId) -> usize {
    let got = ok(socket, Command::GetTask { task_id }).await;
    got["task"]["approval_requests"]
        .as_array()
        .map_or(0, Vec::len)
}

fn request(id: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: ApprovalId::generate(),
        capability: CapabilityId("mutation.patch".to_owned()),
        scope: format!("workspace/{id}"),
        operation_hash: format!("hash-{id}"),
        summary: format!("parked {id}"),
    }
}

/// ADR 0002 no-run leg: Recovering + no pin → Resume lands Paused,
/// no fresh `approval_request`, nothing granted.
#[tokio::test]
async fn reentry_no_run_resume_gates_landed_paused_without_fresh_approval() {
    let dir = test_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let store = Arc::new(StoreWriter::open(&dir).await.unwrap());
    let session = SessionId::generate();
    store.create_session(&session.to_string()).await.unwrap();
    let handle = tachyon_core::create_task(
        session,
        WorkspaceId::generate(),
        "reentry no run".to_owned(),
        store.clone(),
    )
    .await
    .unwrap();
    let context = Arc::new(ToolsContext::new(
        dir.join("ws-never-touched"),
        Policy::trusted_workspace(),
        ArtifactSpool::new(dir.join("artifacts")),
    ));
    let req = request("no-run");
    handle
        .park_approval(context, req.clone())
        .await
        .expect("park is durable");
    let task_id = handle.task_id();
    handle.shutdown().await.unwrap();
    store.close().await;

    let gateway = start_with(&dir, armed_runtime(fake())).await.unwrap();
    let socket = gateway.address().to_owned();

    let state = ok(&socket, Command::GetTask { task_id }).await;
    assert_eq!(state["task"]["status"], "Recovering");
    assert!(
        state["task"]["workspace_root"].is_null(),
        "API-park has no pin: nothing to re-enter"
    );

    let before = approval_request_count(&socket, task_id).await;
    let resume = send(&socket, Command::ResumeTask { task_id }).await;
    assert_eq!(resume.0, 200, "Resume succeeds: {:?}", resume.2);
    let after = ok(&socket, Command::GetTask { task_id }).await;
    assert_eq!(after["task"]["status"], "Paused", "Recovering → Paused");
    assert_eq!(
        before,
        approval_request_count(&socket, task_id).await,
        "no fresh approval_request without a re-run"
    );
    let row = gateway
        .store()
        .load_by_id(&req.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.decision, "expired", "nothing granted by resuming");

    gateway.shutdown().await;
}

/// ADR 0002 run leg: Recovering + pin → Resume re-enters via `start_run`
/// (respawns the shared driver on the pinned root). The empty fake fails
/// the model stage honestly; admission + spawn is what this asserts.
#[tokio::test]
async fn reentry_with_pin_resume_gates_respawns_run() {
    let dir = test_dir();
    let ws = dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(
        ws.join("Cargo.toml"),
        "[package]\nname = \"w\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let ws = std::fs::canonicalize(&ws).unwrap();

    // Durable shape: pin + approval park under the SAME data dir `start_with` uses.
    let store = Arc::new(StoreWriter::open(&dir).await.unwrap());
    let session = SessionId::generate();
    store.create_session(&session.to_string()).await.unwrap();
    let handle = tachyon_core::create_task(
        session,
        WorkspaceId::generate(),
        "reentry with pin".to_owned(),
        store.clone(),
    )
    .await
    .unwrap();
    handle
        .pin_workspace_root(ws.display().to_string())
        .await
        .expect("pin");
    let context = Arc::new(ToolsContext::new(
        ws.clone(),
        Policy::trusted_workspace(),
        ArtifactSpool::new(dir.join("artifacts")),
    ));
    let req = request("with-pin");
    handle
        .park_approval(context, req.clone())
        .await
        .expect("park");
    let task_id = handle.task_id();
    handle.shutdown().await.unwrap();
    store.close().await;

    let runtime: GatewayRuntime = armed_runtime(fake());
    let gateway = start_with(&dir, runtime).await.unwrap();
    let socket = gateway.address().to_owned();

    let state = ok(&socket, Command::GetTask { task_id }).await;
    assert_eq!(state["task"]["status"], "Recovering");
    let pinned = state["task"]["workspace_root"].as_str().unwrap().to_owned();
    assert_eq!(pinned, ws.display().to_string(), "pin survived recovery");

    let before = approval_request_count(&socket, task_id).await;
    let resume = send(&socket, Command::ResumeTask { task_id }).await;
    assert_eq!(
        resume.0, 200,
        "Resume re-enters the pinned run: {:?}",
        resume.2
    );
    // start_run ack carries the pinned root (re-entry path, not Paused).
    assert_eq!(
        resume.1["workspace_root"].as_str(),
        Some(ws.to_str().expect("utf8 ws")),
        "re-entry acknowledged the pinned run: {}",
        resume.1
    );

    // Second StartRun while the respawned drive is in flight must refuse —
    // proves the shared driver was actually admitted.
    let second = send(
        &socket,
        Command::StartRun {
            task_id,
            workspace_root: ws.display().to_string(),
            acceptance: None,
        },
    )
    .await;
    assert!(
        second.0 == 400 && second.2.contains("run_already_active"),
        "drive is in flight after re-entry: {:?}",
        second.2
    );

    // Fresh-id: old approval stays dead; no silent grant of `req.id`.
    let row = gateway
        .store()
        .load_by_id(&req.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.decision, "expired", "pre-restart id stays dead");
    let after = approval_request_count(&socket, task_id).await;
    assert!(
        after >= before,
        "continuation may re-ask under a FRESH id; count {before} → {after}"
    );

    gateway.shutdown().await;
}
