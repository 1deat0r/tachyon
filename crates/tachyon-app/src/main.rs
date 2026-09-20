#![warn(unsafe_code)]

//! `tachyon`: CLI client of the Tachyon runtime.
//!
//! The binary owns argument parsing, configuration, tracing, and output
//! formatting. It owns no agent decision logic: local subcommands inspect
//! the environment (`doctor`, `config`); task subcommands talk to the
//! gateway as a client; `gateway` runs the runtime in the foreground.

mod client;
mod config;
mod doctor;
mod logging;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tachyon_protocol::Command;

use client::{render, send};
use config::{CliOverrides, Config};
use doctor::{all_ok, run_checks};

/// Tachyon: high-performance AI agent harness.
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
    /// Prints machine-readable JSON instead of human lines.
    #[arg(long, global = true)]
    json: bool,
    /// Subcommand to run.
    #[command(subcommand)]
    command: Commands,
}

/// Subcommands. `gateway` runs the runtime; `session`/`task` are clients.
#[derive(Debug, Subcommand)]
enum Commands {
    /// Runs deterministic environment self-checks.
    Doctor,
    /// Prints the resolved configuration and its source.
    Config,
    /// Runs the local gateway in the foreground (Ctrl-C stops it).
    Gateway,
    /// Opens a new persistent session.
    Session {
        /// Session action.
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Task operations against the running gateway.
    Task {
        /// Task action.
        #[command(subcommand)]
        action: TaskAction,
    },
}

/// Session actions.
#[derive(Debug, Subcommand)]
enum SessionAction {
    /// Creates a session and prints its id.
    Create,
}

/// Task actions.
#[derive(Debug, Subcommand)]
enum TaskAction {
    /// Creates a task in a session.
    Create {
        /// Session that will own the task.
        #[arg(long)]
        session: String,
        /// User's objective.
        objective: String,
    },
    /// Lists tasks, optionally restricted to one session.
    List {
        /// Session to filter by.
        #[arg(long)]
        session: Option<String>,
    },
    /// Prints one task's canonical state.
    Get {
        /// Task id.
        task_id: String,
    },
    /// Sends a steering message to a task.
    Send {
        /// Task id.
        task_id: String,
        /// Message words.
        message: Vec<String>,
    },
    /// Pauses a task.
    Pause {
        /// Task id.
        task_id: String,
    },
    /// Resumes a paused task.
    Resume {
        /// Task id.
        task_id: String,
    },
    /// Cancels a task.
    Cancel {
        /// Task id.
        task_id: String,
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

/// Runs the CLI. Returns `Ok(false)` for check/command failures; hard
/// errors (unreadable config, no gateway) are `Err`.
fn run() -> Result<bool> {
    let cli = Cli::parse();
    let overrides = CliOverrides {
        log_level: cli.log_level,
        data_dir: cli.data_dir,
        evidence_grace_ms: cli.evidence_grace_ms,
    };
    let config = Config::load(cli.config, overrides).context("loading configuration")?;
    logging::init_tracing(&config.log_level);
    let json = cli.json;
    match cli.command {
        Commands::Doctor => run_doctor(&config, json),
        Commands::Config => run_config(&config, json),
        Commands::Gateway => run_gateway(&config),
        Commands::Session {
            action: SessionAction::Create,
        } => run_session(&config, json),
        Commands::Task { action } => run_task(&config, action, json),
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

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting async runtime")
}

fn socket_path(config: &Config) -> Result<PathBuf> {
    let info = tachyon_gateway::read_endpoint_info(&config.data_dir.join("gateway.json"))
        .context("reading gateway endpoint (is the gateway running?)")?;
    Ok(info.socket_path)
}

fn run_gateway(config: &Config) -> Result<bool> {
    runtime()?.block_on(async {
        let gateway = tachyon_gateway::start(&config.data_dir)
            .await
            .context("starting gateway")?;
        println!("gateway listening on {}", gateway.socket_path().display());
        tokio::signal::ctrl_c()
            .await
            .context("waiting for Ctrl-C")?;
        println!("shutting down gateway");
        gateway.shutdown().await;
        Ok(true)
    })
}

fn run_session(config: &Config, json: bool) -> Result<bool> {
    let address = socket_path(config)?;
    runtime()?.block_on(async {
        let result = send(&address, Command::CreateSession).await?;
        render(&result, json)
    })
}

fn run_task(config: &Config, action: TaskAction, json: bool) -> Result<bool> {
    let address = socket_path(config)?;
    runtime()?.block_on(async {
        let command = task_command(action)?;
        let result = send(&address, command).await?;
        render(&result, json)
    })
}

fn task_command(action: TaskAction) -> Result<Command> {
    match action {
        TaskAction::Create { session, objective } => Ok(Command::CreateTask {
            session_id: parse_session(&session)?,
            objective,
        }),
        TaskAction::List { session } => Ok(Command::ListTasks {
            session_id: session.map(|raw| parse_session(&raw)).transpose()?,
        }),
        TaskAction::Get { task_id } => Ok(Command::GetTask {
            task_id: parse_task(&task_id)?,
        }),
        TaskAction::Send { task_id, message } => Ok(Command::SendMessage {
            task_id: parse_task(&task_id)?,
            message: message.join(" "),
        }),
        TaskAction::Pause { task_id } => Ok(Command::PauseTask {
            task_id: parse_task(&task_id)?,
        }),
        TaskAction::Resume { task_id } => Ok(Command::ResumeTask {
            task_id: parse_task(&task_id)?,
        }),
        TaskAction::Cancel { task_id } => Ok(Command::CancelTask {
            task_id: parse_task(&task_id)?,
        }),
    }
}

fn parse_session(raw: &str) -> Result<tachyon_types::SessionId> {
    uuid::Uuid::parse_str(raw)
        .map(tachyon_types::SessionId)
        .with_context(|| format!("invalid session id {raw:?}"))
}

fn parse_task(raw: &str) -> Result<tachyon_types::TaskId> {
    uuid::Uuid::parse_str(raw)
        .map(tachyon_types::TaskId)
        .with_context(|| format!("invalid task id {raw:?}"))
}
