//! G5 — the provider label in the TUI: plan G5 requires the label in
//! CLI/TUI output (the CLI half prints `provider: …` on `run`), and the
//! gateway's `GetTask` payload gains an optional top-level
//! `provider_label` (contract with the gateway writer: the string
//! `"scripted test/replay provider"` when `kind=fake`, absent
//! otherwise). The TUI stores it and the status bar renders it beside
//! the other header facts.
//!
//! Both fixtures are **synthetic payloads** — whether the live gateway
//! already emits the key is the gateway writer's half of the contract;
//! the absence case is this crate's closure: no key → no label, no
//! panic.

mod common;

use common::screen;
use tachyon_protocol::{CommandResult, PROTOCOL_VERSION, ResponseEnvelope};
use tachyon_tui::AppState;
use tachyon_types::{EventId, TaskId};

/// One synthetic `GetTask` response around `payload` (the same decode →
/// state → buffer injection point the other rendering gates use).
fn get_task_response(payload: serde_json::Value) -> ResponseEnvelope {
    ResponseEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: EventId::generate(),
        result: CommandResult::Ok { payload },
    }
}

/// G5: a `GetTask` payload carrying the contract's top-level
/// `provider_label` stores it and the status bar renders the label as
/// `provider: …`, the same wording the CLI half prints.
#[test]
fn provider_label_from_get_task_renders_in_the_status_bar() {
    let task = TaskId::generate();
    let mut state = AppState::new(Some(task));

    state.apply_response(get_task_response(serde_json::json!({
        "task": {
            "objective": "labelled objective",
            "status": "Created",
            "revision": 0,
        },
        "provider_label": "scripted test/replay provider",
    })));

    assert_eq!(
        state.provider_label.as_deref(),
        Some("scripted test/replay provider"),
        "the GetTask payload's top-level key is stored"
    );
    // Wide enough that the whole status line (task id + header facts +
    // label) fits without truncation.
    let screen = screen(&state, 200, 12);
    assert!(
        screen.contains("provider: scripted test/replay provider"),
        "the provider label renders in the status/header area:\n{screen}"
    );
    assert!(
        screen.contains("labelled objective"),
        "the GetTask snapshot still decodes alongside the label:\n{screen}"
    );
}

/// G5 absence case: a payload without the key leaves the label unset —
/// the screen renders no label and nothing panics.
#[test]
fn absent_provider_label_renders_no_label_and_never_panics() {
    let task = TaskId::generate();
    let mut state = AppState::new(Some(task));

    state.apply_response(get_task_response(serde_json::json!({
        "task": {
            "objective": "unlabelled objective",
            "status": "Running",
            "revision": 4,
        },
    })));

    assert!(
        state.provider_label.is_none(),
        "no key means no stored label — absence, not a default"
    );
    let screen = screen(&state, 200, 12);
    assert!(
        !screen.contains("provider:"),
        "no label renders when the gateway declared none:\n{screen}"
    );
    assert!(
        screen.contains("unlabelled objective"),
        "the task itself still renders:\n{screen}"
    );
}
