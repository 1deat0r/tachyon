//! Tracing bootstrap.
//!
//! Structured `tracing` output for the whole binary. Diagnostics go to
//! stderr so machine-readable stdout (`--json`) stays parseable. Optional
//! OpenTelemetry export arrives in a later milestone and must never sit on
//! the execution critical path (spec §39).

use tracing_subscriber::EnvFilter;

/// Default filter directive when none is configured.
pub const DEFAULT_LOG_LEVEL: &str = "info";

/// Installs the global tracing subscriber. Safe to call twice (tests,
/// re-entrant entry points): only the first call takes effect.
///
/// Falls back to [`DEFAULT_LOG_LEVEL`] when `log_level` is not a valid
/// filter directive.
pub fn init_tracing(log_level: &str) {
    let filter = EnvFilter::try_new(log_level).unwrap_or_else(|_| {
        eprintln!("invalid log level {log_level:?}, falling back to {DEFAULT_LOG_LEVEL:?}");
        EnvFilter::new(DEFAULT_LOG_LEVEL)
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
