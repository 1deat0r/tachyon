//! G3a — headless `ratatui::backend::TestBackend` rendering gates.
//!
//! **Stated scope: unit-value rendering only.** Fixtures travel
//! `ServerFrame` encode → decode → [`AppState`] → buffer; no crossterm
//! input loop and no live-socket coverage is claimed here (those live in
//! G2/G4). The input mapping assertions live in the crate's unit tests.

mod common;

use common::{feed, journal_frame, render, response_frame, screen, tab_to};
use tachyon_protocol::CommandResult;
use tachyon_tui::{AppState, Key, handle_key};
use tachyon_types::{EventId, TaskId};

/// Plan item 3 non-goals / honest-empty clause: panes whose producers do
/// not exist yet (run-path `stage`/`evidence`, `changed_files`,
/// `Progress` streaming output, task list before its first poll, approval
/// queue before Phase B) must render an explicit honest empty state —
/// never a blank pane, never a fake placeholder value. This state has
/// received **zero** frames: it is exactly what a fresh attach shows
/// before any producer exists.
#[test]
fn fresh_state_renders_honest_empty_states_in_every_producer_pane() {
    let task = TaskId::generate();
    let state = AppState::new(Some(task));

    let conversation = screen(&state, 100, 30);
    assert!(
        conversation.contains("No journal events yet"),
        "conversation must say why it is empty:\n{conversation}"
    );

    let mut state = state;
    tab_to(&mut state, PaneTasks);
    let tasks = screen(&state, 100, 30);
    assert!(
        tasks.contains("No tasks listed yet"),
        "the list pane must state it polls ListTasks:\n{tasks}"
    );
    assert!(
        tasks.contains("polling ListTasks"),
        "the list pane names its bounded poll (D3):\n{tasks}"
    );

    tab_to(&mut state, PaneOperations);
    let operations = screen(&state, 100, 30);
    assert!(
        operations.contains("No live operations yet"),
        "run-path feed must be an honest empty state:\n{operations}"
    );
    assert!(
        operations.contains("No streaming output yet"),
        "Progress (no M11 producer) must be an honest empty state:\n{operations}"
    );
    assert!(
        operations.contains("Progress producer"),
        "the streaming note names the missing producer honestly:\n{operations}"
    );

    tab_to(&mut state, PaneFiles);
    let files = screen(&state, 100, 30);
    assert!(
        files.contains("No changed-file receipts yet"),
        "changed files must be an honest empty state:\n{files}"
    );

    tab_to(&mut state, PaneApprovals);
    let approvals = screen(&state, 100, 30);
    assert!(
        approvals.contains("No approval requests"),
        "approvals must be an honest empty state:\n{approvals}"
    );

    // Graph inspection before GetTask returns says so rather than showing
    // a fabricated graph.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    handle_key(&mut state, Key::Ctrl('g'), &tx);
    let graph = screen(&state, 100, 30);
    assert!(
        graph.contains("No execution-graph detail yet"),
        "graph must be honest before GetTask answers:\n{graph}"
    );
    drop(rx);

    // Narrow terminal: every pane renders without panicking.
    for _ in 0..5 {
        let _ = render(&state, 80, 24);
        tab_to(&mut state, tachyon_tui::Pane::Conversation);
    }
}

use tachyon_tui::Pane::Tasks as PaneTasks;
use tachyon_tui::Pane::{Approvals as PaneApprovals, Conversation as PaneConversation};
use tachyon_tui::Pane::{Files as PaneFiles, Operations as PaneOperations};

// ---------------------------------------------------------------------------
// The nine existing journal kinds (G3a).
// ---------------------------------------------------------------------------

/// G3a: the `created` fixture (full initial state) travels
/// `ServerFrame::event` decode → state → buffer and renders the objective
/// into the conversation plus the status bar's task identity.
#[test]
fn created_event_renders_the_objective_and_status_bar() {
    let task = TaskId::generate();
    let mut state = AppState::new(Some(task));

    let payload = serde_json::json!({
        "t": "Created",
        "v": {
            "state": {
                "id": task.to_string(),
                "objective": "Where is refreshToken defined and used?",
                "status": "Created",
                "revision": 0u64,
            }
        }
    });
    feed(&mut state, &journal_frame(task, 0, "created", payload));

    let screen = screen(&state, 100, 30);
    assert!(
        screen.contains("Where is refreshToken defined and used?"),
        "objective renders in the conversation:\n{screen}"
    );
    assert!(
        screen.contains("Created"),
        "status bar shows the status:\n{screen}"
    );
    assert!(
        screen.contains(&task.to_string()),
        "status bar shows the attached task:\n{screen}"
    );
    assert_eq!(state.last_applied_seq, 0, "seq 0 is applied");
    let _ = PaneConversation;
}

/// G3a: steering `message` and `constraint` events render as conversation
/// lines (steering + durable constraint outcome).
#[test]
fn message_and_constraint_events_render_into_the_conversation() {
    let task = TaskId::generate();
    let mut state = AppState::new(Some(task));

    feed(
        &mut state,
        &journal_frame(
            task,
            1,
            "message",
            serde_json::json!({"t": "Message", "v": {"message": "also check refresh flow"}}),
        ),
    );
    feed(
        &mut state,
        &journal_frame(
            task,
            2,
            "constraint",
            serde_json::json!({
                "t": "Constraint",
                "v": {
                    "constraint": {
                        "id": "01900000-0000-7000-8000-000000000001",
                        "source": "User",
                        "text": "never log tokens",
                        "strength": "Hard",
                        "created_revision": 1u64
                    }
                }
            }),
        ),
    );

    let screen = screen(&state, 100, 30);
    assert!(
        screen.contains("also check refresh flow"),
        "steering message renders:\n{screen}"
    );
    assert!(
        screen.contains("never log tokens"),
        "constraint text renders as a durable outcome:\n{screen}"
    );
}

/// G3a: a `status` transition renders into the status bar and the
/// conversation as a durable outcome.
#[test]
fn status_event_updates_the_status_bar_and_conversation() {
    let task = TaskId::generate();
    let mut state = AppState::new(Some(task));

    feed(
        &mut state,
        &journal_frame(
            task,
            3,
            "status",
            serde_json::json!({"t": "Status", "v": {"from": "Created", "to": "Paused"}}),
        ),
    );

    assert_eq!(state.status.as_deref(), Some("Paused"), "status tracked");
    let screen = screen(&state, 100, 30);
    assert!(
        screen.contains("Paused"),
        "status bar shows the new status:\n{screen}"
    );
    assert!(
        screen.contains("Created") && screen.contains("Paused"),
        "the transition is a conversation outcome:\n{screen}"
    );
}

/// G3a: an `approval` decision renders in the approval-display pane.
#[test]
fn approval_decision_event_renders_in_the_approval_display() {
    let task = TaskId::generate();
    let mut state = AppState::new(Some(task));

    feed(
        &mut state,
        &journal_frame(
            task,
            4,
            "approval",
            serde_json::json!({
                "t": "Approval",
                "v": {
                    "approval": "01900000-0000-7000-8000-000000000002",
                    "granted": true,
                    "reason": "operator allowed the delete"
                }
            }),
        ),
    );

    tab_to(&mut state, PaneApprovals);
    let screen = screen(&state, 100, 30);
    assert!(
        screen.contains("granted"),
        "the decision renders:\n{screen}"
    );
    assert!(
        screen.contains("operator allowed the delete"),
        "the recorded reason renders:\n{screen}"
    );
}

/// G3a: the four verification kinds render their durable outcomes in the
/// conversation.
#[test]
fn verification_events_render_into_the_conversation() {
    let task = TaskId::generate();
    let mut state = AppState::new(Some(task));

    feed(
        &mut state,
        &journal_frame(
            task,
            1,
            "verification_configured",
            serde_json::json!({
                "t": "VerificationConfigured",
                "v": {"contract": {}, "baseline": {}, "risk": "Derived"}
            }),
        ),
    );
    feed(
        &mut state,
        &journal_frame(
            task,
            2,
            "verification_started",
            serde_json::json!({"t": "VerificationStarted", "v": {"graph": {"nodes": []}}}),
        ),
    );
    feed(
        &mut state,
        &journal_frame(
            task,
            3,
            "verification_finished",
            serde_json::json!({
                "t": "VerificationFinished",
                "v": {"report": null, "error": null, "completed": true}
            }),
        ),
    );
    feed(
        &mut state,
        &journal_frame(
            task,
            4,
            "verification_interrupted",
            serde_json::json!({
                "t": "VerificationInterrupted"
            }),
        ),
    );

    let screen = screen(&state, 100, 30);
    assert!(
        screen.contains("verification configured"),
        "configured renders:\n{screen}"
    );
    assert!(
        screen.contains("verification started"),
        "started renders:\n{screen}"
    );
    assert!(
        screen.contains("verification finished"),
        "finished renders:\n{screen}"
    );
    assert!(
        screen.contains("verification interrupted"),
        "interrupted renders:\n{screen}"
    );
}

/// G3a: the list pane renders rows decoded from a `ListTasks` response
/// frame (D3 poll payload), through the same decode → state → buffer path.
#[test]
fn list_tasks_response_renders_rows_in_the_list_pane() {
    let task = TaskId::generate();
    let mut state = AppState::new(Some(task));
    tab_to(&mut state, PaneTasks);
    assert!(
        screen(&state, 100, 30).contains("No tasks listed yet"),
        "starts honest and empty"
    );

    feed(
        &mut state,
        &response_frame(
            EventId::generate(),
            CommandResult::Ok {
                payload: serde_json::json!({
                    "tasks": [{
                        "id": task.to_string(),
                        "session_id": "01900000-0000-7000-8000-000000000003",
                        "objective": "stream probe objective",
                        "status": "Created",
                        "revision": 0i64,
                        "updated_at": 0i64
                    }]
                }),
            },
        ),
    );

    let screen = screen(&state, 100, 30);
    assert!(
        screen.contains("stream probe objective"),
        "the row renders:\n{screen}"
    );
    assert!(
        !screen.contains("No tasks listed yet"),
        "the honest empty state is replaced by real rows:\n{screen}"
    );
}
