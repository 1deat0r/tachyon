//! Render path: pure drawing over [`AppState`] into a ratatui `Frame`.
//!
//! The loop draws only on a state change, rate-capped by [`RENDER_TICK`]
//! (plan item 2: bounded ticks, at most 30 FPS). Nothing here talks to
//! the gateway, the terminal backend, or the input device.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};

use crate::state::{AppState, Confirm, Pane};

/// Minimum time between two draws: 33 ms bounds the loop at ≤ 30 FPS
/// (spec §38).
pub const RENDER_TICK: Duration = Duration::from_millis(33);

/// Honest empty state: conversation before any journal event arrives.
const EMPTY_CONVERSATION_WAITING: &str =
    "No journal events yet — waiting for replay from the gateway.";
/// Honest empty state: no task attached (picker start).
const EMPTY_CONVERSATION_DETACHED: &str = "No task attached — pick one in the Tasks pane.";
/// Honest empty state: task list before its first bounded poll lands (D3).
const EMPTY_TASKS: &str = "No tasks listed yet — polling ListTasks while this pane is visible.";
/// Honest empty state: run-path producer (stage/evidence) has no Phase B events.
const EMPTY_OPERATIONS: &str = "No live operations yet — run-path events (stage, evidence) arrive with the M11 run path (Phase B).";
/// Honest empty state: `GatewayEvent::Progress` has no M11 producer
/// (provider-token streaming is the M6 deferral).
const EMPTY_STREAMING: &str = "No streaming output yet — the Progress producer arrives with provider-token streaming (M6 deferral).";
/// Honest empty state: `changed_files` receipts only exist once a run journalled them.
const EMPTY_FILES: &str = "No changed-file receipts yet — no run has journalled any.";
/// Honest empty state: approval queue (plan item 8, Phase B).
const EMPTY_APPROVALS: &str = "No approval requests or decisions yet.";
/// Honest empty state: graph overlay before `GetTask` answers.
const EMPTY_GRAPH: &str = "No execution-graph detail yet — GetTask has not returned.";

/// Decides whether the loop may draw now: only when state changed since
/// the last draw **and** the bounded tick has elapsed (≤ 30 FPS).
#[must_use]
pub fn should_draw(dirty: bool, last_draw: Instant, now: Instant) -> bool {
    dirty && now.duration_since(last_draw) >= RENDER_TICK
}

/// Draws one full frame of `state`: status bar, the active pane (or the
/// graph overlay when toggled), the input line, and the key footer.
pub fn draw(frame: &mut Frame<'_>, state: &AppState) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(frame.area());

    frame.render_widget(status_bar(state), chunks[0]);
    frame.render_widget(main_area(state), chunks[1]);
    frame.render_widget(input_bar(state), chunks[2]);
    frame.render_widget(footer(state), chunks[3]);
}

fn status_bar(state: &AppState) -> Paragraph<'static> {
    let task = state
        .attached_task
        .map_or_else(|| "-".to_owned(), |task| task.to_string());
    let status = state.status.as_deref().unwrap_or("-");
    let seq = if state.last_applied_seq >= 0 {
        state.last_applied_seq.to_string()
    } else {
        "-".to_owned()
    };
    let mut text = format!(
        "task {task} | status {status} | seq {seq} | pane {}",
        state.pane.title()
    );
    if let Some(label) = &state.provider_label {
        // Same wording as the CLI half prints (plan G5).
        text.push_str(" | provider: ");
        text.push_str(label);
    }
    if let Some(notice) = &state.notice {
        text.push_str(" | ! ");
        text.push_str(notice);
    }
    Paragraph::new(text)
}

fn main_area(state: &AppState) -> Paragraph<'static> {
    let title = if state.graph {
        "Execution graph"
    } else {
        state.pane.title()
    };
    let lines: Vec<Line<'static>> = if state.graph {
        graph_lines(state)
    } else {
        match state.pane {
            Pane::Conversation => conversation_lines(state),
            Pane::Tasks => task_lines(state),
            Pane::Operations => operation_lines(state),
            Pane::Files => file_lines(state),
            Pane::Approvals => approval_lines(state),
        }
    };
    Paragraph::new(lines).block(Block::bordered().title(title.to_owned()))
}

fn conversation_lines(state: &AppState) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(objective) = &state.objective {
        lines.push(Line::from(format!("objective: {objective}")));
    }
    for entry in &state.conversation {
        lines.push(Line::from(entry.clone()));
    }
    if lines.is_empty() {
        lines.push(Line::from(if state.attached_task.is_some() {
            EMPTY_CONVERSATION_WAITING
        } else {
            EMPTY_CONVERSATION_DETACHED
        }));
    }
    lines
}

fn task_lines(state: &AppState) -> Vec<Line<'static>> {
    if state.tasks.is_empty() {
        return vec![Line::from(EMPTY_TASKS)];
    }
    state
        .tasks
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let marker = if index == state.selected { "> " } else { "  " };
            Line::from(format!(
                "{marker}[{}] {} (rev {}) — {}",
                row.status, row.id, row.revision, row.objective
            ))
        })
        .collect()
}

fn operation_lines(state: &AppState) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from("live operations:")];
    if state.operations.is_empty() {
        lines.push(Line::from(EMPTY_OPERATIONS));
    } else {
        for entry in &state.operations {
            lines.push(Line::from(entry.clone()));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from("streaming output:"));
    if state.streaming.is_empty() {
        lines.push(Line::from(EMPTY_STREAMING));
    } else {
        for entry in &state.streaming {
            lines.push(Line::from(entry.clone()));
        }
    }
    lines
}

fn file_lines(state: &AppState) -> Vec<Line<'static>> {
    if state.files.is_empty() {
        return vec![Line::from(EMPTY_FILES)];
    }
    state
        .files
        .iter()
        .map(|entry| Line::from(entry.clone()))
        .collect()
}

fn approval_lines(state: &AppState) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(pending) = &state.pending {
        lines.push(Line::from(format!(
            "PENDING {}: {}",
            pending.id, pending.request
        )));
        lines.push(Line::from("decision: ^a approve, ^d deny"));
        lines.push(Line::from(""));
    }
    for entry in &state.approvals {
        lines.push(Line::from(entry.clone()));
    }
    if lines.is_empty() {
        lines.push(Line::from(EMPTY_APPROVALS));
    }
    lines
}

fn graph_lines(state: &AppState) -> Vec<Line<'static>> {
    match &state.graph_detail {
        Some(detail) => detail
            .lines()
            .map(|row| Line::from(row.to_owned()))
            .collect(),
        None => vec![Line::from(EMPTY_GRAPH)],
    }
}

fn input_bar(state: &AppState) -> Paragraph<'static> {
    Paragraph::new(state.input.clone()).block(Block::bordered().title("input"))
}

fn footer(state: &AppState) -> Paragraph<'static> {
    let lines = match &state.confirm {
        Some(Confirm::Cancel) => vec![
            Line::from("cancel this task? y = confirm cancel | n or Esc = abort"),
            Line::from("nothing was sent yet — the confirmation gates the CancelTask"),
        ],
        Some(Confirm::Approve(id)) => vec![
            Line::from("approve this request? y = confirm approve | n or Esc = abort"),
            Line::from(format!(
                "nothing was sent yet — the confirmation gates Approve {id}"
            )),
        ],
        Some(Confirm::Deny(id)) => vec![
            Line::from("deny this request? y = confirm deny | n or Esc = abort"),
            Line::from(format!(
                "nothing was sent yet — the confirmation gates Deny {id}"
            )),
        ],
        None => vec![
            Line::from("tab:panes  enter:send  ^x:cancel  ^p:pause  ^r:resume"),
            Line::from("^a:approve  ^d:deny  ^g:graph  ^q:detach"),
        ],
    };
    Paragraph::new(lines)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{RENDER_TICK, should_draw};

    /// Plan item 2: render only on state change or a bounded tick —
    /// never faster than 30 FPS, never without a change.
    #[test]
    fn draws_only_on_state_change_and_never_faster_than_30fps() {
        let tick = RENDER_TICK;
        assert!(
            tick >= Duration::from_millis(33),
            "the tick must cap the loop at 30 FPS, got {tick:?}"
        );
        let now = Instant::now();

        assert!(
            !should_draw(false, now, now + Duration::from_secs(5)),
            "no state change means no draw, however long the tick runs"
        );
        assert!(
            !should_draw(true, now, now + Duration::from_millis(1)),
            "a change inside the tick budget waits for the bounded tick"
        );
        assert!(
            should_draw(true, now, now + RENDER_TICK),
            "a change with the tick budget elapsed draws"
        );
    }
}
