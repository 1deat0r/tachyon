mod common;
use common::Workspace;
use tachyon_types::TaskId;
use tachyon_verify::{
    AcceptanceContract, Clause, VerificationPlan, VerificationRisk, WorkspaceSnapshot,
};

#[test]
fn reverse_dependents_join_affected_checks_and_unparseable_manifests_broaden() {
    // `b` depends on `a`: changing `a` must select both, changing `b` only `b`.
    let ws = Workspace::new();
    ws.write("Cargo.toml", "[workspace]\nmembers=[\"a\",\"b\"]\n");
    ws.write("a/Cargo.toml", "[package]\nname = \"alpha\"\n");
    ws.write(
        "b/Cargo.toml",
        "[package]\nname = \"beta\"\n[dependencies]\nalpha = { path = \"../a\" }\n",
    );
    ws.write("a/src/lib.rs", "before");
    ws.write("b/src/lib.rs", "before");
    let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
    ws.write("a/src/lib.rs", "after");
    let selected = VerificationPlan::build(
        TaskId::generate(),
        0,
        &scope(),
        &baseline,
        &[],
        VerificationRisk::Affected,
    )
    .unwrap();
    let args: Vec<_> = selected
        .graph()
        .nodes
        .values()
        .map(|node| node.invocation.args["command"]["args"].clone())
        .collect();
    assert!(
        args.contains(&serde_json::json!([
            "test",
            "--offline",
            "--manifest-path",
            "a/Cargo.toml"
        ])),
        "directly affected member missing: {args:?}"
    );
    assert!(
        args.contains(&serde_json::json!([
            "test",
            "--offline",
            "--manifest-path",
            "b/Cargo.toml"
        ])),
        "reverse dependent missing: {args:?}"
    );
    assert!(
        !args.contains(&serde_json::json!(["test", "--offline", "--workspace"])),
        "no blind workspace expansion when closure is known: {args:?}"
    );

    // Unparseable dependency metadata broadens instead of omitting tests.
    let ws = Workspace::new();
    ws.write("Cargo.toml", "[workspace]");
    ws.write("a/Cargo.toml", "this is not a manifest at all\n= broken");
    ws.write("a/src/lib.rs", "before");
    let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
    ws.write("a/src/lib.rs", "after");
    let plan = VerificationPlan::build(
        TaskId::generate(),
        0,
        &scope(),
        &baseline,
        &[],
        VerificationRisk::Affected,
    )
    .unwrap();
    assert!(
        plan.graph()
            .nodes
            .values()
            .any(|node| node.invocation.args["command"]["args"]
                == serde_json::json!(["test", "--offline", "--workspace"])),
        "unknown dependency impact must broaden"
    );
}

#[test]
fn shared_inputs_and_unknown_impact_expand_to_workspace() {
    for changed in [
        "Cargo.toml",
        "Cargo.lock",
        ".cargo/config.toml",
        "members/a/Cargo.toml",
        "build-support/rules.txt",
        "members/a/data/input.txt",
    ] {
        let ws = Workspace::new();
        ws.write("Cargo.toml", "[workspace]");
        ws.write("members/a/Cargo.toml", "[package]");
        ws.write(changed, "before");
        let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
        ws.write(changed, "after");
        let plan = VerificationPlan::build(
            TaskId::generate(),
            0,
            &scope(),
            &baseline,
            &[],
            VerificationRisk::Affected,
        )
        .unwrap();
        assert!(
            plan.graph()
                .nodes
                .values()
                .any(|node| node.invocation.args["command"]["args"]
                    == serde_json::json!(["test", "--offline", "--workspace"])),
            "not broad for {changed}"
        );
    }
}

#[test]
fn every_hard_requirement_needs_an_exact_unique_top_level_binding() {
    use tachyon_verify::HardRequirement;
    let ws = Workspace::new();
    let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
    let requirement = HardRequirement {
        id: uuid::Uuid::now_v7(),
        text: "Do not touch source".into(),
    };
    let build = |contract: &AcceptanceContract, hard: &[HardRequirement]| {
        VerificationPlan::build(
            TaskId::generate(),
            0,
            contract,
            &baseline,
            hard,
            VerificationRisk::Affected,
        )
    };
    assert!(build(&scope(), std::slice::from_ref(&requirement)).is_err());
    let mut contract = AcceptanceContract {
        clauses: vec![Clause::HardConstraint {
            id: requirement.id,
            text: "weakened".into(),
            check: Box::new(Clause::ChangedPathsWithin { paths: vec![] }),
        }],
    };
    assert!(build(&contract, std::slice::from_ref(&requirement)).is_err());
    if let Clause::HardConstraint { text, .. } = &mut contract.clauses[0] {
        *text = requirement.text.clone();
    }
    assert!(build(&contract, std::slice::from_ref(&requirement)).is_ok());
    assert!(build(&contract, &[requirement.clone(), requirement]).is_err());
    assert!(build(&contract, &[]).is_err());
}

#[test]
fn authorized_planning_rejects_denied_sources_and_foreign_roots() {
    use tachyon_policy::Policy;
    use tachyon_tools::{ToolsContext, artifact::ArtifactSpool};
    let ws = Workspace::new();
    let artifacts = Workspace::new();
    ws.write("private", "not readable after denial");
    let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
    let mut policy = Policy::trusted_workspace();
    policy.deny("fs.read", "workspace/private");
    let context = ToolsContext::new(
        ws.path().to_path_buf(),
        policy,
        ArtifactSpool::new(artifacts.path().to_path_buf()),
    );
    assert!(
        VerificationPlan::build_authorized(
            TaskId::generate(),
            0,
            &scope(),
            &baseline,
            &[],
            VerificationRisk::Affected,
            &context
        )
        .is_err()
    );
    let other = Workspace::new();
    let context = ToolsContext::new(
        other.path().to_path_buf(),
        Policy::trusted_workspace(),
        ArtifactSpool::new(artifacts.path().to_path_buf()),
    );
    assert!(
        VerificationPlan::build_authorized(
            TaskId::generate(),
            0,
            &scope(),
            &baseline,
            &[],
            VerificationRisk::Affected,
            &context
        )
        .is_err()
    );
}

#[test]
fn explicit_commands_survive_focused_planning_without_running_before_it() {
    use std::collections::BTreeMap;
    use tachyon_verify::CommandCheck;
    let ws = Workspace::new();
    ws.write("Cargo.toml", "[workspace]");
    ws.write("member/Cargo.toml", "[package]");
    ws.write("member/src/lib.rs", "before");
    let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
    ws.write("member/src/lib.rs", "after");
    let command = CommandCheck {
        program: "required-check".into(),
        args: vec!["verbatim".into()],
        env: BTreeMap::new(),
        cwd: ".".into(),
        timeout_ms: 1_000,
    };
    let contract = AcceptanceContract {
        clauses: vec![Clause::CommandPasses {
            command: command.clone(),
        }],
    };
    let plan = VerificationPlan::build(
        TaskId::generate(),
        0,
        &contract,
        &baseline,
        &[],
        VerificationRisk::Full,
    )
    .unwrap();
    let focused = plan
        .graph()
        .nodes
        .values()
        .find(|node| {
            node.invocation.args["command"]["args"]
                == serde_json::json!(["test", "--offline", "--manifest-path", "member/Cargo.toml"])
        })
        .unwrap();
    let explicit = plan
        .graph()
        .nodes
        .values()
        .find(|node| node.invocation.args["command"] == serde_json::to_value(&command).unwrap())
        .unwrap();
    let broad = plan
        .graph()
        .nodes
        .values()
        .find(|node| {
            node.invocation.args["command"]["args"]
                == serde_json::json!(["test", "--offline", "--workspace"])
        })
        .unwrap();
    assert!(plan.graph().ancestors(explicit.id).contains(&focused.id));
    assert!(plan.graph().ancestors(broad.id).contains(&explicit.id));
}

fn scope() -> AcceptanceContract {
    AcceptanceContract {
        clauses: vec![Clause::ChangedPathsWithin {
            paths: vec![".".into()],
        }],
    }
}

#[test]
fn affected_rust_checks_target_nearest_manifest_and_full_appends_workspace() {
    let ws = Workspace::new();
    ws.write(
        "Cargo.toml",
        "[workspace]\nmembers=[\"members/a\",\"members/b\"]\n",
    );
    ws.write("members/a/Cargo.toml", "[package]\nname = \"member-a\"\n");
    ws.write("members/b/Cargo.toml", "[package]\nname = \"member-b\"\n");
    ws.write("members/a/src/lib.rs", "before");
    let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
    ws.write("members/a/src/lib.rs", "after");
    let task = TaskId::generate();
    for (risk, count) in [(VerificationRisk::Affected, 1), (VerificationRisk::Full, 2)] {
        let plan = VerificationPlan::build(task, 4, &scope(), &baseline, &[], risk).unwrap();
        let nodes: Vec<_> = plan
            .graph()
            .nodes
            .values()
            .filter(|node| node.invocation.capability.0 == "verify.command")
            .collect();
        assert_eq!(nodes.len(), count);
        let focused = nodes
            .iter()
            .find(|node| {
                node.invocation.args["command"]["args"]
                    == serde_json::json!([
                        "test",
                        "--offline",
                        "--manifest-path",
                        "members/a/Cargo.toml"
                    ])
            })
            .unwrap();
        assert_eq!(focused.task_id, task);
        assert_eq!(focused.planned_revision, 4);
        assert_eq!(focused.executor, tachyon_ir::ExecutorKind::Verification);
        assert_eq!(focused.idempotency, tachyon_ir::Idempotency::Unknown);
        assert!(!focused.effect_class.speculation_safe());
        assert_eq!(focused.retry.attempts, 1);
        assert_eq!(focused.resources.process_slots, 1);
        assert!(!focused.access.writes.is_empty());
        if risk == VerificationRisk::Full {
            let broad = nodes
                .iter()
                .find(|node| {
                    node.invocation.args["command"]["args"]
                        == serde_json::json!(["test", "--offline", "--workspace"])
                })
                .unwrap();
            assert!(plan.graph().ancestors(broad.id).contains(&focused.id));
        }
        plan.graph().validate(task).unwrap();
    }
}
