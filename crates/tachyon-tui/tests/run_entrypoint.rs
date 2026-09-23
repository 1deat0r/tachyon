//! Plan item 2 public entrypoint: `run` must fail fast on a dead
//! endpoint — connecting the command connection happens **before** any
//! terminal mutation, so no TTY is required for this path to be honest.

use std::path::PathBuf;

#[tokio::test]
async fn run_fails_fast_when_the_gateway_endpoint_does_not_exist() {
    let dir = std::env::temp_dir().join(format!("tachyon-m11-tui-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let address: PathBuf = dir.join("never.sock");

    let result = tachyon_tui::run(&address, tachyon_tui::AttachOptions { task_id: None }).await;

    assert!(
        result.is_err(),
        "a dead endpoint must surface as Err, never a hang or a terminal takeover"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
