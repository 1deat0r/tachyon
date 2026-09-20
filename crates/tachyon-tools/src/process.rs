//! Direct process runner (spec §28, M3).
//!
//! Captures stdout and stderr concurrently (no pipe deadlock) with bounded
//! in-memory chunks; full streams spool to the artifact store. Output is
//! redacted through the [`CredentialBroker`] before it is returned.

use crate::artifact::ArtifactSpool;
use crate::credential::CredentialBroker;
use crate::{ToolError, ToolsContext, authorize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// Inline cap per stream: 1 MiB stays in memory, the rest lives in the
/// artifact store.
pub const INLINE_CAP: usize = 1024 * 1024;

/// What to run.
#[derive(Clone, Debug)]
pub struct ProcessSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub timeout: Duration,
}

impl ProcessSpec {
    #[must_use]
    pub fn new(program: &str) -> Self {
        Self {
            program: program.to_owned(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            timeout: Duration::from_secs(120),
        }
    }
}

/// What came back.
#[derive(Clone, Debug)]
pub struct ProcessReceipt {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub stdout_artifact: Option<tachyon_types::ArtifactId>,
    pub stderr_artifact: Option<tachyon_types::ArtifactId>,
}

/// Runs `spec` (policy `process.spawn`, scope = program name).
pub async fn run(context: &ToolsContext, spec: &ProcessSpec) -> Result<ProcessReceipt, ToolError> {
    let operation = serde_json::json!({
        "op": "process.spawn",
        "program": spec.program,
        "args": spec.args,
    });
    authorize(
        &context.policy,
        &context.approvals,
        "process.spawn",
        &spec.program,
        &operation,
        &format!("run {} {}", spec.program, spec.args.join(" ")),
    )?;
    let cwd = match &spec.cwd {
        Some(cwd) => Some(crate::resolve_scope(&context.workspace_root, cwd)?.0),
        None => None,
    };
    let mut command = tokio::process::Command::new(&spec.program);
    command.args(&spec.args);
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    // Drain both streams concurrently so a full pipe never deadlocks us.
    let out_handle = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(stream) = stdout.as_mut() {
            let _ = stream.read_to_end(&mut bytes).await;
        }
        bytes
    });
    let err_handle = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(stream) = stderr.as_mut() {
            let _ = stream.read_to_end(&mut bytes).await;
        }
        bytes
    });
    let status = tokio::time::timeout(spec.timeout, child.wait())
        .await
        .map_err(|_| {
            let _ = child.start_kill();
            ToolError::ProcessTimeout(spec.timeout)
        })?;
    let status = status?;
    let stdout = out_handle.await.unwrap_or_default();
    let stderr = err_handle.await.unwrap_or_default();
    finish(
        &context.artifacts,
        &context.credentials,
        status.code(),
        false,
        &stdout,
        &stderr,
    )
}

fn finish(
    spool: &ArtifactSpool,
    broker: &CredentialBroker,
    exit_code: Option<i32>,
    timed_out: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<ProcessReceipt, ToolError> {
    let redact = |bytes: &[u8]| broker.redact_bytes(bytes);
    let (stdout_inline, stdout_truncated, stdout_artifact) = split_stream(spool, stdout)?;
    let (stderr_inline, stderr_truncated, stderr_artifact) = split_stream(spool, stderr)?;
    Ok(ProcessReceipt {
        exit_code,
        timed_out,
        stdout: redact(&stdout_inline),
        stderr: redact(&stderr_inline),
        stdout_truncated,
        stderr_truncated,
        stdout_artifact,
        stderr_artifact,
    })
}

fn split_stream(
    spool: &ArtifactSpool,
    bytes: &[u8],
) -> Result<(Vec<u8>, bool, Option<tachyon_types::ArtifactId>), ToolError> {
    if bytes.len() <= INLINE_CAP {
        return Ok((bytes.to_vec(), false, None));
    }
    let id = spool.store(bytes)?;
    Ok((bytes[..INLINE_CAP].to_vec(), true, Some(id)))
}
