//! Process ownership regression gate. All writes stay in unique temp roots.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tachyon_policy::{ApprovalRequest, DefaultPosture, Policy};
use tachyon_tools::artifact::ArtifactSpool;
use tachyon_tools::process::{self, ProcessSpec};
use tachyon_tools::{ToolError, ToolsContext};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let root = std::env::temp_dir().join(format!(
                "tachyon-process-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&root) {
                Ok(()) => return Self(root),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create scratch directory: {error}"),
            }
        }
    }

    fn context(&self) -> ToolsContext {
        let mut policy = Policy::new(DefaultPosture::Ask);
        policy.allow("process.spawn", "/bin/sh");
        ToolsContext::new(
            self.0.clone(),
            policy,
            ArtifactSpool::new(self.0.join("artifacts")),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn shell(script: &str) -> ProcessSpec {
    let mut spec = ProcessSpec::new("/bin/sh");
    spec.args = vec!["-c".to_owned(), script.to_owned()];
    spec.timeout = std::time::Duration::from_secs(3);
    spec
}

async fn approved_context(root: &Scratch, spec: &ProcessSpec) -> (ToolsContext, ApprovalRequest) {
    let mut context = root.context();
    context.policy = Policy::new(DefaultPosture::Ask);
    let error = process::run(&context, spec).await.unwrap_err();
    let ToolError::ApprovalRequired { request, .. } = error else {
        panic!("expected approval request: {error:?}");
    };
    context.approvals.decide(*request.clone(), true);
    assert_eq!(
        process::run(&context, spec).await.unwrap().exit_code,
        Some(0)
    );
    (context, *request)
}

#[tokio::test]
async fn approval_invalidated_by_env_change() {
    let root = Scratch::new();
    let mut spec = shell(":");
    let (context, _first) = approved_context(&root, &spec).await;
    spec.env
        .insert("TACHYON_TEST".to_owned(), "changed".to_owned());
    let result = process::run(&context, &spec).await;
    assert!(
        matches!(result, Err(ToolError::ApprovalRequired { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn approval_invalidated_by_cwd_change() {
    let root = Scratch::new();
    std::fs::create_dir(root.0.join("sub")).unwrap();
    let mut spec = shell(":");
    let (context, _first) = approved_context(&root, &spec).await;
    spec.cwd = Some(PathBuf::from("sub"));
    let result = process::run(&context, &spec).await;
    assert!(
        matches!(result, Err(ToolError::ApprovalRequired { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn approval_invalidated_by_timeout_change() {
    let root = Scratch::new();
    let mut spec = shell(":");
    let (context, _first) = approved_context(&root, &spec).await;
    spec.timeout += std::time::Duration::from_nanos(1);
    let result = process::run(&context, &spec).await;
    assert!(
        matches!(result, Err(ToolError::ApprovalRequired { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn normal_and_nonzero_exit_preserve_both_streams() {
    let root = Scratch::new();
    let context = root.context();
    for code in [0, 17] {
        let receipt = process::run(
            &context,
            &shell(&format!("printf out; printf err >&2; exit {code}")),
        )
        .await
        .unwrap();
        assert_eq!(receipt.exit_code, Some(code));
        assert_eq!(receipt.stdout, b"out");
        assert_eq!(receipt.stderr, b"err");
        assert!(!receipt.timed_out);
        assert!(!receipt.stdout_truncated && !receipt.stderr_truncated);
        assert!(receipt.stdout_artifact.is_none() && receipt.stderr_artifact.is_none());
    }
}

#[tokio::test]
async fn approval_accepts_equivalent_effective_cwd_and_env() {
    let root = Scratch::new();
    let mut spec = shell(":");
    spec.env.insert("TACHYON_A".to_owned(), "one".to_owned());
    spec.env.insert("TACHYON_B".to_owned(), "two".to_owned());
    let (context, first_request) = approved_context(&root, &spec).await;
    spec.cwd = Some(root.0.join("."));
    spec.env = [
        ("TACHYON_B".to_owned(), "two".to_owned()),
        ("TACHYON_A".to_owned(), "one".to_owned()),
    ]
    .into_iter()
    .collect();
    // M11 item 8: the first grant was consumed by its one use, so the
    // equivalent operation re-asks — under a fresh approval id but the
    // IDENTICAL operation hash, which is this test's equivalence claim.
    let error = process::run(&context, &spec).await.unwrap_err();
    let ToolError::ApprovalRequired { request, .. } = error else {
        panic!("consumed grant must re-ask: {error:?}");
    };
    assert_ne!(
        request.id, first_request.id,
        "re-ask uses a fresh approval id"
    );
    assert_eq!(
        request.operation_hash, first_request.operation_hash,
        "equivalent effective cwd/env must hash identically"
    );
    // A fresh human re-grant authorizes this one execution.
    context.approvals.decide(*request, true);
    assert_eq!(
        process::run(&context, &spec).await.unwrap().exit_code,
        Some(0)
    );
}

#[tokio::test]
async fn approval_invalidated_when_default_workspace_changes() {
    let root = Scratch::new();
    let other = Scratch::new();
    let spec = shell(":");
    let (mut context, _first) = approved_context(&root, &spec).await;
    context.workspace_root.clone_from(&other.0);
    assert!(matches!(
        process::run(&context, &spec).await,
        Err(ToolError::ApprovalRequired { .. })
    ));
}

#[tokio::test]
async fn symlink_cwd_escape_is_rejected_before_approval() {
    let root = Scratch::new();
    let foreign = Scratch::new();
    std::os::unix::fs::symlink(&foreign.0, root.0.join("escape")).unwrap();
    let mut context = root.context();
    context.policy = Policy::new(DefaultPosture::Ask);
    let mut spec = shell("printf escaped > spawned");
    spec.cwd = Some(PathBuf::from("escape"));
    let result = process::run(&context, &spec).await;
    assert!(
        matches!(result, Err(ToolError::Containment(_))),
        "{result:?}"
    );
    assert!(!foreign.0.join("spawned").exists());
}

#[tokio::test]
async fn oversized_artifacts_are_redacted_before_persistence() {
    let root = Scratch::new();
    let mut context = root.context();
    let secret = b"synthetic-fixture-secret";
    let binary_secret = b"\xff\x00fixture-binary\xfe";
    context.credentials.register(secret, "fixture");
    context
        .credentials
        .register(binary_secret, "binary-fixture");
    let mut payload = vec![b'x'; process::INLINE_CAP - 7];
    payload.extend_from_slice(secret); // Crosses the inline boundary.
    payload.extend_from_slice(binary_secret);
    std::fs::write(root.0.join("payload"), &payload).unwrap();
    let receipt = process::run(&context, &shell("cat payload; cat payload >&2"))
        .await
        .unwrap();
    let expected = context.credentials.redact_bytes(&payload);
    assert!(receipt.stdout_truncated && receipt.stderr_truncated);
    for id in [receipt.stdout_artifact, receipt.stderr_artifact] {
        assert!(
            context.artifacts.fetch(&id.unwrap()).unwrap() == expected,
            "artifact must contain the fully redacted stream"
        );
    }
    assert_eq!(receipt.stdout, expected[..process::INLINE_CAP]);
    assert_eq!(receipt.stderr, expected[..process::INLINE_CAP]);
}

#[tokio::test]
async fn foreign_cwd_is_rejected_before_spawn() {
    let root = Scratch::new();
    let foreign = Scratch::new();
    let context = root.context();
    let mut spec = shell("printf escaped > spawned");
    spec.cwd = Some(foreign.0.clone());
    let result = process::run(&context, &spec).await;
    assert!(
        matches!(result, Err(ToolError::Containment(_))),
        "{result:?}"
    );
    assert!(!foreign.0.join("spawned").exists());
}

#[tokio::test]
async fn default_cwd_is_workspace_not_harness() {
    let root = Scratch::new();
    let context = root.context();
    let receipt = process::run(&context, &shell("pwd -P")).await.unwrap();
    assert_eq!(receipt.exit_code, Some(0));
    assert_eq!(
        Path::new(String::from_utf8(receipt.stdout).unwrap().trim()),
        std::fs::canonicalize(&root.0).unwrap()
    );
}
