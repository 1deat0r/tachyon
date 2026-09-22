# M10 prerequisite builders — approved scopes

Read AGENTS.md and docs/milestones/M10_PLAN.md r2 before editing. Its independent
five-seat R2 unanimously BUILD, batch deleg_474534b9. Owner authorized continuing
Tachyon's next milestone. Baseline HEAD 726a8dea9962d0388d628162e0dc2486a1147e51;
parent verified default 221 tests/53 suites and all-feature 229 tests/53 suites,
fmt/check/strict-Clippy all green. Baseline dirt: only this milestone's new docs.
Other builders will edit DISJOINT file families; never reformat or revert theirs.

Signature: Hermes Agent; parent gpt-6-astra/openai-codex; live configured reasoning
ultra (no override for this model). Skills: TDD, Rust baseline, board review,
subagent-driven development, Tokio runtime. Implement with strict small-cycle
red -> green tests; preserve failing-test command/output, not invented results.
Scratch only $TMPDIR. No commits, pushes, installs, background jobs, or paid APIs.
Do not edit PROGRESS/README/CHANGELOG/the plan; parent owns final integration/docs.
Do not use raw rustc with independently selected cached rlibs: mixed metadata gives
same-name type mismatches; scratch probes use a Cargo manifest with path deps.

Each builder returns <=700 words: exact files, public API, RED then GREEN commands
and output summaries, any remaining integration need. A result is not permission
to mark M10 complete. Run only your targeted crate checks and workspace check;
parent performs canonical final full gates. Do not claim another slice's work.

## OWNERSHIP builder

Own:
- crates/tachyon-core/src/lib.rs and new src/ownership.rs;
- crates/tachyon-core/src/verification.rs ONLY to retain task ownership in the
  existing verification worker, not a broad async refactor;
- crates/tachyon-core tests for ownership/shutdown, and minimal lifecycle changes
  to existing tests so recovery truly follows awaited owner shutdown;
- crates/tachyon-store/src/lib.rs for canonical database identity getter;
- crates/tachyon-gateway/src/server.rs ONLY exhaustive CoreError mapping.

Implement process-global exclusive admission keyed by canonical state.db path AND
TaskId, never Arc identity or lexical alias. Reject duplicate recover before
loading stale state, including two independent StoreWriter handles to the same
DB. The guard belongs to actor and owned workers until drain, not arbitrary
client handles. Handle clones still talk to one actor. Creation reserves identity
before publishing the task. A typed CoreError::TaskAlreadyOwned(TaskId) maps to
`task_already_owned` at gateway. Add StoreWriter::database_path(&self) -> &Path
exposing the canonical DB path (no credentials).

Pin SupervisorHandle::shutdown(&self) -> Result<(), CoreError>: awaited, idempotent
shutdown closes command admission, cancels/drains owned work, and returns only
when ownership is released. Calls through retained clones afterward fail closed;
get_state/recovery must not accidentally resurrect a terminal task. Avoid a
self-owned sender keeping the actor alive. Ensure guard survives panic/abort of
outer wrappers while an owned effect worker still runs; document if proof cannot
be completed in this slice. No busy-poll shutdown waiting loops.

Tests: reproduced duplicate-owner constraint/status loss is now refused; aliases
and separate StoreWriter instances share guard; awaited shutdown then recover
preserves all state; concurrent recover admits exactly one; old handles cannot
write afterward. Existing core and gateway recovery tests stay meaningful/green.
Run `cargo test -p tachyon-core -p tachyon-gateway`, `cargo check --workspace`,
`cargo clippy -p tachyon-core -p tachyon-store -p tachyon-gateway --all-targets -- -D warnings`.
Do not begin the M10 runtime itself.

## RECOVERY builder

Own only crates/tachyon-mutation source/tests/docs. No core/tools/verify edits.
Add strict task/batch-scoped recovery, keeping legacy recover API compatible but
removing blanket marker-name sweeping from it. Unknown/foreign/orphan hidden
marker files must survive. Clean only exact journal-owned paths with verified
expected content identity; journal filename pattern alone never proves ownership.

Pin public API (re-export from lib.rs):
- RecoveryAction::{Finish, Compensate}
- RecoveryDisposition::{Committed, Compensated}
- ScopedRecoveryReport with batch_id: MutationBatchId,
  disposition: RecoveryDisposition, changed: Vec<ChangedFile>,
  cleaned: Vec<PathBuf>.
- MutationEngine::recover_scoped(&self, context: &tachyon_tools::ToolsContext,
  batch_id: MutationBatchId, action: RecoveryAction, allowed_paths: &[String])
  -> Result<ScopedRecoveryReport, MutationError>.

For this narrow API a journal must contain exactly the named batch; any other
batch, missing batch, gap/corruption, descriptor/path mismatch or source divergence
fails BEFORE any workspace mutation. Runtime will use a task/attempt-specific
stable directory so older attempts are not siblings in the same journal.
All normalized target paths must match exact allowed_paths (not directory grants),
and engine/context canonical workspace identities must agree. Perform strict
read-only preflight: journal integrity/shape, every target pre/post image and
needed artifact, no symlink escape, exact per-path policy for metadata/read/write/
cleanup. Authorize deletions as exact operations, not broad sweep permission.
Checks needed for action/cleanup must all pass before effects; recheck at use.
Return Committed only when complete receipt + every current postimage agrees;
Compensated only when rollback receipts + fresh preimages agree. On error the
caller retains Unknown; never fabricate successful rollback because some files
restored. Preserving orphan temps is preferable to deleting unproven user data.
The runtime will hold the shared workspace lease around this synchronous API.

Tests: valid partial finish/compensate; repeated safe reconciliation; foreign temp
and protected marker survive; corrupt/gapped/unknown/sibling journal, denied read/
write/deletion, diverged source and malicious temp/artifact path perform ZERO
workspace mutation. Reproduce parent safety probe from
$TMPDIR/tachyon-m10-safety-probe.Gyvrwy/probe.rs, but new tests assert safe behavior.
Legacy tests expecting unowned strays to be deleted must be revised explicitly
with the new safety requirement, not silently weakened. Run `cargo test -p
tachyon-mutation`, `cargo clippy -p tachyon-mutation --all-targets -- -D warnings`.

## SELECTION builder

Own only crates/tachyon-verify/src/project.rs, tests/planner.rs and new focused
selection tests. DO NOT edit verify runner.rs/lib.rs/README, core tests or parent
plan. Parent is extracting the runner's lease concurrently.

Fix aliased dependency false negatives. Resolve supported renamed/path/workspace/
target-specific dependency forms accurately or broaden unsupported forms to
workspace tests. A conservative explicit fallback is preferable to a hand-written
TOML parser pretending to understand syntax it ignores. No network/new dependency
unless unavoidable and reported first. Keep ordinary auth -> client closure
selective, excluding an unrelated crate. Preserve transitive/cycle behavior.

Prove RED on the reproduced alpha/client renamed-dependency case, GREEN after
fix: previously selected alpha passes but omitted client fails. Add inline and
separate dependency-table alias cases, inherited/workspace/target forms and
malformed cases or safe broad fallback assertions. Execute returned cargo checks
on a real small dependency-free fixture at least once. Parent will add the
supervisor-level no-false-Completed regression after ownership APIs land.
Run `cargo test -p tachyon-verify --test planner` plus new test targets,
`cargo clippy -p tachyon-verify --all-targets -- -D warnings`.

## USAGE builder

Own only crates/tachyon-models source/tests. No core/verify/mutation/docs outside
that crate. Add and re-export:
- UsageProvenance::{Unknown, ProviderReported, Scripted} (Unknown default);
- ModelUsage { input_tokens: Option<u32>, output_tokens: Option<u32>,
  provenance: UsageProvenance }, Default and serde;
- ModelResult.usage: ModelUsage with #[serde(default)].

Keep existing numeric ModelResult input_tokens/output_tokens for compatibility,
but make documented new usage authoritative for availability. HTTP adapter: each
missing/malformed/overflowed count -> None; an explicitly reported zero -> Some(0).
Provenance ProviderReported when a usage object is supplied (even if individual
fields are unavailable), Unknown when none is supplied. FakeModelProvider uses
Scripted, with Some of its configured counters. Never infer fake/real from a
provider name. Legacy deserialization missing usage yields Unknown/None.
Do not change FakeResponse's existing public fields; synthesize its result usage.

Strict RED/GREEN tests: absent object, malformed object/fields, negative/fractional/
overflow values, one-sided usage, explicitly reported zero, normal counts, fake
scripted provenance, legacy serde. Update every owning-crate struct literal so
workspace check stays green; report foreign-callsite changes needed to parent.
Run `cargo test -p tachyon-models`, `cargo check --workspace`,
`cargo clippy -p tachyon-models --all-targets -- -D warnings`.

## Parent ownership

Parent owns shared workspace lease module in tachyon-tools, extraction/adoption
in tachyon-verify/src/runner.rs, fixture data, milestone docs, shared interfaces,
core runtime after OWNERSHIP finishes, integration proof and final checkpoint.
Before the runtime builder starts, parent will verify all prerequisite interfaces
and test gates; nobody codes against a sibling's guessed/unwritten API.
