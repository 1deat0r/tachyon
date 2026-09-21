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
//! journal (`mutation.log`, fsync per record) is crash truth. The task
//! journal owns task state; reconciliation between the two lands in M12.
//!
//! # New-capability checklist (`mutation.patch`)
//!
//! - Why deterministic code cannot solve it: applying validated file
//!   replacements is inherently an effect, not a computation.
//! - Input schema: [`PatchSpec`] per file (path, base hash, new bytes).
//! - Output schema: [`CommitReport`] / [`RecoveryReport`] with
//!   [`ChangedFile`] events.
//! - Access set: workspace file writes declared per path in the batch.
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
    ChangedFile, CommitReport, EFFECT_CLASS, IDEMPOTENCY, MutationEngine, PreparedBatch,
    RecoveryReport, Transition,
};
pub use journal::{BatchJournal, JOURNAL_FILE, JournalRecord, ReplayedBatch};
