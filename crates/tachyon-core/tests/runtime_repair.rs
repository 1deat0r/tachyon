//! M10 runtime slice RED: end-to-end repair through production paths.
//! Broken source fails, runtime-gated correct repair applies via real M8
//! authorized mutation and selected M9 checks pass to durable Completed.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use tachyon_core::runtime::{
    EvidenceRequest, RuntimeBounds, bind_contract, collect_evidence, gate_proposal_writes,
    manifest_of,
};
use tachyon_core::{TaskStatus, create_task, recover_task};
use tachyon_mutation::{MutationEngine, PatchSpec, blake3_hex};
use tachyon_policy::Policy;
use tachyon_store::StoreWriter;
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
use tachyon_types::{MutationBatchId, SessionId, WorkspaceId};
use tachyon_verify::{AcceptanceContract, Clause, CommandCheck, VerificationRisk};

const BROKEN: &str =
    "pub fn answer() -> u8 { 0 }\n#[test] fn regression() { assert_eq!(answer(), 42); }\n";
const FIXED: &str =
    "pub fn answer() -> u8 { 42 }\n#[test] fn regression() { assert_eq!(answer(), 42); }\n";
const WRONG: &str =
    "pub fn answer() -> u8 { 7 }\n#[test] fn regression() { assert_eq!(answer(), 42); }\n";

struct Fixture {
    root: PathBuf,
    ws: PathBuf,
    mutation_dir: PathBuf,
    context: Arc<ToolsContext>,
    store: Arc<StoreWriter>,
}

impl Fixture {
    async fn new(broken: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("tachyon-m10-repair-{}", uuid::Uuid::now_v7()));
        let ws = root.join("ws");
        let mutation_dir = root.join("mutation-state");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::create_dir_all(&mutation_dir).unwrap();
        std::fs::write(
            ws.join("Cargo.toml"),
            "[package]\nname = \"m10-slice\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(ws.join("src/lib.rs"), broken).unwrap();
        let lock = tokio::process::Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(&ws)
            .output()
            .await
            .unwrap();
        assert!(lock.status.success(), "{lock:?}");
        let mut policy = Policy::trusted_workspace();
        policy.allow("mutation.patch", "workspace/**");
        policy.allow("fs.delete", "workspace/**");
        policy.allow("verify.command", "workspace/**");
        let context = Arc::new(ToolsContext::new(
            ws.clone(),
            policy,
            ArtifactSpool::new(root.join("artifacts")),
        ));
        std::fs::create_dir_all(root.join("state")).unwrap();
        let store = Arc::new(StoreWriter::open(&root.join("state")).await.unwrap());
        Self {
            root,
            ws,
            mutation_dir,
            context,
            store,
        }
    }

    fn engine(&self) -> MutationEngine {
        MutationEngine::open(&self.ws, &self.mutation_dir).expect("engine")
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
                        timeout_ms: 120_000,
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

    /// Evidence-bound proposal bytes for `src/lib.rs`, gated pre-mutation.
    fn gated_spec(&self, new_content: &[u8]) -> PatchSpec {
        let bounds = RuntimeBounds::default();
        let mut items = collect_evidence(
            &self.context,
            &[EvidenceRequest {
                capability: "fs.read".into(),
                path: "src/lib.rs".into(),
            }],
            &bounds,
        )
        .expect("evidence");
        // The manifest binds the supplied version; the M8 preimage binds the
        // same bytes under BLAKE3, so re-key the runtime hash to the real
        // content hash before gating (same bytes, two hash views).
        items[0].hash = blake3_hex(&items[0].bytes);
        let manifest = manifest_of(&items);
        let bound = bind_contract(Self::contract(), 0);
        let base_hash = items[0].hash.clone();
        gate_proposal_writes(
            &bound,
            &[tachyon_core::runtime::ProposedFile {
                path: "src/lib.rs".into(),
                base_hash: Some(base_hash.clone()),
                new_content: new_content.to_vec(),
            }],
            &manifest,
            &[],
        )
        .expect("gate");
        PatchSpec {
            path: "src/lib.rs".into(),
            base_hash: Some(base_hash),
            new_content: new_content.to_vec(),
        }
    }

    async fn close(self) {
        self.store.close().await;
        std::fs::remove_dir_all(self.root).unwrap();
    }
}

async fn new_task(f: &Fixture, objective: &str) -> tachyon_core::SupervisorHandle {
    let session = SessionId::generate();
    f.store.create_session(&session.to_string()).await.unwrap();
    create_task(
        session,
        WorkspaceId::generate(),
        objective.into(),
        f.store.clone(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn happy_path_repairs_via_production_supervisor() {
    let f = Fixture::new(BROKEN).await;
    // Broken regression fails before repair (stale-refresh analog: wrong value).
    let before = tokio::process::Command::new("cargo")
        .args(["test", "--offline", "--locked"])
        .current_dir(&f.ws)
        .output()
        .await
        .unwrap();
    assert!(
        !before.status.success(),
        "broken source must fail its regression"
    );
    // Runtime-gated correct repair through real M8 authorized mutation.
    let spec = f.gated_spec(FIXED.as_bytes());
    let engine = f.engine();
    let batch = MutationBatchId::generate();
    let prepared = engine
        .prepare_authorized(&f.context, batch, std::slice::from_ref(&spec))
        .expect("prepare");
    let report = engine
        .commit_authorized_up_to(&f.context, &prepared, usize::MAX)
        .expect("commit");
    assert!(
        report.completed,
        "batch receipt is not task completion authority"
    );
    assert_eq!(
        std::fs::read(f.ws.join("src/lib.rs")).unwrap(),
        FIXED.as_bytes()
    );
    // Production supervisor path decides completion from fresh M9 checks.
    let task = new_task(&f, "repair answer").await;
    task.configure_verification(
        f.context.clone(),
        Fixture::contract(),
        VerificationRisk::Affected,
    )
    .await
    .expect("bind acceptance before completion");
    let state = task
        .verify_and_complete(f.context.clone())
        .await
        .expect("verify");
    assert_eq!(
        state.status,
        TaskStatus::Completed,
        "only fresh passing checks complete"
    );
    task.shutdown().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn wrong_patch_never_completes() {
    let f = Fixture::new(BROKEN).await;
    let spec = f.gated_spec(WRONG.as_bytes());
    let engine = f.engine();
    let batch = MutationBatchId::generate();
    let prepared = engine
        .prepare_authorized(&f.context, batch, std::slice::from_ref(&spec))
        .expect("prepare");
    engine
        .commit_authorized_up_to(&f.context, &prepared, usize::MAX)
        .expect("commit");
    let task = new_task(&f, "wrong repair").await;
    task.configure_verification(
        f.context.clone(),
        Fixture::contract(),
        VerificationRisk::Affected,
    )
    .await
    .unwrap();
    let state = task.verify_and_complete(f.context.clone()).await;
    if let Ok(state) = state {
        assert_ne!(
            state.status,
            TaskStatus::Completed,
            "wrong patch refuses success"
        );
    }
    task.shutdown().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn denied_escaped_and_migration_writes_are_refused_with_zero_writes() {
    let f = Fixture::new(BROKEN).await;
    let before = std::fs::read(f.ws.join("src/lib.rs")).unwrap();
    // Denied policy scope: prepare fails, zero workspace mutation.
    let mut denied_policy = Policy::trusted_workspace();
    denied_policy.allow("mutation.patch", "workspace/**");
    denied_policy.deny("mutation.patch", "workspace/src/lib.rs");
    let denied_ctx = ToolsContext::new(
        f.ws.clone(),
        denied_policy,
        ArtifactSpool::new(f.root.join("artifacts-deny")),
    );
    let spec = PatchSpec {
        path: "src/lib.rs".into(),
        base_hash: Some(blake3_hex(&before)),
        new_content: FIXED.as_bytes().to_vec(),
    };
    let engine = f.engine();
    assert!(
        engine
            .prepare_authorized(
                &denied_ctx,
                MutationBatchId::generate(),
                std::slice::from_ref(&spec)
            )
            .is_err(),
        "denied scope performs zero writes"
    );
    assert_eq!(std::fs::read(f.ws.join("src/lib.rs")).unwrap(), before);
    // Escaped path refused by the runtime gate before mutation.
    let bound = bind_contract(Fixture::contract(), 0);
    let items = collect_evidence(
        &f.context,
        &[EvidenceRequest {
            capability: "fs.read".into(),
            path: "src/lib.rs".into(),
        }],
        &RuntimeBounds::default(),
    )
    .unwrap();
    let manifest = manifest_of(&items);
    assert!(
        gate_proposal_writes(
            &bound,
            &[tachyon_core::runtime::ProposedFile {
                path: "../escape.rs".into(),
                base_hash: Some("h".into()),
                new_content: b"x".to_vec(),
            }],
            &manifest,
            &[],
        )
        .is_err()
    );
    // Migration write refused pre-mutation.
    std::fs::create_dir_all(f.ws.join("migrations")).unwrap();
    std::fs::write(f.ws.join("migrations/001.sql"), b"-- protected").unwrap();
    assert!(
        gate_proposal_writes(
            &bound,
            &[tachyon_core::runtime::ProposedFile {
                path: "migrations/001.sql".into(),
                base_hash: Some("h".into()),
                new_content: b"-- hacked".to_vec(),
            }],
            &manifest,
            &[],
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(f.ws.join("migrations/001.sql")).unwrap(),
        b"-- protected"
    );
    // Stale (non-target-changed) evidence invalidates with zero writes.
    std::fs::write(f.ws.join("src/lib.rs"), BROKEN).unwrap();
    let stale_items = vec![tachyon_core::runtime::EvidenceItem {
        path: "src/lib.rs".into(),
        hash: "stale-hash".into(),
        bytes: BROKEN.as_bytes().to_vec(),
    }];
    let stale_manifest = manifest_of(&stale_items);
    assert!(
        gate_proposal_writes(
            &bound,
            &[tachyon_core::runtime::ProposedFile {
                path: "src/lib.rs".into(),
                base_hash: Some(blake3_hex(BROKEN.as_bytes())),
                new_content: FIXED.as_bytes().to_vec(),
            }],
            &stale_manifest,
            &[],
        )
        .is_err(),
        "base_hash must bind the supplied version"
    );
    assert_eq!(
        std::fs::read(f.ws.join("src/lib.rs")).unwrap(),
        BROKEN.as_bytes()
    );
    f.close().await;
}

#[tokio::test]
async fn duplicate_ownership_and_recovery_semantics() {
    let f = Fixture::new(BROKEN).await;
    let session = SessionId::generate();
    f.store.create_session(&session.to_string()).await.unwrap();
    let task = create_task(
        session,
        WorkspaceId::generate(),
        "owned".into(),
        f.store.clone(),
    )
    .await
    .unwrap();
    task.add_constraint(
        "only correct repair".into(),
        tachyon_core::ConstraintStrength::Hard,
    )
    .await
    .unwrap();
    let id = task.task_id();
    // Duplicate recovery while the owner lives is refused, never a stale actor.
    assert!(recover_task(id, f.store.clone()).await.is_err());
    let state = task.get_state().await.unwrap();
    assert_eq!(state.revision, 1);
    assert_eq!(state.constraints.len(), 1);
    // Only awaited shutdown permits recovery, preserving constraints/revision.
    task.shutdown().await.unwrap();
    let recovered = recover_task(id, f.store.clone())
        .await
        .expect("recover after drain");
    let state2 = recovered.get_state().await.unwrap();
    assert_eq!(state2.revision, 1, "no loss of revision");
    assert_eq!(
        state2.constraints.len(),
        1,
        "no loss of acknowledged constraints"
    );
    recovered.shutdown().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn unbound_hard_constraint_fails_before_mutation() {
    let f = Fixture::new(BROKEN).await;
    let before = std::fs::read(f.ws.join("src/lib.rs")).unwrap();
    let bound = bind_contract(Fixture::contract(), 0);
    let items = collect_evidence(
        &f.context,
        &[EvidenceRequest {
            capability: "fs.read".into(),
            path: "src/lib.rs".into(),
        }],
        &RuntimeBounds::default(),
    )
    .unwrap();
    let manifest = manifest_of(&items);
    assert!(
        gate_proposal_writes(
            &bound,
            &[tachyon_core::runtime::ProposedFile {
                path: "src/lib.rs".into(),
                base_hash: Some(items[0].hash.clone()),
                new_content: FIXED.as_bytes().to_vec(),
            }],
            &manifest,
            &[tachyon_core::runtime::HardBinding {
                id: "late-rule".into(),
                bound_check: None
            }],
        )
        .is_err()
    );
    assert_eq!(std::fs::read(f.ws.join("src/lib.rs")).unwrap(), before);
    f.close().await;
}
