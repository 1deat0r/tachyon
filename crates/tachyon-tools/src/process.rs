//! Direct process runner (spec §29).
//!
//! Captures stdout and stderr concurrently (no pipe deadlock). Whole streams
//! currently buffer in memory; `INLINE_CAP` bounds receipts, not capture memory.
//! Output is redacted through the [`CredentialBroker`] before persistence or
//! truncation, so secrets crossing the inline boundary cannot leak to artifacts.

use crate::artifact::ArtifactSpool;
use crate::credential::CredentialBroker;
use crate::{ToolError, ToolsContext, authorize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;
#[cfg(any(unix, test))]
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

/// Inline receipt cap per stream. Full capture is currently buffered in memory
/// before redaction and artifact storage; this is not a capture memory bound.
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
    run_cancellable(context, spec, CancellationToken::new()).await
}

/// Runs an owned process group. Already-cancelled requests never spawn. A
/// timeout includes inherited stdout/stderr pipes, not just the leader's life.
/// Cancellation/timeout sends TERM, allows 100 ms grace, then sends KILL and
/// reaps the immediate child. Dropping/aborting this future sends KILL without
/// awaiting; immediate-child reaping then relies on Tokio's best-effort reaper.
/// Unsupported platforms fail closed until equivalent tree ownership exists.
pub async fn run_cancellable(
    context: &ToolsContext,
    spec: &ProcessSpec,
    cancel: CancellationToken,
) -> Result<ProcessReceipt, ToolError> {
    if cancel.is_cancelled() {
        return Err(ToolError::ProcessCancelled);
    }
    let cwd = tachyon_policy::contain(
        &context.workspace_root,
        spec.cwd.as_deref().unwrap_or(Path::new(".")),
    )?;
    // Bind the inherited environment too, and execute this exact snapshot.
    let mut env: BTreeMap<std::ffi::OsString, std::ffi::OsString> = std::env::vars_os().collect();
    env.extend(
        spec.env
            .iter()
            .map(|(key, value)| (key.into(), value.into())),
    );
    let encoded_env: Vec<_> = env
        .iter()
        .map(|(key, value)| (key.as_encoded_bytes(), value.as_encoded_bytes()))
        .collect();
    let operation = serde_json::json!({
        "op": "process.spawn",
        "program": spec.program,
        "args": spec.args,
        "cwd": cwd.as_os_str().as_encoded_bytes(),
        "env": encoded_env,
        "timeout": { "secs": spec.timeout.as_secs(), "nanos": spec.timeout.subsec_nanos() },
    });
    authorize(
        &context.policy,
        &context.approvals,
        "process.spawn",
        &spec.program,
        &operation,
        &format!("run {} {}", spec.program, spec.args.join(" ")),
    )?;
    let mut command = tokio::process::Command::new(&spec.program);
    command.args(&spec.args);
    command.env_clear().envs(env);
    command.current_dir(cwd);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let (status, stdout, stderr) = execute(command, spec.timeout, cancel).await?;
    finish(
        &context.artifacts,
        &context.credentials,
        status.code(),
        false,
        &stdout,
        &stderr,
    )
}

#[cfg(unix)]
async fn execute(
    mut command: tokio::process::Command,
    timeout: Duration,
    cancel: CancellationToken,
) -> Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>), ToolError> {
    // Recheck after synchronous containment/approval/environment preparation.
    if cancel.is_cancelled() {
        return Err(ToolError::ProcessCancelled);
    }
    command.process_group(0).kill_on_drop(true);
    let child = command.spawn().map_err(|error| stage_io("spawn", &error))?;
    let pid = child.id().expect("newly spawned child has a PID");
    let mut owned = OwnedChild {
        child,
        group: Some(i32::try_from(pid).expect("Unix PIDs fit pid_t")),
    };
    let stdout =
        owned.child.stdout.take().ok_or_else(|| {
            stage_io("take-stdout", &std::io::Error::other("missing stdout pipe"))
        })?;
    let stderr =
        owned.child.stderr.take().ok_or_else(|| {
            stage_io("take-stderr", &std::io::Error::other("missing stderr pipe"))
        })?;
    // These are scoped futures, not detached reader tasks. The deadline spans
    // both leader exit AND inherited pipes; all readers drop on every exit path.
    let result = tokio::select! {
        biased;
        () = cancel.cancelled() => Err(ToolError::ProcessCancelled),
        () = tokio::time::sleep(timeout) => Err(ToolError::ProcessTimeout(timeout)),
        output = async {
            tokio::try_join!(
                owned.wait_for_exit(pid),
                read_stream(stdout),
                read_stream(stderr)
            )
        } => output.map_err(|error| stage_io("wait-or-read", &error)),
    };
    match result {
        Ok(((), stdout, stderr)) => {
            let status = owned
                .kill_and_reap()
                .await
                .map_err(|error| stage_io("kill-and-reap", &error))?;
            Ok((status, stdout, stderr))
        }
        Err(error) => {
            owned
                .terminate()
                .await
                .map_err(|error| stage_io("terminate", &error))?;
            Err(error)
        }
    }
}

/// Labels an IO failure with the process-lifecycle stage that produced it,
/// keeping the raw OS code in the message for platform diagnosis.
#[cfg(unix)]
fn stage_io(stage: &'static str, error: &std::io::Error) -> ToolError {
    ToolError::Io(std::io::Error::new(
        error.kind(),
        format!("{stage} (os error {:?}): {error}", error.raw_os_error()),
    ))
}

#[cfg(not(unix))]
async fn execute(
    _command: tokio::process::Command,
    _timeout: Duration,
    _cancel: CancellationToken,
) -> Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>), ToolError> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe process-tree ownership is unavailable on this platform",
    )
    .into())
}

#[cfg(any(unix, test))]
async fn read_stream(mut stream: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

/// Own a fresh Unix process group until it has been signalled. Keep the leader
/// unreaped until then, reserving its PID so a stale PGID cannot target a reused
/// process group after an early leader exit. Descendants must not call setsid /
/// setpgid to escape: process groups are lifecycle ownership, not a sandbox.
#[cfg(unix)]
struct OwnedChild {
    child: tokio::process::Child,
    group: Option<libc::pid_t>,
}

#[cfg(unix)]
impl OwnedChild {
    async fn wait_for_exit(&self, pid: u32) -> std::io::Result<()> {
        while !leader_exited(pid)? {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    }

    async fn terminate(&mut self) -> std::io::Result<()> {
        if let Some(group) = self.group {
            // Allow cooperative children to flush and reap descendants, but
            // never let ignored TERM or inherited pipe handles block cleanup.
            if signal_group(group, libc::SIGTERM).is_ok() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        self.kill_and_reap().await?;
        Ok(())
    }

    async fn kill_and_reap(&mut self) -> std::io::Result<std::process::ExitStatus> {
        if let Some(group) = self.group {
            signal_group(group, libc::SIGKILL)?;
            // Disarm only after a successful signal, before reaping frees PID.
            self.group = None;
        }
        self.child.wait().await
    }
}

#[cfg(unix)]
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(group) = self.group {
            // Destructors cannot await a grace period. Force-kill the group
            // synchronously; Child's kill_on_drop + Tokio's orphan reaper are
            // the fallback for immediate-child reaping when the future drops.
            let _ = signal_group(group, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn signal_group(group: libc::pid_t, signal: libc::c_int) -> std::io::Result<()> {
    assert!(group > 1, "only signal an owned child process group");
    // SAFETY: negative positive-PGID targets only the fresh group we created.
    // The owned leader remains unreaped, preventing PID/PGID reuse until disarm.
    if unsafe { libc::kill(-group, signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn leader_exited(pid: u32) -> std::io::Result<bool> {
    // SAFETY: zero is a valid initial siginfo_t representation; waitid writes
    // to this live, aligned buffer. P_PID selects only our child, WNOHANG avoids
    // blocking the executor, and WNOWAIT leaves reaping to the owned Child.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid,
            &raw mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == 0 {
        Ok(info.si_signo != 0)
    } else {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            Ok(false)
        } else {
            Err(error)
        }
    }
}

fn finish(
    spool: &ArtifactSpool,
    broker: &CredentialBroker,
    exit_code: Option<i32>,
    timed_out: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<ProcessReceipt, ToolError> {
    let stdout = broker.redact_bytes(stdout);
    let stderr = broker.redact_bytes(stderr);
    let (stdout_inline, stdout_truncated, stdout_artifact) = split_stream(spool, &stdout)?;
    let (stderr_inline, stderr_truncated, stderr_artifact) = split_stream(spool, &stderr)?;
    Ok(ProcessReceipt {
        exit_code,
        timed_out,
        stdout: stdout_inline,
        stderr: stderr_inline,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    struct BrokenPipe;

    impl tokio::io::AsyncRead for BrokenPipe {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Err(std::io::Error::other("fixture read failure")))
        }
    }

    #[tokio::test]
    async fn stream_read_failure_is_not_successful_empty_output() {
        let result = read_stream(BrokenPipe).await;
        assert!(matches!(result, Err(error) if error.to_string() == "fixture read failure"));
    }
}
