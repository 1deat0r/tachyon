//! M11 plan item 8 / D4 (gateway part): `Command::Approve`/`Deny`
//! routing — store READ resolves approval id -> task scope, a mismatched
//! scope is a typed error, and the decision reaches that task's
//! supervisor only. Missing / late / double decisions flow through the
//! core's typed errors and `core_err` codes.
//!
//! The pending rows are seeded through the store API (the production
//! park path — supervisor-owned `park_approval` — is covered by
//! `tachyon-core`'s `approval_wait` tests); every DECISION below still goes
//! through the supervisor, which is the single logical writer.

use tachyon_gateway::start_with;
use tachyon_protocol::Command;
use tachyon_types::ApprovalId;

mod common;
use common::{armed_runtime, code_of, err, fake, new_task, ok, test_dir};

#[tokio::test]
async fn approve_routes_to_the_owning_supervisor_and_applies_the_grant() {
    let dir = test_dir();
    let gateway = start_with(&dir, armed_runtime(fake())).await.unwrap();
    let socket = gateway.address().to_owned();
    let task = new_task(&socket).await;
    let approval = ApprovalId::generate();
    gateway
        .store()
        .insert_pending(&approval.to_string(), &task, "hash-1")
        .await
        .unwrap();

    let payload = ok(
        &socket,
        Command::Approve {
            task_id: task.parse().unwrap(),
            approval_id: approval,
        },
    )
    .await;
    assert_eq!(
        payload["task"]["id"], task,
        "decision answered with the task"
    );

    let row = gateway
        .store()
        .load_by_id(&approval.to_string())
        .await
        .unwrap()
        .expect("row exists");
    assert_eq!(
        row.decision, "applied",
        "grant lands the durable applied marker before any execution"
    );

    let events = gateway.store().load_events_since(&task, -1).await.unwrap();
    assert!(
        events.iter().any(|event| event.kind == "approval"),
        "the decision is journalled by the supervisor"
    );
    gateway.shutdown().await;
}

#[tokio::test]
async fn approve_on_a_foreign_task_scope_is_a_typed_mismatch() {
    let dir = test_dir();
    let gateway = start_with(&dir, armed_runtime(fake())).await.unwrap();
    let socket = gateway.address().to_owned();
    let owner = new_task(&socket).await;
    let other = new_task(&socket).await;
    let approval = ApprovalId::generate();
    gateway
        .store()
        .insert_pending(&approval.to_string(), &owner, "hash-1")
        .await
        .unwrap();

    let got = err(
        &socket,
        Command::Approve {
            task_id: other.parse().unwrap(),
            approval_id: approval,
        },
    )
    .await;
    assert_eq!(code_of(&got), "approval_task_mismatch", "{got}");
    let row = gateway
        .store()
        .load_by_id(&approval.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.decision, "pending", "a mismatch must decide nothing");
    gateway.shutdown().await;
}

#[tokio::test]
async fn double_decide_is_a_typed_refusal() {
    let dir = test_dir();
    let gateway = start_with(&dir, armed_runtime(fake())).await.unwrap();
    let socket = gateway.address().to_owned();
    let task = new_task(&socket).await;
    let approval = ApprovalId::generate();
    gateway
        .store()
        .insert_pending(&approval.to_string(), &task, "hash-1")
        .await
        .unwrap();

    ok(
        &socket,
        Command::Approve {
            task_id: task.parse().unwrap(),
            approval_id: approval,
        },
    )
    .await;
    let got = err(
        &socket,
        Command::Approve {
            task_id: task.parse().unwrap(),
            approval_id: approval,
        },
    )
    .await;
    assert_eq!(code_of(&got), "approval_not_pending", "{got}");
    gateway.shutdown().await;
}

#[tokio::test]
async fn approve_with_an_unknown_id_is_approval_missing() {
    let dir = test_dir();
    let gateway = start_with(&dir, armed_runtime(fake())).await.unwrap();
    let socket = gateway.address().to_owned();
    let task = new_task(&socket).await;

    let got = err(
        &socket,
        Command::Approve {
            task_id: task.parse().unwrap(),
            approval_id: ApprovalId::generate(),
        },
    )
    .await;
    assert_eq!(code_of(&got), "approval_missing", "{got}");
    gateway.shutdown().await;
}

#[tokio::test]
async fn deny_records_the_reason_and_leaves_the_row_denied() {
    let dir = test_dir();
    let gateway = start_with(&dir, armed_runtime(fake())).await.unwrap();
    let socket = gateway.address().to_owned();
    let task = new_task(&socket).await;
    let approval = ApprovalId::generate();
    gateway
        .store()
        .insert_pending(&approval.to_string(), &task, "hash-1")
        .await
        .unwrap();

    ok(
        &socket,
        Command::Deny {
            task_id: task.parse().unwrap(),
            approval_id: approval,
            reason: "not this time".to_owned(),
        },
    )
    .await;
    let row = gateway
        .store()
        .load_by_id(&approval.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.decision, "denied");
    assert!(row.decided_at > 0, "denial is timestamped");

    let events = gateway.store().load_events_since(&task, -1).await.unwrap();
    let denial = events
        .iter()
        .find(|event| event.kind == "approval")
        .expect("approval event journalled");
    let payload: serde_json::Value = serde_json::from_str(&denial.payload).unwrap();
    assert_eq!(payload["v"]["granted"], false);
    assert_eq!(payload["v"]["reason"], "not this time");
    gateway.shutdown().await;
}
