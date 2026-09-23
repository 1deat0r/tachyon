//! M9: the real supervisor, journal, mutation engine and command verifier.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use tachyon_core::{TaskStatus, create_task, recover_task};
use tachyon_policy::Policy;
use tachyon_store::StoreWriter;
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
use tachyon_types::{SessionId, WorkspaceId};
use tachyon_verify::{AcceptanceContract, Clause, CommandCheck, VerificationRisk};

struct Fixture {
    root: PathBuf,
    context: Arc<ToolsContext>,
    store: Arc<StoreWriter>,
    task: tachyon_core::SupervisorHandle,
}

impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("tachyon-m9-core-{}", uuid::Uuid::now_v7()));
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(
            ws.join("Cargo.toml"),
            "[package]\nname = \"m9-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(ws.join("src/lib.rs"), source(0)).unwrap();
        let lockfile = tokio::process::Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(&ws)
            .output()
            .await
            .unwrap();
        assert!(lockfile.status.success(), "{lockfile:?}");
        let mut policy = Policy::trusted_workspace();
        policy.allow("verify.command", "workspace/**");
        let context = Arc::new(ToolsContext::new(
            ws,
            policy,
            ArtifactSpool::new(root.join("artifacts")),
        ));
        std::fs::create_dir_all(root.join("state")).unwrap();
        let store = Arc::new(StoreWriter::open(&root.join("state")).await.unwrap());
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let task = create_task(
            session,
            WorkspaceId::generate(),
            "Fix the incorrect answer".into(),
            store.clone(),
        )
        .await
        .unwrap();
        Self {
            root,
            context,
            store,
            task,
        }
    }

    fn contract() -> AcceptanceContract {
        AcceptanceContract {
            clauses: vec![
                Clause::CommandPasses {
                    command: CommandCheck {
                        program: "cargo".into(),
                        args: vec!["test".into(), "--offline".into(), "--locked".into()],
                        cwd: ".".into(),
                        env: BTreeMap::new(),
                        timeout_ms: 60_000,
                    },
                },
                Clause::ChangedPathsWithin {
                    paths: vec!["src/lib.rs".into()],
                },
                Clause::FileUnchanged {
                    path: "Cargo.toml".into(),
                },
            ],
        }
    }

    async fn close(self) {
        self.task.shutdown().await.unwrap();
        self.store.close().await;
        std::fs::remove_dir_all(self.root).unwrap();
    }
}

fn source(answer: u8) -> String {
    format!(
        "pub fn answer() -> u8 {{ {answer} }}\n#[test] fn regression() {{ assert_eq!(answer(), 42); }}\n"
    )
}

#[tokio::test]
async fn denied_source_cannot_enter_the_supervisor_baseline() {
    let mut f = Fixture::new().await;
    Arc::get_mut(&mut f.context)
        .unwrap()
        .policy
        .deny("fs.read", "workspace/src/lib.rs");
    assert!(
        f.task
            .configure_verification(
                f.context.clone(),
                Fixture::contract(),
                VerificationRisk::Affected,
            )
            .await
            .is_err(),
        "the supervisor must not persist evidence from a denied source"
    );
    assert!(f.task.get_state().await.unwrap().verification.is_none());
    f.close().await;
}

#[tokio::test]
async fn absent_acceptance_and_contract_replacement_fail_closed() {
    let f = Fixture::new().await;
    assert!(f.task.verify_and_complete(f.context.clone()).await.is_err());
    f.task
        .configure_verification(
            f.context.clone(),
            Fixture::contract(),
            VerificationRisk::Affected,
        )
        .await
        .unwrap();
    let before = f.task.get_state().await.unwrap();
    assert!(
        f.task
            .configure_verification(
                f.context.clone(),
                AcceptanceContract {
                    clauses: vec![Clause::ChangedPathsWithin {
                        paths: vec!["src".into()]
                    }],
                },
                VerificationRisk::Affected
            )
            .await
            .is_err()
    );
    assert_eq!(
        before.acceptance,
        f.task.get_state().await.unwrap().acceptance
    );
    f.close().await;
}

#[tokio::test]
async fn new_hard_constraint_cannot_be_ignored_by_a_passing_command() {
    let f = Fixture::new().await;
    f.task
        .configure_verification(
            f.context.clone(),
            Fixture::contract(),
            VerificationRisk::Affected,
        )
        .await
        .unwrap();
    std::fs::write(f.context.workspace_root.join("src/lib.rs"), source(42)).unwrap();
    f.task
        .add_constraint(
            "Do not alter authentication semantics".into(),
            tachyon_core::ConstraintStrength::Hard,
        )
        .await
        .unwrap();
    assert!(f.task.verify_and_complete(f.context.clone()).await.is_err());
    assert_ne!(
        f.task.get_state().await.unwrap().status,
        TaskStatus::Completed
    );
    f.close().await;
}

#[tokio::test]
async fn interrupted_journal_tail_is_not_replayed_as_a_fresh_verifier() {
    let f = Fixture::new().await;
    f.task
        .configure_verification(
            f.context.clone(),
            Fixture::contract(),
            VerificationRisk::Affected,
        )
        .await
        .unwrap();
    let id = f.task.task_id();
    f.task.shutdown().await.unwrap();
    // Inject the durable point just before executor launch, with old row metadata.
    // No live worker is started by this fixture.
    f.store.append_event(&id.to_string(), "verification_started", &serde_json::json!({
        "t": "VerificationStarted", "v": {"graph": tachyon_ir::ExecutionGraph::empty(id, 1)},
    }).to_string()).await.unwrap();
    let recovered = recover_task(id, f.store.clone()).await.unwrap();
    let state = recovered.get_state().await.unwrap();
    assert_eq!(state.status, TaskStatus::Recovering);
    assert!(state.verification.unwrap().interrupted);
    assert!(
        recovered
            .verify_and_complete(f.context.clone())
            .await
            .is_err()
    );
    assert!(!f.context.workspace_root.join("target").exists());
    recovered.shutdown().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn steering_during_real_verification_stops_work_and_rejects_late_success() {
    let f = Fixture::new().await;
    f.task
        .configure_verification(
            f.context.clone(),
            Fixture::contract(),
            VerificationRisk::Affected,
        )
        .await
        .unwrap();
    std::fs::write(f.context.workspace_root.join("src/lib.rs"),
        "#[test] fn slow() { std::fs::write(\"target/started\", b\"ready\").unwrap(); std::thread::sleep(std::time::Duration::from_secs(30)); }\n").unwrap();
    let task = f.task.clone();
    let context = f.context.clone();
    let waiting = tokio::spawn(async move { task.verify_and_complete(context).await });
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while !f.context.workspace_root.join("target/started").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("real verifier test started");
    let steered = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        f.task.add_message("Stop this approach".into()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(steered.revision, 2);
    assert!(steered.verification.unwrap().interrupted);
    assert!(waiting.await.unwrap().is_err());
    assert_ne!(
        f.task.get_state().await.unwrap().status,
        TaskStatus::Completed
    );
    f.close().await;
}

#[tokio::test]
async fn wrong_patch_refuses_completed_then_correct_patch_completes_durably() {
    let f = Fixture::new().await;
    f.task
        .configure_verification(
            f.context.clone(),
            Fixture::contract(),
            VerificationRisk::Affected,
        )
        .await
        .unwrap();
    let engine =
        tachyon_mutation::MutationEngine::open(&f.context.workspace_root, &f.root.join("mutation"))
            .unwrap();
    for answer in [41, 42] {
        // A model proposal supplies bytes, never a completion verdict.
        let proposal = tachyon_models::AgentDecision::ProposeExecution {
            operations: vec![tachyon_models::ProposedOperation {
                capability: tachyon_types::CapabilityId("mutation.patch".into()),
                args: serde_json::json!({"path":"src/lib.rs", "new_content": source(answer)}),
                reason: "candidate repair".into(),
            }],
        };
        let tachyon_models::AgentDecision::ProposeExecution { operations } = proposal else {
            unreachable!()
        };
        let op = &operations[0];
        assert_eq!(op.capability.0, "mutation.patch");
        let old = std::fs::read(f.context.workspace_root.join("src/lib.rs")).unwrap();
        let prepared = engine
            .prepare(&[tachyon_mutation::PatchSpec {
                path: op.args["path"].as_str().unwrap().into(),
                base_hash: Some(tachyon_mutation::blake3_hex(&old)),
                new_content: op.args["new_content"].as_str().unwrap().as_bytes().to_vec(),
            }])
            .unwrap();
        assert!(engine.commit(&prepared).unwrap().completed);
        let result = f.task.verify_and_complete(f.context.clone()).await;
        let state = f.task.get_state().await.unwrap();
        let report = state
            .verification
            .as_ref()
            .unwrap()
            .report
            .as_ref()
            .unwrap();
        // The journal round-trip retains evidence, not reusable authority.
        assert!(!report.passed());
        if answer == 41 {
            assert!(result.is_err(), "failed cargo test must refuse completion");
            assert_ne!(state.status, TaskStatus::Completed);
            assert!(
                report
                    .checks()
                    .iter()
                    .any(|check| { check.diagnostic().contains("command exited Some(101)") }),
                "the real regression test must fail: {report:?}"
            );
            assert!(
                !report
                    .failures()
                    .iter()
                    .any(|failure| failure.contains("source drift"))
            );
        } else {
            assert_eq!(result.unwrap().status, TaskStatus::Completed);
            assert!(report.failures().is_empty());
            assert!(!report.checks().is_empty());
            assert!(
                report
                    .checks()
                    .iter()
                    .all(|check| { check.status() == tachyon_ir::NodeStatus::Succeeded })
            );
        }
    }
    let id = f.task.task_id();
    let row = f.store.load_task(&id.to_string()).await.unwrap().unwrap();
    assert_eq!(row.status, "Completed");
    f.task.shutdown().await.unwrap();
    let recovered = recover_task(id, f.store.clone()).await.unwrap();
    assert_eq!(
        recovered.get_state().await.unwrap().status,
        TaskStatus::Completed
    );
    assert!(
        recovered
            .add_message("model says redo".into())
            .await
            .is_err()
    );
    recovered.shutdown().await.unwrap();
    f.close().await;
}
