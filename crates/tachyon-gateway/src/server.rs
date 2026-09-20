//! Local gateway server: socket lifecycle, frame loop, command dispatch.
//!
//! One `RunningGateway` owns the listener, the [`StoreWriter`], and the
//! supervisor registry. Each connection is served by its own task; slow
//! clients delay only themselves. Shutdown cancels the accept loop and
//! waits for in-flight connections before releasing the runtime dir.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tachyon_core::{CoreError, SupervisorHandle, TaskStatus, create_task, recover_task};
use tachyon_protocol::{
    Command, CommandResult, PROTOCOL_VERSION, RequestEnvelope, ResponseEnvelope, check_version,
    decode_frame, encode_frame,
};
use tachyon_store::StoreWriter;
use tachyon_types::{ApprovalId, SessionId, TaskId, WorkspaceId};
use thiserror::Error;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::endpoint::{
    ClaimPaths, EndpointError, claim_runtime_dir, release_runtime_dir, write_endpoint,
};

/// Errors produced by the gateway.
#[derive(Debug, Error)]
pub enum GatewayError {
    /// Endpoint claim/setup failure.
    #[error("endpoint error: {0}")]
    Endpoint(#[from] EndpointError),
    /// Durability failure.
    #[error("store error: {0}")]
    Store(#[from] tachyon_store::StoreError),
    /// Task kernel failure.
    #[error("core error: {0}")]
    Core(#[from] CoreError),
    /// Wire protocol failure.
    #[error("protocol error: {0}")]
    Protocol(#[from] tachyon_protocol::ProtocolError),
    /// Socket I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON failure.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// Another gateway owns the runtime.
    #[error("gateway already running (pid {pid})")]
    AlreadyRunning {
        /// Owning pid, when known.
        pid: u32,
    },
}

/// Shared mutable gateway state behind one async mutex.
struct GatewayState {
    store: Arc<StoreWriter>,
    supervisors: Mutex<HashMap<TaskId, SupervisorHandle>>,
}

/// A running gateway. Shut down explicitly; the runtime dir is released
/// after in-flight connections drain.
pub struct RunningGateway {
    socket_path: PathBuf,
    paths: ClaimPaths,
    shutdown: CancellationToken,
    accept_loop: tokio::task::JoinHandle<()>,
}

impl RunningGateway {
    /// Socket clients connect to.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Signals shutdown, waits for connections to drain, releases the
    /// runtime dir.
    pub async fn shutdown(self) {
        self.shutdown.cancel();
        let _ = self.accept_loop.await;
        release_runtime_dir(&self.paths);
    }
}

/// Starts a gateway on `data_dir`: claims the runtime dir, binds the
/// socket, opens the store, and recovers every incomplete task (spec §41).
pub async fn start(data_dir: &Path) -> Result<RunningGateway, GatewayError> {
    let paths = claim_runtime_dir(data_dir).await.map_err(|err| match err {
        EndpointError::AlreadyRunning { pid } => GatewayError::AlreadyRunning { pid },
        other => GatewayError::Endpoint(other),
    })?;
    let listener = UnixListener::bind(&paths.socket)?;
    write_endpoint(&paths)?;
    let store = Arc::new(StoreWriter::open(data_dir).await?);
    let state = Arc::new(GatewayState {
        store,
        supervisors: Mutex::new(HashMap::new()),
    });
    recover_incomplete(&state).await?;
    let shutdown = CancellationToken::new();
    let accept_loop = tokio::spawn(accept_loop(listener, state, shutdown.clone()));
    Ok(RunningGateway {
        socket_path: paths.socket.clone(),
        paths,
        shutdown,
        accept_loop,
    })
}

/// Spawns supervisors for every task that never reached a terminal state.
async fn recover_incomplete(state: &Arc<GatewayState>) -> Result<(), GatewayError> {
    for raw in state.store.incomplete_tasks().await? {
        let task_id = TaskId(Uuid::parse_str(&raw).map_err(|_| {
            GatewayError::Core(CoreError::Corrupt {
                detail: format!("invalid task id {raw:?}"),
            })
        })?);
        let handle = recover_task(task_id, state.store.clone()).await?;
        state.supervisors.lock().await.insert(task_id, handle);
    }
    Ok(())
}

async fn accept_loop(
    listener: UnixListener,
    state: Arc<GatewayState>,
    shutdown: CancellationToken,
) {
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        tokio::spawn(serve_connection(stream, state.clone(), shutdown.clone()));
                    }
                    Err(_) => {
                        if shutdown.is_cancelled() {
                            break;
                        }
                    }
                }
            }
        }
    }
}

async fn serve_connection(
    mut stream: UnixStream,
    state: Arc<GatewayState>,
    shutdown: CancellationToken,
) {
    loop {
        if shutdown.is_cancelled() {
            break;
        }
        let request = match read_request(&mut stream).await {
            Ok(request) => request,
            Err(ReadError::Eof | ReadError::Fatal) => break,
        };
        let response = match request {
            Ok(envelope) => dispatch(&state, &envelope).await,
            Err(response) => response,
        };
        if write_response(&mut stream, &response).await.is_err() {
            break;
        }
    }
}

enum ReadError {
    Eof,
    Fatal,
}

async fn read_request(
    stream: &mut UnixStream,
) -> Result<Result<RequestEnvelope, ResponseEnvelope>, ReadError> {
    let mut prefix = [0_u8; tachyon_protocol::FRAME_PREFIX_LEN];
    if let Err(err) = stream.read_exact(&mut prefix).await {
        return if err.kind() == std::io::ErrorKind::UnexpectedEof {
            Err(ReadError::Eof)
        } else {
            Err(ReadError::Fatal)
        };
    }
    let len = u32::from_le_bytes(prefix) as usize;
    if len > tachyon_protocol::MAX_FRAME_BYTES - tachyon_protocol::FRAME_PREFIX_LEN {
        return Err(ReadError::Fatal);
    }
    let mut payload = vec![0_u8; len];
    if stream.read_exact(&mut payload).await.is_err() {
        return Err(ReadError::Fatal);
    }
    let mut framed = prefix.to_vec();
    framed.extend_from_slice(&payload);
    match decode_frame::<RequestEnvelope>(&framed) {
        Ok((envelope, _)) => {
            if let Err(err) = check_version(envelope.protocol_version) {
                let response = ResponseEnvelope {
                    protocol_version: PROTOCOL_VERSION,
                    request_id: envelope.request_id,
                    result: CommandResult::Err {
                        code: "unsupported_version".to_owned(),
                        message: err.to_string(),
                    },
                };
                Ok(Err(response))
            } else {
                Ok(Ok(envelope))
            }
        }
        Err(_) => Err(ReadError::Fatal),
    }
}

async fn write_response(
    stream: &mut UnixStream,
    response: &ResponseEnvelope,
) -> std::io::Result<()> {
    match encode_frame(response) {
        Ok(bytes) => stream.write_all(&bytes).await,
        Err(_) => Err(std::io::Error::other("response too large")),
    }
}

async fn dispatch(state: &Arc<GatewayState>, envelope: &RequestEnvelope) -> ResponseEnvelope {
    let result = handle_command(state, &envelope.command).await;
    ResponseEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: envelope.request_id,
        result,
    }
}

fn ok(payload: Value) -> CommandResult {
    CommandResult::Ok { payload }
}

fn fail(code: &str, message: String) -> CommandResult {
    CommandResult::Err {
        code: code.to_owned(),
        message,
    }
}

fn core_err(error: &CoreError) -> CommandResult {
    let code = match error {
        CoreError::UnknownTask(_) => "unknown_task",
        CoreError::IllegalTransition { .. } => "illegal_transition",
        CoreError::MailboxFull => "mailbox_full",
        CoreError::SupervisorGone => "supervisor_gone",
        CoreError::Corrupt { .. } => "corrupt_state",
        CoreError::Store(_) | CoreError::Json(_) => "internal",
    };
    fail(code, error.to_string())
}

async fn handle_command(state: &Arc<GatewayState>, command: &Command) -> CommandResult {
    match command {
        Command::Ping => ok(json!({"pong": true, "protocol_version": PROTOCOL_VERSION})),
        Command::GetStatus => {
            let active = state.supervisors.lock().await.len();
            ok(json!({"protocol_version": PROTOCOL_VERSION, "active_tasks": active}))
        }
        Command::CreateSession => {
            let id = tachyon_types::SessionId::generate();
            match state.store.create_session(&id.to_string()).await {
                Ok(()) => ok(json!({"session_id": id.to_string()})),
                Err(err) => fail("internal", err.to_string()),
            }
        }
        Command::CreateTask {
            session_id,
            objective,
        } => create_supervised(state, *session_id, objective.clone()).await,
        Command::ListTasks { session_id } => {
            let filter = session_id.map(|id| id.to_string());
            match state.store.list_tasks(filter.as_deref()).await {
                Ok(tasks) => ok(json!({"tasks": tasks})),
                Err(err) => fail("internal", err.to_string()),
            }
        }
        Command::GetTask { task_id } => match supervisor_for(state, *task_id).await {
            Ok(handle) => match handle.get_state().await {
                Ok(task) => ok(json!({"task": task})),
                Err(error) => core_err(&error),
            },
            Err(result) => result,
        },
        Command::SendMessage { task_id, message } => {
            mutate(state, *task_id, |handle| async move {
                handle.add_message(message.clone()).await
            })
            .await
        }
        Command::PauseTask { task_id } => {
            mutate(
                state,
                *task_id,
                |handle| async move { handle.pause().await },
            )
            .await
        }
        Command::ResumeTask { task_id } => {
            mutate(
                state,
                *task_id,
                |handle| async move { handle.resume().await },
            )
            .await
        }
        Command::CancelTask { task_id } => {
            let outcome = mutate(
                state,
                *task_id,
                |handle| async move { handle.cancel().await },
            )
            .await;
            state.supervisors.lock().await.remove(task_id);
            outcome
        }
        Command::Approve { approval_id } => decide(state, *approval_id, true, String::new()),
        Command::Deny {
            approval_id,
            reason,
        } => decide(state, *approval_id, false, reason.clone()),
        Command::Subscribe { task_id, after_seq } => {
            match state
                .store
                .load_events_since(&task_id.to_string(), *after_seq)
                .await
            {
                Ok(events) => ok(json!({"events": events})),
                Err(err) => fail("internal", err.to_string()),
            }
        }
        Command::GetArtifact { .. } => fail(
            "not_implemented",
            "artifact spool arrives in Milestone 3".to_owned(),
        ),
    }
}

async fn create_supervised(
    state: &Arc<GatewayState>,
    session_id: SessionId,
    objective: String,
) -> CommandResult {
    if !state
        .store
        .session_exists(&session_id.to_string())
        .await
        .unwrap_or(false)
    {
        return fail("unknown_session", format!("no session {session_id}"));
    }
    match create_task(
        session_id,
        WorkspaceId::generate(),
        objective,
        state.store.clone(),
    )
    .await
    {
        Ok(handle) => {
            let task_id = handle.task_id();
            let status = TaskStatus::Created;
            state.supervisors.lock().await.insert(task_id, handle);
            ok(json!({"task_id": task_id.to_string(), "status": status.name()}))
        }
        Err(err) => core_err(&err),
    }
}

async fn supervisor_for(
    state: &Arc<GatewayState>,
    task_id: TaskId,
) -> Result<SupervisorHandle, CommandResult> {
    if let Some(handle) = state.supervisors.lock().await.get(&task_id) {
        return Ok(handle.clone());
    }
    match recover_task(task_id, state.store.clone()).await {
        Ok(handle) => {
            state
                .supervisors
                .lock()
                .await
                .insert(task_id, handle.clone());
            Ok(handle)
        }
        Err(CoreError::UnknownTask(_)) => Err(fail("unknown_task", format!("no task {task_id}"))),
        Err(other) => Err(core_err(&other)),
    }
}

async fn mutate<F, Fut>(state: &Arc<GatewayState>, task_id: TaskId, op: F) -> CommandResult
where
    F: FnOnce(SupervisorHandle) -> Fut,
    Fut: std::future::Future<Output = Result<tachyon_core::TaskState, CoreError>>,
{
    match supervisor_for(state, task_id).await {
        Ok(handle) => match op(handle).await {
            Ok(task) => ok(json!({"task": task})),
            Err(error) => core_err(&error),
        },
        Err(result) => result,
    }
}

fn decide(
    state: &Arc<GatewayState>,
    approval: ApprovalId,
    granted: bool,
    reason: String,
) -> CommandResult {
    // Approvals bind to tasks in Milestone 3; until then there is no pending
    // queue, so decisions have no target to attach to.
    let _ = (state, approval, granted, reason);
    fail(
        "not_implemented",
        "approval queue arrives in Milestone 3".to_owned(),
    )
}
