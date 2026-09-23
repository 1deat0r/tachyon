//! M10 benchmark slice: thin runtime host driving the auth-refresh fixture.
//!
//! This example contains no agent decision logic. It constructs trusted
//! inputs (fixture copy, policy, acceptance contract) and a scripted
//! test/replay provider, then runs the ONE shared production driver
//! (`tachyon_core::driver::drive`): evidence -> model -> patch ->
//! verification. Scripted responses prove runtime integration and
//! verification, never model reasoning quality.
//! Modes: `full` (concurrent evidence, supervisor path), `serial`
//! (sequential evidence, supervisor path, records concurrency 1),
//! `reference` (same shared steps through the driver with no task and no
//! journal — the declared control group, serial control loop; this host
//! runs verification itself). Any other mode reports `unimplemented`.
//! No-speculation/no-judgment configurations coincide with `full` in this
//! slice: no speculative or Jev stage exists, so no difference is reported.
//!
//! ORCHESTRATION NOTE: there is exactly one orchestration implementation
//! in this workspace — the shared core driver. This host only builds
//! trusted inputs and reads its outcome back into a JSON report. For the
//! supervisor path the driver proposes run-ID + task-ID + revision-bound
//! messages and the supervisor acknowledges and journals each one (M10
//! plan §2); the process-wide ownership guard admits exactly one
//! supervisor per (`state-database path`, `TaskId`).
//!
//! JSON report goes to stdout; diagnostics go to stderr.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use tachyon_core::create_task;
use tachyon_core::driver::{DriveHost, EvidenceMode, RunPlan, drive};
use tachyon_core::runtime::{EvidenceRequest, RuntimeBounds, evidence_concurrency, max_overlap};
use tachyon_models::UsageProvenance;
use tachyon_models::fake::{FakeModelProvider, FakeResponse};
use tachyon_mutation::blake3_hex;
use tachyon_policy::Policy;
use tachyon_store::StoreWriter;
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
use tachyon_types::{ProviderId, SessionId, WorkspaceId};
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

    // Scripted test/replay provider: a fixed transformation of the
    // target bytes (broken body -> guarded body), served through a real
    // ModelProvider so the call count and usage provenance are measured.
    // The script is queued before the shared driver runs.
    let broken_bytes = std::fs::read(ws.join(TARGET)).map_err(|e| format!("target read: {e}"))?;
    let broken_text =
        String::from_utf8(broken_bytes.clone()).map_err(|e| format!("fixture utf8: {e}"))?;
    if !broken_text.contains(BROKEN_BODY) {
        return Err("fixture does not contain the known stale-refresh body".into());
    }
    let fixed_text = broken_text.replacen(BROKEN_BODY, FIXED_BODY, 1);
    let provider = Arc::new(FakeModelProvider::new(ProviderId("bench-script".into())));
    let script = serde_json::json!({
        "decision": "propose_execution",
        "operations": [{
            "capability": "mutation.patch",
            "args": {
                "path": TARGET,
                "base_hash": blake3_hex(&broken_bytes),
                "new_content": fixed_text,
            }
        }]
    });
    provider.push_response(FakeResponse::respond(&script.to_string()));

    let requests: Vec<EvidenceRequest> = EVIDENCE_PATHS
        .iter()
        .map(|p| EvidenceRequest {
            capability: "fs.read".into(),
            path: p.to_string(),
        })
        .collect();

    // The supervisor path creates its task BEFORE the run so every stage
    // is journaled through the proposal/ack pattern; the reference mode
    // keeps no task, no journal.
    let store = if mode == "reference" {
        None
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
        Some((task, store))
    };

    // Keep an Arc so this host (the opener) can close the store after
    // the driver leaves the task shut down.
    let mut store_holder: Option<Arc<StoreWriter>> = None;
    let host = match store {
        Some((task, store)) => {
            store_holder = Some(store.clone());
            DriveHost::Supervisor {
                handle: task,
                store,
            }
        }
        None => DriveHost::Reference,
    };
    let plan = RunPlan {
        origin: t0,
        evidence_mode: if mode == "full" {
            EvidenceMode::Concurrent
        } else {
            EvidenceMode::Serial
        },
        evidence: requests.clone(),
        contract: contract(),
        risk: VerificationRisk::Affected,
        mutation_dir,
        batch_id: "bench-batch-1".into(),
        model: "scripted-replay-1".into(),
        requested_checks: vec!["auth-session".to_string(), "client".to_string()],
        available_checks: vec!["auth-session".to_string(), "client".to_string()],
        bounds,
        cancel: tokio_util::sync::CancellationToken::new(),
    };
    let outcome = drive(host, context, provider.clone(), plan)
        .await
        .map_err(|e| format!("drive: {e}"))?;
    if let Some(store) = store_holder {
        store.close().await;
    }

    // Measured concurrency: full mode overlaps for real behind the
    // barrier; serial mode reads sequentially and records 1. Same helper
    // the unit tests pin; report a mismatch instead of hiding it.
    let timings = outcome.node_timings.clone();
    let max_concurrency = if mode == "serial" {
        usize::from(!timings.is_empty())
    } else {
        max_overlap(&outcome.intervals_us)
    };
    let helper_check = evidence_concurrency(&outcome.node_timings);
    eprintln!("max_overlap(us)={max_concurrency} evidence_concurrency(ms)={helper_check}");

    let model_calls = provider.request_count() as u64;
    let usage_provenance = match outcome.usage.provenance {
        UsageProvenance::ProviderReported => "provider_reported",
        UsageProvenance::Scripted => "scripted",
        UsageProvenance::Unknown => "unknown",
    };

    // Verification: supervisor path ran inside the shared driver;
    // reference keeps its declared control-loop tail in this host.
    let (label, task_id, revision, recovery, final_verification_ms) = if mode == "reference" {
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
        (
            outcome
                .outcome
                .clone()
                .ok_or_else(|| "supervisor run returned no outcome".to_string())?,
            outcome.task_id.clone(),
            outcome.revision,
            outcome.recovery.clone(),
            outcome.final_verification_ms,
        )
    };

    // Self-check: checked-in tests/manifests/reference/migrations unchanged.
    let after = snapshot_protected(&ws);
    let fixture_unchanged = before == after;

    Ok(serde_json::json!({
        "mode": mode,
        "outcome": label,
        "speculation": "no-speculation/no-judgment coincide with full: no speculative/Jev stage exists in this slice",
        "broken_first_failed": true,
        "node_timings": outcome.node_timings,
        "max_evidence_concurrency": max_concurrency,
        "model_calls": model_calls,
        "tool_calls": EVIDENCE_PATHS.len() as u64 + 2,
        "tool_calls_note": "evidence reads + mutation prepare/commit; verification subprocess not counted",
        "estimated_tokens": null,
        "billed_tokens": null,
        "usage_provenance": usage_provenance,
        "changed_paths": outcome.changed_paths,
        "selected_checks": outcome.selected_checks,
        "check_broadening": outcome.check_broadening,
        "check_note": "Affected risk runs auth-session + client; unrelated metrics only under Full risk",
        "task_id": task_id,
        "revision": revision,
        "recovery": recovery,
        "wall_ms": ms(Instant::now()),
        "first_evidence_ms": outcome.first_evidence_ms,
        "first_edit_ms": outcome.first_edit_ms,
        "final_verification_ms": final_verification_ms,
        "sample_count": 1,
        "p50_ms": null,
        "p95_ms": null,
        "fixture_unchanged": fixture_unchanged,
        "evidence_graph_nodes": outcome.evidence_graph_nodes,
        "scratch": scratch.display().to_string(),
    }))
}
