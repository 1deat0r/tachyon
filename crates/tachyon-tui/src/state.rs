//! Application state: the single owner of everything the render loop draws.
//!
//! State only changes when the input path or the gateway-event reader hands
//! it something; it never reaches out to the gateway itself (AD-014).

use serde_json::Value;
use tachyon_protocol::{
    CommandResult, EventEnvelope, GatewayEvent, ResponseEnvelope, ServerFrame, check_version,
};
use tachyon_types::TaskId;

/// Which pane the main area currently shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    /// Objective, steering messages, durable outcomes.
    Conversation,
    /// Task status and the polled task list.
    Tasks,
    /// Stage / evidence feed plus streaming-output area.
    Operations,
    /// Changed-file receipts.
    Files,
    /// Pending approval request and decided approvals.
    Approvals,
}

impl Pane {
    /// Cycles to the next pane, wrapping around.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Conversation => Self::Tasks,
            Self::Tasks => Self::Operations,
            Self::Operations => Self::Files,
            Self::Files => Self::Approvals,
            Self::Approvals => Self::Conversation,
        }
    }

    /// Cycles to the previous pane, wrapping around.
    #[must_use]
    pub fn previous(self) -> Self {
        match self {
            Self::Conversation => Self::Approvals,
            Self::Tasks => Self::Conversation,
            Self::Operations => Self::Tasks,
            Self::Files => Self::Operations,
            Self::Approvals => Self::Files,
        }
    }

    /// Pane title as rendered on screen.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Conversation => "Conversation",
            Self::Tasks => "Tasks",
            Self::Operations => "Live operations",
            Self::Files => "Changed files",
            Self::Approvals => "Approvals",
        }
    }
}

/// An armed confirmation; nothing is sent until confirmed (R1 board B6:
/// approve/deny gate like cancel — one wrong keystroke can never decide
/// an ask, and never the wrong task's ask).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// Confirm a `CancelTask`.
    Cancel,
    /// Confirm an `Approve` for this approval id.
    Approve(String),
    /// Confirm a `Deny` for this approval id.
    Deny(String),
}

/// One row of the polled task list (D3).
#[derive(Clone, Debug, PartialEq)]
pub struct TaskRow {
    /// Task identity.
    pub id: TaskId,
    /// Lifecycle status name.
    pub status: String,
    /// The task's objective.
    pub objective: String,
    /// State revision.
    pub revision: i64,
}

/// A parked approval the operator can grant or deny from the TUI.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingApproval {
    /// Approval request id as journalled.
    pub id: String,
    /// Human-readable description of the parked operation.
    pub request: String,
}

/// Everything the TUI renders. One owner: the render loop.
#[derive(Debug)]
pub struct AppState {
    /// Task the subscription is attached to, if any.
    pub attached_task: Option<TaskId>,
    /// Objective shown at the top of the conversation.
    pub objective: Option<String>,
    /// Current lifecycle status, from `status` / `TaskSnapshot` events.
    pub status: Option<String>,
    /// Latest known state revision.
    pub revision: Option<u64>,
    /// Conversation lines: objective, steering, durable outcomes.
    pub conversation: Vec<String>,
    /// Live-operations feed lines (`stage`, `evidence_summary`, …).
    pub operations: Vec<String>,
    /// Streaming-output lines (`Progress`; no producer until M6 lands).
    pub streaming: Vec<String>,
    /// Changed-file receipt lines.
    pub files: Vec<String>,
    /// Decided-approval lines (history).
    pub approvals: Vec<String>,
    /// Parked approval awaiting a decision, when one exists.
    pub pending: Option<PendingApproval>,
    /// Rows from the bounded `ListTasks` poll (D3).
    pub tasks: Vec<TaskRow>,
    /// Selected row in the task list (picker).
    pub selected: usize,
    /// Steering input line.
    pub input: String,
    /// Visible pane.
    pub pane: Pane,
    /// Execution-graph inspection overlay open (fed from `GetTask`).
    pub graph: bool,
    /// Graph detail returned by `GetTask`, when fetched.
    pub graph_detail: Option<String>,
    /// Armed confirmation, if any.
    pub confirm: Option<Confirm>,
    /// Transient notice (errors, connection notes) for the status bar.
    pub notice: Option<String>,
    /// Provider label declared by the gateway — optional top-level
    /// `provider_label` on the `Subscribe` ack / `GetTask` payload
    /// (plan G5: `"scripted test/replay provider"` for the fake
    /// provider, absent otherwise) — rendered in the status bar.
    pub provider_label: Option<String>,
    /// Last journalled seq applied into this state.
    pub last_applied_seq: i64,
}

impl AppState {
    /// Fresh state for `task` (`None` starts the picker on the task list).
    #[must_use]
    pub fn new(task: Option<TaskId>) -> Self {
        let pane = if task.is_some() {
            Pane::Conversation
        } else {
            Pane::Tasks
        };
        Self {
            attached_task: task,
            objective: None,
            status: None,
            revision: None,
            conversation: Vec::new(),
            operations: Vec::new(),
            streaming: Vec::new(),
            files: Vec::new(),
            approvals: Vec::new(),
            pending: None,
            tasks: Vec::new(),
            selected: 0,
            input: String::new(),
            pane,
            graph: false,
            graph_detail: None,
            confirm: None,
            notice: None,
            provider_label: None,
            last_applied_seq: -1,
        }
    }

    /// D3 task switch: wipe everything belonging to the previously
    /// attached task so the new attach's replay starts from a clean
    /// slate — no frame of the old task may survive. The task list is
    /// picker state and survives; the cursor display resets to `-1`.
    pub fn reset_for(&mut self, task_id: TaskId) {
        self.attached_task = Some(task_id);
        self.objective = None;
        self.status = None;
        self.revision = None;
        self.conversation.clear();
        self.operations.clear();
        self.streaming.clear();
        self.files.clear();
        self.approvals.clear();
        self.pending = None;
        self.graph = false;
        self.graph_detail = None;
        self.confirm = None;
        self.notice = None;
        self.provider_label = None;
        self.last_applied_seq = -1;
        self.pane = Pane::Conversation;
    }

    /// Applies one decoded server frame — the decode → state → buffer path
    /// the rendering gates inject fixtures through.
    pub fn apply_frame(&mut self, frame: ServerFrame) {
        match frame {
            ServerFrame::Event(envelope) => self.apply_envelope(envelope),
            ServerFrame::Response(response) => self.apply_response(response),
        }
    }

    /// Applies one pushed event. Journal frames are seq-guarded here as
    /// defence in depth: the reader cursor (D2) already dropped replays,
    /// and a duplicate can never regress this state.
    ///
    /// While a task is attached, envelopes of any *other* task are
    /// dropped before any arm runs (D2/D3): the reader's 256-slot buffer
    /// can still be draining old-task frames when the picker's
    /// `reset_for` repoints the attach — those frames must neither
    /// pollute the new task's display nor inflate `last_applied_seq`
    /// (which would silently seq-drop the new task's replay). Detached
    /// state (`attached_task == None`, the picker) keeps applying every
    /// envelope as before.
    pub fn apply_envelope(&mut self, envelope: EventEnvelope) {
        if let Some(attached) = self.attached_task
            && envelope.task_id != attached
        {
            return;
        }
        match envelope.event {
            GatewayEvent::Journal { kind, payload } => {
                if envelope.seq <= self.last_applied_seq {
                    return;
                }
                self.last_applied_seq = envelope.seq;
                self.apply_journal(&kind, &payload);
            }
            GatewayEvent::TaskSnapshot {
                status, revision, ..
            } => {
                self.status = Some(status);
                self.revision = Some(revision);
            }
            GatewayEvent::Progress { message, .. } => self.streaming.push(message),
            GatewayEvent::Error { message, .. } => self.notice = Some(message),
            // The reader owns resubscription; nothing to show here.
            GatewayEvent::ResyncRequired { .. } => {}
        }
    }

    /// Routes one command-connection response into state (`ListTasks`
    /// rows, `GetTask` snapshot / graph detail, typed errors as notices).
    pub fn apply_response(&mut self, response: ResponseEnvelope) {
        if let Err(error) = check_version(response.protocol_version) {
            self.notice = Some(error.to_string());
            return;
        }
        match response.result {
            CommandResult::Ok { payload } => {
                // Plan G5: `GetTask` may declare the provider label as a
                // top-level key; absence declares nothing.
                if let Some(label) = payload.get("provider_label").and_then(Value::as_str) {
                    self.provider_label = Some(label.to_owned());
                }
                if let Some(tasks) = payload.get("tasks").and_then(Value::as_array) {
                    self.tasks = tasks.iter().filter_map(parse_task_row).collect();
                    if self.selected >= self.tasks.len() && !self.tasks.is_empty() {
                        self.selected = self.tasks.len() - 1;
                    }
                } else if let Some(task) = payload.get("task") {
                    if let Some(objective) = task.get("objective").and_then(Value::as_str) {
                        self.objective = Some(objective.to_owned());
                    }
                    if let Some(status) = task.get("status").and_then(Value::as_str) {
                        self.status = Some(status.to_owned());
                    }
                    if let Some(revision) = task.get("revision").and_then(Value::as_u64) {
                        self.revision = Some(revision);
                    }
                    if let Some(graph) = task.get("graph") {
                        self.graph_detail = Some(
                            serde_json::to_string_pretty(graph)
                                .unwrap_or_else(|_| graph.to_string()),
                        );
                    }
                }
            }
            CommandResult::Err { code, message } => {
                self.notice = Some(format!("{code}: {message}"));
            }
        }
    }

    /// Records the provider label from a `Subscribe` ack (plan G5): a
    /// declared label (`Some`) is stored for the status bar; an ack
    /// without the top-level `provider_label` key (`None`) declares
    /// nothing and leaves the current value untouched.
    pub(crate) fn note_provider_label(&mut self, provider_label: Option<String>) {
        if provider_label.is_some() {
            self.provider_label = provider_label;
        }
    }

    /// Decodes one journalled transition by `kind` (plan D2: match the
    /// kind, unknown kinds get a safe placeholder — never a panic).
    #[allow(clippy::too_many_lines)]
    fn apply_journal(&mut self, kind: &str, payload: &Value) {
        let value = payload.get("v").unwrap_or(payload);
        match kind {
            "created" => {
                let state = value.get("state");
                if let Some(objective) = state
                    .and_then(|state| state.get("objective"))
                    .and_then(Value::as_str)
                {
                    self.objective = Some(objective.to_owned());
                }
                if let Some(status) = state
                    .and_then(|state| state.get("status"))
                    .and_then(Value::as_str)
                {
                    self.status = Some(status.to_owned());
                }
                if let Some(revision) = state
                    .and_then(|state| state.get("revision"))
                    .and_then(Value::as_u64)
                {
                    self.revision = Some(revision);
                }
            }
            "message" => {
                if let Some(text) = value.get("message").and_then(Value::as_str) {
                    self.conversation.push(format!("steering: {text}"));
                }
            }
            "constraint" => {
                if let Some(text) = value
                    .get("constraint")
                    .and_then(|constraint| constraint.get("text"))
                    .and_then(Value::as_str)
                {
                    self.conversation.push(format!("constraint: {text}"));
                }
            }
            "status" => {
                let from = json_text(value.get("from"));
                let to = json_text(value.get("to"));
                self.status = Some(to.clone());
                self.conversation.push(format!("status: {from} → {to}"));
            }
            "approval" => {
                let id = json_text(value.get("approval"));
                let granted = value.get("granted").and_then(Value::as_bool);
                let reason = json_text(value.get("reason"));
                let decision = match granted {
                    Some(true) => "granted",
                    Some(false) => "denied",
                    None => "decided",
                };
                self.approvals
                    .push(format!("{decision} approval {id}: {reason}"));
                if granted.is_some() {
                    // A decided approval clears any pending view of it.
                    self.pending = None;
                }
            }
            "verification_configured" => {
                let risk = json_text(value.get("risk"));
                self.conversation
                    .push(format!("verification configured (risk: {risk})"));
            }
            "verification_started" => {
                self.conversation.push("verification started".to_owned());
            }
            "verification_finished" => {
                let completed = value.get("completed").and_then(Value::as_bool);
                let error = value.get("error").and_then(Value::as_str);
                let line = match (completed, error) {
                    (Some(true), _) => "verification finished: passed".to_owned(),
                    (_, Some(error)) => format!("verification finished: failed — {error}"),
                    _ => "verification finished: not completed".to_owned(),
                };
                self.conversation.push(line);
            }
            "verification_interrupted" => {
                self.conversation.push(
                    "verification interrupted — verifier effects require reconciliation".to_owned(),
                );
            }
            // New durable vocabulary (plan item 7): the supervisor's run
            // events, in the shapes core actually commits (StateEvent
            // serde, t/v tagged): Stage.record, EvidenceSummary.entries,
            // ChangedFiles.files (PathHash {path, hash}),
            // ApprovalRequest.request (tachyon-policy ApprovalRequest).
            "stage" => {
                let record = value.get("record");
                let name = json_text(record.and_then(|record| record.get("stage")));
                let detail = json_text(record.and_then(|record| record.get("detail")));
                let line = if detail == "-" {
                    format!("stage: {name}")
                } else {
                    format!("stage: {name} — {detail}")
                };
                self.operations.push(line);
            }
            "evidence_summary" => {
                let entries = value
                    .get("entries")
                    .or_else(|| value.get("paths"))
                    .or_else(|| value.get("evidence"))
                    .or_else(|| value.get("items"))
                    .and_then(Value::as_array);
                match entries {
                    Some(entries) if !entries.is_empty() => {
                        for entry in entries {
                            self.operations.push(evidence_line(entry));
                        }
                    }
                    _ => self
                        .operations
                        .push(format!("evidence: {}", json_text(value.get("summary")))),
                }
            }
            "changed_files" => {
                let entries = value
                    .get("files")
                    .or_else(|| value.get("paths"))
                    .and_then(Value::as_array);
                match entries {
                    Some(entries) => {
                        for entry in entries {
                            let receipt = match entry {
                                Value::String(path) => path.clone(),
                                Value::Object(fields) => {
                                    let path =
                                        fields.get("path").and_then(Value::as_str).unwrap_or("?");
                                    match fields.get("hash").and_then(Value::as_str) {
                                        Some(hash) => format!("{path} #{hash}"),
                                        None => path.to_owned(),
                                    }
                                }
                                other => json_text(Some(other)),
                            };
                            self.files.push(format!("changed: {receipt}"));
                        }
                    }
                    None => self
                        .files
                        .push(format!("changed: {}", json_text(value.get("summary")))),
                }
            }
            "agent_message" => {
                if let Some(text) = value
                    .get("message")
                    .or_else(|| value.get("text"))
                    .and_then(Value::as_str)
                {
                    self.conversation.push(format!("agent: {text}"));
                }
            }
            "approval_request" => {
                // Real shape: the ask nests under `v.request`.
                let request = value.get("request").unwrap_or(value);
                let id = request
                    .get("id")
                    .or_else(|| request.get("approval_id"))
                    .or_else(|| request.get("approval"))
                    .and_then(Value::as_str);
                let operation = request
                    .get("summary")
                    .or_else(|| request.get("operation"))
                    .or_else(|| request.get("description"))
                    .and_then(Value::as_str)
                    .unwrap_or("-");
                if let Some(id) = id {
                    self.pending = Some(PendingApproval {
                        id: id.to_owned(),
                        request: operation.to_owned(),
                    });
                }
            }
            // Unknown kinds (plan D2/G3b): additive kinds must never panic
            // here; render a safe, kind-naming placeholder in the feed.
            _ => {
                self.operations.push(format!(
                    "unknown journal kind `{kind}` — no renderer yet (opaque passthrough)"
                ));
            }
        }
    }
}

/// One `evidence_summary` entry: path plus its hash, never a source blob.
fn evidence_line(entry: &Value) -> String {
    match entry {
        Value::String(path) => format!("evidence: {path}"),
        Value::Object(fields) => {
            let path = fields.get("path").and_then(Value::as_str).unwrap_or("?");
            match fields.get("hash").and_then(Value::as_str) {
                Some(hash) => format!("evidence: {path} #{hash}"),
                None => format!("evidence: {path}"),
            }
        }
        other => format!("evidence: {other}"),
    }
}

/// Compact, honest rendering of an optional JSON field: strings show
/// themselves, anything else shows its JSON, absent shows `-`.
fn json_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => "-".to_owned(),
    }
}

/// Parses one `ListTasks` row; rows without a valid task id are skipped
/// rather than shown wrong.
fn parse_task_row(row: &Value) -> Option<TaskRow> {
    let id = row.get("id")?.as_str()?.parse().ok()?;
    Some(TaskRow {
        id,
        status: row
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_owned(),
        objective: row
            .get("objective")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        revision: row.get("revision").and_then(Value::as_i64).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tachyon_protocol::{EventEnvelope, GatewayEvent, PROTOCOL_VERSION};
    use tachyon_types::{EventId, TaskId, Timestamp};

    use super::AppState;

    fn envelope(task_id: TaskId, seq: i64, event: GatewayEvent) -> EventEnvelope {
        EventEnvelope {
            seq,
            event_id: EventId::generate(),
            schema_version: PROTOCOL_VERSION,
            task_id,
            timestamp: Timestamp::now(),
            event,
        }
    }

    fn journal(task_id: TaskId, seq: i64, kind: &str, payload: Value) -> EventEnvelope {
        envelope(
            task_id,
            seq,
            GatewayEvent::Journal {
                kind: kind.to_owned(),
                payload,
            },
        )
    }

    fn snapshot(task_id: TaskId, seq: i64, status: &str, revision: u64) -> EventEnvelope {
        envelope(
            task_id,
            seq,
            GatewayEvent::TaskSnapshot {
                task_id,
                status: status.to_owned(),
                revision,
            },
        )
    }

    /// M11 board blocker (state-layer task guard): Enter-attach runs
    /// `reset_for` while old-task envelopes the reader already accepted
    /// sit in the 256-slot buffer. They drain afterwards — none may
    /// pollute the new task's state or inflate `last_applied_seq` (which
    /// would silently seq-drop the new task's replay), while the new
    /// task's full replay must still apply. The detached picker state
    /// (`attached_task == None`) keeps accepting envelopes as today.
    #[test]
    fn buffered_old_task_envelopes_after_the_switch_never_pollute_the_new_attach() {
        let first = TaskId::generate();
        let second = TaskId::generate();
        let mut state = AppState::new(Some(first));

        // The picker's Enter-attach wipes the display and repoints the
        // attach; buffered old-task frames may only arrive after this.
        state.reset_for(second);

        // Old-task envelopes draining late: journal + snapshot alike.
        state.apply_envelope(journal(
            first,
            9,
            "message",
            json!({"message": "stale old-task note"}),
        ));
        state.apply_envelope(snapshot(first, 9, "Paused", 3));
        assert!(
            state.conversation.is_empty(),
            "a late old-task journal line must not enter the new conversation: {:?}",
            state.conversation
        );
        assert_eq!(
            state.status, None,
            "a late old-task TaskSnapshot must not set the new task's status"
        );
        assert_eq!(
            state.last_applied_seq, -1,
            "a late old-task seq must not advance the new task's cursor"
        );

        // The new task's full replay then applies untouched (0, 1 from -1).
        state.apply_envelope(journal(
            second,
            0,
            "created",
            json!({"state": {"objective": "new objective", "status": "Created", "revision": 0}}),
        ));
        state.apply_envelope(journal(
            second,
            1,
            "message",
            json!({"message": "fresh note"}),
        ));
        assert_eq!(
            state.last_applied_seq, 1,
            "the new task's replay applies in full — nothing seq-dropped"
        );
        assert_eq!(
            state.objective.as_deref(),
            Some("new objective"),
            "the new task's created frame decodes"
        );
        assert!(
            state
                .conversation
                .iter()
                .any(|line| line.contains("fresh note")),
            "the new task's steering arrives: {:?}",
            state.conversation
        );

        // The detached picker state (None) keeps accepting envelopes.
        let mut picker = AppState::new(None);
        picker.apply_envelope(snapshot(first, 9, "Running", 4));
        picker.apply_envelope(journal(
            first,
            9,
            "message",
            json!({"message": "picker-side noise"}),
        ));
        assert_eq!(
            picker.status.as_deref(),
            Some("Running"),
            "detached state still applies envelopes — only an attach guards"
        );
    }

    /// G5: the `Subscribe` ack path stores the provider label when the
    /// gateway declares one (top-level `provider_label`, contract with
    /// the gateway writer), and an ack without the key declares nothing
    /// — absence keeps whatever is known (fresh state stays `None`, so
    /// the status bar renders no label and never panics).
    #[test]
    fn note_provider_label_stores_a_declared_label_and_absence_declares_nothing() {
        let mut state = AppState::new(Some(TaskId::generate()));
        state.note_provider_label(Some("scripted test/replay provider".to_owned()));
        assert_eq!(
            state.provider_label.as_deref(),
            Some("scripted test/replay provider"),
            "the declared label is stored for the header"
        );

        state.note_provider_label(None);
        assert_eq!(
            state.provider_label.as_deref(),
            Some("scripted test/replay provider"),
            "an ack without the key declares nothing — it does not erase"
        );

        assert!(
            AppState::new(Some(TaskId::generate()))
                .provider_label
                .is_none(),
            "fresh state has no label until a payload carries one"
        );
    }
}
