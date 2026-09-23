//! Shared helpers for the rendering gates. Included by every integration
//! test target; the `allow` keeps `--all-targets -D warnings` green when
//! one target uses a helper another does not.
#![allow(dead_code)]

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tachyon_protocol::{
    EventEnvelope, GatewayEvent, PROTOCOL_VERSION, ServerFrame, decode_server_frame,
    encode_server_frame,
};
use tachyon_tui::{AppState, Key, Pane, handle_key};
use tachyon_types::{EventId, TaskId, Timestamp};

/// Encodes `frame` as a wire frame, decodes it back, and applies it to
/// `state` — the G3a injection point: `ServerFrame` decode → state (never
/// hand-built journal state).
pub fn feed(state: &mut AppState, frame: &ServerFrame) {
    let bytes = encode_server_frame(frame).expect("encode server frame");
    let (decoded, used) = decode_server_frame(&bytes).expect("decode server frame");
    assert_eq!(used, bytes.len(), "the frame round-trips whole");
    state.apply_frame(decoded);
}

/// One journalled event as the gateway would push it.
pub fn journal_frame(
    task_id: TaskId,
    seq: i64,
    kind: &str,
    payload: serde_json::Value,
) -> ServerFrame {
    ServerFrame::Event(EventEnvelope {
        seq,
        event_id: EventId::generate(),
        schema_version: PROTOCOL_VERSION,
        task_id,
        timestamp: Timestamp::now(),
        event: GatewayEvent::Journal {
            kind: kind.to_owned(),
            payload,
        },
    })
}

/// One command-connection response frame (e.g. the `ListTasks` poll).
pub fn response_frame(request_id: EventId, result: tachyon_protocol::CommandResult) -> ServerFrame {
    ServerFrame::Response(tachyon_protocol::ResponseEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        result,
    })
}

/// Presses Tab until `target` is the visible pane (exercises the real
/// input mapping rather than poking `state.pane`).
pub fn tab_to(state: &mut AppState, target: Pane) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut guard = 0;
    while state.pane != target {
        handle_key(state, Key::Tab, &tx);
        guard += 1;
        assert!(guard <= 8, "pane cycle never reached {target:?}");
    }
    drop(rx);
}

/// Renders `state` headlessly and returns the buffer as row strings.
pub fn render(state: &AppState, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| tachyon_tui::draw(frame, state))
        .expect("draw frame");
    let buffer = terminal.backend().buffer();
    let mut rows = Vec::with_capacity(usize::from(height));
    for y in 0..height {
        let mut row = String::new();
        for x in 0..width {
            row.push_str(buffer[(x, y)].symbol());
        }
        rows.push(row);
    }
    rows
}

/// All rows joined — panes may render anywhere in the buffer.
pub fn screen(state: &AppState, width: u16, height: u16) -> String {
    render(state, width, height).join("\n")
}
