//! Verifier admission must use the same lease as other workspace stages.
use std::{path::PathBuf, sync::Arc, time::Duration};
use tachyon_policy::Policy;
use tachyon_tools::{ToolsContext, artifact::ArtifactSpool, workspace::WorkspaceLease};
use tachyon_types::TaskId;
use tachyon_verify::{
    AcceptanceContract, Clause, VerificationPlan, VerificationRisk, WorkspaceSnapshot,
};
use tokio_util::sync::CancellationToken;

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("tachyon-verifier-lease-{}", TaskId::generate()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn verifier_waits_for_an_evidence_or_mutation_stage_lease() {
    let ws = Workspace::new();
    let artifacts = Workspace::new();
    let baseline = WorkspaceSnapshot::capture(&ws.0).unwrap();
    let plan = VerificationPlan::build(
        TaskId::generate(),
        0,
        &AcceptanceContract {
            clauses: vec![Clause::ChangedPathsWithin { paths: vec![] }],
        },
        &baseline,
        &[],
        VerificationRisk::Affected,
    )
    .unwrap();
    let context = Arc::new(ToolsContext::new(
        ws.0.clone(),
        Policy::trusted_workspace(),
        ArtifactSpool::new(artifacts.0.clone()),
    ));
    let other_stage = WorkspaceLease::acquire(&ws.0, &CancellationToken::new())
        .await
        .unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let mut job = tokio::spawn(async move {
        started.send(()).unwrap();
        tachyon_verify::run(plan, context, CancellationToken::new()).await
    });
    ready.await.unwrap();
    let pending = tokio::time::timeout(Duration::from_millis(200), &mut job).await;
    drop(other_stage);
    let (blocked, result) = match pending {
        Ok(result) => (false, result),
        Err(_) => (
            true,
            tokio::time::timeout(Duration::from_secs(3), job)
                .await
                .unwrap(),
        ),
    };
    assert!(result.unwrap().unwrap().passed());
    assert!(blocked, "verifier bypassed the shared workspace lease");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn aborted_run_retains_workspace_until_process_cleanup_and_reap() {
    use std::collections::BTreeMap;
    use tachyon_verify::CommandCheck;
    let ws = Workspace::new();
    let artifacts = Workspace::new();
    let baseline = WorkspaceSnapshot::capture(&ws.0).unwrap();
    let script = "import os, pathlib, signal, sys, time\np=pathlib.Path('target'); p.mkdir(exist_ok=True)\ndef stop(*args):\n time.sleep(0.04); (p/'drained').write_text('yes'); sys.exit(0)\nsignal.signal(signal.SIGTERM, stop)\n(p/'pid').write_text(str(os.getpid()))\ntime.sleep(3)";
    let plan = VerificationPlan::build(
        TaskId::generate(),
        0,
        &AcceptanceContract {
            clauses: vec![Clause::CommandPasses {
                command: CommandCheck {
                    program: "python3".into(),
                    args: vec!["-c".into(), script.into()],
                    cwd: ".".into(),
                    env: BTreeMap::new(),
                    timeout_ms: 5_000,
                },
            }],
        },
        &baseline,
        &[],
        VerificationRisk::Affected,
    )
    .unwrap();
    let mut policy = Policy::trusted_workspace();
    policy.allow("verify.command", "workspace/**");
    policy.allow("process.spawn", "python3");
    let context = Arc::new(ToolsContext::new(
        ws.0.clone(),
        policy,
        ArtifactSpool::new(artifacts.0.clone()),
    ));
    let job = tokio::spawn(tachyon_verify::run(plan, context, CancellationToken::new()));
    tokio::time::timeout(Duration::from_secs(3), async {
        while !ws.0.join("target/pid").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let pid = std::fs::read_to_string(ws.0.join("target/pid")).unwrap();
    job.abort();
    assert!(job.await.unwrap_err().is_cancelled());
    let _next = tokio::time::timeout(
        Duration::from_secs(3),
        WorkspaceLease::acquire(&ws.0, &CancellationToken::new()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        ws.0.join("target/drained").exists(),
        "workspace admitted a conflicting owner before actual process cleanup"
    );
    assert!(
        !PathBuf::from(format!("/proc/{pid}")).exists(),
        "workspace admitted a conflicting owner before immediate-child reap"
    );
}
