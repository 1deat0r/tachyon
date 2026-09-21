#[cfg(unix)]
#[test]
fn symlinks_and_nonregular_sources_fail_closed() {
    use std::os::unix::{fs::symlink, net::UnixListener};
    for name in ["link", "target"] {
        let ws = Workspace::new();
        let outside = Workspace::new();
        outside.write("private", "not in source root");
        symlink(outside.path(), ws.path().join(name)).unwrap();
        assert!(
            WorkspaceSnapshot::capture(ws.path()).is_err(),
            "followed {name}"
        );
    }
    let ws = Workspace::new();
    let _socket = UnixListener::bind(ws.path().join("socket")).unwrap();
    assert!(WorkspaceSnapshot::capture(ws.path()).is_err());
    assert!(WorkspaceSnapshot::capture(&ws.path().join("missing")).is_err());
}

#[cfg(unix)]
#[test]
fn source_mode_changes_and_lockfile_edits_are_not_build_exclusions() {
    use std::os::unix::fs::PermissionsExt;
    let ws = Workspace::new();
    ws.write("source", "same bytes");
    ws.write("Cargo.lock", "before");
    let before = WorkspaceSnapshot::capture(ws.path()).unwrap();
    let mut mode = std::fs::metadata(ws.path().join("source"))
        .unwrap()
        .permissions();
    mode.set_mode(mode.mode() ^ 0o100);
    std::fs::set_permissions(ws.path().join("source"), mode).unwrap();
    ws.write("Cargo.lock", "after");
    let after = WorkspaceSnapshot::capture(ws.path()).unwrap();
    assert_eq!(before.changed_paths(&after), vec!["Cargo.lock", "source"]);
}

mod common;
use common::Workspace;
use tachyon_verify::WorkspaceSnapshot;

#[test]
fn authorized_snapshot_refuses_an_exact_file_denial_before_reading() {
    use tachyon_policy::Policy;
    use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
    let ws = Workspace::new();
    let artifacts = Workspace::new();
    ws.write("nested/private", "must not be read");
    let mut policy = Policy::trusted_workspace();
    policy.deny("fs.read", "workspace/nested/private");
    let context = ToolsContext::new(
        ws.path().to_path_buf(),
        policy,
        ArtifactSpool::new(artifacts.path().to_path_buf()),
    );
    let error = WorkspaceSnapshot::capture_authorized(&context)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("fs.read") && error.contains("workspace/nested/private"),
        "{error}"
    );
}

#[test]
fn authorized_snapshot_refuses_an_exact_root_metadata_denial() {
    use tachyon_policy::Policy;
    use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
    let ws = Workspace::new();
    let artifacts = Workspace::new();
    ws.write("src/lib.rs", "evidence");
    let mut policy = Policy::trusted_workspace();
    policy.deny("fs.metadata", "workspace/");
    let context = ToolsContext::new(
        ws.path().to_path_buf(),
        policy,
        ArtifactSpool::new(artifacts.path().to_path_buf()),
    );
    let error = WorkspaceSnapshot::capture_authorized(&context)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("fs.metadata"),
        "root metadata must be authorized, not assumed: {error}"
    );
}

#[test]
fn snapshots_detect_real_added_deleted_and_modified_sources() {
    let ws = Workspace::new();
    ws.write("src/changed.rs", "before");
    ws.write("deleted.txt", "gone");
    ws.write(".hidden", "tracked");
    let before = WorkspaceSnapshot::capture(ws.path()).unwrap();
    ws.write("src/changed.rs", "after");
    ws.write("added.txt", "new");
    ws.write("target/debug/generated", "ignored");
    ws.write(".git/HEAD", "ignored");
    std::fs::remove_file(ws.path().join("deleted.txt")).unwrap();
    let after = WorkspaceSnapshot::capture(ws.path()).unwrap();
    assert_eq!(
        before.changed_paths(&after),
        vec!["added.txt", "deleted.txt", "src/changed.rs"]
    );
    assert!(!before.same_sources(&after));
    assert_eq!(after.root(), ws.path().canonicalize().unwrap());
    assert!(after.same_sources(&WorkspaceSnapshot::capture(ws.path()).unwrap()));
    let other = Workspace::new();
    assert!(!after.same_sources(&WorkspaceSnapshot::capture(other.path()).unwrap()));
}
