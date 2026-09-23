mod common;

use common::Workspace;
use std::process::{Command, Output};
use tachyon_types::TaskId;
use tachyon_verify::{
    AcceptanceContract, Clause, CommandCheck, VerificationPlan, VerificationRisk, WorkspaceSnapshot,
};

fn fixture(dependencies: &str) -> Workspace {
    let ws = Workspace::new();
    ws.write(
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\"alpha\", \"client\", \"unrelated\"]\n",
    );
    for name in ["alpha", "client", "unrelated"] {
        ws.write(
            &format!("{name}/Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        );
        ws.write(
            &format!("{name}/src/lib.rs"),
            "pub fn value() -> u32 { 1 }\n",
        );
    }
    ws.write(
        "client/Cargo.toml",
        &format!(
            "[package]\nname = \"client\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{dependencies}"
        ),
    );
    ws
}

fn selected(baseline: &WorkspaceSnapshot) -> Vec<CommandCheck> {
    let plan = VerificationPlan::build(
        TaskId::generate(),
        0,
        &AcceptanceContract {
            clauses: vec![Clause::ChangedPathsWithin {
                paths: vec![".".into()],
            }],
        },
        baseline,
        &[],
        VerificationRisk::Affected,
    )
    .unwrap();
    let mut commands: Vec<CommandCheck> = plan
        .graph()
        .nodes
        .values()
        .filter_map(|node| node.invocation.args.get("command"))
        .map(|command| serde_json::from_value(command.clone()).unwrap())
        .collect();
    commands.sort_by(|a, b| a.args.cmp(&b.args));
    commands
}

fn run(ws: &Workspace, target: &Workspace, args: &[String]) -> Output {
    let output = Command::new("cargo")
        .args(args)
        .current_dir(ws.path())
        .env("CARGO_TARGET_DIR", target.path())
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .unwrap();
    eprintln!(
        "cargo {} => {}\n{}\n{}",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn nonflat_dependency_sections_fall_back_to_workspace() {
    let mut omitted = Vec::new();
    for section in [
        "dependencies.renamed",
        "dev-dependencies.renamed",
        "build-dependencies.renamed",
        "target.'cfg(unix)'.dependencies.renamed",
        "target.'cfg(unix)'.dev-dependencies.renamed",
        "target.'cfg(unix)'.build-dependencies.renamed",
        "\"dependencies\"",
        "target",
    ] {
        let ws = fixture(&format!(
            "[{section}]\npackage = \"alpha\"\npath = \"../alpha\"\n"
        ));
        let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
        ws.write("alpha/src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        if !selected(&baseline)
            .iter()
            .any(|command| command.args == ["test", "--offline", "--workspace"])
        {
            omitted.push(section);
        }
    }
    assert!(
        omitted.is_empty(),
        "unsupported dependency sections silently omitted dependents: {omitted:?}"
    );
}

#[test]
fn unknown_manifest_structure_falls_back_to_workspace() {
    let package = "[package]\nname = \"client\"\nversion = \"0.1.0\"\n";
    let dependency = "renamed = { package = \"alpha\", path = \"../alpha\" }\n";
    let mut omitted = Vec::new();
    for (label, manifest) in [
        (
            "dotted dependencies",
            format!("dependencies.{dependency}{package}"),
        ),
        (
            "escaped package name",
            format!(
                "{}[dependencies]\n{dependency}",
                package.replace("client", "cl\\u0069ent")
            ),
        ),
        (
            "quoted package key",
            format!(
                "{}[dependencies]\n{dependency}",
                package.replace("name =", "\"name\" =")
            ),
        ),
        (
            "missing package name",
            format!("[package]\nversion = \"0.1.0\"\n[dependencies]\n{dependency}"),
        ),
        (
            "duplicate package name",
            format!("{package}name = \"wrong\"\n[dependencies]\n{dependency}"),
        ),
        (
            "legacy dependency section",
            format!("{package}[dev_dependencies]\n{dependency}"),
        ),
        (
            "multiline value",
            format!("{package}description = \"\"\"\ntext\n\"\"\"\n[dependencies]\n{dependency}"),
        ),
    ] {
        let ws = fixture("");
        ws.write("client/Cargo.toml", &manifest);
        let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
        ws.write("alpha/src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        if !selected(&baseline)
            .iter()
            .any(|command| command.args == ["test", "--offline", "--workspace"])
        {
            omitted.push(label);
        }
    }
    assert!(omitted.is_empty(), "unsafe narrow plans: {omitted:?}");
}

#[test]
fn unsupported_or_malformed_dependency_values_fall_back_to_workspace() {
    for dependency in [
        "renamed = { workspace = true }",
        "renamed.workspace = true",
        "renamed = { package = \"alpha\", path = \"../alpha\", features = [] }",
        "renamed = { package = \"alpha\", path = \"../alpha\", optional = true }",
        "\"renamed\" = { package = \"alpha\", path = \"../alpha\" }",
        r#"renamed = { package = "\u0061lpha", path = "../alpha" }"#,
        "renamed = { package = \"alpha\", path = \"../alpha\"",
        "renamed = { package = 42, path = \"../alpha\" }",
        "renamed = { package = \"wrong\", package = \"alpha\", path = \"../alpha\" }",
        "renamed = { package = \"alpha\", path = \"../alpha#literal\" }",
        "renamed = { package = \"alpha\", path = \"../alpha,comma\" }",
        "renamed = {",
        "= broken",
    ] {
        let ws = fixture(&format!("[dependencies]\n{dependency}\n"));
        let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
        ws.write("alpha/src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        assert!(
            selected(&baseline)
                .iter()
                .any(|command| command.args == ["test", "--offline", "--workspace"]),
            "unsupported dependency was not broadened: {dependency}"
        );
    }
}

#[test]
fn ordinary_auth_client_selection_excludes_unrelated_crates() {
    for (changed, expected) in [("auth", vec!["auth", "client"]), ("client", vec!["client"])] {
        let ws = Workspace::new();
        ws.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"auth\", \"client\", \"unrelated\"]\n",
        );
        for name in ["auth", "client", "unrelated"] {
            ws.write(
                &format!("{name}/Cargo.toml"),
                &format!("[package]\nname = \"{name}\"\n"),
            );
            ws.write(&format!("{name}/src/lib.rs"), "before");
        }
        ws.write(
            "client/Cargo.toml",
            "[package]\nname = \"client\"\n[dependencies]\nauth = { path = \"../auth\" }\n",
        );
        let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
        ws.write(&format!("{changed}/src/lib.rs"), "after");
        let commands = selected(&baseline);
        let expected: Vec<Vec<String>> = expected
            .into_iter()
            .map(|name| {
                vec![
                    "test".into(),
                    "--offline".into(),
                    "--manifest-path".into(),
                    format!("{name}/Cargo.toml"),
                ]
            })
            .collect();
        assert_eq!(
            commands
                .into_iter()
                .map(|command| command.args)
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn aliased_transitive_dependents_and_cycles_remain_selective() {
    let ws = fixture("[dependencies]\nrenamed = { path = '../alpha', package = 'alpha' }\n");
    ws.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"alpha\", \"client\", \"frontend\", \"unrelated\"]\n",
    );
    ws.write(
        "alpha/Cargo.toml",
        "[package]\nname = \"alpha\"\n[dev-dependencies]\nclient = { path = \"../client\" }\n",
    );
    ws.write(
        "frontend/Cargo.toml",
        "[package]\nname = \"frontend\"\n[build-dependencies]\nclient = { path = \"../client\" }\n",
    );
    ws.write("frontend/src/lib.rs", "before");
    let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
    ws.write("alpha/src/lib.rs", "after");
    assert_eq!(
        selected(&baseline)
            .into_iter()
            .map(|command| command.args)
            .collect::<Vec<_>>(),
        ["alpha", "client", "frontend"].map(|name| vec![
            "test".to_owned(),
            "--offline".into(),
            "--manifest-path".into(),
            format!("{name}/Cargo.toml"),
        ])
    );
}

#[test]
fn renamed_dependency_checks_catch_real_client_regression() {
    real_client_regression(
        &fixture("[dependencies]\nrenamed = { package = \"alpha\", path = \"../alpha\" }\n"),
        false,
    );
}

#[test]
fn conservative_checks_catch_real_client_regressions() {
    for dependencies in [
        "[dependencies.renamed]\npackage = \"alpha\"\npath = \"../alpha\"\n",
        "[target.'cfg(all())'.dependencies]\nrenamed = { package = \"alpha\", path = \"../alpha\" }\n",
        "[target.'cfg(all())'.dev-dependencies]\nrenamed = { package = \"alpha\", path = \"../alpha\" }\n",
        "[dependencies]\nrenamed = { workspace = true }\n",
    ] {
        let ws = fixture(dependencies);
        if dependencies.contains("workspace") {
            ws.write("Cargo.toml", "[workspace]\nresolver = \"2\"\nmembers = [\"alpha\", \"client\", \"unrelated\"]\n[workspace.dependencies]\nrenamed = { package = \"alpha\", path = \"alpha\" }\n");
        }
        real_client_regression(&ws, true);
    }
}

fn real_client_regression(ws: &Workspace, broad: bool) {
    let target = Workspace::new();
    ws.write(
        "client/src/lib.rs",
        "#[test]\nfn preserves_client_contract() { assert_eq!(renamed::value(), 1); }\n",
    );
    assert!(
        run(
            ws,
            &target,
            &["generate-lockfile".into(), "--offline".into()]
        )
        .status
        .success()
    );
    let full = ["test".into(), "--offline".into(), "--workspace".into()];
    assert!(run(ws, &target, &full).status.success());
    let baseline = WorkspaceSnapshot::capture(ws.path()).unwrap();
    ws.write("alpha/src/lib.rs", "pub fn value() -> u32 { 2 }\n");

    let commands = selected(&baseline);
    let selected_passed = commands
        .iter()
        .map(|command| run(ws, &target, &command.args).status.success())
        .collect::<Vec<_>>();
    let full_result = run(ws, &target, &full);
    assert!(!full_result.status.success());
    assert!(String::from_utf8_lossy(&full_result.stdout).contains("preserves_client_contract"));
    assert!(
        selected_passed.contains(&false),
        "selected checks all passed despite a failing renamed dependent: {commands:?}"
    );
    assert_eq!(commands.len(), 2);
    assert_eq!(
        commands[0].args,
        ["test", "--offline", "--manifest-path", "alpha/Cargo.toml"]
    );
    let expected: &[&str] = if broad {
        &["test", "--offline", "--workspace"]
    } else {
        &["test", "--offline", "--manifest-path", "client/Cargo.toml"]
    };
    assert_eq!(commands[1].args, expected);

    ws.write(
        "alpha/src/lib.rs",
        "pub fn value() -> u32 { 1 } // repaired\n",
    );
    for command in selected(&baseline) {
        assert!(run(ws, &target, &command.args).status.success());
    }
}
