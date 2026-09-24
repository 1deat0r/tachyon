//! Tachyon Core.
//!
//! The Task Supervisor: single logical writer of canonical task state
//! (spec §3, §15). Every state transition is journalled through
//! [`tachyon_store::StoreWriter`] before the caller is answered, and
//! snapshots let a restarted process rebuild state from snapshot plus
//! journal tail (spec §18, §41).
//!
//! Lifecycle and M9 verification are supervisor-owned. M10 joins the
//! investigation/model/mutation graph to this same completion boundary.

#![warn(unsafe_code)]

pub mod driver;
pub mod ownership;
pub mod runtime;
mod verification;
pub use tachyon_verify::AcceptanceContract;
pub use verification::VerificationState;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use ownership::{OwnedWorkers, TaskLifecycle, TaskOwnership};
use serde::{Deserialize, Serialize};
use tachyon_ir::ExecutionGraph;
use tachyon_store::{ApprovalOutcome, JournalEvent, StoreWriter, TaskRow};
use tachyon_tools::ToolsContext;
use tachyon_types::{ApprovalId, SessionId, TaskId, Timestamp, WorkspaceId};
use tachyon_verify::{VerificationReport, VerificationRisk, VerifyError, WorkspaceSnapshot};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// Mailbox capacity per supervisor (spec §15: backpressure over growth).
pub const SUPERVISOR_MAILBOX: usize = 256;

/// Snapshot after this many journalled events since the last snapshot
/// (spec §18: initial policy of 100), plus every terminal transition.
pub const SNAPSHOT_EVERY_EVENTS: i64 = 100;

/// Errors produced by the task kernel.
#[derive(Debug, Error)]
pub enum CoreError {
    /// Durability failure.
    #[error("store error: {0}")]
    Store(#[from] tachyon_store::StoreError),
    /// JSON failure.
    #[error("state serialization error: {0}")]
    Json(#[from] serde_json::Error),
    /// No task (or no supervisor) for this id.
    #[error("unknown task: {0}")]
    UnknownTask(TaskId),
    /// Another live supervisor or draining worker owns this durable task.
    #[error("task already owned: {0}")]
    TaskAlreadyOwned(TaskId),
    /// Transition is not allowed from the current status.
    #[error("illegal transition from {from} to {to}")]
    IllegalTransition {
        /// Current status.
        from: TaskStatus,
        /// Requested status.
        to: TaskStatus,
    },
    /// Supervisor mailbox is full; the caller must back off and retry.
    #[error("supervisor mailbox full")]
    MailboxFull,
    /// Supervisor task ended before answering.
    #[error("supervisor gone")]
    SupervisorGone,
    /// Required verification is absent, failed, stale, or unresolved.
    #[error("completion blocked: {0}")]
    VerificationBlocked(String),
    #[error("verification: {0}")]
    Verification(#[from] VerifyError),
    /// Stored state does not parse.
    #[error("corrupt task state: {detail}")]
    Corrupt {
        /// What failed to parse.
        detail: String,
    },
    /// A run proposal arrived with a revision the task has moved past
    /// (steering or any other revision bump invalidates it).
    #[error("stale run proposal: task at revision {expected}, proposal bound to {got}")]
    StaleRunProposal {
        /// The task's current revision.
        expected: u64,
        /// The revision the proposal was bound to.
        got: u64,
    },
    /// A proposal claimed a different task id than the one owning this
    /// supervisor: foreign workers never write task state.
    #[error("foreign run proposal for task {task_id}")]
    ForeignProposal {
        /// The task id the proposal claimed.
        task_id: TaskId,
    },
    /// A record was proposed for a run this supervisor has not
    /// acknowledged (or has recovered past).
    #[error("unknown run: {run_id}")]
    UnknownRun {
        /// The run id the proposal claimed.
        run_id: String,
    },
    /// A second, different run was proposed while one is active.
    #[error("run already active: {active}")]
    RunAlreadyActive {
        /// The active run id.
        active: String,
    },
    /// An approval decision referenced an id with no approval row.
    #[error("no pending approval row for {approval}")]
    ApprovalMissing {
        /// The approval id that has no row.
        approval: ApprovalId,
    },
    /// An approval decision hit a row that is no longer `pending`
    /// (already decided, applied, or expired) — double decide is refused.
    #[error("approval {approval} is {decision}, not pending")]
    ApprovalNotPending {
        /// The approval id.
        approval: ApprovalId,
        /// The row's current decision state.
        decision: String,
    },
    /// A workspace root was pinned before; the pin is set once (M11
    /// item 5) and a second, different root is refused, never swapped.
    #[error("workspace already pinned to {pinned}")]
    WorkspaceAlreadyPinned {
        /// The root pinned when this task's run first started.
        pinned: String,
    },
}

/// Task lifecycle status (spec §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Just created; no routing yet.
    Created,
    /// Router is classifying (Milestone 5 owns this transition).
    Routing,
    /// Building the execution graph (Milestone 2+).
    Planning,
    /// Scheduler is running nodes (Milestone 2 owns this transition).
    Executing,
    /// Verifiers are gating completion (Milestone 9 owns this transition).
    Verifying,
    /// Blocked on a policy approval (Milestone 3 arms this).
    WaitingApproval,
    /// Paused by the user; nothing dispatches.
    Paused,
    /// Rebuilding state after a restart; transient.
    Recovering,
    /// Acceptance passed; terminal.
    Completed,
    /// Unrecoverable failure; terminal.
    Failed,
    /// Cancelled by the user; terminal.
    Cancelled,
}

impl TaskStatus {
    /// Terminal statuses accept no further commands except reads.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Canonical status name as stored in SQLite.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Created => "Created",
            Self::Routing => "Routing",
            Self::Planning => "Planning",
            Self::Executing => "Executing",
            Self::Verifying => "Verifying",
            Self::WaitingApproval => "WaitingApproval",
            Self::Paused => "Paused",
            Self::Recovering => "Recovering",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for TaskStatus {
    type Err = CoreError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "Created" => Ok(Self::Created),
            "Routing" => Ok(Self::Routing),
            "Planning" => Ok(Self::Planning),
            "Executing" => Ok(Self::Executing),
            "Verifying" => Ok(Self::Verifying),
            "WaitingApproval" => Ok(Self::WaitingApproval),
            "Paused" => Ok(Self::Paused),
            "Recovering" => Ok(Self::Recovering),
            "Completed" => Ok(Self::Completed),
            "Failed" => Ok(Self::Failed),
            "Cancelled" => Ok(Self::Cancelled),
            other => Err(CoreError::Corrupt {
                detail: format!("unknown task status {other:?}"),
            }),
        }
    }
}

/// Where a task constraint came from (spec §4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstraintSource {
    /// Stated by the user.
    User,
    /// Imposed by policy.
    Policy,
    /// Comes with the workspace.
    Workspace,
    /// Harness-level invariant.
    System,
    /// Inferred by the runtime.
    Derived,
}

/// Hard constraints gate IR validation and completion; preferences guide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstraintStrength {
    /// Cannot be weakened by model output.
    Hard,
    /// Advisory.
    Preference,
}

/// One task constraint (spec §4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskConstraint {
    /// Constraint identity.
    pub id: Uuid,
    /// Provenance.
    pub source: ConstraintSource,
    /// Constraint text.
    pub text: String,
    /// Enforcement strength.
    pub strength: ConstraintStrength,
    /// Revision that introduced it.
    pub created_revision: u64,
}

/// One established fact about the task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    /// Fact identity.
    pub id: Uuid,
    /// Fact text.
    pub text: String,
}

/// One working hypothesis under investigation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hypothesis {
    /// Hypothesis identity.
    pub id: Uuid,
    /// Hypothesis text.
    pub text: String,
}

/// One open question blocking or guiding the task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenQuestion {
    /// Question identity.
    pub id: Uuid,
    /// Question text.
    pub text: String,
}

/// Canonical task state: the supervisor is its only logical writer (spec §3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskState {
    /// Task identity.
    pub id: TaskId,
    /// Owning session.
    pub session_id: SessionId,
    /// Workspace under operation.
    pub workspace_id: WorkspaceId,
    /// Canonical workspace root pinned exactly once at `StartRun` before any
    /// policy or lease boundary (M11 plan item 5). `None` until the pin;
    /// journalled through the supervisor's single-writer path
    /// (`workspace_pinned`), so recovery replays it without a snapshot.
    #[serde(default)]
    pub workspace_root: Option<String>,
    /// User's objective.
    pub objective: String,
    /// Revision; bumped by steering (spec §5).
    pub revision: u64,
    /// Active constraints.
    pub constraints: Vec<TaskConstraint>,
    /// Established facts.
    pub facts: Vec<Fact>,
    /// Working hypotheses.
    pub hypotheses: Vec<Hypothesis>,
    /// Open questions.
    pub open_questions: Vec<OpenQuestion>,
    /// Completion terms.
    pub acceptance: AcceptanceContract,
    /// Baseline and latest executable evidence; absent on legacy tasks.
    #[serde(default)]
    pub verification: Option<VerificationState>,
    /// Validated execution graph (empty until Milestone 2 plans).
    pub graph: ExecutionGraph,
    /// Durable `stage` journal records (append-only, display-facing).
    /// Never bumps `revision`: steering owns that counter.
    #[serde(default)]
    pub stages: Vec<StageRecord>,
    /// Durable `evidence_summary` receipts: paths + content hashes, never
    /// source bytes (M11 item 7).
    #[serde(default)]
    pub evidence_summary: Vec<PathHash>,
    /// Durable `changed_files` receipts: paths + postimage hashes
    /// (M11 item 7).
    #[serde(default)]
    pub changed_files: Vec<PathHash>,
    /// Durable `agent_message` records: model answers (display-relevant
    /// text only, bounded by the writer; never source blobs) (M11 item 7).
    #[serde(default)]
    pub agent_messages: Vec<String>,
    /// Durable `approval_request` records: the parked ask, carried verbatim
    /// from [`tachyon_policy::ApprovalRequest`] (M11 item 8 / D4).
    #[serde(default)]
    pub approval_requests: Vec<tachyon_policy::ApprovalRequest>,
    /// Lifecycle status.
    pub status: TaskStatus,
    /// Creation time.
    pub created_at: Timestamp,
    /// Last transition time.
    pub updated_at: Timestamp,
}

/// One durable `stage` journal record: which runtime stage moved and a
/// display-facing detail line. Carries no source content, hashes, or
/// provider data — only what the live-operations feed shows (M11 item 7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageRecord {
    /// Stage name (`evidence`, `model`, `mutation`, `verify`, `run`).
    pub stage: String,
    /// Human-readable transition detail.
    pub detail: String,
}

/// One path + content-hash pair carried by `evidence_summary` and
/// `changed_files` receipts. Deliberately incapable of holding source
/// content: display/receipt data only (M11 item 7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathHash {
    /// Workspace-relative path in journal-key form.
    pub path: String,
    /// Content hash binding the recorded version.
    pub hash: String,
}

/// A worker's proposed durable record for an acknowledged run. The
/// supervisor validates the run/task/revision binding (M10 plan §2), then
/// journals it; only the five M11 item-7 display kinds may be proposed
/// here — `approval_request` is supervisor-owned via `park_approval`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunRecord {
    /// A stage transition for the live-operations feed.
    Stage {
        /// Stage name (`evidence`, `model`, `mutation`, `verify`).
        stage: String,
        /// Human-readable transition detail.
        detail: String,
    },
    /// Evidence actually supplied: paths + hashes, never source bytes.
    EvidenceSummary {
        /// `(path, hash)` receipts.
        entries: Vec<PathHash>,
    },
    /// Committed file changes: paths + postimage hashes.
    ChangedFiles {
        /// `(path, hash)` receipts.
        files: Vec<PathHash>,
    },
    /// A durable model answer (display-relevant text only).
    AgentMessage {
        /// The answer text.
        message: String,
    },
}

impl RunRecord {
    fn into_event(self) -> StateEvent {
        match self {
            Self::Stage { stage, detail } => StateEvent::Stage {
                record: StageRecord { stage, detail },
            },
            Self::EvidenceSummary { entries } => StateEvent::EvidenceSummary { entries },
            Self::ChangedFiles { files } => StateEvent::ChangedFiles { files },
            Self::AgentMessage { message } => StateEvent::AgentMessage { message },
        }
    }
}

/// One private, run-ID + task-ID + revision-bound worker proposal
/// (M10 plan §2). The supervisor acknowledges only an exact binding
/// against its live state; stale or foreign proposals are typed errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunProposal {
    /// Worker-minted run identity (must match an acknowledged `start_run`).
    pub run_id: String,
    /// Task the proposal claims; must own this supervisor.
    pub task_id: TaskId,
    /// Revision the worker observed when it was admitted to the run.
    pub revision: u64,
    /// The record to journal on acknowledgement.
    pub record: RunRecord,
}

/// Journal transition payloads. `Created` carries the full initial state so
/// recovery can rebuild even when no snapshot exists yet.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
enum StateEvent {
    Created {
        state: Box<TaskState>,
    },
    Message {
        message: String,
    },
    Constraint {
        constraint: TaskConstraint,
    },
    Status {
        from: TaskStatus,
        to: TaskStatus,
    },
    Approval {
        approval: ApprovalId,
        granted: bool,
        reason: String,
    },
    VerificationConfigured {
        contract: AcceptanceContract,
        baseline: WorkspaceSnapshot,
        risk: VerificationRisk,
    },
    VerificationStarted {
        graph: ExecutionGraph,
    },
    VerificationFinished {
        report: Option<VerificationReport>,
        error: Option<String>,
        completed: bool,
    },
    VerificationInterrupted,
    Stage {
        record: StageRecord,
    },
    EvidenceSummary {
        entries: Vec<PathHash>,
    },
    ChangedFiles {
        files: Vec<PathHash>,
    },
    AgentMessage {
        message: String,
    },
    ApprovalRequest {
        request: tachyon_policy::ApprovalRequest,
    },
    /// The canonical workspace root, pinned once at `StartRun` (M11 item 5).
    WorkspacePinned {
        /// Canonical path pinned into durable state.
        root: String,
    },
}

/// Commands the supervisor owns (spec §15). Node/provider events arrive
/// with Milestones 2 and 6; the enum grows then.
enum SupervisorCommand {
    AddUserMessage {
        message: String,
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    AddConstraint {
        text: String,
        strength: ConstraintStrength,
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    Pause {
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    Resume {
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    Cancel {
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    DecideApproval {
        approval: ApprovalId,
        granted: bool,
        reason: String,
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    GetState {
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    /// Pin the canonical workspace root into durable state exactly once
    /// (M11 plan item 5), journalled through this single-writer path.
    PinWorkspace {
        /// Canonical root to pin.
        root: String,
        /// Acknowledgement with the resulting state.
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    ConfigureVerification {
        context: Arc<ToolsContext>,
        contract: AcceptanceContract,
        risk: VerificationRisk,
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    VerifyAndComplete {
        context: Arc<ToolsContext>,
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    /// Worker start proposal: acked and journalled only when `revision`
    /// matches the live task revision and no other run is active.
    StartRun {
        /// Worker-minted run identity.
        run_id: String,
        /// Revision the worker observed.
        revision: u64,
        /// Acknowledgement.
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    /// Worker record proposal under an active run (M10 plan §2).
    Propose {
        /// The bound proposal.
        proposal: RunProposal,
        /// Acknowledgement after the journal accepts the record.
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
    /// A supervisor-owned job hit `ToolError::ApprovalRequired`: park the
    /// task, journal `approval_request`, insert the pending row, and hold
    /// the job until a decision resolves `resolution`.
    ParkApproval {
        /// Tools context owning the policy/approval registry for this job.
        context: Arc<ToolsContext>,
        /// The exact ask that blocked the operation.
        request: tachyon_policy::ApprovalRequest,
        /// Carries the decision back to the parked worker.
        resolution: oneshot::Sender<ApprovalResolution>,
        /// Acknowledgement once the park is durable.
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    },
}

/// How a parked approval resolved for the waiting worker (M11 item 8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalResolution {
    /// Granted; the durable row was flipped to `applied` before this
    /// message was sent, so the worker's single re-run is authorized.
    Granted,
    /// Denied; carries the recorded reason so the waiter receives the
    /// human's failure text.
    Denied {
        /// The recorded denial reason.
        reason: String,
    },
    /// The task was cancelled (or recovered) while waiting: the parked
    /// operation must never run.
    Cancelled,
}

/// The parked half of an approval wait, resolved by the supervisor's
/// decision (or cancellation). Dropping it never authorizes anything.
pub struct ApprovalWaiter {
    rx: oneshot::Receiver<ApprovalResolution>,
}

impl ApprovalWaiter {
    /// Waits for the supervisor's decision. `SupervisorGone` means no
    /// decision can arrive; it is never permission to run.
    pub async fn wait(self) -> Result<ApprovalResolution, CoreError> {
        self.rx.await.map_err(|_| CoreError::SupervisorGone)
    }
}

/// Cloneable handle to a running supervisor.
#[derive(Clone, Debug)]
pub struct SupervisorHandle {
    task_id: TaskId,
    tx: mpsc::Sender<SupervisorCommand>,
    lifecycle: TaskLifecycle,
}

impl SupervisorHandle {
    /// Task this handle drives.
    #[must_use]
    pub fn task_id(&self) -> TaskId {
        self.task_id
    }

    /// Current canonical state.
    pub async fn get_state(&self) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::GetState { reply }).await;
        receive(rx).await?
    }

    /// Pins the canonical workspace root into durable task state exactly
    /// once (M11 plan item 5), through the supervisor's single-writer
    /// journal path (`workspace_pinned`). Re-pinning the same root is
    /// idempotent; a different root is [`CoreError::WorkspaceAlreadyPinned`];
    /// a terminal task refuses the pin as an illegal transition. The pin
    /// never bumps `revision` (steering owns that counter).
    pub async fn pin_workspace_root(&self, root: String) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::PinWorkspace { root, reply })
            .await;
        receive(rx).await?
    }

    /// Steering message; bumps revision.
    pub async fn add_message(&self, message: String) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::AddUserMessage { message, reply })
            .await;
        receive(rx).await?
    }

    /// New constraint; bumps revision.
    pub async fn add_constraint(
        &self,
        text: String,
        strength: ConstraintStrength,
    ) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::AddConstraint {
            text,
            strength,
            reply,
        })
        .await;
        receive(rx).await?
    }

    /// Pauses dispatch.
    pub async fn pause(&self) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::Pause { reply }).await;
        receive(rx).await?
    }

    /// Resumes a paused task (back to `Created` until Milestone 2).
    pub async fn resume(&self) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::Resume { reply }).await;
        receive(rx).await?
    }

    /// Cancels the task; terminal.
    pub async fn cancel(&self) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::Cancel { reply }).await;
        receive(rx).await?
    }

    /// Records an approval decision (journaled; enforced in Milestone 3).
    pub async fn decide_approval(
        &self,
        approval: ApprovalId,
        granted: bool,
        reason: String,
    ) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::DecideApproval {
            approval,
            granted,
            reason,
            reply,
        })
        .await;
        receive(rx).await?
    }

    /// Worker side of the M10 plan §2 start proposal: bound to `run_id`
    /// and the caller-observed `revision`. The supervisor acknowledges and
    /// journals a `stage` record; a replay of the same run/revision is
    /// idempotent, a different active run or stale revision is a typed error.
    pub async fn start_run(&self, run_id: String, revision: u64) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::StartRun {
            run_id,
            revision,
            reply,
        })
        .await;
        receive(rx).await?
    }

    /// Proposes one durable record for the acknowledged run. The
    /// supervisor validates run-ID + task-ID + revision, journals the
    /// record, then acknowledges; stale/foreign/unknown proposals are
    /// typed errors with zero writes.
    pub async fn propose(&self, proposal: RunProposal) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::Propose { proposal, reply })
            .await;
        receive(rx).await?
    }

    /// Parks this supervisor-owned job on a human approval: transitions to
    /// `WaitingApproval`, journals `approval_request`, inserts the pending
    /// store row (`decision='pending'`, `decided_at=0`), then holds the
    /// job. The returned waiter resolves only through a supervisor
    /// decision or cancellation (M11 item 8 / D4).
    pub async fn park_approval(
        &self,
        context: Arc<ToolsContext>,
        request: tachyon_policy::ApprovalRequest,
    ) -> Result<ApprovalWaiter, CoreError> {
        let (resolution, rx) = oneshot::channel();
        let (reply, response) = oneshot::channel();
        self.send(SupervisorCommand::ParkApproval {
            context,
            request,
            resolution,
            reply,
        })
        .await;
        // Ack means the park (status + journal + pending row) is durable.
        receive(response).await??;
        Ok(ApprovalWaiter { rx })
    }

    /// Close command admission, cancel owned work and await exclusive-owner release.
    /// Idempotent across clones; retained handles fail closed after shutdown starts.
    /// Dropping this future does not revoke the shutdown request. This is not a
    /// task cancellation transition: already acknowledged durable state survives.
    /// Non-abortable work may delay release; timing out the caller's wait does
    /// not authorize a second owner or abandon that work.
    pub async fn shutdown(&self) -> Result<(), CoreError> {
        self.lifecycle.shutdown.cancel();
        self.lifecycle.released.cancelled().await;
        Ok(())
    }

    async fn send(&self, command: SupervisorCommand) {
        tokio::select! {
            biased;
            () = self.lifecycle.shutdown.cancelled() => {},
            result = self.tx.send(command) => { let _ = result; },
        }
    }
}

async fn receive(
    rx: oneshot::Receiver<Result<TaskState, CoreError>>,
) -> Result<Result<TaskState, CoreError>, CoreError> {
    rx.await.map_err(|_| CoreError::SupervisorGone)
}

/// Creates a task row plus supervisor, returning a live handle.
pub async fn create_task(
    session_id: SessionId,
    workspace_id: WorkspaceId,
    objective: String,
    store: Arc<StoreWriter>,
) -> Result<SupervisorHandle, CoreError> {
    let now = Timestamp::now();
    let task_id = TaskId::generate();
    let ownership = TaskOwnership::acquire(store.database_path(), task_id)?;
    let state = TaskState {
        id: task_id,
        session_id,
        workspace_id,
        objective: objective.clone(),
        revision: 0,
        workspace_root: None,
        constraints: Vec::new(),
        facts: Vec::new(),
        hypotheses: Vec::new(),
        open_questions: Vec::new(),
        acceptance: AcceptanceContract::default(),
        graph: ExecutionGraph::empty(task_id, 0),
        stages: Vec::new(),
        evidence_summary: Vec::new(),
        changed_files: Vec::new(),
        agent_messages: Vec::new(),
        approval_requests: Vec::new(),
        verification: None,
        status: TaskStatus::Created,
        created_at: now,
        updated_at: now,
    };
    state
        .graph
        .validate(task_id)
        .map_err(|err| CoreError::Corrupt {
            detail: format!("fresh graph invalid: {err}"),
        })?;
    let snapshot = serde_json::to_string(&state)?;
    let created = serde_json::to_string(&StateEvent::Created {
        state: Box::new(state.clone()),
    })?;
    store
        .create_task(
            &task_id.to_string(),
            &session_id.to_string(),
            &workspace_id.to_string(),
            &objective,
            TaskStatus::Created.name(),
            &snapshot,
            &created,
        )
        .await?;
    Ok(spawn(state, 0, Some(0), store, ownership))
}

/// Rebuilds a supervisor for an existing task: loads the snapshot, replays
/// the journal tail, marks the task `Recovering` during reconstruction,
/// then restores its pre-crash status.
///
/// Returns [`CoreError::TaskAlreadyOwned`] before reading any state if an actor
/// or draining worker still owns this task. Await [`SupervisorHandle::shutdown`]
/// before an in-process restart; merely dropping a handle is not a drain barrier.
pub async fn recover_task(
    task_id: TaskId,
    store: Arc<StoreWriter>,
) -> Result<SupervisorHandle, CoreError> {
    // Reserve before even reading a snapshot: a second writer must never
    // reconstruct stale state while the admitted actor advances its journal.
    let ownership = TaskOwnership::acquire(store.database_path(), task_id)?;
    let row = store
        .load_task(&task_id.to_string())
        .await?
        .ok_or(CoreError::UnknownTask(task_id))?;
    let (mut state, mut covered) = starting_state(&row)?;

    for event in store
        .load_events_since(&task_id.to_string(), covered)
        .await?
    {
        apply_journal(&mut state, &event)?;
        covered = event.seq;
    }
    // M11 item 8 restart-during-wait: stale `pending` approval rows are
    // expired by the supervisor. A task journalled in `WaitingApproval`
    // replayed cleanly just above; it must not resume the wait — the
    // decision is required again under a fresh request. `granted`-never-
    // `applied` orphans (crash between `decide` and `mark_applied`) expire
    // too: nothing could have executed, since execution needs the waiter
    // resolved after `applied`.
    for row in store.load_pending_for_task(&task_id.to_string()).await? {
        store.expire(&row.id).await?;
    }
    for row in store.load_granted_for_task(&task_id.to_string()).await? {
        store.expire_granted(&row.id).await?;
    }
    // M12 §19: reconcile still-`prepared` effect rows by idempotency
    // class. Keyed/Queryable/Compensatable stay `prepared` for the
    // re-entry path to retry or inspect; NonIdempotent/Unknown — and any
    // unrecognized class — become `unknown_after_crash` and are never
    // blindly replayed (fail-safe default).
    for effect in store.load_effects_for_task(&task_id.to_string()).await? {
        if effect.state != "prepared" {
            continue;
        }
        match effect.idempotency.as_str() {
            "Keyed" | "Queryable" | "Compensatable" | "Idempotent" | "Pure" => {}
            _ => {
                store.mark_effect_unknown_after_crash(&effect.id).await?;
            }
        }
    }
    // The journal tail, not stale task-row metadata, is recovery truth.
    state.updated_at = Timestamp::now();
    let interrupted = state.verification.as_ref().is_some_and(|v| v.in_progress);
    let snapshot_base = row.snapshot_seq;
    let mut app = Loop::new(
        state,
        row.snapshot_seq.unwrap_or(-1).max(covered),
        snapshot_base,
        store.clone(),
        ownership,
    );
    if interrupted {
        // Commands can have unknown effects; never silently replay after crash.
        app.transition_journalled(StateEvent::VerificationInterrupted)
            .await?;
    }
    if app.state.status == TaskStatus::WaitingApproval {
        // Adjudicated plan variant: restart lands in spec §41 Recovering,
        // never silently back inside the approval wait.
        app.transition_journalled(StateEvent::Status {
            from: TaskStatus::WaitingApproval,
            to: TaskStatus::Recovering,
        })
        .await?;
    }
    let snapshot_base = app.snapshot_base;
    Ok(spawn(
        app.state,
        app.covered,
        snapshot_base,
        store,
        app.ownership,
    ))
}

/// Snapshot state plus the sequence it covers.
fn starting_state(row: &TaskRow) -> Result<(TaskState, i64), CoreError> {
    if let (Some(json), Some(seq)) = (&row.snapshot_json, row.snapshot_seq) {
        let state: TaskState = serde_json::from_str(json).map_err(|err| CoreError::Corrupt {
            detail: format!("snapshot does not parse: {err}"),
        })?;
        return Ok((state, seq));
    }
    let id = parse_task(&row.id)?;
    let state = TaskState {
        id,
        session_id: parse_session(&row.session_id)?,
        workspace_id: parse_workspace(&row.workspace_id)?,
        objective: row.objective.clone(),
        revision: u64::try_from(row.revision).unwrap_or(0),
        workspace_root: None,
        constraints: Vec::new(),
        facts: Vec::new(),
        hypotheses: Vec::new(),
        open_questions: Vec::new(),
        acceptance: AcceptanceContract::default(),
        graph: ExecutionGraph::empty(id, 0),
        stages: Vec::new(),
        evidence_summary: Vec::new(),
        changed_files: Vec::new(),
        agent_messages: Vec::new(),
        approval_requests: Vec::new(),
        verification: None,
        status: TaskStatus::from_str(&row.status)?,
        created_at: Timestamp::from_micros(row.created_at),
        updated_at: Timestamp::from_micros(row.updated_at),
    };
    Ok((state, -1))
}

fn parse_task(raw: &str) -> Result<TaskId, CoreError> {
    uuid_parse(raw, "task id").map(TaskId)
}

fn parse_session(raw: &str) -> Result<SessionId, CoreError> {
    uuid_parse(raw, "session id").map(SessionId)
}

fn parse_workspace(raw: &str) -> Result<WorkspaceId, CoreError> {
    uuid_parse(raw, "workspace id").map(WorkspaceId)
}

fn uuid_parse(raw: &str, what: &str) -> Result<uuid::Uuid, CoreError> {
    Uuid::parse_str(raw).map_err(|_| CoreError::Corrupt {
        detail: format!("invalid {what} {raw:?}"),
    })
}

/// Replays one journal event onto `state`.
fn apply_journal(state: &mut TaskState, event: &JournalEvent) -> Result<(), CoreError> {
    let payload: StateEvent =
        serde_json::from_str(&event.payload).map_err(|err| CoreError::Corrupt {
            detail: format!("journal seq {} does not parse: {err}", event.seq),
        })?;
    match payload {
        StateEvent::Created { state: fresh } => {
            *state = *fresh;
        }
        StateEvent::Message { .. } => {
            state.revision += 1;
            if let Some(v) = &mut state.verification {
                v.report = None;
            }
        }
        StateEvent::Constraint { constraint } => {
            state.constraints.push(constraint);
            state.revision += 1;
            if let Some(v) = &mut state.verification {
                v.report = None;
            }
        }
        StateEvent::Status { to, .. } => {
            state.status = to;
        }
        StateEvent::Approval { .. } => {}
        StateEvent::VerificationConfigured {
            contract,
            baseline,
            risk,
        } => {
            state.acceptance = contract;
            state.verification = Some(VerificationState::new(baseline, risk));
            state.revision += 1;
        }
        StateEvent::VerificationStarted { graph } => {
            state.graph = graph;
            state.status = TaskStatus::Verifying;
            if let Some(v) = &mut state.verification {
                v.in_progress = true;
                v.report = None;
                v.error = None;
            }
        }
        StateEvent::VerificationFinished {
            report,
            error,
            completed,
        } => {
            if let Some(v) = &mut state.verification {
                v.in_progress = false;
                v.report = report;
                v.error = error;
            }
            state.status = if completed {
                TaskStatus::Completed
            } else {
                TaskStatus::Executing
            };
        }
        StateEvent::VerificationInterrupted => {
            if let Some(v) = &mut state.verification {
                v.in_progress = false;
                v.interrupted = true;
                v.report = None;
                v.error = Some("interrupted verifier; effects require reconciliation".into());
            }
            state.status = TaskStatus::Recovering;
        }
        // Append-only display record: no status, no revision, no
        // verification invalidation (only steering owns those).
        StateEvent::Stage { record } => {
            state.stages.push(record);
        }
        StateEvent::EvidenceSummary { entries } => {
            state.evidence_summary.extend(entries);
        }
        StateEvent::ChangedFiles { files } => {
            state.changed_files.extend(files);
        }
        StateEvent::AgentMessage { message } => {
            state.agent_messages.push(message);
        }
        StateEvent::ApprovalRequest { request } => {
            state.approval_requests.push(request);
        }
        StateEvent::WorkspacePinned { root } => {
            state.workspace_root = Some(root);
        }
    }
    Ok(())
}

/// Starts the supervisor loop for `state`, which already covers journal
/// sequence `covered`. `snapshot_base` is the sequence the last durable
/// snapshot covers; it must survive recovery so the 100-event cadence is
/// measured since the last snapshot, not since the last restart.
fn spawn(
    state: TaskState,
    covered: i64,
    snapshot_base: Option<i64>,
    store: Arc<StoreWriter>,
    ownership: TaskOwnership,
) -> SupervisorHandle {
    let task_id = state.id;
    let (tx, rx) = mpsc::channel(SUPERVISOR_MAILBOX);
    let lifecycle = ownership.lifecycle();
    tokio::spawn(run_loop(
        state,
        covered,
        snapshot_base,
        store,
        ownership,
        rx,
    ));
    SupervisorHandle {
        task_id,
        tx,
        lifecycle,
    }
}

pub(crate) struct Loop {
    pub(crate) ownership: TaskOwnership,
    state: TaskState,
    covered: i64,
    snapshot_base: Option<i64>,
    store: Arc<StoreWriter>,
    /// Owned effect workers. Every job carries its own private operation key and
    /// its workspace-lease guard; the actor never awaits one inside a handler.
    jobs: OwnedWorkers<verification::JobResult>,
    active: Option<verification::ActiveVerification>,
    /// Control acknowledgements that wait for the actual effect drain.
    drain: Option<verification::DrainAck>,
    /// The worker-minted run this supervisor has acknowledged (in-memory
    /// only: after a restart the worker must `start_run` afresh).
    active_run: Option<String>,
    /// Parked approval jobs keyed by approval id: each holds the exact
    /// ask, the job's tools context, and its decision channel.
    approvals: HashMap<ApprovalId, ParkedJob>,
}

/// One parked supervisor-owned job awaiting a human decision.
struct ParkedJob {
    /// The ask that blocked the operation (re-armed into the registry on
    /// a grant, one-shot per M11 item 8).
    request: tachyon_policy::ApprovalRequest,
    /// The job's tools context; its `Approvals` registry is armed here.
    context: Arc<ToolsContext>,
    /// Carries the decision back to the waiting worker.
    resolution: oneshot::Sender<ApprovalResolution>,
}

async fn run_loop(
    state: TaskState,
    covered: i64,
    snapshot_base: Option<i64>,
    store: Arc<StoreWriter>,
    ownership: TaskOwnership,
    mut rx: mpsc::Receiver<SupervisorCommand>,
) {
    let lifecycle = ownership.lifecycle();
    let _close_on_drop = lifecycle.shutdown.clone().drop_guard();
    let mut app = Loop::new(state, covered, snapshot_base, store, ownership);
    // The loop lives until every handle is dropped, so terminal tasks keep
    // answering reads and rejecting mutations with IllegalTransition.
    loop {
        tokio::select! {
            biased;
            () = lifecycle.shutdown.cancelled() => {
                rx.close();
                // Refuse queued commands too, rather than applying them after
                // shutdown has closed admission through every handle clone.
                drop(rx);
                app.stop_owned_work().await;
                break;
            }
            command = rx.recv() => {
                if let Some(command) = command {
                    app.handle(command).await;
                } else {
                    app.stop_owned_work().await;
                    break;
                }
            }
            joined = app.jobs.join_next(), if !app.jobs.is_empty() => {
                if let Some(joined) = joined { app.finish_job(joined).await; }
            }
        }
    }
}

/// Journal kind name for each transition.
fn event_kind(event: &StateEvent) -> &'static str {
    match event {
        StateEvent::Created { .. } => "created",
        StateEvent::Message { .. } => "message",
        StateEvent::Constraint { .. } => "constraint",
        StateEvent::Status { .. } => "status",
        StateEvent::Approval { .. } => "approval",
        StateEvent::VerificationConfigured { .. } => "verification_configured",
        StateEvent::VerificationStarted { .. } => "verification_started",
        StateEvent::VerificationFinished { .. } => "verification_finished",
        StateEvent::VerificationInterrupted => "verification_interrupted",
        StateEvent::Stage { .. } => "stage",
        StateEvent::EvidenceSummary { .. } => "evidence_summary",
        StateEvent::ChangedFiles { .. } => "changed_files",
        StateEvent::AgentMessage { .. } => "agent_message",
        StateEvent::ApprovalRequest { .. } => "approval_request",
        StateEvent::WorkspacePinned { .. } => "workspace_pinned",
    }
}

impl Loop {
    fn new(
        state: TaskState,
        covered: i64,
        snapshot_base: Option<i64>,
        store: Arc<StoreWriter>,
        ownership: TaskOwnership,
    ) -> Self {
        Self {
            jobs: OwnedWorkers::new(ownership.clone()),
            ownership,
            state,
            covered,
            snapshot_base,
            store,
            active: None,
            drain: None,
            active_run: None,
            approvals: HashMap::new(),
        }
    }

    async fn handle(&mut self, command: SupervisorCommand) {
        match command {
            SupervisorCommand::ConfigureVerification {
                context,
                contract,
                risk,
                reply,
            } => {
                self.configure_verification(context, contract, risk, reply);
            }
            SupervisorCommand::VerifyAndComplete { context, reply } => {
                self.start_verification(context, reply);
            }
            SupervisorCommand::GetState { reply } => {
                let _ = reply.send(Ok(self.state.clone()));
            }
            SupervisorCommand::PinWorkspace { root, reply } => {
                let outcome = self.pin_workspace(root).await;
                let _ = reply.send(outcome);
            }
            SupervisorCommand::AddUserMessage { message, reply } => {
                let outcome = self.steer(StateEvent::Message { message }).await;
                let _ = reply.send(outcome);
            }
            SupervisorCommand::AddConstraint {
                text,
                strength,
                reply,
            } => {
                let constraint = TaskConstraint {
                    id: Uuid::now_v7(),
                    source: ConstraintSource::User,
                    text,
                    strength,
                    created_revision: self.state.revision + 1,
                };
                let outcome = self.steer(StateEvent::Constraint { constraint }).await;
                let _ = reply.send(outcome);
            }
            SupervisorCommand::Pause { reply } => {
                self.control(TaskStatus::Paused, reply).await;
            }
            SupervisorCommand::Resume { reply } => {
                let outcome = self.resume().await;
                let _ = reply.send(outcome);
            }
            SupervisorCommand::Cancel { reply } => {
                // Cancel wins over an approval wait: the pending row is
                // expired and the parked worker is told `Cancelled` BEFORE
                // the terminal transition is acknowledged, so by the time
                // cancel() returns nothing can still be granted or run.
                if self.state.status == TaskStatus::WaitingApproval {
                    self.release_parked(ApprovalResolution::Cancelled).await;
                }
                self.control(TaskStatus::Cancelled, reply).await;
            }
            SupervisorCommand::DecideApproval {
                approval,
                granted,
                reason,
                reply,
            } => {
                let outcome = self.decide_approval(approval, granted, reason).await;
                let _ = reply.send(outcome);
            }
            SupervisorCommand::StartRun {
                run_id,
                revision,
                reply,
            } => {
                let outcome = self.start_run(run_id, revision).await;
                let _ = reply.send(outcome);
            }
            SupervisorCommand::Propose { proposal, reply } => {
                let outcome = self.propose(proposal).await;
                let _ = reply.send(outcome);
            }
            SupervisorCommand::ParkApproval {
                context,
                request,
                resolution,
                reply,
            } => {
                let outcome = self.park(context, request, resolution).await;
                let _ = reply.send(outcome);
            }
        }
    }

    /// Journals `event`, replays it onto a scratch copy (the single
    /// transition path shared with crash recovery), snapshots per policy,
    /// then commits the scratch copy as canonical. In-memory state only
    /// moves forward after the journal accepts the transition.
    async fn transition_journalled(&mut self, event: StateEvent) -> Result<TaskState, CoreError> {
        let target = match &event {
            StateEvent::Status { to, .. } => *to,
            _ => self.state.status,
        };
        if self.state.status.is_terminal() {
            return Err(CoreError::IllegalTransition {
                from: self.state.status,
                to: target,
            });
        }
        let payload = serde_json::to_string(&event)?;
        let mut next = self.state.clone();
        apply_journal(
            &mut next,
            &JournalEvent {
                seq: self.covered + 1,
                event_id: String::new(),
                schema_version: 1,
                kind: event_kind(&event).to_owned(),
                payload: payload.clone(),
                created_at: Timestamp::now().as_micros(),
            },
        )?;
        next.updated_at = Timestamp::now();
        let base = self.snapshot_base.unwrap_or(self.covered);
        let snapshot =
            if self.covered + 1 - base >= SNAPSHOT_EVERY_EVENTS || next.status.is_terminal() {
                Some(serde_json::to_string(&next)?)
            } else {
                None
            };
        let seq = self
            .store
            .append_transition(
                &next.id.to_string(),
                event_kind(&event),
                &payload,
                tachyon_store::TransitionState {
                    status: next.status.name(),
                    revision: i64::try_from(next.revision).unwrap_or(i64::MAX),
                    snapshot_json: snapshot.as_deref(),
                },
            )
            .await?;
        self.covered = seq;
        if snapshot.is_some() {
            self.snapshot_base = Some(seq);
        }
        self.state = next;
        Ok(self.state.clone())
    }

    async fn move_to(&mut self, to: TaskStatus) -> Result<TaskState, CoreError> {
        if to == TaskStatus::Completed {
            return Err(CoreError::VerificationBlocked(
                "only executable acceptance may complete a task".into(),
            ));
        }
        let from = self.state.status;
        if from.is_terminal() {
            return Err(CoreError::IllegalTransition { from, to });
        }
        if from == to {
            return Ok(self.state.clone());
        }
        self.transition_journalled(StateEvent::Status { from, to })
            .await
    }

    async fn resume(&mut self) -> Result<TaskState, CoreError> {
        // A pending control acknowledgement is still draining real effect
        // workers; resuming would dispatch beside them.
        if self.drain.is_some() || !self.jobs.is_empty() {
            return Err(CoreError::VerificationBlocked(
                "control acknowledgement is still draining owned effect workers".into(),
            ));
        }
        match self.state.status {
            // Milestone 2's scheduler will resume into Executing; until then
            // the only live status is Created.
            TaskStatus::Paused => self.move_to(TaskStatus::Created).await,
            // ADR 0002: no in-flight run to re-enter — land in Paused so
            // the normal Paused path applies. In-flight re-entry is the
            // gateway's ResumeTask handler (respawns drive), not here.
            TaskStatus::Recovering => self.move_to(TaskStatus::Paused).await,
            from => Err(CoreError::IllegalTransition {
                from,
                to: TaskStatus::Created,
            }),
        }
    }

    /// Durable set-once workspace pin (M11 item 5): same root again is
    /// idempotent, a different root is a typed refusal (never a swap), a
    /// terminal task fails as an illegal transition, and a first pin is
    /// journalled (`workspace_pinned`) so recovery replays it without a
    /// snapshot. No revision bump: steering owns the revision counter.
    async fn pin_workspace(&mut self, root: String) -> Result<TaskState, CoreError> {
        match &self.state.workspace_root {
            Some(pinned) if *pinned == root => Ok(self.state.clone()),
            Some(pinned) => Err(CoreError::WorkspaceAlreadyPinned {
                pinned: pinned.clone(),
            }),
            None => {
                self.transition_journalled(StateEvent::WorkspacePinned { root })
                    .await
            }
        }
    }

    /// M10 plan §2 start proposal. Acknowledged (and journalled as a
    /// `stage` record) only for the live revision with no other run
    /// active; a replay of the same run/revision is idempotent, so a
    /// crashed worker can re-propose without duplicating the journal.
    async fn start_run(&mut self, run_id: String, revision: u64) -> Result<TaskState, CoreError> {
        if revision != self.state.revision {
            return Err(CoreError::StaleRunProposal {
                expected: self.state.revision,
                got: revision,
            });
        }
        match self.active_run.as_deref() {
            Some(active) if active == run_id => return Ok(self.state.clone()),
            Some(active) => {
                return Err(CoreError::RunAlreadyActive {
                    active: active.to_owned(),
                });
            }
            None => {}
        }
        if self.state.status.is_terminal() {
            return Err(CoreError::IllegalTransition {
                from: self.state.status,
                to: self.state.status,
            });
        }
        let state = self
            .transition_journalled(StateEvent::Stage {
                record: StageRecord {
                    stage: "run".to_owned(),
                    detail: "started".to_owned(),
                },
            })
            .await?;
        self.active_run = Some(run_id);
        Ok(state)
    }

    /// Worker record proposal: exact run-ID + task-ID + revision binding
    /// against live state, then journal, then acknowledge. Any mismatch is
    /// a typed error with zero writes (stale steering discards the work).
    async fn propose(&mut self, proposal: RunProposal) -> Result<TaskState, CoreError> {
        let RunProposal {
            run_id,
            task_id,
            revision,
            record,
        } = proposal;
        if task_id != self.state.id {
            return Err(CoreError::ForeignProposal { task_id });
        }
        match self.active_run.as_deref() {
            Some(active) if active == run_id => {}
            _ => {
                return Err(CoreError::UnknownRun { run_id });
            }
        }
        if revision != self.state.revision {
            return Err(CoreError::StaleRunProposal {
                expected: self.state.revision,
                got: revision,
            });
        }
        self.transition_journalled(record.into_event()).await
    }

    /// The decision path (M11 item 8 / D4): validate the pending row
    /// first (missing or non-pending is a typed error — double decide can
    /// never overwrite), move the row `pending -> granted | denied` with
    /// the durable `applied` marker on a grant, THEN journal the decision
    /// (the journal describes rows that already moved, never the reverse),
    /// and arm the one-shot registry BEFORE the waiter may re-run anything.
    /// Only then does the wait resolve and the task leave
    /// `WaitingApproval`.
    async fn decide_approval(
        &mut self,
        approval: ApprovalId,
        granted: bool,
        reason: String,
    ) -> Result<TaskState, CoreError> {
        let row = self
            .store
            .load_by_id(&approval.to_string())
            .await?
            .ok_or(CoreError::ApprovalMissing { approval })?;
        if row.decision != "pending" {
            return Err(CoreError::ApprovalNotPending {
                approval,
                decision: row.decision,
            });
        }
        let job = self.approvals.remove(&approval);
        let outcome = if granted {
            ApprovalOutcome::Granted
        } else {
            ApprovalOutcome::Denied
        };
        // Durable row FIRST (R1 board B4): the journal may lag the row —
        // a crash then loses an audit event and recovery expires/re-asks
        // honestly — but it must never LEAD the row: a journalled grant
        // over a still-pending row would forge a decision the store never
        // recorded.
        self.store.decide(&approval.to_string(), outcome).await?;
        if granted {
            // Durable `applied` BEFORE any re-run can start (G6); the
            // registry grant authorizes exactly one authorize() success.
            self.store.mark_applied(&approval.to_string()).await?;
        }
        // The decision event now describes a row that already moved.
        self.transition_journalled(StateEvent::Approval {
            approval,
            granted,
            reason: reason.clone(),
        })
        .await?;
        if granted && let Some(job) = &job {
            job.context.approvals.decide(job.request.clone(), true);
        }
        // The wait is over either way: control returns to the worker with
        // execution resumed; the worker delivers the recorded failure on a
        // denial.
        if self.state.status == TaskStatus::WaitingApproval {
            self.transition_journalled(StateEvent::Status {
                from: TaskStatus::WaitingApproval,
                to: TaskStatus::Executing,
            })
            .await?;
        }
        if let Some(job) = job {
            let _ = job.resolution.send(if granted {
                ApprovalResolution::Granted
            } else {
                ApprovalResolution::Denied { reason }
            });
        }
        Ok(self.state.clone())
    }

    /// Releases every parked approval job with `resolution`, expiring its
    /// pending row first. Row expiry is best-effort at the store layer
    /// (`pending -> expired` only), so an already-decided row survives.
    async fn release_parked(&mut self, resolution: ApprovalResolution) {
        for (id, job) in std::mem::take(&mut self.approvals) {
            let _ = self.store.expire(&id.to_string()).await;
            let _ = job.resolution.send(resolution.clone());
        }
        // Defensive sweep: a pending row whose in-memory job is gone still
        // must not linger as decidable after the wait is released.
        if let Ok(rows) = self
            .store
            .load_pending_for_task(&self.state.id.to_string())
            .await
        {
            for row in rows {
                let _ = self.store.expire(&row.id).await;
            }
        }
    }

    /// Durable approval park (M11 item 8): `WaitingApproval` first, then
    /// the `approval_request` journal event, then the pending store row —
    /// the plan's order — and only then does the worker hold its job on
    /// `resolution`.
    async fn park(
        &mut self,
        context: Arc<ToolsContext>,
        request: tachyon_policy::ApprovalRequest,
        resolution: oneshot::Sender<ApprovalResolution>,
    ) -> Result<TaskState, CoreError> {
        // M12 fault point: kill here = durable park without a resolved waiter.
        tachyon_tools::fault::reach("approval.park").await;
        let from = self.state.status;
        if from != TaskStatus::WaitingApproval {
            self.transition_journalled(StateEvent::Status {
                from,
                to: TaskStatus::WaitingApproval,
            })
            .await?;
        }
        self.transition_journalled(StateEvent::ApprovalRequest {
            request: request.clone(),
        })
        .await?;
        self.store
            .insert_pending(
                &request.id.to_string(),
                &self.state.id.to_string(),
                &request.operation_hash,
            )
            .await?;
        self.approvals.insert(
            request.id,
            ParkedJob {
                request,
                context,
                resolution,
            },
        );
        Ok(self.state.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{ConstraintStrength, TaskStatus, create_task, recover_task};
    use super::{SessionId, WorkspaceId};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tachyon_store::StoreWriter;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[tokio::test]
    async fn aborting_actor_wrapper_retains_ownership_until_effect_worker_drains() {
        actor_failure_retains_ownership(false).await;
    }

    #[tokio::test]
    async fn panicking_actor_wrapper_retains_ownership_until_effect_worker_drains() {
        actor_failure_retains_ownership(true).await;
    }

    async fn actor_failure_retains_ownership(panic: bool) {
        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "owned effect".into(),
            store.clone(),
        )
        .await
        .unwrap();
        let state = handle.get_state().await.unwrap();
        handle.shutdown().await.unwrap();
        let owner = super::TaskOwnership::acquire(store.database_path(), state.id).unwrap();
        let lifecycle = owner.lifecycle();
        let mut app = super::Loop::new(state.clone(), 0, Some(0), store.clone(), owner);
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, blocked) = tokio::sync::oneshot::channel();
        let (finished, drained) = tokio::sync::oneshot::channel();
        let effect_path = dir.join("effect.receipt");
        let worker_path = effect_path.clone();
        app.jobs.spawn(async move {
            tokio::task::spawn_blocking(move || {
                let _ = entered.send(());
                let _ = blocked.blocking_recv();
                std::fs::write(worker_path, b"committed at safe boundary").unwrap();
                let _ = finished.send(());
            })
            .await
            .unwrap();
            super::verification::test_support::unowned_job(state.id)
        });
        let (fail, failure) = tokio::sync::oneshot::channel();
        let actor = tokio::spawn(async move {
            let _app = app;
            failure.await.unwrap();
            panic!("injected actor wrapper panic");
        });
        started.await.unwrap();
        if panic {
            fail.send(()).unwrap();
            assert!(actor.await.unwrap_err().is_panic());
        } else {
            actor.abort();
            assert!(actor.await.unwrap_err().is_cancelled());
        }
        let duplicate = super::TaskOwnership::acquire(store.database_path(), state.id);
        let admitted_early = duplicate.is_ok();
        drop(duplicate);
        assert!(!effect_path.exists());
        // Always release the real blocking worker before asserting the failure.
        release.send(()).unwrap();
        drained.await.unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            lifecycle.released.cancelled(),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(effect_path).unwrap(),
            b"committed at safe boundary"
        );
        store.close().await;
        std::fs::remove_dir_all(dir).unwrap();
        assert!(
            !admitted_early,
            "actor failure released ownership while its effect still ran"
        );
    }

    #[tokio::test]
    async fn shutdown_bypasses_full_mailbox_and_rejects_blocked_senders() {
        use std::future::Future as _;
        use std::task::Poll;
        let task_id = tachyon_types::TaskId::generate();
        let owner = super::TaskOwnership::acquire(
            &std::env::temp_dir().join("tachyon-full-mailbox.db"),
            task_id,
        )
        .unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let handle = super::SupervisorHandle {
            task_id,
            tx,
            lifecycle: owner.lifecycle(),
        };
        let (reply, _response) = tokio::sync::oneshot::channel();
        handle
            .tx
            .try_send(super::SupervisorCommand::GetState { reply })
            .ok()
            .unwrap();
        let mut command = Box::pin(handle.add_message("blocked admission".into()));
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(command.as_mut().poll(cx).is_pending())).await
        );
        let mut shutdown = Box::pin(handle.shutdown());
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(shutdown.as_mut().poll(cx).is_pending())).await
        );
        drop(shutdown);
        assert!(matches!(
            command.await,
            Err(super::CoreError::SupervisorGone)
        ));
        assert!(matches!(
            handle.get_state().await,
            Err(super::CoreError::SupervisorGone)
        ));
        assert!(!handle.lifecycle.released.is_cancelled());
        drop(owner);
        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dropping_last_client_does_not_leave_a_self_owned_sender() {
        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "drop".into(),
            store.clone(),
        )
        .await
        .unwrap();
        let id = handle.task_id();
        let lifecycle = handle.lifecycle.clone();
        drop(handle);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            lifecycle.released.cancelled(),
        )
        .await
        .unwrap();
        let recovered = recover_task(id, store.clone()).await.unwrap();
        recovered.shutdown().await.unwrap();
        store.close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }

    async fn open_test_store() -> (Arc<StoreWriter>, std::path::PathBuf) {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("tachyon-core-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(StoreWriter::open(&dir).await.unwrap());
        (store, dir)
    }

    #[tokio::test]
    async fn lifecycle_transitions_and_revisions() {
        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "probe".to_owned(),
            store.clone(),
        )
        .await
        .unwrap();

        let first = handle
            .add_message("first steering".to_owned())
            .await
            .unwrap();
        assert_eq!(first.revision, 1);
        let second = handle
            .add_constraint("no network".to_owned(), ConstraintStrength::Hard)
            .await
            .unwrap();
        assert_eq!(second.revision, 2);
        assert_eq!(second.constraints.len(), 1);

        let paused = handle.pause().await.unwrap();
        assert_eq!(paused.status, TaskStatus::Paused);
        let resumed = handle.resume().await.unwrap();
        assert_eq!(resumed.status, TaskStatus::Created);

        let cancelled = handle.cancel().await.unwrap();
        assert_eq!(cancelled.status, TaskStatus::Cancelled);
        let err = handle.add_message("too late".to_owned()).await.unwrap_err();
        assert!(matches!(err, super::CoreError::IllegalTransition { .. }));
        handle.shutdown().await.unwrap();
        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn recovery_rebuilds_state_and_continues() {
        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "durable".to_owned(),
            store.clone(),
        )
        .await
        .unwrap();
        let task_id = handle.task_id();
        handle.add_message("before crash".to_owned()).await.unwrap();
        handle.shutdown().await.unwrap();

        let recovered = recover_task(task_id, store.clone()).await.unwrap();
        let state = recovered.get_state().await.unwrap();
        assert_eq!(state.revision, 1);
        assert_eq!(state.objective, "durable");
        let continued = recovered
            .add_message("after crash".to_owned())
            .await
            .unwrap();
        assert_eq!(continued.revision, 2);
        recovered.shutdown().await.unwrap();
        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn snapshot_policy_advances_the_base() {
        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "many".to_owned(),
            store.clone(),
        )
        .await
        .unwrap();
        for index in 0..105 {
            handle.add_message(format!("note {index}")).await.unwrap();
        }
        let row = store
            .load_task(&handle.task_id().to_string())
            .await
            .unwrap()
            .unwrap();
        assert!(row.snapshot_seq.unwrap_or(-1) >= 100);
        handle.shutdown().await.unwrap();
        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn snapshot_cadence_survives_recovery() {
        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "cadence".to_owned(),
            store.clone(),
        )
        .await
        .unwrap();
        let task_id = handle.task_id();
        for index in 0..99 {
            handle.add_message(format!("note {index}")).await.unwrap();
        }
        let row = store
            .load_task(&task_id.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.snapshot_seq, Some(0));
        handle.shutdown().await.unwrap();
        // Restart must not reset the cadence to the journal tail: the 100th
        // transition since snapshot 0 still snapshots.
        let recovered = recover_task(task_id, store.clone()).await.unwrap();
        recovered
            .add_message("across restart".to_owned())
            .await
            .unwrap();
        let row = store
            .load_task(&task_id.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.snapshot_seq, Some(100));
        recovered.shutdown().await.unwrap();
        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ---- M11 item 7: durable journal vocabulary --------------------------

    #[tokio::test]
    async fn stage_events_replay_into_durable_stage_records() {
        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "vocab".to_owned(),
            store.clone(),
        )
        .await
        .unwrap();
        let task_id = handle.task_id();
        let event = super::StateEvent::Stage {
            record: super::StageRecord {
                stage: "evidence".to_owned(),
                detail: "collected 5 files".to_owned(),
            },
        };
        store
            .append_event(
                &task_id.to_string(),
                super::event_kind(&event),
                &serde_json::to_string(&event).unwrap(),
            )
            .await
            .unwrap();
        handle.shutdown().await.unwrap();

        let recovered = recover_task(task_id, store.clone()).await.unwrap();
        let state = recovered.get_state().await.unwrap();
        assert_eq!(
            state.stages,
            vec![super::StageRecord {
                stage: "evidence".to_owned(),
                detail: "collected 5 files".to_owned(),
            }]
        );
        assert_eq!(
            state.revision, 0,
            "display records must not bump steering revision"
        );
        assert_eq!(state.status, TaskStatus::Created);
        recovered.shutdown().await.unwrap();
        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn evidence_summary_events_replay_paths_and_hashes_never_bytes() {
        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "vocab".to_owned(),
            store.clone(),
        )
        .await
        .unwrap();
        let task_id = handle.task_id();
        let entries = vec![super::PathHash {
            path: "src/lib.rs".to_owned(),
            hash: "blake3-of-content".to_owned(),
        }];
        let event = super::StateEvent::EvidenceSummary {
            entries: entries.clone(),
        };
        store
            .append_event(
                &task_id.to_string(),
                super::event_kind(&event),
                &serde_json::to_string(&event).unwrap(),
            )
            .await
            .unwrap();
        // The payload must carry paths + hashes only — no byte content key.
        let payload: serde_json::Value = serde_json::from_str(
            &store
                .load_events_since(&task_id.to_string(), 0)
                .await
                .unwrap()[0]
                .payload,
        )
        .unwrap();
        let serialized = serde_json::to_string(&payload).unwrap();
        assert!(serialized.contains("src/lib.rs"));
        assert!(
            !serialized.contains("bytes"),
            "evidence payloads never carry source blobs"
        );
        handle.shutdown().await.unwrap();

        let recovered = recover_task(task_id, store.clone()).await.unwrap();
        let state = recovered.get_state().await.unwrap();
        assert_eq!(state.evidence_summary, entries);
        assert_eq!(state.revision, 0);
        recovered.shutdown().await.unwrap();
        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn changed_files_receipts_replay_paths_and_hashes() {
        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "vocab".to_owned(),
            store.clone(),
        )
        .await
        .unwrap();
        let task_id = handle.task_id();
        let files = vec![super::PathHash {
            path: "auth-session/src/session.rs".to_owned(),
            hash: "postimage-blake3".to_owned(),
        }];
        let event = super::StateEvent::ChangedFiles {
            files: files.clone(),
        };
        store
            .append_event(
                &task_id.to_string(),
                super::event_kind(&event),
                &serde_json::to_string(&event).unwrap(),
            )
            .await
            .unwrap();
        handle.shutdown().await.unwrap();

        let recovered = recover_task(task_id, store.clone()).await.unwrap();
        let state = recovered.get_state().await.unwrap();
        assert_eq!(state.changed_files, files);
        assert_eq!(state.revision, 0);
        recovered.shutdown().await.unwrap();
        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn agent_message_events_replay_as_durable_model_answers() {
        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "vocab".to_owned(),
            store.clone(),
        )
        .await
        .unwrap();
        let task_id = handle.task_id();
        let event = super::StateEvent::AgentMessage {
            message: "the stale-refresh guard was missing a generation check".to_owned(),
        };
        store
            .append_event(
                &task_id.to_string(),
                super::event_kind(&event),
                &serde_json::to_string(&event).unwrap(),
            )
            .await
            .unwrap();
        handle.shutdown().await.unwrap();

        let recovered = recover_task(task_id, store.clone()).await.unwrap();
        let state = recovered.get_state().await.unwrap();
        assert_eq!(
            state.agent_messages,
            vec!["the stale-refresh guard was missing a generation check".to_owned()]
        );
        assert_eq!(state.revision, 0, "model answers are not user steering");
        recovered.shutdown().await.unwrap();
        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn approval_request_events_replay_the_full_ask_payload() {
        use tachyon_policy::ApprovalRequest;
        use tachyon_types::CapabilityId;

        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "vocab".to_owned(),
            store.clone(),
        )
        .await
        .unwrap();
        let task_id = handle.task_id();
        let request = ApprovalRequest {
            id: tachyon_types::ApprovalId::generate(),
            capability: CapabilityId("mutation.patch".to_owned()),
            scope: "workspace/src/lib.rs".to_owned(),
            operation_hash: "blake3-op".to_owned(),
            summary: "patch src/lib.rs".to_owned(),
        };
        let event = super::StateEvent::ApprovalRequest {
            request: request.clone(),
        };
        assert_eq!(super::event_kind(&event), "approval_request");
        store
            .append_event(
                &task_id.to_string(),
                super::event_kind(&event),
                &serde_json::to_string(&event).unwrap(),
            )
            .await
            .unwrap();
        handle.shutdown().await.unwrap();

        let recovered = recover_task(task_id, store.clone()).await.unwrap();
        let state = recovered.get_state().await.unwrap();
        assert_eq!(state.approval_requests, vec![request]);
        assert_eq!(state.revision, 0);
        recovered.shutdown().await.unwrap();
        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Plan item 7: create -> append (all five kinds) -> recover leaves
    /// state identical to a from-scratch deterministic replay of the same
    /// journal, and replaying twice yields byte-identical state.
    #[tokio::test]
    async fn create_append_recover_round_trip_is_deterministic_and_identical() {
        use tachyon_policy::ApprovalRequest;
        use tachyon_types::CapabilityId;

        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "round trip".to_owned(),
            store.clone(),
        )
        .await
        .unwrap();
        let task_id = handle.task_id();
        handle.shutdown().await.unwrap();

        let events = vec![
            super::StateEvent::Stage {
                record: super::StageRecord {
                    stage: "evidence".to_owned(),
                    detail: "started".to_owned(),
                },
            },
            super::StateEvent::EvidenceSummary {
                entries: vec![super::PathHash {
                    path: "a.rs".to_owned(),
                    hash: "h1".to_owned(),
                }],
            },
            super::StateEvent::ChangedFiles {
                files: vec![super::PathHash {
                    path: "a.rs".to_owned(),
                    hash: "h2".to_owned(),
                }],
            },
            super::StateEvent::AgentMessage {
                message: "guarded the generation check".to_owned(),
            },
            super::StateEvent::ApprovalRequest {
                request: ApprovalRequest {
                    id: tachyon_types::ApprovalId::generate(),
                    capability: CapabilityId("mutation.patch".to_owned()),
                    scope: "workspace/a.rs".to_owned(),
                    operation_hash: "op".to_owned(),
                    summary: "patch a.rs".to_owned(),
                },
            },
        ];
        for event in &events {
            let kind = super::event_kind(event);
            let seq = store
                .append_event(
                    &task_id.to_string(),
                    kind,
                    &serde_json::to_string(event).unwrap(),
                )
                .await
                .unwrap();
            // Journal schema_version stays 1 for every new kind.
            let row = &store
                .load_events_since(&task_id.to_string(), seq - 1)
                .await
                .unwrap()[0];
            assert_eq!(row.schema_version, 1);
            assert_eq!(row.kind, kind);
        }

        // Recover from snapshot + tail: identical to replaying the journal
        // twice from the same base (deterministic apply).
        let recovered = recover_task(task_id, store.clone()).await.unwrap();
        let recovered_state = recovered.get_state().await.unwrap();
        recovered.shutdown().await.unwrap();

        let row = store
            .load_task(&task_id.to_string())
            .await
            .unwrap()
            .unwrap();
        let (base, _) = super::starting_state(&row).unwrap();
        let journal = store
            .load_events_since(&task_id.to_string(), -1)
            .await
            .unwrap();
        let mut pass_one = base.clone();
        let mut pass_two = base;
        for event in &journal {
            super::apply_journal(&mut pass_one, event).unwrap();
            super::apply_journal(&mut pass_two, event).unwrap();
        }
        assert_eq!(pass_one, pass_two, "replay must be deterministic");
        let mut normalized = recovered_state.clone();
        normalized.updated_at = pass_one.updated_at;
        assert_eq!(
            normalized, pass_one,
            "recovery must leave exactly the replayed state"
        );
        assert_eq!(pass_one.stages.len(), 1);
        assert_eq!(pass_one.evidence_summary.len(), 1);
        assert_eq!(pass_one.changed_files.len(), 1);
        assert_eq!(pass_one.agent_messages.len(), 1);
        assert_eq!(pass_one.approval_requests.len(), 1);
        assert_eq!(pass_one.revision, 0);

        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Fail-closed invariant: a kind/payload this build does not know still
    /// recovers as `Corrupt`, never as silent success (plan item 7 tail).
    #[tokio::test]
    async fn unknown_journal_kind_still_fails_closed_as_corrupt() {
        let (store, dir) = open_test_store().await;
        let session = SessionId::generate();
        store.create_session(&session.to_string()).await.unwrap();
        let handle = create_task(
            session,
            WorkspaceId::generate(),
            "unknown kind".to_owned(),
            store.clone(),
        )
        .await
        .unwrap();
        let task_id = handle.task_id();
        handle.shutdown().await.unwrap();
        store
            .append_event(
                &task_id.to_string(),
                "future_kind",
                "{\"t\":\"future_kind\",\"v\":{}}",
            )
            .await
            .unwrap();

        let err = recover_task(task_id, store.clone()).await.unwrap_err();
        assert!(
            matches!(err, super::CoreError::Corrupt { .. }),
            "expected Corrupt, got {err:?}"
        );
        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
