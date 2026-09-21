//! Scheduler-backed execution. Only executor-private evidence can pass a check.
#[cfg(all(test, unix))]
#[path = "runner_tests.rs"]
mod tests;
use crate::{Clause, VerificationPlan, VerifyError, WorkspaceSnapshot, plan::leaf};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tachyon_ir::{ExecutionNode, ExecutorKind, NodeStatus};
use tachyon_scheduler::{
    Budgets, Executor, ExecutorRegistry, NodeOutcome, ResolvedInputs, SchedulerHandle,
};
use tachyon_tools::{
    ToolsContext,
    process::{ProcessSpec, run_cancellable},
};
use tachyon_types::{ArtifactId, NodeId, TaskId};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReport {
    task_id: TaskId,
    revision: u64,
    binding: String,
    snapshot: WorkspaceSnapshot,
    failures: Vec<String>,
    checks: Vec<CheckEvidence>,
    // Deserialized evidence is historical, not fresh completion authority.
    #[serde(skip)]
    fresh: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckEvidence {
    node_id: NodeId,
    command_hash: Option<String>,
    status: NodeStatus,
    scheduler_status: Option<NodeStatus>,
    exit_code: Option<i32>,
    stdout_artifact: Option<ArtifactId>,
    stderr_artifact: Option<ArtifactId>,
    diagnostic: String,
}

impl VerificationReport {
    #[must_use]
    pub fn passed(&self) -> bool {
        self.fresh
            && self.failures.is_empty()
            && !self.checks.is_empty()
            && self.checks.iter().all(|check| {
                check.status == NodeStatus::Succeeded
                    && check.scheduler_status == Some(NodeStatus::Succeeded)
            })
    }
    #[must_use]
    pub fn failures(&self) -> &[String] {
        &self.failures
    }
    #[must_use]
    pub fn snapshot(&self) -> &WorkspaceSnapshot {
        &self.snapshot
    }
    #[must_use]
    pub fn task_id(&self) -> TaskId {
        self.task_id
    }
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub fn checks(&self) -> &[CheckEvidence] {
        &self.checks
    }
}

impl CheckEvidence {
    fn empty(node_id: NodeId) -> Self {
        Self {
            node_id,
            command_hash: None,
            status: NodeStatus::Failed,
            scheduler_status: None,
            exit_code: None,
            stdout_artifact: None,
            stderr_artifact: None,
            diagnostic: String::new(),
        }
    }
    #[must_use]
    pub fn status(&self) -> NodeStatus {
        self.status
    }
    #[must_use]
    pub fn diagnostic(&self) -> &str {
        &self.diagnostic
    }
    #[must_use]
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }
}

struct CheckRunner {
    plan: Arc<VerificationPlan>,
    context: Arc<ToolsContext>,
    evidence: Mutex<BTreeMap<NodeId, CheckEvidence>>,
}

#[derive(Default)]
struct Workers {
    closed: bool,
    tasks: tokio::task::JoinSet<()>,
}

struct VerificationExecutor {
    worker: Arc<CheckRunner>,
    workers: Arc<Mutex<Workers>>,
    scope: CancellationToken,
}

#[async_trait]
impl Executor for VerificationExecutor {
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Verification
    }
    async fn execute(
        &self,
        node: &ExecutionNode,
        inputs: ResolvedInputs,
        cancel: CancellationToken,
    ) -> NodeOutcome {
        let started = Instant::now();
        if cancel.is_cancelled() || self.scope.is_cancelled() {
            return NodeOutcome::cancelled(started.elapsed());
        }
        let token = self.scope.child_token();
        // The scheduler drops its execute future on cancellation or timeout.
        // This guard signals our owned worker instead of dropping its process.
        let _cancel_on_drop = token.clone().drop_guard();
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut workers = self.workers.lock().expect("verification workers poisoned");
            if workers.closed {
                return NodeOutcome::cancelled(started.elapsed());
            }
            let worker = self.worker.clone();
            let node = node.clone();
            workers.tasks.spawn(async move {
                let outcome = worker.execute_owned(&node, inputs, token).await;
                let _ = tx.send(outcome);
            });
        }
        rx.await.unwrap_or_else(|_| {
            NodeOutcome::failed("verification worker lost".into(), started.elapsed())
        })
    }
}

impl CheckRunner {
    async fn execute_owned(
        &self,
        node: &ExecutionNode,
        inputs: ResolvedInputs,
        cancel: CancellationToken,
    ) -> NodeOutcome {
        let started = Instant::now();
        if !inputs.is_empty() || self.plan.graph.nodes.get(&node.id) != Some(node) {
            return NodeOutcome::failed("unbound verification node".into(), started.elapsed());
        }
        if let Err(error) = self.plan.validate_node(node) {
            return NodeOutcome::failed(error.to_string(), started.elapsed());
        }
        let result = self.check(node, cancel).await;
        let mut evidence = CheckEvidence::empty(node.id);
        let outcome = match result {
            Ok(record) => {
                evidence = record;
                if evidence.status == NodeStatus::Succeeded {
                    NodeOutcome::success(serde_json::Map::new(), started.elapsed())
                } else {
                    NodeOutcome::failed(evidence.diagnostic.clone(), started.elapsed())
                }
            }
            Err(error) => {
                evidence.diagnostic = bounded(&error.to_string());
                NodeOutcome::failed(evidence.diagnostic.clone(), started.elapsed())
            }
        };
        self.evidence
            .lock()
            .expect("verification evidence poisoned")
            .insert(node.id, evidence);
        outcome
    }
}

impl CheckRunner {
    async fn check(
        &self,
        node: &ExecutionNode,
        cancel: CancellationToken,
    ) -> Result<CheckEvidence, VerifyError> {
        let clause = self
            .plan
            .checks
            .get(&node.id)
            .ok_or_else(|| VerifyError::Blocked("missing required check".into()))?;
        let mut record = CheckEvidence::empty(node.id);
        let before = capture(self.context.clone()).await?;
        if !self.plan.planned.same_sources(&before) {
            return Err(VerifyError::Blocked(
                "source drift before verification check".into(),
            ));
        }
        let Clause::CommandPasses { command } = leaf(clause) else {
            crate::plan::evaluate_clause(clause, &self.plan.baseline, &self.plan.planned)
                .map_err(VerifyError::Blocked)?;
            record.status = NodeStatus::Succeeded;
            return Ok(record);
        };
        if !self.plan.preflight_failures.is_empty() {
            return Err(VerifyError::Blocked(
                "required acceptance clauses already blocked".into(),
            ));
        }
        command.validate()?;
        let (resolved, scope) = resolve_command_target(&self.context, command)?;
        tachyon_tools::authorize(
            &self.context.policy,
            &self.context.approvals,
            "verify.command",
            &scope,
            &serde_json::json!({
                "invocation": node.invocation.args,
                "resolved_scope": scope,
            }),
            "execute required verification command",
        )
        .map_err(blocked)?;
        let spec = ProcessSpec {
            program: command.program.clone(),
            args: command.args.clone(),
            cwd: Some(resolved),
            env: command
                .env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            timeout: Duration::from_millis(command.timeout_ms),
        };
        let receipt = run_cancellable(&self.context, &spec, cancel)
            .await
            .map_err(|error| VerifyError::Blocked(error.to_string()))?;
        record.command_hash = Some(
            blake3::hash(
                &serde_json::to_vec(command)
                    .map_err(|error| VerifyError::InvalidContract(error.to_string()))?,
            )
            .to_hex()
            .to_string(),
        );
        record.exit_code = receipt.exit_code;
        record.status = if receipt.exit_code == Some(0) && !receipt.timed_out {
            NodeStatus::Succeeded
        } else {
            NodeStatus::Failed
        };
        record.diagnostic = if record.status == NodeStatus::Succeeded {
            String::new()
        } else {
            format!(
                "command exited {:?}: {}",
                receipt.exit_code,
                bounded(&String::from_utf8_lossy(&receipt.stderr))
            )
        };
        let spool = self.context.artifacts.clone();
        let (out, err) = tokio::task::spawn_blocking(move || {
            Ok::<_, tachyon_tools::ToolError>((
                receipt
                    .stdout_artifact
                    .map_or_else(|| spool.store(&receipt.stdout), Ok)?,
                receipt
                    .stderr_artifact
                    .map_or_else(|| spool.store(&receipt.stderr), Ok)?,
            ))
        })
        .await
        .map_err(|error| VerifyError::Blocked(error.to_string()))?
        .map_err(|error| VerifyError::Blocked(error.to_string()))?;
        let after = capture(self.context.clone()).await?;
        if !self.plan.planned.same_sources(&after) {
            record.status = NodeStatus::Failed;
            record.diagnostic = "source drift during verification command".into();
        }
        record.stdout_artifact = Some(out);
        record.stderr_artifact = Some(err);
        Ok(record)
    }
}

/// Executes fresh work, never cached model-supplied success claims.
pub async fn run(
    plan: VerificationPlan,
    context: Arc<ToolsContext>,
    cancel: CancellationToken,
) -> Result<VerificationReport, VerifyError> {
    plan.contract.validate()?;
    plan.validate_graph()?;
    let context_root = context.workspace_root.clone();
    let actual_root = tokio::task::spawn_blocking(move || context_root.canonicalize())
        .await
        .map_err(blocked)??;
    if actual_root != plan.baseline.root() {
        return Err(VerifyError::Blocked(
            "verification context root differs from baseline".into(),
        ));
    }
    // One canonical workspace runs one verification at a time process-wide.
    // Each run owns an independent scheduler whose grants cannot conflict
    // with another run's grants, so without this lease two supervisors could
    // execute conflicting `dir:/workspace/**` write claims concurrently.
    // The guard is held through scheduler shutdown and worker drain, so a
    // timed-out scheduler cannot release its grant while its process is
    // still handling TERM and the next run starts early.
    let lease = acquire_workspace_lease(&actual_root, &cancel).await?;
    let plan = Arc::new(plan);
    let worker = Arc::new(CheckRunner {
        plan: plan.clone(),
        context,
        evidence: Mutex::new(BTreeMap::new()),
    });
    let workers = Arc::new(Mutex::new(Workers::default()));
    let scope = cancel.child_token();
    let executor = Arc::new(VerificationExecutor {
        worker: worker.clone(),
        workers: workers.clone(),
        scope: scope.clone(),
    });
    let registry: ExecutorRegistry = HashMap::from([(
        ExecutorKind::Verification,
        executor.clone() as Arc<dyn Executor>,
    )]);
    let (handle, join) = tachyon_scheduler::spawn(Budgets::default(), registry);
    let owner = SchedulerOwner {
        handle: Some(handle),
        join: Some(join),
        workers,
        scope,
    };
    let scheduler = owner.handle.as_ref().expect("owned scheduler");
    scheduler
        .submit(plan.task_id, plan.graph.clone())
        .await
        .map_err(blocked)?;
    let budget = plan
        .graph
        .nodes
        .values()
        .map(|node| node.timeout.hard_ms.unwrap_or(30_000))
        .sum::<u64>()
        .saturating_add(5_000);
    let statuses = tokio::select! {
        biased;
        () = cancel.cancelled() => {
            scheduler.cancel_task(plan.task_id).await.map_err(blocked)?;
            scheduler.status(plan.task_id).await.map_err(blocked)?
        }
        status = scheduler.wait_finished(plan.task_id, Duration::from_millis(budget)) => status.map_err(blocked)?,
    };
    owner.close().await?;
    let snapshot = capture(worker.context.clone()).await?;
    let records = worker
        .evidence
        .lock()
        .expect("verification evidence poisoned");
    drop(lease);
    let mut failures = plan.preflight_failures.clone();
    if !plan.planned.same_sources(&snapshot) {
        failures.push("source drift after verification".into());
    }
    let mut checks = Vec::new();
    for id in plan.graph.nodes.keys() {
        let mut record = records
            .get(id)
            .cloned()
            .unwrap_or_else(|| CheckEvidence::empty(*id));
        record.scheduler_status = statuses.statuses.get(id).copied();
        if record.status != NodeStatus::Succeeded
            || record.scheduler_status != Some(NodeStatus::Succeeded)
            || statuses.attempts.get(id) != Some(&1)
        {
            failures.push(format!(
                "required check {id}: {:?}: {}",
                record.scheduler_status, record.diagnostic
            ));
        }
        checks.push(record);
    }
    Ok(VerificationReport {
        task_id: plan.task_id,
        revision: plan.revision,
        binding: plan.binding.clone(),
        snapshot,
        failures,
        checks,
        fresh: true,
    })
}

async fn capture(context: Arc<ToolsContext>) -> Result<WorkspaceSnapshot, VerifyError> {
    tokio::task::spawn_blocking(move || WorkspaceSnapshot::capture_authorized(&context))
        .await
        .map_err(blocked)?
}

/// Resolves a command cwd to its canonical target and policy scope.
/// Authorization and execution must both use this resolved target: the
/// lexical alias may point through a symlink at a denied directory.
fn resolve_command_target(
    context: &ToolsContext,
    command: &crate::CommandCheck,
) -> Result<(PathBuf, String), VerifyError> {
    let (resolved, _) =
        tachyon_tools::resolve_scope(&context.workspace_root, Path::new(&command.cwd))
            .map_err(blocked)?;
    let canonical_root = context.workspace_root.canonicalize().map_err(blocked)?;
    let relative = resolved
        .strip_prefix(&canonical_root)
        .map_err(|_| blocked("verification cwd escapes workspace"))?;
    let relative = relative
        .to_str()
        .ok_or_else(|| blocked("non-UTF8 verification cwd"))?
        .replace(std::path::MAIN_SEPARATOR, "/");
    let scope = if relative.is_empty() {
        "workspace/".to_owned()
    } else {
        format!("workspace/{relative}")
    };
    Ok((resolved, scope))
}

async fn acquire_workspace_lease(
    root: &Path,
    cancel: &CancellationToken,
) -> Result<tokio::sync::OwnedMutexGuard<()>, VerifyError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(VerifyError::Blocked(
            "verification cancelled before workspace lease".into(),
        )),
        guard = workspace_lock(root).lock_owned() => Ok(guard),
    }
}

fn bounded(value: &str) -> String {
    value.chars().take(2_048).collect()
}
fn blocked(error: impl std::fmt::Display) -> VerifyError {
    VerifyError::Blocked(error.to_string())
}

fn workspace_locks() -> &'static std::sync::Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>> {
    static LOCKS: OnceLock<std::sync::Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn workspace_lock(root: &Path) -> Arc<tokio::sync::Mutex<()>> {
    workspace_locks()
        .lock()
        .expect("workspace locks poisoned")
        .entry(root.to_path_buf())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

struct SchedulerOwner {
    handle: Option<SchedulerHandle>,
    join: Option<tokio::task::JoinHandle<()>>,
    workers: Arc<Mutex<Workers>>,
    scope: CancellationToken,
}
impl SchedulerOwner {
    async fn close(mut self) -> Result<(), VerifyError> {
        self.handle.take();
        if let Some(join) = self.join.as_mut() {
            join.await.map_err(blocked)?;
        }
        self.join.take();
        self.scope.cancel();
        let mut tasks = {
            let mut workers = self.workers.lock().expect("verification workers poisoned");
            workers.closed = true;
            std::mem::take(&mut workers.tasks)
        };
        while let Some(result) = tasks.join_next().await {
            result.map_err(blocked)?;
        }
        Ok(())
    }
}
impl Drop for SchedulerOwner {
    fn drop(&mut self) {
        self.scope.cancel();
        if let Some(join) = &self.join {
            join.abort();
        }
        if let Ok(mut workers) = self.workers.lock() {
            workers.closed = true;
            workers.tasks.abort_all();
        }
    }
}
