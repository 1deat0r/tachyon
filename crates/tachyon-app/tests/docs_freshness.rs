//! Docs-freshness tripwires: structural invariants between code and docs.
//!
//! These tests fail when documentation drifts from the workspace instead of
//! letting it rot silently. Fuzzy prose consistency (does the README describe
//! what the code actually does?) is reserved for the `tachyon-judgment`
//! milestone, where a bounded Jev judge fits; everything checked here is
//! exact, so it is checked deterministically.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(name: &str) -> String {
    let path = workspace_root().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{name} unreadable: {error}"))
}

/// `Milestone N` markers in `text`, in order of appearance.
fn milestones(text: &str) -> Vec<u64> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("Milestone ") {
        rest = &rest[index + "Milestone ".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_numeric()).collect();
        if let Ok(number) = digits.parse::<u64>() {
            found.push(number);
        }
    }
    found
}

fn workspace_members() -> Vec<String> {
    let manifest = read("Cargo.toml");
    let mut members = Vec::new();
    let mut in_members = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with("members") {
            in_members = true;
            continue;
        }
        if in_members {
            if line.starts_with(']') {
                break;
            }
            let name = line.trim_matches(|c| c == '"' || c == ',' || c == ' ');
            let name = name.strip_prefix("crates/").unwrap_or(name);
            if !name.is_empty() {
                members.push(name.to_owned());
            }
        }
    }
    members
}

/// Crates whose milestone is complete MUST be implemented: the scaffold
/// marker must be gone from their `lib.rs`. Move a crate into this list
/// with the milestone that implements it — the test forces the docs move
/// to happen alongside the code.
const IMPLEMENTED: &[&str] = &[
    "tachyon-types",
    "tachyon-protocol",
    "tachyon-ir",
    "tachyon-store",
    "tachyon-core",
    "tachyon-gateway",
    "tachyon-app",
    "tachyon-scheduler",
    "tachyon-policy",
    "tachyon-tools",
    "tachyon-repo",
    "tachyon-router",
    "tachyon-telemetry",
];

#[test]
fn readme_status_matches_progress_gates() {
    let readme = read("README.md");
    let progress = read("PROGRESS.md");
    let gates_section = progress
        .split("## Completed gates")
        .nth(1)
        .expect("PROGRESS.md needs a Completed gates section");
    let completed: BTreeSet<u64> = milestones(gates_section).into_iter().collect();
    assert!(!completed.is_empty(), "no completed gates in PROGRESS.md");
    let latest = completed.iter().max().copied().unwrap_or(0);
    let status = readme
        .lines()
        .find(|line| line.contains("Status ("))
        .expect("README.md needs a Status line");
    let claimed: BTreeSet<u64> = milestones(status).into_iter().collect();
    assert!(
        claimed.contains(&latest),
        "README status ({status}) lags PROGRESS.md gates (latest Milestone {latest})"
    );
}

#[test]
fn current_milestone_follows_gates() {
    let progress = read("PROGRESS.md");
    let current = progress
        .split("## Current milestone")
        .nth(1)
        .expect("PROGRESS.md needs a Current milestone section");
    let current: BTreeSet<u64> = milestones(current).into_iter().collect();
    let gates = progress.split("## Completed gates").nth(1).unwrap_or("");
    let completed: BTreeSet<u64> = milestones(gates).into_iter().collect();
    let latest = completed.iter().max().copied().unwrap_or(0);
    assert!(
        current.iter().any(|milestone| *milestone == latest + 1),
        "current milestone {current:?} should be Milestone {} after gates {completed:?}",
        latest + 1
    );
}

#[test]
fn changelog_covers_completed_milestones() {
    let changelog = read("CHANGELOG.md");
    let progress = read("PROGRESS.md");
    let gates = progress.split("## Completed gates").nth(1).unwrap_or("");
    let completed: BTreeSet<u64> = milestones(gates).into_iter().collect();
    for milestone in completed {
        assert!(
            changelog.contains(&format!("Milestone {milestone}")),
            "CHANGELOG.md has no entry for completed Milestone {milestone}"
        );
    }
}

#[test]
fn implemented_crates_left_the_scaffold() {
    let members = workspace_members();
    assert!(!members.is_empty(), "no workspace members parsed");
    for name in members {
        let dir = workspace_root().join("crates").join(&name).join("src");
        let lib = dir.join("lib.rs");
        let main = dir.join("main.rs");
        assert!(
            lib.exists() || main.exists(),
            "missing src/lib.rs or src/main.rs for {name}"
        );
        if !lib.exists() {
            continue;
        }
        let source = std::fs::read_to_string(&lib).unwrap();
        if IMPLEMENTED.contains(&name.as_str()) {
            assert!(
                !source.contains("Scaffold only"),
                "{name} is listed IMPLEMENTED but still carries the scaffold marker"
            );
        }
    }
}
