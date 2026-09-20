//! Local runtime endpoint: directory, socket path, endpoint file, and
//! stale-instance detection (spec §37).
//!
//! The runtime directory is user-only (0o700). The endpoint file carries
//! connection metadata only — never secrets. A live socket behind a stale
//! endpoint file means another gateway owns the runtime; a dead socket
//! means the previous owner is gone and its files may be replaced.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tachyon_protocol::PROTOCOL_VERSION;
use tachyon_types::Timestamp;
use thiserror::Error;

use crate::transport::connect;

/// Errors from endpoint claim/setup.
#[derive(Debug, Error)]
pub enum EndpointError {
    /// Filesystem failure.
    #[error("endpoint I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Endpoint file is not valid JSON.
    #[error("endpoint file is corrupt: {0}")]
    Corrupt(#[from] serde_json::Error),
    /// Another gateway answered on the recorded socket.
    #[error("gateway already running (pid {pid})")]
    AlreadyRunning {
        /// Pid recorded by the live owner.
        pid: u32,
    },
}

/// Connection metadata for one running gateway.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EndpointInfo {
    /// Socket clients connect to.
    pub socket_path: PathBuf,
    /// Pid of the owning process.
    pub pid: u32,
    /// When the owner started (micros since epoch).
    pub started_at_micros: i64,
    /// Wire protocol version the owner speaks.
    pub protocol_version: u16,
}

/// Paths claimed for one gateway instance.
#[derive(Clone, Debug)]
pub struct ClaimPaths {
    /// Directory holding socket, endpoint file, and state.
    pub dir: PathBuf,
    /// Unix socket path.
    pub socket: PathBuf,
    /// Endpoint metadata file.
    pub endpoint_file: PathBuf,
}

/// Ensures the runtime dir exists with user-only permissions and evicts a
/// stale previous owner. Fails with [`EndpointError::AlreadyRunning`] when
/// a live gateway answers.
pub async fn claim_runtime_dir(data_dir: &Path) -> Result<ClaimPaths, EndpointError> {
    std::fs::create_dir_all(data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(data_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let paths = ClaimPaths {
        dir: data_dir.to_owned(),
        socket: data_dir.join("gateway.sock"),
        endpoint_file: data_dir.join("gateway.json"),
    };
    if paths.endpoint_file.exists() {
        let info = read_endpoint(&paths.endpoint_file)?;
        if probe_socket(&live_address(&paths)).await {
            return Err(EndpointError::AlreadyRunning { pid: info.pid });
        }
        let _ = std::fs::remove_file(&paths.socket);
        let _ = std::fs::remove_file(&paths.endpoint_file);
    } else if paths.socket.exists() && !probe_socket(&live_address(&paths)).await {
        // Socket file without endpoint metadata: leftover of a crash.
        let _ = std::fs::remove_file(&paths.socket);
    } else if paths.socket.exists() {
        return Err(EndpointError::AlreadyRunning { pid: 0 });
    }
    Ok(paths)
}

/// Address a live owner listens on: the socket file on Unix, the derived
/// pipe name on Windows.
fn live_address(paths: &ClaimPaths) -> PathBuf {
    #[cfg(unix)]
    {
        paths.socket.clone()
    }
    #[cfg(windows)]
    {
        crate::transport::pipe_name_for(&paths.dir)
    }
}

/// Writes endpoint metadata for the bound `address` (socket path on Unix,
/// pipe name on Windows) after a successful bind.
pub fn write_endpoint(paths: &ClaimPaths, address: &Path) -> Result<EndpointInfo, EndpointError> {
    let info = EndpointInfo {
        socket_path: address.to_owned(),
        pid: std::process::id(),
        started_at_micros: Timestamp::now().as_micros(),
        protocol_version: PROTOCOL_VERSION,
    };
    let bytes = serde_json::to_vec_pretty(&info)?;
    std::fs::write(&paths.endpoint_file, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&paths.endpoint_file, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(info)
}

/// Removes socket and endpoint files. Best-effort; missing files are fine.
pub fn release_runtime_dir(paths: &ClaimPaths) {
    let _ = std::fs::remove_file(&paths.socket);
    let _ = std::fs::remove_file(&paths.endpoint_file);
}

fn read_endpoint(path: &Path) -> Result<EndpointInfo, EndpointError> {
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(EndpointError::from)
}

/// Reads endpoint metadata written by a running gateway.
pub fn read_endpoint_info(path: &Path) -> Result<EndpointInfo, EndpointError> {
    read_endpoint(path)
}

/// True when something accepts connections on `address` (socket or pipe).
async fn probe_socket(address: &Path) -> bool {
    tokio::time::timeout(std::time::Duration::from_secs(2), connect(address))
        .await
        .is_ok_and(|result| result.is_ok())
}
