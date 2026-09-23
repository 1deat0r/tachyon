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
#[cfg(any(unix, windows, test))]
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

/// Windows process-tree ownership via Job Objects (spec: "Job Object or
/// equivalent tree ownership"). The child is assigned to a fresh job with
/// `KILL_ON_JOB_CLOSE`; every descendant joins the same job unless it holds
/// breakaway rights, so closing or terminating the job ends the whole tree.
/// Reaping the owned leader still reserves its PID until `Child::wait`.
#[cfg(windows)]
#[allow(unsafe_code)]
async fn execute(
    mut command: tokio::process::Command,
    timeout: Duration,
    cancel: CancellationToken,
) -> Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>), ToolError> {
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };
    if cancel.is_cancelled() {
        return Err(ToolError::ProcessCancelled);
    }
    command.kill_on_drop(true);
    let child = command
        .spawn()
        .map_err(|error| win_stage_io("spawn", &error))?;
    // Open our own handle: the leader is unreaped so its PID cannot be
    // recycled under us. PROCESS_SET_QUOTA + PROCESS_TERMINATE is the
    // documented access for job assignment.
    let pid = child.id().expect("newly spawned child has a PID");
    // SAFETY: `pid` is our live, unreaped child; the handle is owned and
    // closed below right after assignment.
    let raw = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
    if raw.is_null() {
        return Err(win_stage_io(
            "open-process",
            &std::io::Error::last_os_error(),
        ));
    }
    let job = JobObject::create().map_err(|error| win_stage_io("create-job", &error))?;
    // SAFETY: `raw` is the live handle of our just-spawned child; the job
    // outlives this call inside `OwnedChild`.
    let assigned = unsafe { AssignProcessToJobObject(job.handle, raw) };
    // SAFETY: assignment copied what it needs; our open handle is now excess.
    unsafe { CloseHandle(raw) };
    if assigned == 0 {
        return Err(win_stage_io("assign-job", &std::io::Error::last_os_error()));
    }
    let mut owned = OwnedChild { child, job };
    let stdout = owned.child.stdout.take().ok_or_else(|| {
        win_stage_io("take-stdout", &std::io::Error::other("missing stdout pipe"))
    })?;
    let stderr = owned.child.stderr.take().ok_or_else(|| {
        win_stage_io("take-stderr", &std::io::Error::other("missing stderr pipe"))
    })?;
    let result = tokio::select! {
        biased;
        () = cancel.cancelled() => Err(ToolError::ProcessCancelled),
        () = tokio::time::sleep(timeout) => Err(ToolError::ProcessTimeout(timeout)),
        output = async {
            tokio::try_join!(
                owned.child.wait(),
                read_stream(stdout),
                read_stream(stderr)
            )
        } => output.map_err(|error| win_stage_io("wait-or-read", &error)),
    };
    match result {
        Ok((status, stdout, stderr)) => {
            owned
                .terminate()
                .await
                .map_err(|error| win_stage_io("terminate-job", &error))?;
            Ok((status, stdout, stderr))
        }
        Err(error) => {
            owned
                .terminate()
                .await
                .map_err(|error| win_stage_io("terminate", &error))?;
            Err(error)
        }
    }
}

/// Labels an IO failure with the process-lifecycle stage that produced it,
/// keeping the raw OS code in the message for platform diagnosis.
#[cfg(windows)]
fn win_stage_io(stage: &'static str, error: &std::io::Error) -> ToolError {
    ToolError::Io(std::io::Error::new(
        error.kind(),
        format!("{stage} (os error {:?}): {error}", error.raw_os_error()),
    ))
}

/// An owned Windows Job Object: closing the last handle kills the tree when
/// `KILL_ON_JOB_CLOSE` is set, which it always is here.
#[cfg(windows)]
struct JobObject {
    handle: HANDLE,
}

/// SAFETY: the handle is owned (created by us, closed in `Drop`); only the
/// owning `OwnedChild` touches it, and all methods take `&self` across awaits
/// without transferring ownership.
#[cfg(windows)]
#[allow(unsafe_code)]
unsafe impl Send for JobObject {}
#[cfg(windows)]
#[allow(unsafe_code)]
unsafe impl Sync for JobObject {}

#[cfg(windows)]
#[allow(unsafe_code)]
impl JobObject {
    fn create() -> std::io::Result<Self> {
        // SAFETY: null security/name creates an unnamed job owned by us.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `info` is a live, aligned struct of the documented size.
        let set = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .expect("job limits struct fits u32"),
            )
        };
        if set == 0 {
            // SAFETY: the handle is valid and owned; closing exactly once here.
            unsafe { CloseHandle(handle) };
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { handle })
    }

    fn terminate(&self) -> std::io::Result<()> {
        // SAFETY: handle is a valid owned job; exit code is arbitrary.
        let ended = unsafe { TerminateJobObject(self.handle, 1) };
        if ended == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
impl Drop for JobObject {
    fn drop(&mut self) {
        // KILL_ON_JOB_CLOSE ends the tree as the last handle closes.
        // SAFETY: valid owned handle, closed exactly once.
        unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

/// Windows tree owner: the job ends every member; the reaped leader's PID is
/// still reserved by `Child` until waited, as on Unix.
#[cfg(windows)]
struct OwnedChild {
    child: tokio::process::Child,
    job: JobObject,
}

#[cfg(windows)]
impl OwnedChild {
    /// Best-effort tree kill, then reap the leader. Already-dead trees and
    /// an already-reaped leader both resolve without error.
    async fn terminate(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let _ = self.job.terminate();
        self.child.wait().await
    }
}

#[cfg(any(unix, windows, test))]
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
    } else if error.raw_os_error() == Some(libc::EPERM) {
        // macOS reports EPERM (not ESRCH) when the group holds no live,
        // signalable process — the exited-but-unreaped leader plus, at most,
        // zombies. Same-UID live members are always signalable, so EPERM
        // means no worker remains; treating it as fatal would fail every
        // reaping of an already-exited tree. (setuid-root descendants are
        // outside the workspace-exclusion threat model: they can escape
        // containment regardless of signaling.)
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

    /// Signaling an exited leader's group must not error: on macOS the group
    /// holds no live member and `kill` reports EPERM rather than ESRCH. Pins
    /// the tolerance so reaping an already-exited tree stays green there.
    #[cfg(unix)]
    #[tokio::test]
    async fn exited_group_kill_is_not_an_error() {
        use std::os::unix::process::CommandExt as _;
        let mut child = std::process::Command::new("true");
        child.process_group(0);
        let mut child = child.spawn().expect("spawn true");
        let pid = child.id();
        while !leader_exited(pid).expect("waitid") {
            tokio::task::yield_now().await;
        }
        signal_group(pid as libc::pid_t, libc::SIGKILL).expect("exited group kill");
        child.wait().expect("reap");
    }
}
