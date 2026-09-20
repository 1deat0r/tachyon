#![warn(unsafe_code)]

//! `tachyon`: CLI client of the Tachyon runtime (Milestone 0).
//!
//! The binary owns argument parsing, configuration, tracing, and output
//! formatting. It owns no agent decision logic: every subcommand either
//! inspects the local environment (`doctor`, `config`) or will, from
//! Milestone 1 on, talk to the gateway as a client.

mod config;
mod doctor;
mod logging;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use config::{CliOverrides, Config};
use doctor::{all_ok, run_checks};

/// Tachyon: high-performance AI agent harness (Milestone 0 CLI).
#[derive(Debug, Parser)]
#[command(name = "tachyon", version, about)]
#[command(arg_required_else_help = true)]
struct Cli {
    /// Config file path. Overrides `TACHYON_CONFIG` and the default path.
    #[arg(long, env = "TACHYON_CONFIG")]
    config: Option<PathBuf>,
    /// Tracing filter directive. Overrides `TACHYON_LOG_LEVEL`.
    #[arg(long, env = "TACHYON_LOG_LEVEL")]
    log_level: Option<String>,
    /// Runtime state directory. Overrides `TACHYON_DATA_DIR`.
    #[arg(long, env = "TACHYON_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Evidence grace window in ms. Overrides `TACHYON_EVIDENCE_GRACE_MS`.
    #[arg(long, env = "TACHYON_EVIDENCE_GRACE_MS")]
    evidence_grace_ms: Option<u64>,
    /// Subcommand to run.
    #[command(subcommand)]
    command: Commands,
}

/// Milestone 0 subcommands. Task commands arrive with the gateway (M1+).
#[derive(Debug, Subcommand)]
enum Commands {
    /// Runs deterministic environment self-checks.
    Doctor {
        /// Emits machine-readable JSON instead of human lines.
        #[arg(long)]
        json: bool,
    },
    /// Prints the resolved configuration and its source.
    Config {
        /// Emits machine-readable JSON instead of human lines.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(healthy) => {
            if healthy {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(err) => {
            eprintln!("tachyon: {err:?}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the CLI. Returns `Ok(false)` when `doctor` checks fail; hard
/// errors (unreadable config, unserializable output) are `Err`.
fn run() -> Result<bool> {
    let cli = Cli::parse();
    let overrides = CliOverrides {
        log_level: cli.log_level,
        data_dir: cli.data_dir,
        evidence_grace_ms: cli.evidence_grace_ms,
    };
    let config = Config::load(cli.config, overrides).context("loading configuration")?;
    logging::init_tracing(&config.log_level);
    match cli.command {
        Commands::Doctor { json } => run_doctor(&config, json),
        Commands::Config { json } => run_config(&config, json),
    }
}

fn run_doctor(config: &Config, json: bool) -> Result<bool> {
    let checks = run_checks(config);
    if json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
    } else {
        for check in &checks {
            let status = if check.ok { "ok" } else { "FAIL" };
            println!("{status:4} {:12} {}", check.name, check.detail);
        }
    }
    Ok(all_ok(&checks))
}

fn run_config(config: &Config, json: bool) -> Result<bool> {
    if json {
        println!("{}", serde_json::to_string_pretty(config)?);
    } else {
        println!("log_level:         {}", config.log_level);
        println!("data_dir:          {}", config.data_dir.display());
        println!("evidence_grace_ms: {}", config.evidence_grace_ms);
        match &config.source_file {
            Some(path) => println!("config_file:       {}", path.display()),
            None => println!("config_file:       (none; defaults and environment)"),
        }
    }
    Ok(true)
}
