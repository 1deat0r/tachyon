//! Shared cross-run exclusion: canonical identity and actual-worker lifetime.
use std::path::PathBuf;
use std::time::Duration;
use tachyon_tools::{ToolError, workspace::WorkspaceLease};
use tokio_util::sync::CancellationToken;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tachyon-workspace-lease-{}",
            tachyon_types::WorkspaceId::generate()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn same_workspace_waits_until_every_lease_owner_releases() {
    let f = Fixture::new();
    let token = CancellationToken::new();
    let lease = WorkspaceLease::acquire(&f.0, &token).await.unwrap();
    assert_eq!(lease.root(), f.0.canonicalize().unwrap());
    let retained = lease.clone();
    let root = f.0.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let mut waiter = tokio::spawn(async move {
        started.send(()).unwrap();
        WorkspaceLease::acquire(&root, &CancellationToken::new()).await
    });
    ready.await.unwrap();
    drop(lease);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), &mut waiter)
            .await
            .is_err()
    );
    drop(retained);
    let next = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(next.root(), f.0.canonicalize().unwrap());
}

#[tokio::test]
async fn independent_workspaces_do_not_serialize() {
    let first = Fixture::new();
    let second = Fixture::new();
    let token = CancellationToken::new();
    let _first = WorkspaceLease::acquire(&first.0, &token).await.unwrap();
    let next = tokio::time::timeout(
        Duration::from_secs(2),
        WorkspaceLease::acquire(&second.0, &token),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(next.root(), second.0.canonicalize().unwrap());
}

#[tokio::test]
async fn cancelled_waiter_never_acquires_or_blocks_a_later_owner() {
    let f = Fixture::new();
    let lease = WorkspaceLease::acquire(&f.0, &CancellationToken::new())
        .await
        .unwrap();
    let cancelled = CancellationToken::new();
    let child = cancelled.clone();
    let root = f.0.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let waiter = tokio::spawn(async move {
        started.send(()).unwrap();
        WorkspaceLease::acquire(&root, &child).await
    });
    ready.await.unwrap();
    cancelled.cancel();
    let result = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(ToolError::WorkspaceLeaseCancelled)));
    drop(lease);
    assert!(
        WorkspaceLease::acquire(&f.0, &CancellationToken::new())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn already_cancelled_request_cannot_acquire_an_idle_workspace() {
    let f = Fixture::new();
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        WorkspaceLease::acquire(&f.0, &token).await,
        Err(ToolError::WorkspaceLeaseCancelled)
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn canonical_aliases_share_the_same_lease() {
    let f = Fixture::new();
    let root = f.0.join("workspace");
    let alias = f.0.join("alias");
    std::fs::create_dir(&root).unwrap();
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    let lease = WorkspaceLease::acquire(&root, &CancellationToken::new())
        .await
        .unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let mut waiter = tokio::spawn(async move {
        started.send(()).unwrap();
        WorkspaceLease::acquire(&alias, &CancellationToken::new()).await
    });
    ready.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(40), &mut waiter)
            .await
            .is_err()
    );
    drop(lease);
    let next = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(next.root(), root.canonicalize().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborting_outer_future_does_not_release_a_blocking_effect_workers_lease() {
    let f = Fixture::new();
    let lease = WorkspaceLease::acquire(&f.0, &CancellationToken::new())
        .await
        .unwrap();
    let worker_lease = lease.clone();
    let (release, until_release) = std::sync::mpsc::channel();
    let (entered, running) = tokio::sync::oneshot::channel();
    let (done, completed) = tokio::sync::oneshot::channel();
    let actual_worker = tokio::task::spawn_blocking(move || {
        let owned = worker_lease;
        entered.send(()).unwrap();
        let _ = until_release.recv();
        drop(owned);
        let _ = done.send(());
    });
    let proxy = tokio::spawn(async move { actual_worker.await.unwrap() });
    running.await.unwrap();
    proxy.abort();
    assert!(proxy.await.unwrap_err().is_cancelled());
    drop(lease);
    let root = f.0.clone();
    let mut waiter =
        tokio::spawn(
            async move { WorkspaceLease::acquire(&root, &CancellationToken::new()).await },
        );
    let blocked = tokio::time::timeout(Duration::from_millis(40), &mut waiter)
        .await
        .is_err();
    // Always release the real worker before asserting, including negative control.
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), completed)
        .await
        .unwrap()
        .unwrap();
    assert!(
        blocked,
        "outer abort released the lease before actual worker drain"
    );
    tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
