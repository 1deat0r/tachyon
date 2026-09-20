//! Tachyon Scheduler.
//!
//! Dependency-aware, effect-aware DAG execution (spec §11–§14).
//! Readiness, atomic conflict/resource grants, critical-path priority,
//! retries, timeouts, and structured cancellation live here; executors
//! own only single-node work.

#![warn(unsafe_code)]

pub mod executor;
pub mod scheduler;

pub use executor::{Executor, FakeExecutor, NodeOutcome, OutcomeStatus, ResolvedInputs, Tracker};
pub use scheduler::{
    Budgets, ExecutorRegistry, SchedulerCommand, SchedulerError, SchedulerHandle, TaskRunSnapshot,
    spawn,
};
