use std::collections::BTreeMap;
use tachyon_verify::{AcceptanceContract, Clause, CommandCheck};

fn command() -> CommandCheck {
    CommandCheck {
        program: "true".into(),
        args: vec![],
        cwd: ".".into(),
        env: BTreeMap::new(),
        timeout_ms: 1_000,
    }
}

#[test]
fn malformed_commands_are_rejected_before_execution() {
    for bad in [
        CommandCheck {
            program: " ".into(),
            ..command()
        },
        CommandCheck {
            program: "true\0".into(),
            ..command()
        },
        CommandCheck {
            timeout_ms: 0,
            ..command()
        },
        CommandCheck {
            timeout_ms: u64::MAX,
            ..command()
        },
        CommandCheck {
            cwd: "../outside".into(),
            ..command()
        },
        CommandCheck {
            cwd: "/absolute".into(),
            ..command()
        },
        CommandCheck {
            cwd: "src\\alias".into(),
            ..command()
        },
        CommandCheck {
            cwd: "src//alias".into(),
            ..command()
        },
        CommandCheck {
            cwd: "src/./alias".into(),
            ..command()
        },
    ] {
        assert!(
            AcceptanceContract {
                clauses: vec![Clause::CommandPasses {
                    command: bad.clone()
                }]
            }
            .validate()
            .is_err(),
            "accepted {bad:?}"
        );
    }
}

#[test]
fn path_and_hard_clauses_are_validated_too() {
    let id = uuid::Uuid::now_v7();
    let hard = |check| Clause::HardConstraint {
        id,
        text: "keep scope".into(),
        check: Box::new(check),
    };
    for clause in [
        Clause::FileUnchanged {
            path: "../outside".into(),
        },
        Clause::FileUnchanged {
            path: "target/protected".into(),
        },
        Clause::ChangedPathsWithin {
            paths: vec![".git/config".into()],
        },
        hard(Clause::CommandPasses {
            command: CommandCheck {
                timeout_ms: 0,
                ..command()
            },
        }),
        hard(hard(Clause::ChangedPathsWithin { paths: vec![] })),
    ] {
        assert!(
            AcceptanceContract {
                clauses: vec![clause.clone()]
            }
            .validate()
            .is_err(),
            "accepted {clause:?}"
        );
    }
    let clause = hard(Clause::ChangedPathsWithin { paths: vec![] });
    assert!(
        AcceptanceContract {
            clauses: vec![clause.clone(), clause]
        }
        .validate()
        .is_err()
    );
}

#[test]
fn legacy_text_recovers_as_unresolved_and_unknown_fields_are_rejected() {
    let contract: AcceptanceContract =
        serde_json::from_str(r#"{"clauses":["looks good"]}"#).unwrap();
    assert!(
        matches!(&contract.clauses[0], Clause::Unresolved { description } if description == "looks good")
    );
    for value in [
        serde_json::json!({"clauses": [], "trusted": true}),
        serde_json::json!({"clauses": [{"kind": "FileUnchanged", "path": "a", "passed": true}]}),
        serde_json::json!({"clauses": [{"kind": "CommandPasses", "command": {"program": "true", "args": [], "cwd": ".", "env": {}, "timeout_ms": 1, "shell": true}}]}),
    ] {
        assert!(serde_json::from_value::<AcceptanceContract>(value).is_err());
    }
}

#[test]
fn empty_contract_cannot_authorize_completion() {
    assert!(AcceptanceContract::default().validate().is_err());
}
