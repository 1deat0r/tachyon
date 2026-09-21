//! Supervisor-owned verification. Only this module can commit completion.
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tachyon_ir::ExecutorKind;
use tachyon_tools::ToolsContext;
use tachyon_verify::{
    AcceptanceContract, HardRequirement, VerificationPlan, VerificationReport, VerificationRisk,
    VerifyError, WorkspaceSnapshot,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::{
    ConstraintStrength, CoreError, Loop, StateEvent, SupervisorCommand, SupervisorHandle,
    TaskState, TaskStatus, receive,
};

/// Durable baseline and latest run evidence. Never accepted from a model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationState {
    pub baseline: WorkspaceSnapshot,
    pub risk: VerificationRisk,
    pub in_progress: bool,
    /// Unknown effects after an interrupted verifier must be reconciled first.
    pub interrupted: bool,
    pub report: Option<VerificationReport>,
    pub error: Option<String>,
}

impl VerificationState {
    pub(super) fn new(baseline: WorkspaceSnapshot, risk: VerificationRisk) -> Self {
        Self {
            baseline,
            risk,
            in_progress: false,
            interrupted: false,
            report: None,
            error: None,
        }
    }
}

pub(super) struct ActiveVerification {
    cancel: CancellationToken,
    revision: u64,
    context: Arc<ToolsContext>,
    reply: oneshot::Sender<Result<TaskState, CoreError>>,
}

impl SupervisorHandle {
    /// Trusted runtime API: bind acceptance and a source baseline BEFORE mutation.
    /// This is intentionally not a model capability or gateway command. Once bound,
    /// neither model output nor a second configuration can weaken the contract.
    pub async fn configure_verification(
        &self,
        context: Arc<ToolsContext>,
        contract: AcceptanceContract,
        risk: VerificationRisk,
    ) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::ConfigureVerification {
            context,
            contract,
            risk,
            reply,
        })
        .await;
        receive(rx).await?
    }

    /// Run fresh executable checks and let the supervisor decide completion.
    /// There is no public "mark completed" or "accept this report" API.
    pub async fn verify_and_complete(
        &self,
        context: Arc<ToolsContext>,
    ) -> Result<TaskState, CoreError> {
        let (reply, rx) = oneshot::channel();
        self.send(SupervisorCommand::VerifyAndComplete { context, reply })
            .await;
        receive(rx).await?
    }
}

impl Loop {
    pub(super) async fn configure_verification(
        &mut self,
        context: Arc<ToolsContext>,
        contract: AcceptanceContract,
        risk: VerificationRisk,
    ) -> Result<TaskState, CoreError> {
        self.verification_status_allowed()?;
        if self.state.verification.is_some() || self.active.is_some() {
            return Err(blocked(
                "acceptance is already bound; cannot replace the baseline or required checks",
            ));
        }
        if !self.state.graph.nodes.is_empty() {
            return Err(blocked("bind acceptance before executing work"));
        }
        contract.validate()?;
        let baseline = capture(context).await?;
        self.transition_journalled(StateEvent::VerificationConfigured {
            contract,
            baseline,
            risk,
        })
        .await
    }

    fn verification_status_allowed(&self) -> Result<(), CoreError> {
        if !matches!(
            self.state.status,
            TaskStatus::Created | TaskStatus::Planning | TaskStatus::Executing
        ) {
            return Err(CoreError::IllegalTransition {
                from: self.state.status,
                to: TaskStatus::Verifying,
            });
        }
        Ok(())
    }

    async fn prepare_verification(
        &self,
        context: Arc<ToolsContext>,
    ) -> Result<VerificationPlan, CoreError> {
        self.verification_status_allowed()?;
        if self.active.is_some() {
            return Err(blocked("verification is already running"));
        }
        let Some(verification) = &self.state.verification else {
            return Err(blocked("no authoritative acceptance contract and baseline"));
        };
        if verification.interrupted || verification.in_progress {
            return Err(blocked(
                "interrupted verifier effects require reconciliation; no blind replay",
            ));
        }
        // M10 will supply terminal work/effect bookkeeping for general graphs.
        // Until then do not silently replace a graph containing unfinished work.
        if self
            .state
            .graph
            .nodes
            .values()
            .any(|n| n.executor != ExecutorKind::Verification)
        {
            return Err(blocked("non-verifier work has no terminal/effect evidence"));
        }
        let contract = self.state.acceptance.clone();
        let baseline = verification.baseline.clone();
        let risk = verification.risk;
        let task_id = self.state.id;
        let revision = self.state.revision;
        let hard = self
            .state
            .constraints
            .iter()
            .filter(|c| c.strength == ConstraintStrength::Hard)
            .map(|c| HardRequirement {
                id: c.id,
                text: c.text.clone(),
            })
            .collect::<Vec<_>>();
        tokio::task::spawn_blocking(move || {
            VerificationPlan::build_authorized(
                task_id, revision, &contract, &baseline, &hard, risk, &context,
            )
        })
        .await
        .map_err(|e| blocked(&format!("verification planner failed: {e}")))?
        .map_err(CoreError::from)
    }

    pub(super) async fn start_verification(
        &mut self,
        context: Arc<ToolsContext>,
        reply: oneshot::Sender<Result<TaskState, CoreError>>,
    ) {
        let plan = match self.prepare_verification(context.clone()).await {
            Ok(plan) => plan,
            Err(error) => {
                let _ = reply.send(Err(error));
                return;
            }
        };
        if let Err(error) = self
            .transition_journalled(StateEvent::VerificationStarted {
                graph: plan.graph().clone(),
            })
            .await
        {
            let _ = reply.send(Err(error));
            return;
        }
        // The durable Started record precedes any process effect.
        let cancel = CancellationToken::new();
        self.active = Some(ActiveVerification {
            cancel: cancel.clone(),
            revision: self.state.revision,
            context: context.clone(),
            reply,
        });
        self.jobs.spawn(tachyon_verify::run(plan, context, cancel));
    }

    pub(super) async fn finish_verification(
        &mut self,
        joined: Result<Result<VerificationReport, VerifyError>, tokio::task::JoinError>,
    ) {
        let Some(active) = self.active.take() else {
            return;
        };
        let result = match joined {
            Ok(result) => result.map_err(CoreError::from),
            Err(error) => Err(blocked(&format!("verifier worker failed: {error}"))),
        };
        let (report, error) = match result {
            Ok(report) => {
                let error = self.completion_error(&report, &active).await;
                (Some(report), error)
            }
            Err(error) => (None, Some(error.to_string())),
        };
        let completed = error.is_none();
        let event = StateEvent::VerificationFinished {
            report,
            error: error.clone(),
            completed,
        };
        let outcome = self.transition_journalled(event).await.and_then(|state| {
            error.map_or(Ok(state), |error| {
                Err(CoreError::VerificationBlocked(error))
            })
        });
        let _ = active.reply.send(outcome);
    }

    async fn completion_error(
        &self,
        report: &VerificationReport,
        active: &ActiveVerification,
    ) -> Option<String> {
        if self.state.status != TaskStatus::Verifying
            || self.state.revision != active.revision
            || report.revision() != active.revision
            || report.task_id() != self.state.id
        {
            return Some("stale/foreign verification result".into());
        }
        if !report.passed() {
            return Some(format!(
                "required acceptance failed: {}",
                report.failures().join("; ")
            ));
        }
        match capture(active.context.clone()).await {
            Ok(now) if report.snapshot().same_sources(&now) => None,
            Ok(_) => Some("sources changed after verification; fresh checks required".into()),
            Err(error) => Some(error.to_string()),
        }
    }

    pub(super) async fn stop_verification(&mut self) -> Result<(), CoreError> {
        let Some(active) = self.active.take() else {
            return Ok(());
        };
        active.cancel.cancel();
        if tokio::time::timeout(Duration::from_secs(5), self.jobs.join_next())
            .await
            .is_err()
        {
            self.jobs.abort_all();
        }
        while self.jobs.join_next().await.is_some() {}
        let _ = active.reply.send(Err(blocked(
            "verification interrupted by steering/cancellation",
        )));
        self.transition_journalled(StateEvent::VerificationInterrupted)
            .await?;
        Ok(())
    }
}

fn blocked(message: &str) -> CoreError {
    CoreError::VerificationBlocked(message.to_owned())
}

async fn capture(context: Arc<ToolsContext>) -> Result<WorkspaceSnapshot, CoreError> {
    tokio::task::spawn_blocking(move || WorkspaceSnapshot::capture_authorized(&context))
        .await
        .map_err(|e| blocked(&format!("source snapshot worker failed: {e}")))?
        .map_err(CoreError::from)
}
