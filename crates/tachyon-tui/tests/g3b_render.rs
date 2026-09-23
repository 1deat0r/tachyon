//! G3b — the full journal vocabulary renders through the same
//! decode → state → buffer harness as G3a: the five new kinds plus a
//! synthetic unknown kind's safe placeholder (no panic, forward-compat
//! per plan D2).
//!
//! Same stated scope as G3a: unit-value rendering only — no crossterm
//! input loop, no live-socket coverage.

mod common;

use common::{feed, journal_frame, screen, tab_to};
use tachyon_tui::{AppState, Pane};

/// G3b: `stage` and `evidence_summary` render as live-operations feed
/// lines (the plan names the operations feed as `evidence_summary`'s
/// render host). Fixtures follow the real committed `StateEvent` serde
/// shapes (core: `#[serde(tag = "t", content = "v")]`): stage =
/// `record {stage, detail}`, `evidence_summary` = `entries` of `{path, hash}`
/// (paths + hashes, never source blobs).
#[test]
fn stage_and_evidence_summary_render_as_live_operations_feed_lines() {
    let task = tachyon_types::TaskId::generate();
    let mut state = AppState::new(Some(task));
    tab_to(&mut state, Pane::Operations);
    assert!(
        screen(&state, 100, 30).contains("No live operations yet"),
        "starts honest and empty"
    );

    feed(
        &mut state,
        &journal_frame(
            task,
            1,
            "stage",
            serde_json::json!({
                "t": "Stage",
                "v": {"record": {"stage": "model", "detail": "evidence → model"}}
            }),
        ),
    );
    feed(
        &mut state,
        &journal_frame(
            task,
            2,
            "evidence_summary",
            serde_json::json!({
                "t": "EvidenceSummary",
                "v": {"entries": [{"path": "src/lib.rs", "hash": "abcd1234eff5"}]}
            }),
        ),
    );

    let screen = screen(&state, 100, 30);
    assert!(
        screen.contains("stage") && screen.contains("evidence") && screen.contains("model"),
        "the stage transition renders in the feed:\n{screen}"
    );
    assert!(
        screen.contains("src/lib.rs") && screen.contains("abcd1234eff5"),
        "the evidence path and hash render:\n{screen}"
    );
    assert!(
        !screen.contains("No live operations yet"),
        "the empty state is replaced by real feed lines:\n{screen}"
    );
    assert_eq!(state.last_applied_seq, 2, "both events advanced the seq");
}

/// G3b: a synthetic unknown `kind` renders the safe placeholder without
/// panicking — forward-compatible client-side passthrough (plan D2/G3b).
/// Unknown kinds are feed material, so the placeholder lives in the
/// live-operations feed, not the curated conversation.
#[test]
fn unknown_journal_kind_renders_the_safe_placeholder_without_panicking() {
    let task = tachyon_types::TaskId::generate();
    let mut state = AppState::new(Some(task));

    feed(
        &mut state,
        &journal_frame(
            task,
            7,
            "quantum_flux_reconciled",
            serde_json::json!({"t": "QuantumFlux", "v": {"shards": 3}}),
        ),
    );

    tab_to(&mut state, Pane::Operations);
    let screen = screen(&state, 100, 30);
    assert!(
        screen.contains("quantum_flux_reconciled"),
        "the unknown kind names itself in the placeholder:\n{screen}"
    );
    assert!(
        screen.contains("no renderer yet"),
        "the placeholder is explicit about being unrendered:\n{screen}"
    );
    assert_eq!(
        state.last_applied_seq, 7,
        "an unknown kind still advances the cursor (it is durable)"
    );
}

/// G3b: `changed_files` receipts render in the changed-files pane.
/// Fixture shape follows the real `StateEvent::ChangedFiles { files }`:
/// `PathHash` objects of `{path, hash}`.
#[test]
fn changed_files_receipts_render_in_the_files_pane() {
    let task = tachyon_types::TaskId::generate();
    let mut state = AppState::new(Some(task));
    tab_to(&mut state, Pane::Files);
    assert!(
        screen(&state, 100, 30).contains("No changed-file receipts yet"),
        "starts honest and empty"
    );

    feed(
        &mut state,
        &journal_frame(
            task,
            1,
            "changed_files",
            serde_json::json!({
                "t": "ChangedFiles",
                "v": {
                    "files": [
                        {"path": "crates/tachyon-tui/src/lib.rs", "hash": "1111aaaa2222bbbb"},
                        {"path": "README.md", "hash": "3333cccc4444dddd"}
                    ]
                }
            }),
        ),
    );

    let screen = screen(&state, 100, 30);
    assert!(
        screen.contains("changed: crates/tachyon-tui/src/lib.rs"),
        "the first receipt renders as a receipt line:\n{screen}"
    );
    assert!(
        screen.contains("1111aaaa2222bbbb") && screen.contains('#'),
        "the receipt carries its content hash:\n{screen}"
    );
    assert!(
        screen.contains("changed: README.md"),
        "the second receipt renders:\n{screen}"
    );
    assert!(
        !screen.contains("No changed-file receipts yet"),
        "the empty state is replaced by receipts:\n{screen}"
    );
}

/// G3b: `agent_message` (durable model answer) renders in the
/// conversation pane alongside steering.
#[test]
fn agent_message_renders_in_the_conversation() {
    let task = tachyon_types::TaskId::generate();
    let mut state = AppState::new(Some(task));

    feed(
        &mut state,
        &journal_frame(
            task,
            1,
            "agent_message",
            serde_json::json!({
                "t": "AgentMessage",
                "v": {"message": "refreshToken is defined in src/auth.ts"}
            }),
        ),
    );

    let screen = screen(&state, 100, 30);
    assert!(
        screen.contains("refreshToken is defined in src/auth.ts"),
        "the durable model answer renders:\n{screen}"
    );
}

/// G3b: `approval_request` parks in the approval pane with its id and
/// description, showing the decision keys (sending them is covered by the
/// input-mapping unit test). Fixture shape follows the real
/// `StateEvent::ApprovalRequest { request }`: the parked ask nests under
/// `v.request {id, summary, …}` (tachyon-policy `ApprovalRequest`).
#[test]
fn approval_request_parks_in_the_approval_pane_with_its_id() {
    let task = tachyon_types::TaskId::generate();
    let approval = tachyon_types::ApprovalId::generate();
    let mut state = AppState::new(Some(task));
    tab_to(&mut state, Pane::Approvals);
    assert!(
        screen(&state, 100, 30).contains("No approval requests"),
        "starts honest and empty"
    );

    feed(
        &mut state,
        &journal_frame(
            task,
            1,
            "approval_request",
            serde_json::json!({
                "t": "ApprovalRequest",
                "v": {
                    "request": {
                        "id": approval.to_string(),
                        "capability": "fs.write",
                        "scope": "workspace",
                        "operation_hash": "cafebabecafebabe",
                        "summary": "shell: rm -rf target"
                    }
                }
            }),
        ),
    );

    assert_eq!(
        state.pending.as_ref().map(|pending| pending.id.clone()),
        Some(approval.to_string()),
        "the pending approval carries the journalled id"
    );
    let screen = screen(&state, 100, 30);
    assert!(
        screen.contains(&approval.to_string()),
        "the pending id renders:\n{screen}"
    );
    assert!(
        screen.contains("shell: rm -rf target"),
        "the parked operation renders:\n{screen}"
    );
    assert!(
        screen.contains("^a approve"),
        "the decision keys are offered:\n{screen}"
    );
}
