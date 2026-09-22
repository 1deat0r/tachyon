//! M10 benchmark slice: thin runtime host driving the auth-refresh fixture.
//!
//! This example contains no agent decision logic. It constructs trusted
//! inputs (fixture copy, policy, acceptance contract) and a scripted
//! test/replay provider, then runs the production path:
//! evidence -> model -> patch -> verification. Scripted responses prove
//! runtime integration and verification, never model reasoning quality.
//!
//! Modes: `full` (concurrent evidence, supervisor path), `serial`
//! (sequential evidence, supervisor path, records concurrency 1),
//! `reference` (same provider/operations/acceptance, serial control loop,
//! NOT the supervisor path). Any other mode reports `unimplemented`.
//! No-speculation/no-judgment configurations coincide with `full` in this
//! slice: no speculative or Jev stage exists, so no difference is reported.
//!
//! ORCHESTRATION NOTE: this host is a single-attempt scripted driver, not a
//! second orchestration implementation. It calls the trusted start/gate
//! helpers and the M8 engine directly for one scripted repair, with the
//! supervisor owning lifecycle, verification and completion. There is no
//! competing writer: the process-wide ownership guard admits exactly one
//! supervisor per (`state-database path`, `TaskId`), and the run holds no second
//! actor. A future multi-attempt runtime must route proposals through the
//! supervisor actor path instead of extending this driver.
//!
//! JSON report goes to stdout; diagnostics go to stderr.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use tachyon_core::runtime::{
    EvidenceRequest, ModelProposal, MutationIntent, NodeTiming, ProposedFile, RuntimeBounds,
    SelectionResolution, bind_contract, collect_evidence, compile_evidence_graph,
    evidence_concurrency, gate_proposal_writes, manifest_of, max_overlap, parse_proposal,
    persist_intent, resolve_check_selection,
};
use tachyon_core::{TaskStatus, create_task, recover_task};
use tachyon_models::fake::{FakeModelProvider, FakeResponse};
use tachyon_models::{AgentDecision, ModelProvider, ModelRequest, Role, UsageProvenance};
use tachyon_mutation::{MutationEngine, PatchSpec, blake3_hex};
use tachyon_policy::Policy;
use tachyon_store::StoreWriter;
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
use tachyon_types::{MutationBatchId, ProviderId, SessionId, WorkspaceId};
use tachyon_verify::{AcceptanceContract, Clause, CommandCheck, VerificationRisk};

const TARGET: &str = "auth-session/src/session.rs";
const EVIDENCE_PATHS: &[&str] = &[
    "auth-session/src/session.rs",
    "auth-session/src/reference.rs",
    "client/src/lib.rs",
    "auth-session/tests/refresh.rs",
    "logs/refresh.log",
];
const PROTECTED: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "auth-session/Cargo.toml",
    "auth-session/src/lib.rs",
    "auth-session/src/reference.rs",
    "auth-session/tests/refresh.rs",
    "client/Cargo.toml",
    "client/src/lib.rs",
    "migrations/001_initial.sql",
    "logs/refresh.log",
];
const BROKEN_BODY: &str = "    pub fn complete_refresh(&mut self, ticket: RefreshTicket, token: impl Into<String>) {\n        self.active_generation = ticket.generation;\n        self.token = token.into();\n    }";
const FIXED_BODY: &str = "    pub fn complete_refresh(&mut self, ticket: RefreshTicket, token: impl Into<String>) {\n        if ticket.generation > self.active_generation {\n            self.active_generation = ticket.generation;\n            self.token = token.into();\n        }\n    }";

fn contract() -> AcceptanceContract {
    AcceptanceContract {
        clauses: vec![
            Clause::CommandPasses {
                command: CommandCheck {
                    program: "cargo".into(),
                    args: vec!["test".into(), "--offline".into(), "--locked".into()],
                    cwd: ".".into(),
                    env: BTreeMap::new(),
                    timeout_ms: 180_000,
                },
            },
            Clause::ChangedPathsWithin {
                paths: vec![TARGET.into()],
            },
            Clause::FileUnchanged {
                path: "Cargo.toml".into(),
            },
            Clause::FileUnchanged {
                path: "Cargo.lock".into(),
            },
            Clause::FileUnchanged {
                path: "auth-session/src/reference.rs".into(),
            },
            Clause::FileUnchanged {
                path: "auth-session/tests/refresh.rs".into(),
            },
            Clause::FileUnchanged {
                path: "migrations/001_initial.sql".into(),
            },
        ],
    }
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

fn snapshot_protected(ws: &Path) -> BTreeMap<String, Vec<u8>> {
    PROTECTED
        .iter()
        .map(|rel| {
            (
                rel.to_string(),
                std::fs::read(ws.join(rel)).unwrap_or_default(),
            )
        })
        .collect()
}

async fn cargo_test(ws: &Path, target_dir: &Path) -> bool {
    let out = tokio::process::Command::new("cargo")
        .args(["test", "--offline", "--locked"])
        .current_dir(ws)
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .await;
    match out {
        Ok(o) => o.status.success(),
        Err(err) => {
            eprintln!("cargo test could not start: {err}");
            false
        }
    }
}

#[tokio::main]
async fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "full".into());
    if !matches!(mode.as_str(), "full" | "serial" | "reference") {
        println!(
            "{}",
            serde_json::json!({"mode": mode, "outcome": "unimplemented",
                "note": "only full/serial/reference are implemented in this slice"})
        );
        return;
    }
    match run(&mode).await {
        Ok(report) => println!("{report}"),
        Err(err) => {
            println!(
                "{}",
                serde_json::json!({"mode": mode, "outcome": "error", "error": err})
            );
            std::process::exit(1);
        }
    }
}

async fn run(mode: &str) -> Result<serde_json::Value, String> {
    let t0 = Instant::now();
    let ms = |t: Instant| u64::try_from(t.duration_since(t0).as_millis()).unwrap_or(u64::MAX);
    let us = |t: Instant| u64::try_from(t.duration_since(t0).as_micros()).unwrap_or(u64::MAX);
    let bounds = RuntimeBounds::default();

    // Fresh scratch fixture copy; the checked-in fixture is never patched.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_src = manifest_dir.join("../../fixtures/auth-refresh");
    let scratch =
        std::env::temp_dir().join(format!("tachyon-m10-bench-{mode}-{}", uuid::Uuid::now_v7()));
    let ws = scratch.join("ws");
    copy_dir(&fixture_src, &ws).map_err(|e| format!("fixture copy: {e}"))?;
    let before = snapshot_protected(&ws);
    eprintln!("scratch workspace: {}", ws.display());

    let mut policy = Policy::trusted_workspace();
    policy.allow("fs.read", "workspace/**");
    policy.allow("mutation.patch", "workspace/**");
    policy.allow("fs.delete", "workspace/**");
    policy.allow("verify.command", "workspace/**");
    let context = Arc::new(ToolsContext::new(
        ws.clone(),
        policy,
        ArtifactSpool::new(scratch.join("artifacts")),
    ));
    let mutation_dir = scratch.join("mutation-state");
    std::fs::create_dir_all(&mutation_dir).map_err(|e| format!("mutation dir: {e}"))?;

    // Broken regression must fail first (stale-refresh behavior, not setup).
    let fixture_target = scratch.join("target");
    let broken_failed = !cargo_test(&ws, &fixture_target).await;
    if !broken_failed {
        return Err("checked-in fixture unexpectedly passes; cannot benchmark a repair".into());
    }
    eprintln!("broken-first check: regression fails as expected");

    // Evidence through the production collector. Full mode runs one
    // single-request collection per file concurrently behind a barrier so
    // the overlap below is real measured concurrency; serial mode reads
    // sequentially and records 1.
    let requests: Vec<EvidenceRequest> = EVIDENCE_PATHS
        .iter()
        .map(|p| EvidenceRequest {
            capability: "fs.read".into(),
            path: p.to_string(),
        })
        .collect();
    let mut items = Vec::new();
    let mut intervals_us: Vec<(u64, u64)> = Vec::new();
    let mut timings: Vec<NodeTiming> = Vec::new();
    if mode == "full" {
        let barrier = Arc::new(tokio::sync::Barrier::new(requests.len() + 1));
        let mut handles = Vec::new();
        for req in &requests {
            let ctx = context.clone();
            let b = bounds;
            let r = req.clone();
            let gate = barrier.clone();
            handles.push(tokio::spawn(async move {
                gate.wait().await;
                let s = Instant::now();
                let out = collect_evidence(&ctx, std::slice::from_ref(&r), &b);
                let e = Instant::now();
                (r.path, out, s, e)
            }));
        }
        barrier.wait().await;
        for h in handles {
            let (path, out, s, e) = h.await.map_err(|e| format!("evidence join: {e}"))?;
            let mut got = out.map_err(|e| format!("collect_evidence {path}: {e}"))?;
            assert_eq!(got.len(), 1);
            items.push(got.pop().unwrap());
            intervals_us.push((us(s), us(e)));
            timings.push(NodeTiming {
                node: format!("fs.read:{path}"),
                start_ms: ms(s),
                end_ms: ms(e),
            });
        }
    } else {
        for req in &requests {
            let s = Instant::now();
            let mut got = collect_evidence(&context, std::slice::from_ref(req), &bounds)
                .map_err(|e| format!("collect_evidence {}: {e}", req.path))?;
            let e = Instant::now();
            items.push(got.pop().unwrap());
            intervals_us.push((us(s), us(e)));
            timings.push(NodeTiming {
                node: format!("fs.read:{}", req.path),
                start_ms: ms(s),
                end_ms: ms(e),
            });
        }
    }
    items.sort_by(|a, b| a.path.cmp(&b.path));
    let first_evidence_ms = timings.iter().map(|t| t.end_ms).min();
    let max_concurrency = if mode == "serial" {
        usize::from(!timings.is_empty())
    } else {
        max_overlap(&intervals_us)
    };
    // Same helper the unit tests pin; report a mismatch instead of hiding it.
    let helper_check = evidence_concurrency(&timings);
    eprintln!("max_overlap(us)={max_concurrency} evidence_concurrency(ms)={helper_check}");

    // Re-key the runtime hash to the authoritative M8 content hash (same
    // bytes, two hash views) so the gate binds the supplied version.
    for item in &mut items {
        item.hash = blake3_hex(&item.bytes);
    }
    let manifest = manifest_of(&items);
    let Some(target) = items.iter().find(|i| i.path == TARGET) else {
        return Err("target not among evidence".into());
    };
    let broken_bytes = target.bytes.clone();
    let base_hash = target.hash.clone();

    // Scripted test/replay provider: a fixed transformation of the supplied
    // evidence (broken body -> guarded body), served through a real
    // ModelProvider so the call count and usage provenance are measured.
    let broken_text =
        String::from_utf8(broken_bytes.clone()).map_err(|e| format!("fixture utf8: {e}"))?;
    if !broken_text.contains(BROKEN_BODY) {
        return Err("fixture does not contain the known stale-refresh body".into());
    }
    let fixed_text = broken_text.replacen(BROKEN_BODY, FIXED_BODY, 1);
    let provider = FakeModelProvider::new(ProviderId("bench-script".into()));
    let script = serde_json::json!({
        "decision": "propose_execution",
        "operations": [{
            "capability": "mutation.patch",
            "args": {
                "path": TARGET,
                "base_hash": base_hash,
                "new_content": fixed_text,
            }
        }]
    });
    provider.push_response(FakeResponse::respond(&script.to_string()));
    let (sink, _events) = tokio::sync::mpsc::unbounded_channel();
    let request = ModelRequest {
        role: Role::Primary,
        model: "scripted-replay-1".into(),
        context: Vec::new(),
        max_output_tokens: 1024,
        require_structured_output: false,
    };
    let result = provider
        .invoke(request, sink)
        .await
        .map_err(|e| format!("scripted provider: {e}"))?;
    let model_calls = provider.request_count() as u64;
    let usage_provenance = match result.usage.provenance {
        UsageProvenance::ProviderReported => "provider_reported",
        UsageProvenance::Scripted => "scripted",
        UsageProvenance::Unknown => "unknown",
    };
    let AgentDecision::Respond { message } = result.decision else {
        return Err("script must return its JSON as a Respond message".into());
    };
    let proposal_value: serde_json::Value =
        serde_json::from_str(&message).map_err(|e| format!("script json: {e}"))?;
    let proposal =
        parse_proposal(&proposal_value, &bounds).map_err(|e| format!("parse_proposal: {e}"))?;
    let ModelProposal::Patch { files } = proposal else {
        return Err("script must propose a patch".into());
    };
    let files: Vec<ProposedFile> = files
        .into_iter()
        .map(|f| ProposedFile {
            path: f.path,
            base_hash: f.base_hash,
            new_content: f.new_content,
        })
        .collect();

    // Pre-mutation gate against the bound contract, then real M8 mutation.
    // The task id below is the supervisor's once created; validate the
    // evidence graph IR first with a placeholder-free compile.
    let bound = bind_contract(contract(), 0);
    gate_proposal_writes(&bound, &files, &manifest, &[])
        .map_err(|e| format!("gate_proposal_writes: {e}"))?;
    let compile_graph =
        compile_evidence_graph(tachyon_types::TaskId::generate(), 0, &requests, &bounds)
            .map_err(|e| format!("compile_evidence_graph: {e}"))?;
    persist_intent(
        &mutation_dir,
        "bench-batch-1",
        &MutationIntent::authorized("bench-batch-1", &files).map_err(|e| format!("intent: {e}"))?,
    )
    .map_err(|e| format!("persist_intent: {e}"))?;
    let engine = MutationEngine::open(&ws, &mutation_dir).map_err(|e| format!("engine: {e}"))?;
    let spec = PatchSpec {
        path: TARGET.into(),
        base_hash: Some(base_hash.clone()),
        new_content: fixed_text.as_bytes().to_vec(),
    };
    let batch = MutationBatchId::generate();
    let prepared = engine
        .prepare_authorized(&context, batch, std::slice::from_ref(&spec))
        .map_err(|e| format!("prepare: {e}"))?;
    let commit = engine
        .commit_authorized_up_to(&context, &prepared, usize::MAX)
        .map_err(|e| format!("commit: {e}"))?;
    if !commit.completed {
        return Err("mutation batch did not complete".into());
    }
    let first_edit_ms = ms(Instant::now());
    if std::fs::read(ws.join(TARGET)).map_err(|e| format!("read back: {e}"))?
        != fixed_text.as_bytes()
    {
        return Err("repaired bytes differ from proposal".into());
    }

    // Selected-check resolution is reported honestly; Affected risk runs the
    // affected crates, unrelated metrics only under Full risk.
    let available = vec!["auth-session".to_string(), "client".to_string()];
    let mut selected_checks = Vec::new();
    let mut broadened = false;
    for want in ["auth-session", "client"] {
        match resolve_check_selection(want, &available) {
            SelectionResolution::Exact(hit) => selected_checks.push(hit),
            SelectionResolution::BroadenedWorkspace => {
                broadened = true;
                selected_checks.push("workspace".into());
            }
            SelectionResolution::Ignored => {}
        }
    }

    // Verification: supervisor path for full/serial, direct control for
    // reference. Completion is granted only by fresh passing checks.
    let (outcome, task_id, revision, recovery, final_verification_ms) = if mode == "reference" {
        let passed = cargo_test(&ws, &fixture_target).await;
        let done_ms = ms(Instant::now());
        (
            if passed {
                "completed_reference"
            } else {
                "verification_failed"
            }
            .to_string(),
            None,
            None,
            None,
            Some(done_ms),
        )
    } else {
        let state_dir = scratch.join("state");
        std::fs::create_dir_all(&state_dir).map_err(|e| format!("state dir: {e}"))?;
        let store = Arc::new(
            StoreWriter::open(&state_dir)
                .await
                .map_err(|e| format!("store: {e}"))?,
        );
        let session = SessionId::generate();
        store
            .create_session(&session.to_string())
            .await
            .map_err(|e| format!("session: {e}"))?;
        let task = create_task(
            session,
            WorkspaceId::generate(),
            "Find why authentication occasionally fails after token refresh and fix it.".into(),
            store.clone(),
        )
        .await
        .map_err(|e| format!("create_task: {e}"))?;
        let id_str = task.task_id().to_string();
        task.configure_verification(context.clone(), contract(), VerificationRisk::Affected)
            .await
            .map_err(|e| format!("configure: {e}"))?;
        let state = task
            .verify_and_complete(context.clone())
            .await
            .map_err(|e| format!("verify: {e}"))?;
        let done_ms = ms(Instant::now());
        let status = state.status;
        let rev = state.revision;
        let outcome = if status == TaskStatus::Completed {
            "completed"
        } else {
            "verification_failed"
        }
        .to_string();
        // Real recovery round-trip: shutdown, reopen the same task identity,
        // confirm the durable status survives.
        task.shutdown()
            .await
            .map_err(|e| format!("shutdown: {e}"))?;
        let recovered = recover_task(task.task_id(), store.clone())
            .await
            .map_err(|e| format!("recover: {e}"))?;
        let restate = recovered
            .get_state()
            .await
            .map_err(|e| format!("get_state: {e}"))?;
        let recovery_label = format!("recovered_{:?}", restate.status).to_lowercase();
        recovered
            .shutdown()
            .await
            .map_err(|e| format!("shutdown2: {e}"))?;
        store.close().await;
        (
            outcome,
            Some(id_str),
            Some(rev),
            Some(recovery_label),
            Some(done_ms),
        )
    };

    // Self-check: checked-in tests/manifests/reference/migrations unchanged.
    let after = snapshot_protected(&ws);
    let fixture_unchanged = before == after;

    Ok(serde_json::json!({
        "mode": mode,
        "outcome": outcome,
        "speculation": "no-speculation/no-judgment coincide with full: no speculative/Jev stage exists in this slice",
        "broken_first_failed": true,
        "node_timings": timings,
        "max_evidence_concurrency": max_concurrency,
        "model_calls": model_calls,
        "tool_calls": EVIDENCE_PATHS.len() as u64 + 2,
        "tool_calls_note": "evidence reads + mutation prepare/commit; verification subprocess not counted",
        "estimated_tokens": null,
        "billed_tokens": null,
        "usage_provenance": usage_provenance,
        "changed_paths": [TARGET],
        "selected_checks": selected_checks,
        "check_broadening": broadened,
        "check_note": "Affected risk runs auth-session + client; unrelated metrics only under Full risk",
        "task_id": task_id,
        "revision": revision,
        "recovery": recovery,
        "wall_ms": ms(Instant::now()),
        "first_evidence_ms": first_evidence_ms,
        "first_edit_ms": first_edit_ms,
        "final_verification_ms": final_verification_ms,
        "sample_count": 1,
        "p50_ms": null,
        "p95_ms": null,
        "fixture_unchanged": fixture_unchanged,
        "evidence_graph_nodes": compile_graph.nodes.len(),
        "scratch": scratch.display().to_string(),
    }))
}
