//! Recoverable multi-file mutation (spec §20, M8).
//!
//! Filesystem mutation is **recoverable**, not globally atomic: verify
//! preimages, stage temps beside their targets, journal the plan, rename
//! per file, journal each commit, then complete. Recovery reconciles the
//! journal with disk truth and either finishes a still-valid batch or
//! compensates it back to preimages. Content matching neither hash is
//! `Diverged`: reported, never written.
//!
//! Preimages rest in the content-addressed artifact spool; the batch
//! journal (`mutation.log`, fsync per record) is crash truth. M10 runtimes
//! apply an ordinary patch through
//! [`MutationEngine::prepare_authorized`] and
//! [`MutationEngine::commit_authorized_up_to`]: exact per-target and
//! per-derived-temp policy is preflighted before any workspace, spool or
//! journal write, and each pending file is authorized again immediately
//! before its real rename so the runtime can recheck cancellation, revision
//! and policy at a `limit = 1` boundary. [`MutationEngine::recover_scoped`]
//! remains the only reconciliation entry, under the same task/attempt
//! binding and shared workspace lease. Strict read-only preflight precedes
//! all effects; cleanup removes only exact journal-owned, content-verified
//! temps. The legacy bulk entry is not a task-policy boundary. See
//! `RECOVERY.md` for exact policy scopes, dispositions and remaining
//! isolation limits.
//!
//! # New-capability checklist (`mutation.patch`)
//!
//! - Why deterministic code cannot solve it: applying validated file
//!   replacements is inherently an effect, not a computation.
//! - Input schema: [`PatchSpec`] per file (path, base hash, new bytes).
//! - Output schema: [`CommitReport`] / [`ScopedRecoveryReport`] (or legacy
//!   [`RecoveryReport`]) with [`ChangedFile`] events.
//! - Access set: exact target and temp reads/writes/deletions, plus retained
//!   artifact reads. Recovery validates the caller's target allowlist and
//!   exact policy operations before source mutation or cleanup.
//! - Effect class: `ReversibleLocalMutation` ([`EFFECT_CLASS`]).
//! - Idempotency: `Compensatable` ([`IDEMPOTENCY`]) — rollback restores
//!   preimages; commits never auto-retry after a crash.
//! - Resource claim: one process slot; temp disk beside each target.
//! - Cancellation: a batch mid-commit stops after the current rename;
//!   the journal records exactly what committed.
//! - Retry policy: never retry a commit after an unknown crash state —
//!   run recovery first, then act on its report.
//! - Verification: Slice C — the incorrect token-refresh implementation
//!   is replaced, content asserted, journal shows completion
//!   (see `tests/mutation_gate.rs`).
//! - Crash-recovery: crash injection between every file commit recovers
//!   to a coherent documented state; stale preimages refuse writes.
//! - Expected latency class: local disk milliseconds. No measured claims
//!   yet — M13 calibrates.

#![warn(unsafe_code)]

pub mod batch;
pub mod engine;
pub mod journal;

pub use batch::{
    FileMutation, FileState, MutationError, PatchSpec, blake3_hex, file_hash, normalize_rel,
};
pub use engine::{
    AuthorizedOp, ChangedFile, CommitReport, EFFECT_CLASS, IDEMPOTENCY, MutationEngine,
    PreparedBatch, RecoveryAction, RecoveryDisposition, RecoveryReport, ScopedRecoveryReport,
    Transition,
};
pub use journal::{BatchJournal, JOURNAL_FILE, JournalRecord, ReplayedBatch};
