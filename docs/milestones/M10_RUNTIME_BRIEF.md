# M10 runtime integration slice — debugging stage compiler + supervisor wiring

Read AGENTS.md and docs/milestones/M10_PLAN.md r2 (APPROVED FOR IMPLEMENTATION,
plan R2 unanimous BUILD). Owner requested continuation; parent owns milestone
docs, fixtures, final full gates, code board and checkpoint. Do not claim
milestone complete.

Signature: Hermes Agent; parent muse-spark / meta-ai. Skills: TDD, Tokio
runtime, Rust baseline, subagent-driven development. No paid/model HTTP calls,
no push/deploy, no commits. Scratch only under $TMPDIR. In every cargo command,
REMOVE inherited CARGO_TARGET_DIR and pass `--target-dir "$TMPDIR/tachyon-m10/runtime-target"`.
Format only your files, never the workspace.

## Ownership (one writer)

Write ONLY `crates/tachyon-core/src/runtime.rs` (new) plus focused integration
tests `crates/tachyon-core/tests/runtime_*.rs` (new). One-line `mod runtime;`
+ re-export in `src/lib.rs` is allowed. Do NOT touch Cargo manifests, other
crates, fixtures, docs, or existing core APIs/tests. Existing public M9 APIs
stay compatible; all existing gates must keep passing.

## Delivered dependencies (already tested, reuse — do not reinvent)

- `create_task` / `recover_task` / `SupervisorHandle` (pause/cancel/steering,
  awaited `shutdown`) in `tachyon-core::lib`.
- `TaskOwnership`, `OwnedWorkers` (`src/ownership.rs`); owned actor jobs for
  baseline capture, planner scans, final rehash holding the shared lease
  through the durable completion transaction.
- `tachyon_tools::workspace::WorkspaceLease` shared by ALL stages; verifier
  `run_with_lifetime` keeps lease/owner anchors in actual workers.
- `MutationEngine::prepare_authorized` /
  `commit_authorized_up_to` (`tachyon-mutation::engine::authorized`);
  strict task-scoped `recover_scoped` — never bulk `recover`.
- Model boundary: `ModelProvider` trait + `ModelResult` usage/provenance
  (`ProviderReported`/`Scripted`/`Unknown`); missing usage is null, never zero.
- Fixture `fixtures/auth-refresh/` (separate workspace, deliberately broken
  `auth-session`, correct reference, protected `migrations/`, unrelated crate).
  Copy it to fresh scratch per run; never patch the checked-in fixture.

## Build

A narrow debugging module: stages evidence -> model -> proposed patch ->
verification, all through the production supervisor path (no second
orchestration implementation):

1. Trusted `start` taking runtime-only deps: `Arc<ToolsContext>`,
   `Arc<dyn ModelProvider>`, role/model selection, bounded evidence requests,
   task-specific mutation dir OUTSIDE the source snapshot and stable across
   recovery, immutable acceptance contract + risk, execution budgets. Provider
   objects/secrets never enter task state.
2. Stage compiler: every evidence/provider/mutation op becomes IR with real
   access/resource/effect declarations, validated before launch. Lower any
   router placeholder nodes (empty access/rev 0); revalidate.
3. Slice support: typed `fs.read`/lexical evidence + `mutation.patch` only.
   Unknown capabilities, untyped args, empty/oversize proposals, raw shell,
   credentials/network, model-supplied access/effect metadata fail closed.
   `Complete` is never completion authority. `RequestEvidence`/`NeedUserInput`
   yield durable blocked outcomes, not fabricated progress. One bounded retry
   of a rejected/failed proposal; no unbounded loop.
4. Bounds (defaults): <=16 evidence requests, <=256 KiB evidence/stage, <=8
   patch files + 1 MiB replacement bytes/attempt, bounded model deadline, no
   retries for consequential effects. Enforce pre-allocation where practical.
5. Evidence freshness: bind the FULL canonical file/hash set actually supplied
   to reasoning; under the mutation lease revalidate EVERY member. Changed,
   denied or unverifiable evidence invalidates the proposal with zero writes.
   `base_hash` binds the supplied version, never a fresh substitution.
6. Acceptance + source baseline bound BEFORE any mutation; later start/retry
   or model output cannot weaken them. Exact ID/text hard-constraint bindings
   constrain proposed writes BEFORE mutation. New unbound hard constraint
   stops writes fail-closed.
7. Persist mutation intent before prepare, batch identity before commit.
   Outcomes Committed/Compensated/ProvenNoEffect/Unknown per plan; only
   Committed (matching receipts + current postimages) counts as success.
8. Crash recovery: in-flight stage becomes Recovering, never Completed. Rerun
   safe evidence/model work only after explicit continuation; reconcile via
   strict scoped preflight; zero workspace writes on denied/corrupt/unknown.
9. Steering: delayed-model steering/pause/cancel promptly accepted, revision
   bumped, old proposal discarded, zero stale writes; ack only after workers
   drain. Resume reuses persisted contract/baseline.
10. Selection: renamed/path/workspace/target-specific deps resolve exactly or
    conservatively broaden to workspace verification; never silently ignore
    aliases. Add the `alpha` -> `client` counterexample as a supervisor test.
11. Measurements for the later benchmark host: verified outcome, node
    start/end times, max independent-evidence concurrency (real overlapping
    intervals, >=2), model/tool counts, estimated vs billed tokens, changed
    paths, selected checks, task ID/revision, recovery outcome,
    wall/first-evidence/first-edit/final-verification times. Unknown = null.

## Required RED/GREEN tests (real filesystem/scheduler, no synthetic receipts)

- Happy path on a scratch fixture copy: broken regression fails, correct
  repair applies via M8, selected M9 checks pass, durable Completed.
- Wrong patch, provider-Complete-alone, malformed/unknown capability,
  denied/escaped path, migration write, stale (non-target) evidence: all
  refuse success / forbid writes.
- Delayed-provider steering/pause/cancel: actor responds, no late write;
  unbound hard constraint fails before mutation.
- Duplicate create/recover/start: no loss of constraints/acceptance/revision;
  only awaited shutdown permits recovery.
- Subprocess death after journaled partial mutation; fresh `recover_task`
  reconciles without reapplying stale proposals (same task/contract).
- Alias-spelling competing mutation cannot enter between final rehash and
  durable commit (deterministic barrier).

## Gates

New RED/GREEN tests, then `cargo test --offline -p tachyon-core`,
`cargo check --offline --workspace`, strict all-target core Clippy, all with
`--target-dir` above. Report <=800 words: exact files/APIs, executed gates
with pass/fail outputs, remaining limitations and the extension points the
benchmark-host writer will use. Parent integrates and runs final gates.
