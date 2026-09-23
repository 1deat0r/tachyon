# M10 next prerequisite slices — responsive actor and authorized effects

Read AGENTS.md and approved docs/milestones/M10_PLAN.md r2. Owner requested the
next milestone; independent plan R2 unanimously BUILD. Parent independently ran
`cargo check --offline --workspace` and `cargo test --offline --workspace` after
all first-wave builders: exit 0, 275 passed / zero failed, 59 result suites.
Logs: $TMPDIR/tachyon-m10/prerequisites-parent/{0,1}.log. HEAD still 726a8de;
all current tracked/untracked diffs belong to the named M10 builders/parent.
No commits/push/install/background jobs. Scratch only $TMPDIR. Use strict TDD;
preserve actual failing and passing outputs. Report <=800 English words, exact
files/APIs and executed gates, remaining limitations. Parent owns milestone docs,
fixtures, final full gates, independent code board and checkpoint.

Signature: Hermes Agent; configured parent gpt-6-astra/openai-codex, effort ultra
(live resolution recorded in M10_PLAN). Skills: TDD, Tokio runtime, Rust baseline,
subagent-driven development, expert-board review. Do not claim milestone complete.

Build isolation: REMOVE inherited CARGO_TARGET_DIR in each command and pass
`--target-dir "$TMPDIR/tachyon-m10/actor-target"` (ACTOR) or
`--target-dir "$TMPDIR/tachyon-m10/authorized-target"` (MUTATION). Environment
exports can leak between terminal callers; nested Cargo fixture commands must not
inherit an outer CARGO_TARGET_DIR. Format only your files/crate, never workspace.

## ACTOR slice (one writer)

Own only tachyon-core/src/lib.rs, src/verification.rs, src/ownership.rs as needed,
and focused core actor/verification tests. Do not touch Cargo manifests, mutation,
models, tools, verify crate, gateway, fixtures or milestone docs. Existing public
M9 APIs stay compatible. No debugging/model/mutation orchestration yet; this slice
makes the existing actor ready and closes M10's M9 liveness/completion gaps.

Delivered dependencies (already independently tested):
- TaskOwnership::lifetime() -> Arc<dyn Send + Sync>, OwnedWorkers<T> keeps real
  workers/admission alive through actor panic/abort. shutdown() is awaited and
  idempotent, closes mailbox admission, waits for final owner release.
- WorkspaceLease::acquire(root: &Path, cancel: &CancellationToken)
  -> Result<WorkspaceLease, ToolError>; .root() returns canonical &Path; cloneable,
  lock released only after last clone. All workspace stages share its registry.
- tachyon_verify::run_with_lifetime(plan, context, cancel, lifetime) keeps shared
  lease/owner anchor in actual workers and blocking scans/spool writes. Aborting
  the outer run or its close waiter cancels but drains actual process cleanup.

Implement owned asynchronous actor jobs for:
1. ConfigureVerification baseline capture, under shared lease.
2. Verification planning/source scans, under shared lease; release before calling
   verifier (no recursive lease).
3. Final authorized source rehash, holding the lease continuously through the
   SUPERVISOR'S durable VerificationFinished/Completed transaction.
4. Control cancellation/drain, without awaiting a worker inline in a mailbox
   handler. GetState and steering remain serviceable during any of these jobs.

Bind each result to its private operation/run identity AND task revision. Results
from a cancelled/stale operation cannot configure acceptance, start verification,
replace newer state, or complete. Internal job results carry guards, not serialized
proof fields a caller can forge. Keep the current non-verifier graph safety guard
until the later debugging integration adds private terminal/effect proof.

Persist steering/control intent promptly before lengthy drain. Pause/cancel reply
only after actual workers drain. Further controls/GetState must remain responsive
while those acknowledgements wait. If preserving terminal-state semantics requires
a defaulted pending-control field, use one coherent journal/replay path; don't mark
Cancelled then require illegal post-terminal state writes. Shutdown bypasses full
mailbox, cancels pending work, and does not resurrect terminal tasks. A dropped
caller reply receiver does not abort cleanup or release admission early.

Every spawn_blocking closure carries BOTH lease and ownership lifetime anchors.
Do not solve responsiveness by detached writes. OwnedWorkers/drain jobs remain
owned; every select join_next arm is guarded against empty sets. Sends/replies are
cancellation-aware; don't introduce a sender kept alive by its own actor. Any
small private test barrier must pause a real production path, not replace it with
a fake assertion. Avoid sleeps as the sole evidence of a critical race.

Required RED/GREEN tests through actual supervisor:
- Held workspace lease stalls baseline/planner while get_state/steering respond.
- Stalled final rehash does not block steering; stale completion is refused.
- Competing same-root mutation/lease cannot enter between final rehash and durable
  Completed (deterministic barrier); test canonical alias spelling too.
- Pause/cancel acknowledgement waits for actual verifier process cleanup/reap, but
  another GetState/steering request is serviceable during that drain.
- Full/pending channels, dropped waiters and owner shutdown don't deadlock.
Retain existing ownership and M9 gates, including wrong patch and stale result.

Run targeted new RED/GREEN tests, then `cargo test --offline -p tachyon-core`,
`cargo check --offline --workspace`, and strict all-target core Clippy. Use
--target-dir as above. Parent will review/gate combined source after both slices.
Describe the internal job/control extension points the later debugging writer can
reuse, without implementing that next slice now.

## MUTATION slice (one writer)

Own only tachyon-mutation source/tests/docs. Existing recovery changes are approved
prerequisites in this working tree; preserve them. No core, tools, verify or model
edits. Parent is not editing this crate while you work.

Integration gap: normal M8 prepare/commit are trusted low-level APIs without
ToolsContext. Core cannot authorize exact random temp paths before prepare unless
the batch ID is known. The approved plan requires exact policy before preparation
and before consequential use, plus safe per-file cancellation boundaries.

Add/re-export any needed types but preserve existing APIs. Pin these new methods:

MutationEngine::prepare_authorized(
    &self, context: &ToolsContext, batch_id: MutationBatchId, specs: &[PatchSpec]
) -> Result<PreparedBatch, MutationError>

MutationEngine::commit_authorized_up_to(
    &self, context: &ToolsContext, prepared: &PreparedBatch, limit: usize
) -> Result<CommitReport, MutationError>

The trusted runtime reserves/persists batch_id BEFORE prepare and uses a unique
stable task/attempt directory. Reject reused IDs/sibling batches instead of
truncating unowned temp files or silently adopting an old journal. Normal prepare
has no unknown batch ID gap; a crash before its complete journal record is still
Unknown, never blindly replayed. State and workspace must be canonical, disjoint
in BOTH directions; reject source/state overlap, aliases, symlink/nonregular
source/temp paths. Parent may use metadata getters if you add them; document them.

Preflight ALL specs/paths/permissions before any workspace temp/source write:
exact normalized target uniqueness; preimage hashes; per-target fs.metadata,
fs.read, mutation.patch, fs.write; exact derived temp fs.metadata/fs.write and
consumption fs.delete. Refuse existing unowned temp even if bytes match. The exact
operation approval binding includes batch identity, paths and pre/post hashes.
Internal engine state/artifact-spool writes are trusted storage, not extra source
permissions. Reading retained artifacts when needed follows the existing strict
recovery authorization rules. Specific deny/Ask always wins broad Allow. A failed
preflight performs ZERO workspace mutation. Recheck at each consequential use.

commit_authorized_up_to validates the descriptor against strict task-scoped journal
truth and authorizes every pending operation before the requested boundary. It
must work repeatedly with the original prepared descriptor: derive already
committed files from the journal, not stale Prepared flags. `limit=1` lets the
runtime recheck cancellation/revision/policy between real per-file commits.
It does not itself grant completion or blindly resume after a crash; the runtime
still selects explicit recover_scoped for unknown effects. Do not weaken strict
scoped recovery or erase torn-tail evidence to make normal writes proceed.

Required RED/GREEN real-filesystem tests:
- target denial and exact temp write/delete denial cause zero workspace mutation;
- specific deny behind broad grants; Ask cannot self-approve;
- unauthorized reads occur before any preparation effects;
- repeated one-file commits produce correct completed receipt and skip prior files;
- cancellation boundary simulated by stopping after one return preserves a true
  partial batch; explicit scoped Finish/Compensate remains valid;
- alias/escaped paths, state overlap, foreign temp collision, unknown/sibling ID,
  malformed journal/descriptor and stale source cannot authorize a write.
Use no synthetic success receipts. Existing low-level trusted APIs stay compatible.
Run mutation tests and strict Clippy, plus workspace check. Document exact normal
write/recovery grants in RECOVERY.md; return APIs and integration caveats.
