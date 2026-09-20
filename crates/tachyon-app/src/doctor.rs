//! `tachyon doctor`: deterministic environment self-checks.
//!
//! Every check is local and side-effect-free except a probe file briefly
//! written to, read from, and removed inside the data directory.

use std::fs;

use serde::{Deserialize, Serialize};
use tachyon_protocol::{Command, RequestEnvelope, check_version, decode_frame, encode_frame};
use tachyon_protocol::{PROTOCOL_VERSION, ProtocolError};
use tachyon_types::{EventId, TaskId};

use crate::config::Config;

/// One named self-check result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    /// Stable check name.
    pub name: String,
    /// Whether the check passed.
    pub ok: bool,
    /// Human-readable detail.
    pub detail: String,
}

/// Runs all Milestone 0 self-checks against `config`.
#[must_use]
pub fn run_checks(config: &Config) -> Vec<DoctorCheck> {
    vec![
        check_config_source(config),
        check_strict_env(),
        check_data_dir(config),
        check_identifiers(),
        check_protocol_loopback(),
        check_socket_path(config),
    ]
}

/// True when every check passed.
#[must_use]
pub fn all_ok(checks: &[DoctorCheck]) -> bool {
    checks.iter().all(|check| check.ok)
}

fn pass(name: &str, detail: String) -> DoctorCheck {
    DoctorCheck {
        name: name.to_owned(),
        ok: true,
        detail,
    }
}

fn fail(name: &str, detail: String) -> DoctorCheck {
    DoctorCheck {
        name: name.to_owned(),
        ok: false,
        detail,
    }
}

fn check_config_source(config: &Config) -> DoctorCheck {
    let detail = match &config.source_file {
        Some(path) => format!(
            "loaded {} (log_level={}, grace_ms={})",
            path.display(),
            config.log_level,
            config.evidence_grace_ms
        ),
        None => format!(
            "no config file; using defaults/env (log_level={}, grace_ms={})",
            config.log_level, config.evidence_grace_ms
        ),
    };
    pass("config", detail)
}

fn check_strict_env() -> DoctorCheck {
    match Config::check_process_env() {
        Ok(()) => pass("env", "TACHYON_* overrides parse".to_owned()),
        Err(err) => fail("env", err.to_string()),
    }
}

fn check_data_dir(config: &Config) -> DoctorCheck {
    if let Err(err) = fs::create_dir_all(&config.data_dir) {
        return fail(
            "data_dir",
            format!("cannot create {}: {err}", config.data_dir.display()),
        );
    }
    let probe = config.data_dir.join(".tachyon-doctor-probe");
    let write = fs::write(&probe, b"tachyon-doctor");
    let read = write.and_then(|()| fs::read(&probe));
    let _ = fs::remove_file(&probe);
    match read {
        Ok(bytes) if bytes == b"tachyon-doctor" => pass(
            "data_dir",
            format!("writable at {}", config.data_dir.display()),
        ),
        Ok(_) => fail("data_dir", "probe round-trip mismatch".to_owned()),
        Err(err) => fail("data_dir", format!("probe failed: {err}")),
    }
}

fn check_identifiers() -> DoctorCheck {
    let first = TaskId::generate();
    let second = TaskId::generate();
    if first == second {
        fail(
            "identifiers",
            "UUIDv7 generator produced a duplicate".to_owned(),
        )
    } else {
        pass("identifiers", format!("UUIDv7 ok, e.g. {first}").to_owned())
    }
}

fn check_protocol_loopback() -> DoctorCheck {
    let envelope = RequestEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: EventId::generate(),
        command: Command::Ping,
    };
    let result: Result<bool, ProtocolError> = (|| {
        check_version(envelope.protocol_version)?;
        let bytes = encode_frame(&envelope)?;
        let (back, used): (RequestEnvelope, usize) = decode_frame(&bytes)?;
        Ok(used == bytes.len() && back == envelope)
    })();
    match result {
        Ok(true) => pass(
            "protocol",
            format!("frame round-trip ok (protocol v{PROTOCOL_VERSION})"),
        ),
        Ok(false) => fail("protocol", "frame round-trip mismatch".to_owned()),
        Err(err) => fail("protocol", err.to_string()),
    }
}

fn check_socket_path(config: &Config) -> DoctorCheck {
    let path = config.data_dir.join("gateway.sock");
    let len = path.as_os_str().len();
    #[cfg(unix)]
    {
        // `sun_path` limit for Unix domain sockets.
        if len < 108 {
            pass(
                "socket_path",
                format!("{} fits sun_path ({len} bytes)", path.display()),
            )
        } else {
            fail(
                "socket_path",
                format!("{} is {len} bytes; must fit in 108", path.display()),
            )
        }
    }
    #[cfg(not(unix))]
    {
        pass(
            "socket_path",
            format!("{} reserved ({len} bytes)", path.display()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{all_ok, run_checks};
    use crate::config::Config;

    #[test]
    fn checks_pass_with_writable_temp_data_dir() {
        let dir = std::env::temp_dir().join(format!("tachyon-doctor-{}", std::process::id()));
        let config = Config {
            log_level: "info".to_owned(),
            data_dir: dir.clone(),
            evidence_grace_ms: 75,
            source_file: None,
        };
        let checks = run_checks(&config);
        assert!(all_ok(&checks), "{checks:?}");
        assert_eq!(checks.len(), 6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_ok_rejects_any_failure() {
        let mut checks = run_checks(&Config {
            log_level: "info".to_owned(),
            data_dir: std::env::temp_dir().join("tachyon-doctor-unused"),
            evidence_grace_ms: 75,
            source_file: None,
        });
        assert!(all_ok(&checks));
        checks[0].ok = false;
        assert!(!all_ok(&checks));
    }
}
