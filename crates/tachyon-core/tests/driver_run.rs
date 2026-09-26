//! M11 item 6 (core part): ONE shared run driver. The `bench_matrix`
//! example (M14, formerly `auth_refresh`) and every future host (gateway
//! `tachyon run`) execute the
//! evidence -> model -> patch -> verification sequence through
//! `tachyon_core::driver::drive`, which runs the worker half of the M10
//! plan §2 proposal/ack path: `start_run` first, then revision-bound
//! record proposals acknowledged by the supervisor before each stage.
//! There is no second orchestration implementation.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tachyon_core::driver::{DriveHost, EvidenceMode, RunPlan, drive};
use tachyon_core::{TaskStatus, create_task};
use tachyon_models::fake::{FakeModelProvider, FakeResponse};
use tachyon_mutation::blake3_hex;
use tachyon_policy::Policy;
use tachyon_store::StoreWriter;
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
use tachyon_types::{ProviderId, SessionId, WorkspaceId};
use tachyon_verify::{AcceptanceContract, Clause, VerificationRisk};

use tachyon_core::runtime::{EvidenceRequest, RuntimeBounds};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const TARGET: &str = "src/lib.rs";
const BROKEN: &str = "pub fn answer() -> u8 { 7 }\n";
const FIXED: &str = "pub fn answer() -> u8 { 42 }\n";

#[tokio::test]
async fn driver_runs_one_shared_path_and_journals_the_supervisor_records() {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("tachyon-driver-run-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ws = dir.join("ws");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::create_dir_all(ws.join("notes")).unwrap();
    std::fs::write(ws.join(TARGET), BROKEN).unwrap();
    std::fs::write(ws.join("notes/readme.txt"), "keep me\n").unwrap();

    let mut policy = Policy::trusted_workspace();
    policy.allow("mutation.patch", "workspace/**");
    policy.allow("fs.delete", "workspace/**");
    let context = Arc::new(ToolsContext::new(
        ws.clone(),
        policy,
        ArtifactSpool::new(dir.join("artifacts")),
    ));

    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::create_dir_all(dir.join("mutation-state")).unwrap();
    let store = Arc::new(StoreWriter::open(&dir.join("state")).await.unwrap());
    let session = SessionId::generate();
    store.create_session(&session.to_string()).await.unwrap();
    let task = create_task(
        session,
        WorkspaceId::generate(),
        "Fix the wrong answer".to_owned(),
        store.clone(),
    )
    .await
    .unwrap();
    let task_id = task.task_id();

    // Scripted test/replay provider, example-owned inputs (plan item 6).
    let provider = Arc::new(FakeModelProvider::new(ProviderId("bench-script".into())));
    let script = serde_json::json!({
        "decision": "propose_execution",
        "operations": [{
            "capability": "mutation.patch",
            "args": {
                "path": TARGET,
                "base_hash": blake3_hex(BROKEN.as_bytes()),
                "new_content": FIXED,
            }
        }]
    });
    provider.push_response(FakeResponse::respond(&script.to_string()));

    let plan = RunPlan {
        origin: Instant::now(),
        evidence_mode: EvidenceMode::Serial,
        evidence: vec![
            EvidenceRequest {
                capability: "fs.read".to_owned(),
                path: TARGET.to_owned(),
            },
            EvidenceRequest {
                capability: "fs.read".to_owned(),
                path: "notes/readme.txt".to_owned(),
            },
        ],
        contract: AcceptanceContract {
            clauses: vec![
                Clause::ChangedPathsWithin {
                    paths: vec![TARGET.to_owned()],
                },
                Clause::FileUnchanged {
                    path: "Cargo.toml".to_owned(),
                },
            ],
        },
        risk: VerificationRisk::Affected,
        mutation_dir: dir.join("mutation-state"),
        batch_id: "driver-batch-1".to_owned(),
        model: "scripted-replay-1".to_owned(),
        requested_checks: Vec::new(),
        available_checks: Vec::new(),
        bounds: RuntimeBounds::default(),
        cancel: tokio_util::sync::CancellationToken::new(),
    };

    let host = DriveHost::Supervisor {
        handle: task,
        store: store.clone(),
    };
    let outcome = drive(host, context, provider, plan).await.unwrap();

    // Durable completion through the shared supervisor path.
    assert_eq!(outcome.outcome.as_deref(), Some("completed"));
    assert_eq!(outcome.recovery.as_deref(), Some("recovered_completed"));
    let state = outcome
        .state
        .expect("supervisor runs return the recovered state");
    assert_eq!(state.status, TaskStatus::Completed);
    assert_eq!(
        outcome.task_id.as_deref(),
        Some(task_id.to_string().as_str())
    );
    assert_eq!(
        std::fs::read_to_string(ws.join(TARGET)).unwrap(),
        FIXED,
        "the scripted repair must be on disk"
    );
    assert_eq!(outcome.changed_paths, vec![TARGET.to_owned()]);
    assert!(!outcome.check_broadening);
    assert!(outcome.selected_checks.is_empty());

    // The run's journal carries the M11 vocabulary live (G5-visible kinds).
    let journal = store
        .load_events_since(&task_id.to_string(), -1)
        .await
        .unwrap();
    let kinds: Vec<&str> = journal.iter().map(|e| e.kind.as_str()).collect();
    for expected in [
        "stage",
        "evidence_summary",
        "agent_message",
        "changed_files",
        "verification_configured",
        "verification_finished",
    ] {
        assert!(kinds.contains(&expected), "missing {expected} in {kinds:?}");
    }
    assert_eq!(
        journal[0].schema_version, 1,
        "journal schema_version stays 1"
    );

    // Typed replay state: every display record landed, and the model
    // answer carries no source blob (new_content is redacted).
    assert!(state.stages.iter().any(|s| s.stage == "run"));
    assert!(state.stages.iter().any(|s| s.stage == "evidence"));
    assert!(state.stages.iter().any(|s| s.stage == "model"));
    assert!(state.stages.iter().any(|s| s.stage == "mutation"));
    assert!(state.stages.iter().any(|s| s.stage == "verify"));
    assert_eq!(state.evidence_summary.len(), 2);
    assert_eq!(state.changed_files.len(), 1);
    assert_eq!(state.agent_messages.len(), 1);
    let answer = &state.agent_messages[0];
    assert!(
        !answer.contains("pub fn answer"),
        "model answers must never journal source content: {answer}"
    );

    // Measured intervals exist for the benchmark host formulas.
    assert_eq!(outcome.node_timings.len(), 2);
    assert_eq!(outcome.intervals_us.len(), 2);

    store.close().await;
    let mut scratch = PathBuf::from(&dir);
    scratch.pop();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// G6 through the shared driver: a policy ask during the evidence stage
/// parks the supervisor-owned job (`WaitingApproval` + pending row +
/// `approval_request` event); the grant is written `applied` before the
/// re-run, the run completes under an Ask posture, and the granted
/// operation authorizes exactly once (the second use would park again —
/// one `approval_request` per file proves the one-shot grant).
#[tokio::test]
async fn driver_parks_on_evidence_ask_and_resumes_exactly_once_after_grant() {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("tachyon-driver-park-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ws = dir.join("ws");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::create_dir_all(ws.join("notes")).unwrap();
    std::fs::write(ws.join(TARGET), BROKEN).unwrap();
    std::fs::write(ws.join("notes/readme.txt"), "keep me\n").unwrap();
    std::fs::create_dir_all(ws.join("target")).unwrap();
    std::fs::write(ws.join("target/evidence-note.txt"), "probe\n").unwrap();

    // Ask posture everywhere except the mutation/verification
    // capabilities the shared driver needs; fs.read of the evidence
    // file is a genuine ask.
    let mut policy = tachyon_policy::Policy::new(tachyon_policy::DefaultPosture::Ask);
    policy.allow("mutation.patch", "workspace/**");
    policy.allow("fs.delete", "workspace/**");
    policy.allow("fs.write", "workspace/**");
    policy.allow("fs.metadata", "workspace/**");
    policy.allow("fs.list", "workspace/**");
    // fs.read is allowed for every source file the M8 engine and the
    // verification snapshot need (mutation's error type and the verify
    // snapshot both erase a typed approval request), and NOT for the
    // evidence probe under target/ — the verification snapshot skips
    // target/, so the ask belongs to the evidence stage alone, where
    // ToolError::ApprovalRequired surfaces typed and parks.
    policy.allow("fs.read", "workspace/src/**");
    policy.allow("fs.read", "workspace/notes/**");
    let context = Arc::new(ToolsContext::new(
        ws.clone(),
        policy,
        ArtifactSpool::new(dir.join("artifacts")),
    ));

    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::create_dir_all(dir.join("mutation-state")).unwrap();
    let store = Arc::new(StoreWriter::open(&dir.join("state")).await.unwrap());
    let session = SessionId::generate();
    store.create_session(&session.to_string()).await.unwrap();
    let task = create_task(
        session,
        WorkspaceId::generate(),
        "Fix under an ask posture".to_owned(),
        store.clone(),
    )
    .await
    .unwrap();
    let observed = task.clone(); // keep a second handle for mid-run G6 asserts
    let task_id = observed.task_id();

    let provider = Arc::new(FakeModelProvider::new(ProviderId("bench-script".into())));
    let script = serde_json::json!({
        "decision": "propose_execution",
        "operations": [{
            "capability": "mutation.patch",
            "args": {
                "path": TARGET,
                "base_hash": blake3_hex(BROKEN.as_bytes()),
                "new_content": FIXED,
            }
        }]
    });
    provider.push_response(FakeResponse::respond(&script.to_string()));

    let plan = RunPlan {
        origin: Instant::now(),
        evidence_mode: EvidenceMode::Serial,
        evidence: vec![
            EvidenceRequest {
                capability: "fs.read".to_owned(),
                path: TARGET.to_owned(),
            },
            EvidenceRequest {
                capability: "fs.read".to_owned(),
                path: "target/evidence-note.txt".to_owned(),
            },
        ],
        contract: AcceptanceContract {
            clauses: vec![
                Clause::ChangedPathsWithin {
                    paths: vec![TARGET.to_owned()],
                },
                Clause::FileUnchanged {
                    path: "Cargo.toml".to_owned(),
                },
            ],
        },
        risk: VerificationRisk::Affected,
        mutation_dir: dir.join("mutation-state"),
        batch_id: "driver-park-batch".to_owned(),
        model: "scripted-replay-1".to_owned(),
        requested_checks: Vec::new(),
        available_checks: Vec::new(),
        bounds: RuntimeBounds::default(),
        cancel: tokio_util::sync::CancellationToken::new(),
    };

    let host = DriveHost::Supervisor {
        handle: task,
        store: store.clone(),
    };
    let mut run = tokio::spawn(drive(host, context, provider, plan));

    // Wait for the park, assert the G6 park row, then grant it.
    let mut decided: Vec<String> = Vec::new();
    let deadline = Instant::now() + std::time::Duration::from_secs(20);
    loop {
        tokio::select! {
            joined = &mut run => {
                let outcome = joined.expect("join").expect("drive must complete after the grant");
                assert_eq!(outcome.outcome.as_deref(), Some("completed"));
                break;
            }
            () = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                assert!(Instant::now() < deadline, "drive never finished");
                for row in store.load_pending_for_task(&task_id.to_string()).await.unwrap() {
                    if decided.contains(&row.id) { continue; }
                    // While parked: WaitingApproval + pending row +
                    // approval_request event (G6 observable mid-run).
                    let state = observed.get_state().await.unwrap();
                    assert_eq!(
                        state.status,
                        TaskStatus::WaitingApproval,
                        "a pending row exists only while the job is parked"
                    );
                    assert!(!state.approval_requests.is_empty());
                    decided.push(row.id.clone());
                    let approval = state
                        .approval_requests
                        .last()
                        .expect("parked request journalled")
                        .id;
                    observed
                        .decide_approval(approval, true, "granted by test".into())
                        .await
                        .expect("grant the parked ask");
                }
            }
        }
    }

    // The granted operation ran once under one consumed grant: journal
    // shows exactly one park per asked file, rows end `applied`.
    assert_eq!(decided.len(), 1, "one file asked, one grant");
    let row = store.load_by_id(&decided[0]).await.unwrap().unwrap();
    assert_eq!(row.decision, "applied", "applied before the re-run");
    assert!(row.decided_at > 0);
    let journal = store
        .load_events_since(&task_id.to_string(), -1)
        .await
        .unwrap();
    let kinds: Vec<&str> = journal.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds.iter().filter(|k| **k == "approval_request").count(),
        1
    );
    observed.shutdown().await.unwrap();
    store.close().await;
    std::fs::remove_dir_all(&dir).unwrap();
}

/// M11 closure (typed parking, mutation stage): a policy ask surfaced
/// from the M8 authorized engine (typed `MutationError`, not a string)
/// parks the supervisor-owned job exactly like the evidence stage —
/// `WaitingApproval` + pending row + `approval_request` — until every
/// asked operation is granted; the granted operation then re-runs, the
/// batch commits, and the committed effect lands exactly once (one
/// `changed_files` receipt, one `committed` stage row) despite the
/// stage re-running per grant.
#[tokio::test]
async fn driver_parks_on_mutation_ask_and_commits_exactly_one_effect_after_grant() {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "tachyon-driver-park-mut-{}-{id}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let ws = dir.join("ws");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::create_dir_all(ws.join("notes")).unwrap();
    std::fs::write(ws.join(TARGET), BROKEN).unwrap();
    std::fs::write(ws.join("notes/readme.txt"), "keep me\n").unwrap();
    std::fs::create_dir_all(ws.join("target")).unwrap();
    std::fs::write(ws.join("target/evidence-note.txt"), "probe\n").unwrap();

    // Ask posture everywhere. Every read the evidence and verification
    // stages need is allowed (they must NOT park — this test isolates
    // the mutation stage); `mutation.patch` is intentionally not
    // allowed, so the model-proposed repair asks, typed through
    // tachyon-mutation, and parks.
    let mut policy = tachyon_policy::Policy::new(tachyon_policy::DefaultPosture::Ask);
    policy.allow("fs.delete", "workspace/**");
    policy.allow("fs.write", "workspace/**");
    policy.allow("fs.metadata", "workspace/**");
    policy.allow("fs.list", "workspace/**");
    policy.allow("fs.read", "workspace/**");
    let context = Arc::new(ToolsContext::new(
        ws.clone(),
        policy,
        ArtifactSpool::new(dir.join("artifacts")),
    ));

    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::create_dir_all(dir.join("mutation-state")).unwrap();
    let store = Arc::new(StoreWriter::open(&dir.join("state")).await.unwrap());
    let session = SessionId::generate();
    store.create_session(&session.to_string()).await.unwrap();
    let task = create_task(
        session,
        WorkspaceId::generate(),
        "Fix under an ask posture (mutation)".to_owned(),
        store.clone(),
    )
    .await
    .unwrap();
    let observed = task.clone(); // keep a second handle for mid-run G6 asserts
    let task_id = observed.task_id();

    let provider = Arc::new(FakeModelProvider::new(ProviderId("bench-script".into())));
    let script = serde_json::json!({
        "decision": "propose_execution",
        "operations": [{
            "capability": "mutation.patch",
            "args": {
                "path": TARGET,
                "base_hash": blake3_hex(BROKEN.as_bytes()),
                "new_content": FIXED,
            }
        }]
    });
    provider.push_response(FakeResponse::respond(&script.to_string()));

    let plan = RunPlan {
        origin: Instant::now(),
        evidence_mode: EvidenceMode::Serial,
        evidence: vec![
            EvidenceRequest {
                capability: "fs.read".to_owned(),
                path: TARGET.to_owned(),
            },
            EvidenceRequest {
                capability: "fs.read".to_owned(),
                path: "target/evidence-note.txt".to_owned(),
            },
        ],
        contract: AcceptanceContract {
            clauses: vec![
                Clause::ChangedPathsWithin {
                    paths: vec![TARGET.to_owned()],
                },
                Clause::FileUnchanged {
                    path: "Cargo.toml".to_owned(),
                },
            ],
        },
        risk: VerificationRisk::Affected,
        mutation_dir: dir.join("mutation-state"),
        batch_id: "driver-park-mut-batch".to_owned(),
        model: "scripted-replay-1".to_owned(),
        requested_checks: Vec::new(),
        available_checks: Vec::new(),
        bounds: RuntimeBounds::default(),
        cancel: tokio_util::sync::CancellationToken::new(),
    };

    let host = DriveHost::Supervisor {
        handle: task,
        store: store.clone(),
    };
    let mut run = tokio::spawn(drive(host, context, provider, plan));

    // Grant every parked ask; each park must be a real mid-run park.
    let mut decided: Vec<String> = Vec::new();
    let deadline = Instant::now() + std::time::Duration::from_secs(20);
    loop {
        tokio::select! {
            joined = &mut run => {
                let outcome = joined.expect("join").expect("drive must complete after every grant");
                assert_eq!(outcome.outcome.as_deref(), Some("completed"));
                break;
            }
            () = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                assert!(
                    Instant::now() < deadline,
                    "drive never finished after {} grants",
                    decided.len()
                );
                for row in store.load_pending_for_task(&task_id.to_string()).await.unwrap() {
                    if decided.contains(&row.id) { continue; }
                    let state = observed.get_state().await.unwrap();
                    assert_eq!(
                        state.status,
                        TaskStatus::WaitingApproval,
                        "a pending row exists only while the job is parked"
                    );
                    assert!(!state.approval_requests.is_empty());
                    decided.push(row.id.clone());
                    let approval = state
                        .approval_requests
                        .last()
                        .expect("parked request journalled")
                        .id;
                    observed
                        .decide_approval(approval, true, "granted by test".into())
                        .await
                        .expect("grant the parked ask");
                }
            }
        }
    }

    // Every ask parked at least once, every grant applied exactly once,
    // and — despite the stage re-running per grant — the committed
    // effect landed exactly once.
    assert!(!decided.is_empty(), "the mutation ask must park");
    for row_id in &decided {
        let row = store.load_by_id(row_id).await.unwrap().unwrap();
        assert_eq!(row.decision, "applied", "applied before the re-run");
        assert!(row.decided_at > 0);
    }
    let journal = store
        .load_events_since(&task_id.to_string(), -1)
        .await
        .unwrap();
    let kinds: Vec<&str> = journal.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds.iter().filter(|k| **k == "approval_request").count(),
        decided.len()
    );
    assert_eq!(
        kinds.iter().filter(|k| **k == "approval").count(),
        decided.len()
    );
    // The drive's own shutdown/recovery round-trip ended the original
    // supervisor, so the settled state is read through one fresh
    // recovery — also proving the completed run survives a restart.
    let settled = tachyon_core::recover_task(task_id, store.clone())
        .await
        .unwrap();
    let state = settled.get_state().await.unwrap();
    assert_eq!(
        state.changed_files.len(),
        1,
        "one granted effect, one changed-files receipt"
    );
    assert_eq!(
        state
            .stages
            .iter()
            .filter(|s| s.stage == "mutation" && s.detail.starts_with("committed"))
            .count(),
        1,
        "exactly one commit row across the park re-runs"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join(TARGET)).unwrap(),
        FIXED,
        "the granted mutation ran to completion"
    );

    settled.shutdown().await.unwrap();
    store.close().await;
    std::fs::remove_dir_all(&dir).unwrap();
}

/// M11 closure (typed parking, verification stage): the acceptance
/// command authorize asks under an Ask posture, the typed request
/// surfaces through tachyon-verify (not `VerifyError::Blocked`), the
/// job parks exactly like the evidence stage, and a recorded DENY
/// fails the run with `DriveError::ApprovalDenied` carrying the exact
/// recorded reason (G6 deny-reason leg).
#[tokio::test]
async fn driver_verification_ask_denies_with_the_recorded_reason() {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "tachyon-driver-park-ver-{}-{id}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let ws = dir.join("ws");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::create_dir_all(ws.join("notes")).unwrap();
    std::fs::write(ws.join(TARGET), BROKEN).unwrap();
    std::fs::write(ws.join("notes/readme.txt"), "keep me\n").unwrap();

    // Everything the run needs is allowed; `verify.command` is not —
    // the acceptance command asks and parks at the verification stage.
    let mut policy = tachyon_policy::Policy::new(tachyon_policy::DefaultPosture::Ask);
    policy.allow("mutation.patch", "workspace/**");
    policy.allow("fs.delete", "workspace/**");
    policy.allow("fs.write", "workspace/**");
    policy.allow("fs.metadata", "workspace/**");
    policy.allow("fs.list", "workspace/**");
    policy.allow("fs.read", "workspace/**");
    let context = Arc::new(ToolsContext::new(
        ws.clone(),
        policy,
        ArtifactSpool::new(dir.join("artifacts")),
    ));

    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::create_dir_all(dir.join("mutation-state")).unwrap();
    let store = Arc::new(StoreWriter::open(&dir.join("state")).await.unwrap());
    let session = SessionId::generate();
    store.create_session(&session.to_string()).await.unwrap();
    let task = create_task(
        session,
        WorkspaceId::generate(),
        "Fix under an ask posture (verification)".to_owned(),
        store.clone(),
    )
    .await
    .unwrap();
    let observed = task.clone();
    let task_id = observed.task_id();

    let provider = Arc::new(FakeModelProvider::new(ProviderId("bench-script".into())));
    let script = serde_json::json!({
        "decision": "propose_execution",
        "operations": [{
            "capability": "mutation.patch",
            "args": {
                "path": TARGET,
                "base_hash": blake3_hex(BROKEN.as_bytes()),
                "new_content": FIXED,
            }
        }]
    });
    provider.push_response(FakeResponse::respond(&script.to_string()));

    let plan = RunPlan {
        origin: Instant::now(),
        evidence_mode: EvidenceMode::Serial,
        evidence: vec![EvidenceRequest {
            capability: "fs.read".to_owned(),
            path: TARGET.to_owned(),
        }],
        contract: AcceptanceContract {
            clauses: vec![
                Clause::ChangedPathsWithin {
                    paths: vec![TARGET.to_owned()],
                },
                Clause::FileUnchanged {
                    path: "Cargo.toml".to_owned(),
                },
                Clause::CommandPasses {
                    command: tachyon_verify::CommandCheck {
                        program: "cargo".to_owned(),
                        args: vec!["--version".to_owned()],
                        cwd: ".".to_owned(),
                        env: std::collections::BTreeMap::new(),
                        timeout_ms: 5_000,
                    },
                },
            ],
        },
        risk: VerificationRisk::Affected,
        mutation_dir: dir.join("mutation-state"),
        batch_id: "driver-park-ver-batch".to_owned(),
        model: "scripted-replay-1".to_owned(),
        requested_checks: Vec::new(),
        available_checks: Vec::new(),
        bounds: RuntimeBounds::default(),
        cancel: tokio_util::sync::CancellationToken::new(),
    };

    let host = DriveHost::Supervisor {
        handle: task,
        store: store.clone(),
    };
    let mut run = tokio::spawn(drive(host, context, provider, plan));

    // Deny every parked ask with a recorded reason.
    let mut denied: Vec<String> = Vec::new();
    let failure: Option<String>;
    let deadline = Instant::now() + std::time::Duration::from_secs(20);
    loop {
        tokio::select! {
            joined = &mut run => {
                let error = joined
                    .expect("join")
                    .expect_err("the denied verification ask must fail the run");
                let detail = match &error {
                    tachyon_core::driver::DriveError::ApprovalDenied { reason } => {
                        assert!(
                            reason.contains("verification command refused by operator"),
                            "the recorded reason must reach the run failure: {reason}"
                        );
                        reason.clone()
                    }
                    other => panic!("expected DriveError::ApprovalDenied, got {other}"),
                };
                failure = Some(detail);
                break;
            }
            () = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                assert!(Instant::now() < deadline, "drive never parked on the verification ask");
                for row in store.load_pending_for_task(&task_id.to_string()).await.unwrap() {
                    if denied.contains(&row.id) { continue; }
                    let state = observed.get_state().await.unwrap();
                    assert_eq!(
                        state.status,
                        TaskStatus::WaitingApproval,
                        "a pending row exists only while the job is parked"
                    );
                    denied.push(row.id.clone());
                    let approval = state
                        .approval_requests
                        .last()
                        .expect("parked request journalled")
                        .id;
                    observed
                        .decide_approval(
                            approval,
                            false,
                            "verification command refused by operator".into(),
                        )
                        .await
                        .expect("deny the parked ask");
                }
            }
        }
    }

    assert!(!denied.is_empty(), "the verification ask must park");
    let reason = failure.expect("run failed with the recorded reason");
    assert_eq!(reason, "verification command refused by operator");
    let row = store.load_by_id(&denied[0]).await.unwrap().unwrap();
    assert_eq!(row.decision, "denied", "the denial is durable");

    observed.shutdown().await.unwrap();
    store.close().await;
    std::fs::remove_dir_all(&dir).unwrap();
}
