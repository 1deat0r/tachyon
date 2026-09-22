#[tokio::test]
async fn missing_required_nodes_never_form_a_passing_report() {
    let ws = Workspace::new();
    let artifacts = Workspace::new();
    let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
    let mut plan = VerificationPlan::build(
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
    plan.graph.nodes.clear();
    let context = Arc::new(ToolsContext::new(
        ws.path().to_path_buf(),
        Policy::trusted_workspace(),
        ArtifactSpool::new(artifacts.path().to_path_buf()),
    ));
    assert!(run(plan, context, CancellationToken::new()).await.is_err());
}

use super::*;
use crate::{AcceptanceContract, CommandCheck, VerificationRisk, test_support::Workspace};
use tachyon_policy::Policy;
use tachyon_scheduler::OutcomeStatus;
use tachyon_tools::artifact::ArtifactSpool;

#[tokio::test]
async fn forged_node_schema_access_and_retries_cannot_reach_processes() {
    let ws = Workspace::new();
    let artifacts = Workspace::new();
    let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
    let command = CommandCheck { program: "python3".into(), args: vec!["-c".into(), "import pathlib; pathlib.Path('target').mkdir(exist_ok=True); pathlib.Path('target/marker').write_text('ran')".into()], cwd: ".".into(), env: BTreeMap::new(), timeout_ms: 1_000 };
    let plan = VerificationPlan::build(
        TaskId::generate(),
        0,
        &AcceptanceContract {
            clauses: vec![Clause::CommandPasses { command }],
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
        ws.path().to_path_buf(),
        policy,
        ArtifactSpool::new(artifacts.path().to_path_buf()),
    ));
    for variant in 0..5 {
        let mut forged_plan = plan.clone();
        let node = forged_plan.graph.nodes.values_mut().next().unwrap();
        match variant {
            0 => node.invocation.args["unexpected"] = serde_json::json!(true),
            1 => node.access.writes.clear(),
            2 => node.retry.attempts = 2,
            3 => node.idempotency = tachyon_ir::Idempotency::Pure,
            _ => node.resources.process_slots = 0,
        }
        let node = node.clone();
        let runner = CheckRunner {
            plan: Arc::new(forged_plan),
            context: context.clone(),
            evidence: Mutex::new(BTreeMap::new()),
            lease: WorkspaceLease::acquire(ws.path(), &CancellationToken::new())
                .await
                .unwrap(),
            lifetime: Arc::new(()),
        };
        let outcome = runner
            .execute_owned(&node, serde_json::Map::new(), CancellationToken::new())
            .await;
        assert!(
            matches!(outcome.status, OutcomeStatus::Failed { .. }),
            "accepted forged declaration {variant}"
        );
        assert!(!ws.path().join("target/marker").exists());
    }
}
