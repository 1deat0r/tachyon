//! Configuration loading and precedence.
//!
//! Precedence, weakest to strongest:
//!
//! 1. built-in defaults;
//! 2. JSON config file (`--config`, then `TACHYON_CONFIG`, then the platform
//!    default path);
//! 3. environment variables (`TACHYON_LOG_LEVEL`, `TACHYON_DATA_DIR`,
//!    `TACHYON_EVIDENCE_GRACE_MS`);
//! 4. CLI flags.
//!
//! A missing config file is fine (defaults apply). A present-but-malformed
//! file, or a malformed environment value, is an error.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::logging::DEFAULT_LOG_LEVEL;

/// Initial evidence grace window in milliseconds (spec §23 candidate).
pub const DEFAULT_EVIDENCE_GRACE_MS: u64 = 75;

/// Prefix for configuration environment variables.
pub const ENV_PREFIX: &str = "TACHYON_";

/// Errors produced while loading configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Config file could not be read.
    #[error("cannot read config file {path}: {source}", path = .path.display())]
    UnreadableFile {
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
        /// File that could not be read.
        path: PathBuf,
    },
    /// Config file is not valid JSON.
    #[error("invalid JSON in config file {path}: {source}", path = .path.display())]
    InvalidFile {
        /// Underlying JSON failure.
        #[source]
        source: serde_json::Error,
        /// File that could not be parsed.
        path: PathBuf,
    },
    /// An environment variable holds a value of the wrong shape.
    #[error("environment variable {var} holds invalid value {value:?}: {reason}")]
    InvalidEnv {
        /// Variable name.
        var: String,
        /// Offending value.
        value: String,
        /// Why it was rejected.
        reason: String,
    },
}

/// Partial configuration as read from a JSON file. Every field is optional;
/// unset fields fall through to the next weaker layer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileConfig {
    /// Log filter, e.g. `"info"` or `"tachyon=debug"`.
    pub log_level: Option<String>,
    /// Directory for runtime state (`state.db`, sockets, artifacts).
    pub data_dir: Option<PathBuf>,
    /// Evidence grace window in milliseconds.
    pub evidence_grace_ms: Option<u64>,
}

/// Strongest layer: explicit CLI flag values. `None` means "flag not given".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliOverrides {
    /// `--log-level` value, if given.
    pub log_level: Option<String>,
    /// `--data-dir` value, if given.
    pub data_dir: Option<PathBuf>,
    /// `--evidence-grace-ms` value, if given.
    pub evidence_grace_ms: Option<u64>,
}

/// Fully resolved runtime configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Tracing filter directive.
    pub log_level: String,
    /// Directory for runtime state.
    pub data_dir: PathBuf,
    /// Evidence grace window in milliseconds.
    pub evidence_grace_ms: u64,
    /// Config file actually loaded, if any.
    pub source_file: Option<PathBuf>,
}

impl Config {
    /// Loads configuration honoring the documented precedence.
    ///
    /// `explicit_path` is the `--config` value. Environment supplies
    /// `TACHYON_CONFIG` (file path) plus per-field overrides; `cli` wins.
    pub fn load(explicit_path: Option<PathBuf>, cli: CliOverrides) -> Result<Self, ConfigError> {
        let required = explicit_path.is_some() || env::var_os("TACHYON_CONFIG").is_some();
        let path = resolve_config_path(explicit_path);
        let (file, loaded_from) = read_file_config(path.as_deref(), required)?;
        let env_map: HashMap<String, String> = env::vars()
            .filter(|(key, _)| key.starts_with(ENV_PREFIX))
            .collect();
        Ok(Self::resolve(file, &env_map, cli, loaded_from))
    }

    /// Pure merge of the layers; `loaded_from` records the file actually read.
    /// Layer order: defaults, then `file`, then `env`, then `cli`.
    #[must_use]
    pub fn resolve(
        file: FileConfig,
        env: &HashMap<String, String>,
        cli: CliOverrides,
        loaded_from: Option<PathBuf>,
    ) -> Self {
        let log_level = cli
            .log_level
            .or_else(|| env.get("TACHYON_LOG_LEVEL").cloned())
            .or(file.log_level)
            .unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_owned());
        let data_dir = cli
            .data_dir
            .or_else(|| env.get("TACHYON_DATA_DIR").map(PathBuf::from))
            .or(file.data_dir)
            .unwrap_or_else(default_data_dir);
        // Lenient parse: invalid env values are ignored here; `doctor`
        // surfaces them strictly via `strict_env_grace`.
        let env_grace = env
            .get("TACHYON_EVIDENCE_GRACE_MS")
            .and_then(|raw| raw.parse::<u64>().ok());
        let evidence_grace_ms = cli
            .evidence_grace_ms
            .or(env_grace)
            .or(file.evidence_grace_ms)
            .unwrap_or(DEFAULT_EVIDENCE_GRACE_MS);
        Self {
            log_level,
            data_dir,
            evidence_grace_ms,
            source_file: loaded_from,
        }
    }

    /// Strict variant used when callers want malformed env values to fail.
    /// [`Self::load`] is lenient by design; `doctor` surfaces strict errors.
    pub fn strict_env_grace(env: &HashMap<String, String>) -> Result<Option<u64>, ConfigError> {
        env.get("TACHYON_EVIDENCE_GRACE_MS")
            .map(|raw| {
                raw.parse::<u64>().map_err(|_| ConfigError::InvalidEnv {
                    var: "TACHYON_EVIDENCE_GRACE_MS".to_owned(),
                    value: raw.clone(),
                    reason: "expected a non-negative integer of milliseconds".to_owned(),
                })
            })
            .transpose()
    }

    /// Strictly validates the process environment's `TACHYON_*` values.
    /// Invalid values are errors here even though [`Self::load`] stays lenient.
    pub fn check_process_env() -> Result<(), ConfigError> {
        let env_map: HashMap<String, String> = env::vars()
            .filter(|(key, _)| key.starts_with(ENV_PREFIX))
            .collect();
        Self::strict_env_grace(&env_map).map(|_| ())
    }
}

/// Picks the config file path: explicit flag, then `TACHYON_CONFIG` env,
/// then the platform default. Returns `None` only when unreachable.
fn resolve_config_path(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path);
    }
    if let Some(path) = env::var_os("TACHYON_CONFIG").map(PathBuf::from) {
        return Some(path);
    }
    default_config_path()
}

/// Reads and parses the file at `path`. Missing files yield defaults unless
/// `required` (explicit `--config`/`TACHYON_CONFIG`), in which case absence
/// is an error. Returns the file config and the path actually loaded, if any.
fn read_file_config(
    path: Option<&Path>,
    required: bool,
) -> Result<(FileConfig, Option<PathBuf>), ConfigError> {
    let Some(path) = path else {
        return Ok((FileConfig::default(), None));
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && !required => {
            return Ok((FileConfig::default(), None));
        }
        Err(err) => {
            return Err(ConfigError::UnreadableFile {
                source: err,
                path: path.to_owned(),
            });
        }
    };
    serde_json::from_slice(&bytes)
        .map(|file| (file, Some(path.to_owned())))
        .map_err(|err| ConfigError::InvalidFile {
            source: err,
            path: path.to_owned(),
        })
}

/// Platform config directory: `$XDG_CONFIG_HOME`, else `~/.config`
/// (Windows: `%APPDATA%`).
#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    config_base_dir().map(|base| base.join("tachyon").join("config.json"))
}

/// Platform data directory: `$XDG_DATA_HOME`, else `~/.local/share/tachyon`
/// (Windows: `%LOCALAPPDATA%/tachyon`).
#[must_use]
pub fn default_data_dir() -> PathBuf {
    data_base_dir().map_or_else(
        || PathBuf::from(".tachyon-data"),
        |base| base.join("tachyon"),
    )
}

fn config_base_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        return Some(dir);
    }
    home_dir().map(|home| home.join(".config"))
}

fn data_base_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        return Some(dir);
    }
    home_dir().map(|home| home.join(".local").join("share"))
}

#[cfg(unix)]
fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

#[cfg(windows)]
fn home_dir() -> Option<PathBuf> {
    env::var_os("USERPROFILE").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{CliOverrides, Config, ConfigError, FileConfig};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn precedence_is_cli_over_env_over_file_over_defaults() {
        let file = FileConfig {
            log_level: Some("warn".to_owned()),
            data_dir: Some(PathBuf::from("/file")),
            evidence_grace_ms: Some(10),
        };
        let env_map = env(&[
            ("TACHYON_LOG_LEVEL", "debug"),
            ("TACHYON_DATA_DIR", "/env"),
            ("TACHYON_EVIDENCE_GRACE_MS", "20"),
        ]);
        let cli = CliOverrides {
            log_level: Some("error".to_owned()),
            data_dir: None,
            evidence_grace_ms: None,
        };
        let config = Config::resolve(file, &env_map, cli, None);
        assert_eq!(config.log_level, "error");
        assert_eq!(config.data_dir, PathBuf::from("/env"));
        assert_eq!(config.evidence_grace_ms, 20);
    }

    #[test]
    fn file_values_apply_when_env_and_cli_are_silent() {
        let file = FileConfig {
            log_level: Some("warn".to_owned()),
            data_dir: Some(PathBuf::from("/file")),
            evidence_grace_ms: Some(10),
        };
        let config = Config::resolve(file, &env(&[]), CliOverrides::default(), None);
        assert_eq!(config.log_level, "warn");
        assert_eq!(config.data_dir, PathBuf::from("/file"));
        assert_eq!(config.evidence_grace_ms, 10);
    }

    #[test]
    fn invalid_env_grace_is_reported_strictly() {
        let env_map = env(&[("TACHYON_EVIDENCE_GRACE_MS", "soon")]);
        let err = Config::strict_env_grace(&env_map).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidEnv { .. }));
        assert!(Config::strict_env_grace(&env(&[])).unwrap().is_none());
    }

    #[test]
    fn missing_file_config_yields_defaults() {
        let missing = PathBuf::from("/definitely/not/here/tachyon-config.json");
        let (file, loaded) = super::read_file_config(Some(missing.as_path()), false).unwrap();
        assert_eq!(file, FileConfig::default());
        assert_eq!(loaded, None);
    }

    #[test]
    fn missing_required_file_config_is_an_error() {
        let missing = PathBuf::from("/definitely/not/here/tachyon-config.json");
        let err = super::read_file_config(Some(missing.as_path()), true).unwrap_err();
        assert!(matches!(err, super::ConfigError::UnreadableFile { .. }));
    }
}
