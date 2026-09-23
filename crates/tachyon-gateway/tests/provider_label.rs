//! M11 plan G5 / closure 6: the honest provider label rides the payloads
//! the TUI reads. Contract (with the parallel TUI writer): top-level
//! optional string key `"provider_label"` in BOTH the `Subscribe`
//! acknowledgement and the `GetTask` payload — present with
//! [`FAKE_PROVIDER_LABEL`] exactly when this gateway runs `kind = "fake"`
//! (the config maps that kind to the constant), ABSENT otherwise. The
//! key name and top-level position are the wire contract.

use tachyon_gateway::{FAKE_PROVIDER_LABEL, GatewayRuntime, start_with};
use tachyon_protocol::Command;

mod common;
use common::{armed_runtime, fake, new_task, ok, test_dir};

#[tokio::test]
async fn provider_label_rides_subscribe_and_gettask_for_a_fake_runtime() {
    let dir = test_dir();
    let gateway = start_with(&dir, armed_runtime(fake())).await.unwrap();
    let socket = gateway.address().to_owned();
    let task = new_task(&socket).await;

    // GetTask: top-level optional key, present with the honest label.
    let got = ok(
        &socket,
        Command::GetTask {
            task_id: task.parse().unwrap(),
        },
    )
    .await;
    assert_eq!(
        got["provider_label"], FAKE_PROVIDER_LABEL,
        "GetTask must carry the honest provider label for kind=fake: {got}"
    );

    // Subscribe ack: the same top-level key, same value.
    let ack = ok(
        &socket,
        Command::Subscribe {
            task_id: task.parse().unwrap(),
            after_seq: -1,
        },
    )
    .await;
    assert_eq!(ack["subscribed"], true, "ack shape: {ack}");
    assert_eq!(
        ack["provider_label"], FAKE_PROVIDER_LABEL,
        "Subscribe ack must carry the honest provider label: {ack}"
    );

    gateway.shutdown().await;
}

#[tokio::test]
async fn provider_label_key_is_absent_for_a_non_fake_runtime() {
    let dir = test_dir();
    // A non-fake runtime (config kind != "fake" labels itself, e.g.
    // "openai_compat"): no provider needed — this test never starts a run.
    let runtime = GatewayRuntime {
        provider: None,
        label: "openai_compat".to_owned(),
        model: "some-model".to_owned(),
        redactor: tachyon_tools::credential::CredentialBroker::default(),
    };
    let gateway = start_with(&dir, runtime).await.unwrap();
    let socket = gateway.address().to_owned();
    let task = new_task(&socket).await;

    let got = ok(
        &socket,
        Command::GetTask {
            task_id: task.parse().unwrap(),
        },
    )
    .await;
    assert!(
        got.get("provider_label").is_none(),
        "key must be ABSENT (not null, not another label) for kind!=fake: {got}"
    );

    let ack = ok(
        &socket,
        Command::Subscribe {
            task_id: task.parse().unwrap(),
            after_seq: -1,
        },
    )
    .await;
    assert!(
        ack.get("provider_label").is_none(),
        "key must be ABSENT in the Subscribe ack too: {ack}"
    );

    gateway.shutdown().await;
}
