//! Milestone 3 gate: trusted local tools run automatically, outside writes
//! need approval, traversal never escapes, secrets never leak.

use std::path::{Path, PathBuf};
use tachyon_policy::Policy;
use tachyon_tools::artifact::ArtifactSpool;
use tachyon_tools::{ToolError, ToolsContext, process};

fn test_context(root: &Path) -> ToolsContext {
    ToolsContext::new(
        root.to_path_buf(),
        Policy::trusted_workspace(),
        ArtifactSpool::new(root.join("artifacts")),
    )
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tachyon-m3-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn workspace_local_read_write_automatic() {
    let root = scratch("local");
    let context = test_context(&root);
    tachyon_tools::fs::write(&context, Path::new("src/main.rs"), b"fn main() {}").unwrap();
    let bytes = tachyon_tools::fs::read(&context, Path::new("src/main.rs")).unwrap();
    assert_eq!(bytes, b"fn main() {}");
    let entries = tachyon_tools::fs::list(&context, Path::new("src")).unwrap();
    assert_eq!(entries.len(), 1);
    let meta = tachyon_tools::fs::metadata(&context, Path::new("src/main.rs")).unwrap();
    assert!(meta.is_file);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn outside_workspace_write_needs_approval() {
    let root = scratch("outside");
    let mut context = test_context(&root);
    let outside = root
        .join("..")
        .join(format!("tachyon-m3-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    let target = outside.join("note.txt");
    let error = tachyon_tools::fs::write(&context, &target, b"hello").unwrap_err();
    let ToolError::ApprovalRequired { request, .. } = error else {
        panic!("expected ApprovalRequired, got {error:?}");
    };
    // Approving the exact operation lets the identical write through.
    context.approvals.decide(*request, true);
    let bytes = tachyon_tools::fs::write(&context, &target, b"hello").unwrap();
    assert_eq!(bytes, 5);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn denied_posture_refuses_outside_write() {
    let root = scratch("deny");
    let context = ToolsContext::new(
        root.clone(),
        Policy::new(tachyon_policy::DefaultPosture::Deny),
        ArtifactSpool::new(root.join("artifacts")),
    );
    let outside = std::env::temp_dir().join(format!("tachyon-m3-deny-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&outside);
    let error = tachyon_tools::fs::write(&context, &outside.join("x.txt"), b"x").unwrap_err();
    assert!(
        matches!(error, ToolError::Denied { .. }),
        "expected Denied, got {error:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn traversal_is_a_hard_error() {
    let root = scratch("traversal");
    let context = test_context(&root);
    let error = tachyon_tools::fs::read(&context, Path::new("../../etc/passwd")).unwrap_err();
    assert!(
        matches!(error, ToolError::Containment(_)),
        "expected Containment, got {error:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn symlink_escape_never_runs() {
    use std::os::unix::fs::symlink;
    let root = scratch("symlink");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    symlink("/etc", root.join("sub/evil")).unwrap();
    let context = test_context(&root);
    let error = tachyon_tools::fs::read(&context, Path::new("sub/evil/hostname")).unwrap_err();
    assert!(
        matches!(
            error,
            ToolError::Containment(_)
                | ToolError::Denied { .. }
                | ToolError::ApprovalRequired { .. }
        ),
        "escape must not read through: {error:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn trusted_build_tool_runs() {
    let root = scratch("proc");
    let context = test_context(&root);
    let mut spec = process::ProcessSpec::new("cargo");
    spec.args = vec!["--version".to_owned()];
    let receipt = process::run(&context, &spec).await.unwrap();
    assert_eq!(receipt.exit_code, Some(0));
    assert!(String::from_utf8_lossy(&receipt.stdout).contains("cargo"));
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn unknown_program_asks() {
    let root = scratch("proc-ask");
    let context = test_context(&root);
    let spec = process::ProcessSpec::new("definitely-not-a-real-binary-xyz");
    let error = process::run(&context, &spec).await.unwrap_err();
    assert!(
        matches!(error, ToolError::ApprovalRequired { .. }),
        "expected ApprovalRequired, got {error:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn git_allowlist_holds() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let context = test_context(&workspace);
    let status = tachyon_tools::git::git_read(&context, "status", Path::new("."))
        .await
        .unwrap();
    assert!(status.contains("##") || status.contains('M') || status.is_empty());
    let error = tachyon_tools::git::git_read(&context, "push", Path::new("."))
        .await
        .unwrap_err();
    assert!(
        matches!(error, ToolError::InvalidArgs(_)),
        "expected InvalidArgs, got {error:?}"
    );
}

#[test]
fn artifact_roundtrip_with_compression_boundary() {
    let root = scratch("artifact");
    let spool = ArtifactSpool::new(root.join("artifacts"));
    let small = b"tiny".to_vec();
    let id = spool.store(&small).unwrap();
    assert_eq!(spool.fetch(&id).unwrap(), small);
    // Idempotent: same bytes, same id, no rewrite needed.
    assert_eq!(spool.store(&small).unwrap(), id);
    // Large payload crosses the 64 KiB threshold and compresses.
    let large: Vec<u8> = (0..100_000)
        .map(|i: u32| u8::try_from(i % 251).unwrap())
        .collect();
    let large_id = spool.store(&large).unwrap();
    assert_eq!(spool.fetch(&large_id).unwrap(), large);
    assert_ne!(id, large_id);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn secrets_stay_behind_handles() {
    let root = scratch("cred");
    let mut context = test_context(&root);
    let secret = b"super-secret-token-123";
    let handle = context.credentials.register(secret, "github");
    assert_eq!(context.credentials.use_handle(&handle).unwrap(), secret);
    let scrubbed = context
        .credentials
        .redact("using super-secret-token-123 now");
    assert!(!scrubbed.contains("super-secret-token-123"));
    assert!(scrubbed.contains("[redacted:"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn registry_lists_native_capabilities() {
    let registry = tachyon_tools::registry::Registry::native();
    assert!(registry.len() >= 9);
    assert!(
        registry
            .get(&tachyon_types::CapabilityId("fs.read".to_owned()))
            .is_some()
    );
    assert!(
        registry
            .get(&tachyon_types::CapabilityId("nope".to_owned()))
            .is_none()
    );
}
