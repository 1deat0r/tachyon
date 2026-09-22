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

pub(crate) mod ownership;
pub mod runtime;
mod verification;
pub use tachyon_verify::AcceptanceContract;
pub use verification::VerificationState;

use std::str::FromStr;
use std::sync::Arc;

use ownership::{OwnedWorkers, TaskLifecycle, TaskOwnership};
use serde::{Deserialize, Serialize};
use tachyon_ir::ExecutionGraph;
use tachyon_store::{JournalEvent, StoreWriter, TaskRow};
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
    /// Lifecycle status.
    pub status: TaskStatus,
    /// Creation time.
    pub created_at: Timestamp,
    /// Last transition time.
    pub updated_at: Timestamp,
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
        constraints: Vec::new(),
        facts: Vec::new(),
        hypotheses: Vec::new(),
        open_questions: Vec::new(),
        acceptance: AcceptanceContract::default(),
        graph: ExecutionGraph::empty(task_id, 0),
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
        constraints: Vec::new(),
        facts: Vec::new(),
        hypotheses: Vec::new(),
        open_questions: Vec::new(),
        acceptance: AcceptanceContract::default(),
        graph: ExecutionGraph::empty(id, 0),
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
                self.control(TaskStatus::Cancelled, reply).await;
            }
            SupervisorCommand::DecideApproval {
                approval,
                granted,
                reason,
                reply,
            } => {
                let outcome = self
                    .transition_journalled(StateEvent::Approval {
                        approval,
                        granted,
                        reason,
                    })
                    .await;
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
        if self.state.status != TaskStatus::Paused {
            return Err(CoreError::IllegalTransition {
                from: self.state.status,
                to: TaskStatus::Created,
            });
        }
        // Milestone 2's scheduler will resume into Executing; until then the
        // only live status is Created.
        self.move_to(TaskStatus::Created).await
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
}
