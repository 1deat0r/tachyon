//! M10 prerequisite: one live writer per durable task identity.
use std::path::PathBuf;
use std::sync::Arc;
use tachyon_core::{ConstraintStrength, CoreError, SupervisorHandle, create_task, recover_task};
use tachyon_store::StoreWriter;
use tachyon_types::{SessionId, WorkspaceId};

struct Fixture {
    root: PathBuf,
    store: Arc<StoreWriter>,
    task: SupervisorHandle,
}

impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("tachyon-m10-owner-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Arc::new(StoreWriter::open(&root).await.unwrap());
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let task = create_task(
            session,
            WorkspaceId::generate(),
            "Preserve acknowledged constraints".into(),
            store.clone(),
        )
        .await
        .unwrap();
        Self { root, store, task }
    }

    async fn close(self) {
        self.task.shutdown().await.unwrap();
        self.store.close().await;
        std::fs::remove_dir_all(self.root).unwrap();
    }
}

#[tokio::test]
async fn duplicate_recovery_cannot_overwrite_acknowledged_constraints_or_status() {
    let f = Fixture::new().await;
    f.task
        .add_constraint("never write migrations".into(), ConstraintStrength::Hard)
        .await
        .unwrap();
    let before = f.task.pause().await.unwrap();
    let duplicate = recover_task(f.task.task_id(), f.store.clone()).await;
    assert!(
        matches!(duplicate, Err(CoreError::TaskAlreadyOwned(id)) if id == f.task.task_id()),
        "duplicate recovery admitted a second writer over revision {} and {:?}",
        before.revision,
        before.status
    );
    assert_eq!(before, f.task.get_state().await.unwrap());
    f.close().await;
}

#[tokio::test]
async fn awaited_shutdown_releases_owner_but_retained_clones_fail_closed() {
    let f = Fixture::new().await;
    let workspace = f.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("protected.rs"), "unchanged").unwrap();
    let context = Arc::new(tachyon_tools::ToolsContext::new(
        workspace,
        tachyon_policy::Policy::trusted_workspace(),
        tachyon_tools::artifact::ArtifactSpool::new(f.root.join("artifacts")),
    ));
    f.task
        .configure_verification(
            context,
            tachyon_verify::AcceptanceContract {
                clauses: vec![tachyon_verify::Clause::FileUnchanged {
                    path: "protected.rs".into(),
                }],
            },
            tachyon_verify::VerificationRisk::Affected,
        )
        .await
        .unwrap();
    let retained = f.task.clone();
    retained
        .add_constraint("never write migrations".into(), ConstraintStrength::Hard)
        .await
        .unwrap();
    let before = f.task.cancel().await.unwrap();
    f.task.shutdown().await.unwrap();
    retained.shutdown().await.unwrap();
    assert!(matches!(
        retained.get_state().await,
        Err(CoreError::SupervisorGone)
    ));
    assert!(matches!(
        retained.add_message("must not resurrect".into()).await,
        Err(CoreError::SupervisorGone)
    ));
    let recovered = recover_task(f.task.task_id(), f.store.clone())
        .await
        .unwrap();
    let mut after = recovered.get_state().await.unwrap();
    // Recovery updates only this timestamp; all acknowledged task truth survives.
    after.updated_at = before.updated_at;
    assert_eq!(after, before);
    assert!(matches!(
        recovered.resume().await,
        Err(CoreError::IllegalTransition { .. })
    ));
    assert!(matches!(
        recover_task(f.task.task_id(), f.store.clone()).await,
        Err(CoreError::TaskAlreadyOwned(_))
    ));
    recovered.shutdown().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn independent_stores_and_database_aliases_share_admission_before_any_read() {
    let f = Fixture::new().await;
    std::fs::create_dir_all(f.root.join("walk")).unwrap();
    #[allow(unused_mut)]
    let mut aliases = vec![f.root.clone(), f.root.join("."), f.root.join("walk/..")];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&f.root, f.root.join("directory-alias")).unwrap();
        aliases.push(f.root.join("directory-alias"));
        let file_alias = f.root.join("file-alias");
        std::fs::create_dir_all(&file_alias).unwrap();
        std::os::unix::fs::symlink(f.root.join("state.db"), file_alias.join("state.db")).unwrap();
        aliases.push(file_alias);
    }
    for alias in aliases {
        let other = Arc::new(StoreWriter::open(&alias).await.unwrap());
        assert_eq!(
            other.database_path(),
            f.root.join("state.db").canonicalize().unwrap()
        );
        assert_eq!(other.database_path(), f.store.database_path());
        assert!(!Arc::ptr_eq(&other, &f.store));
        // If recovery tries a read before admission, a closed pool returns a
        // store error instead of the required typed ownership conflict.
        other.close().await;
        assert!(matches!(
            recover_task(f.task.task_id(), other).await,
            Err(CoreError::TaskAlreadyOwned(id)) if id == f.task.task_id()
        ));
    }
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_recoveries_admit_exactly_one_writer() {
    const CONTENDERS: usize = 12;
    let f = Fixture::new().await;
    let before = f
        .task
        .add_constraint("preserve schema".into(), ConstraintStrength::Hard)
        .await
        .unwrap();
    f.task.shutdown().await.unwrap();
    let other = Arc::new(StoreWriter::open(&f.root.join(".")).await.unwrap());
    let barrier = Arc::new(tokio::sync::Barrier::new(CONTENDERS));
    let mut contenders = tokio::task::JoinSet::new();
    for index in 0..CONTENDERS {
        let store = if index % 2 == 0 {
            f.store.clone()
        } else {
            other.clone()
        };
        let barrier = barrier.clone();
        let id = f.task.task_id();
        contenders.spawn(async move {
            barrier.wait().await;
            recover_task(id, store).await
        });
    }
    let mut winners = Vec::new();
    let mut conflicts = 0;
    while let Some(result) = contenders.join_next().await {
        match result.unwrap() {
            Ok(handle) => winners.push(handle),
            Err(CoreError::TaskAlreadyOwned(id)) if id == f.task.task_id() => conflicts += 1,
            other => panic!("unexpected recovery result: {other:?}"),
        }
    }
    assert_eq!(winners.len(), 1);
    assert_eq!(conflicts, CONTENDERS - 1);
    let winner = winners.pop().unwrap();
    let mut after = winner.get_state().await.unwrap();
    after.updated_at = before.updated_at;
    assert_eq!(after, before);
    let (first, second) = tokio::join!(winner.shutdown(), winner.shutdown());
    first.unwrap();
    second.unwrap();
    other.close().await;
    f.close().await;
}

#[tokio::test]
async fn identical_task_ids_in_different_databases_have_independent_owners() {
    let f = Fixture::new().await;
    let other_dir = f.root.join("independent");
    std::fs::create_dir_all(&other_dir).unwrap();
    let other = Arc::new(StoreWriter::open(&other_dir).await.unwrap());
    let state = f.task.get_state().await.unwrap();
    other
        .create_session(&state.session_id.to_string())
        .await
        .unwrap();
    let events = f
        .store
        .load_events_since(&state.id.to_string(), -1)
        .await
        .unwrap();
    other
        .create_task(
            &state.id.to_string(),
            &state.session_id.to_string(),
            &state.workspace_id.to_string(),
            &state.objective,
            state.status.name(),
            &serde_json::to_string(&state).unwrap(),
            &events[0].payload,
        )
        .await
        .unwrap();
    let independent = recover_task(state.id, other.clone()).await.unwrap();
    independent
        .add_message("only this database".into())
        .await
        .unwrap();
    assert_eq!(f.task.get_state().await.unwrap(), state);
    independent.shutdown().await.unwrap();
    other.close().await;
    f.close().await;
}

#[tokio::test]
async fn failed_recovery_releases_its_reservation() {
    let f = Fixture::new().await;
    let id = tachyon_types::TaskId::generate();
    for _ in 0..2 {
        assert!(
            matches!(recover_task(id, f.store.clone()).await, Err(CoreError::UnknownTask(missing)) if missing == id)
        );
    }
    f.close().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn shutdown_drains_a_real_verifier_process_before_recovery() {
    use std::collections::BTreeMap;
    use std::time::Duration;
    use tokio::io::AsyncBufReadExt as _;

    let f = Fixture::new().await;
    let workspace = f.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("source.txt"), "stable").unwrap();
    let mut policy = tachyon_policy::Policy::trusted_workspace();
    policy.allow("verify.command", "workspace/**");
    policy.allow("process.spawn", "python3");
    let context = Arc::new(tachyon_tools::ToolsContext::new(
        workspace,
        policy,
        tachyon_tools::artifact::ArtifactSpool::new(f.root.join("artifacts")),
    ));
    // A loopback readiness handshake proves the actual command is alive; no
    // sleep/marker polling. Dropping the stream also unblocks it on test failure.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let command = tachyon_verify::CommandCheck {
        program: "python3".into(),
        args: vec!["-c".into(),
            "import os,socket,sys; s=socket.create_connection(('127.0.0.1',int(sys.argv[1]))); s.sendall((str(os.getpid())+'\\n').encode()); s.recv(1)".into(),
            port.to_string()],
        cwd: ".".into(),
        env: BTreeMap::new(),
        timeout_ms: 30_000,
    };
    f.task
        .configure_verification(
            context.clone(),
            tachyon_verify::AcceptanceContract {
                clauses: vec![tachyon_verify::Clause::CommandPasses { command }],
            },
            tachyon_verify::VerificationRisk::Affected,
        )
        .await
        .unwrap();
    let handle = f.task.clone();
    let mut pending = tokio::spawn(async move { handle.verify_and_complete(context).await });
    let (stream, _) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            result = &mut pending => panic!("verifier exited before readiness: {result:?}"),
            accepted = listener.accept() => accepted.unwrap(),
        }
    })
    .await
    .unwrap();
    let mut stream = tokio::io::BufReader::new(stream);
    let mut pid = String::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_line(&mut pid))
        .await
        .unwrap()
        .unwrap();
    let pid: u32 = pid.trim().parse().unwrap();
    let process = PathBuf::from(format!("/proc/{pid}"));
    assert!(process.exists());
    assert!(matches!(
        recover_task(f.task.task_id(), f.store.clone()).await,
        Err(CoreError::TaskAlreadyOwned(_))
    ));
    tokio::time::timeout(Duration::from_secs(5), f.task.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(
        !process.exists(),
        "awaited shutdown returned before child reap"
    );
    assert!(pending.await.unwrap().is_err());
    let recovered = recover_task(f.task.task_id(), f.store.clone())
        .await
        .unwrap();
    let state = recovered.get_state().await.unwrap();
    assert_eq!(state.status, tachyon_core::TaskStatus::Recovering);
    assert!(state.verification.unwrap().interrupted);
    recovered.shutdown().await.unwrap();
    f.close().await;
}
