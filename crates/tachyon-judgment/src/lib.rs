//! Provider-neutral bounded judgment (spec §24, M7).
//!
//! Judgment answers tiny closed questions — boolean, choice, score — over
//! shared context. It never plans, never executes, and never substitutes
//! for verification. `OpenJEV` is one adapter behind the `openjev` feature;
//! this crate compiles and runs without it, and the registry resolves
//! everything to evidence when no provider is registered.
//!
//! # New-capability checklist (bounded judgment call)
//!
//! - Why deterministic code cannot solve it: ambiguous route intent
//!   (no rule fires strongly) is a judgment over phrasing, not lookup.
//! - Input schema: [`JudgmentBatch`] (shared trusted context + closed
//!   items with certainty policies).
//! - Output schema: [`JudgmentOutcome`]s paired by id, folded through
//!   [`apply_policy`] into [`PolicyAction`] (accept or explicit fallback).
//! - Access set: none — judges receive context text only.
//! - Effect class: metered remote inference with no Tachyon-persisted
//!   effect, same as model calls.
//! - Idempotency: safe to rerun while no outcome committed (spec §41).
//! - Resource claim: one network slot per batch.
//! - Cancellation: the caller drops the call; policy fallbacks apply.
//! - Retry policy: registry-owned chain across providers, guided by
//!   [`JudgmentError::is_retryable`]; providers never retry internally.
//! - Verification: A/B gate test — judgment-routed ambiguous requests
//!   avoid model calls versus the serial reference at equal verified
//!   success (see `tests/judgment_gate.rs`; synthetic fakes, real A/B in
//!   M13/M14).
//! - Crash-recovery: uncommitted calls rerun or fall back to evidence;
//!   committed outcomes are journaled by the supervisor.
//! - Expected latency class: fast-judge milliseconds locally; hosted
//!   sub-second. No measured claims yet — M13 calibrates.

#![warn(unsafe_code)]

pub mod batch;
pub mod fake;
pub mod provider;
pub mod registry;
pub mod route;

#[cfg(feature = "openjev")]
pub mod openjev;

pub use batch::{
    CertaintyPolicy, JudgmentBatch, JudgmentItem, JudgmentKind, JudgmentOutcome, JudgmentValue,
    PolicyAction, UncertainAction, apply_policy,
};
pub use fake::FakeJudgmentProvider;
#[cfg(feature = "openjev")]
pub use openjev::{JudgmentTransport, OpenJevProvider, TcpJudgmentTransport};
pub use provider::{JudgmentCapabilities, JudgmentError, JudgmentProvider};
pub use registry::{
    JudgmentRegistry, JudgmentSource, RegisteredJudgment, RemoteJudgmentConfig, ResolvedBatch,
    ResolvedItem,
};
pub use route::{ROUTE_OPTIONS, decide_route, replan, route_choice_item};
