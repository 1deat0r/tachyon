# M10 — full token-refresh debugging task

Revision: r2. Status: APPROVED FOR IMPLEMENTATION. Plan R2 unanimously BUILD;
implementation and the separate code/evidence gate remain outstanding.

## Authority and baseline

- Owner request: "Continue with best next milestone" (2026-09-22).
- Dependency gate: M9 complete at `726a8dea9962d0388d628162e0dc2486a1147e51`;
  clean `main`, prior desktop development session ended after checkpointing M9.
- Parent independently reran the pinned Rust 1.98.1 baseline: fmt, workspace
  check, workspace test (53 suites / 221 tests / zero failures), strict workspace
  Clippy, all exit 0. Logs: `$TMPDIR/tachyon-m10/baseline-{0,1,2,3}.log`.
- Independent all-feature check/test/strict-Clippy also passed (229 tests in
  53 suites). Logs: `$TMPDIR/tachyon-m10/baseline-all-features-{0,1,2}.log`.
- Governing contracts: `AGENTS.md`, architecture freeze, implementation spec,
  implementation plan M10, and acceptance/benchmark specification A4.
- Execution signature: Hermes Agent; parent model `gpt-6-astra`, provider
  `openai-codex`; reasoning effort `ultra` resolved from the default profile's
  live configuration on 2026-09-22 (no model-specific override for this model).
  Tools: file read/search/write/patch, terminal, session_search, delegate_task.
  Skills: rust-workspace-setup, test-driven-development,
  board-of-expert-agents-review, subagent-driven-development; code-review skill
  will be loaded before the implementation board.

## Deliverable and non-goals

Deliver a usable, provider-neutral **core runtime** that joins existing evidence,
model, IR/scheduler, recoverable mutation, and supervisor-owned verification.
Exercise it with a runnable A4 benchmark/example and deterministic CI tests:

> Find why authentication occasionally fails after token refresh and fix it.

Do not implement agent logic in the CLI, TUI, benchmark driver, or gateway.
The benchmark is a thin runtime host: construct trusted inputs/provider, start
and observe the supervisor, optionally steer it, serialize real outcomes.
Fake/scripted model responses are explicitly test/replay providers, never evidence
of genuine model reasoning. The runtime accepts the existing `ModelProvider`
trait; an optional local HTTP adapter run may be reported separately if available.
M10 completion does not claim a production live-model quality evaluation.

No TUI polish, web/browser access, distributed execution, new providers, external
network side effects, arbitrary shell, credential capabilities, or M13 speed
claims. Systematic kill-at-every-boundary testing remains M12; one real
subprocess kill/reopen/reconciliation proof is required here. Existing public M9
verification APIs and tests remain supported.

## Runtime ownership and composition

1. Add a narrow debugging module under `tachyon-core`. `SupervisorHandle`
   provides a trusted start API taking runtime-only dependencies (canonical
   `Arc<ToolsContext>`, `Arc<dyn ModelProvider>`, role/model selection, bounded
   evidence requests, task-specific mutation state directory, immutable
   acceptance contract/risk if not already bound, and execution budgets).
   Provider objects/secrets are never serialized into task state.
2. Admit exactly ONE supervisor per `(canonical state-database path, TaskId)`
   process-wide, including separate StoreWriter handles/alias spellings. Acquire
   an exclusive ownership guard BEFORE recovery reads or creation publication;
   duplicate recovery returns a typed ownership error, never a stale second
   actor. Handle clones share the admitted actor. Ownership remains held until
   actor AND owned effect workers drain; abort/drop cannot release it while an
   effect still runs. Supply an awaited shutdown/drain API for genuine restart
   tests; update existing tests that incorrectly recover while the old owner is
   live. Host-wide multi-process writers remain outside this milestone's scope.
   The task supervisor alone journals graph revisions, stage/status transitions,
   evidence summaries/artifact references, changed-file receipts, and completion.
   A worker may propose a stage graph/result, but a private, run-ID + task-ID +
   revision-bound message and supervisor acknowledgement are required before
   scheduling each stage. A stale or foreign worker cannot overwrite task state.
   There is no public `mark_success`, `accept_report`, or arbitrary state setter.
3. Use bounded worker/control channels and owned join handles. Lease acquisition,
   snapshot scans, planner scans, final rehash and worker drains run as owned
   actor jobs, NOT inline awaits that stop mailbox service. Stage message sends
   and acknowledgement waits select on cancellation/owner loss, including full
   channels. The actor keeps processing controls while drain jobs are pending;
   it never waits for a worker which itself waits for an actor acknowledgement.
   Persist the steering revision/control intent promptly; a successful pause or
   cancel acknowledgement is emitted only after actual effect workers drain.
   A cancelled stage cannot launch more nodes or commit a queued late proposal.
   No detached `spawn_blocking` writes may outlive a held grant/owner. Barrier
   tests cover stalled scans, full channels, pending acknowledgements, and
   cancellation between graph acknowledgement, lease acquisition and commit.
4. Execution proceeds in bounded stages: evidence -> model -> proposed patch ->
   verification. Every evidence, provider, and mutation operation is compiled to
   IR with real access/resource/effect declarations and validated before launch.
   The M5 router may choose initial evidence/classification, but its placeholder
   nodes (empty access/revision 0) MUST be lowered/revalidated by the M10 compiler.
   Trusted explicit evidence requests supplement routing; no secret inference.
5. The first slice supports typed `fs.read`/repository lexical evidence and
   `mutation.patch` proposals only. Unknown capabilities, extra/untyped arguments,
   empty proposals, excessive sizes, raw shell, credentials/network, and arbitrary
   provider-supplied access/effect/retry metadata fail closed. Provider `Complete`
   is never completion authority. Unsupported `RequestEvidence`/`NeedUserInput`
   return a durable, intelligible blocked outcome, not fabricated progress.
   A bounded explicit subsequent attempt can handle a rejected/failed proposal;
   no automatic unbounded reasoning loop is introduced.

## Evidence and model boundaries

- Gather at least two independent real workspace evidence operations through
  the scheduler. Policy checks apply to exact canonical file paths BEFORE reads;
  lexical search cannot walk/read a specifically denied file under a broad grant.
- Compact source excerpts use the existing provenance-bearing EvidencePackage and
  model context assembler. Preserve full-file hashes associated with excerpts.
  Repository/log text remains `WorkspaceData`, including injection-like content.
- Trusted user objective, steering messages and hard constraints enter the
  appropriate trusted context. Never promote workspace text into a grant.
- Default hard bounds: at most 16 evidence requests, at most 256 KiB returned
  evidence per stage, at most 8 patch files and 1 MiB replacement bytes per attempt,
  configurable bounded model deadline, no retries for consequential effects.
  Enforce bounds before allocation/spooling when practical, not just after reading.
- Bind the complete canonical file/hash set whose content was actually supplied
  to reasoning, including non-target dependency/reference/log evidence. Under
  the mutation lease, revalidate EVERY member before preparing anything. Changed,
  denied or unverifiable evidence invalidates the entire proposal with zero
  preparation/source writes; a delayed-provider test changes only a non-target
  evidence file. Bind patch `base_hash` to that supplied version, never a fresh
  hash substituted for stale model evidence. Unsupported creation/unevidenced
  files are refused in this slice.
- Stream progress is not durable truth. Drain/drop ephemeral events without an
  unbounded accumulation; record bounded diagnostics, measured call counts and
  token counts as reported by the provider (unknown cost is null, not zero).
- Add per-result usage availability/provenance at the ModelResult boundary:
  optional input/output counters plus ProviderReported/Scripted/Unknown source.
  Keep any legacy numeric counters for compatibility, but benchmark output MUST
  use the new metadata, not infer availability from zero or provider names.
  Missing/malformed/overflowed adapter usage is null, explicitly reported zero
  remains zero, fake counts are Scripted; estimates are separate. Test all cases.

## Policy, constraints and mutation

- Acceptance and source baseline are supervisor-bound BEFORE any mutation and
  cannot be weakened by a later start/retry or model output. A task-specific
  mutation directory must remain outside the source snapshot, be stable across
  recovery, and be bound to the canonical workspace/task identity.
- Supported hard executable constraints are exact ID/text bindings to acceptance
  clauses. ChangedPathsWithin/FileUnchanged clauses must also constrain proposed
  writes BEFORE mutation, not merely reject completion after forbidden damage.
  Any newly introduced unbound hard constraint stops further writes fail-closed.
- Authorize `mutation.patch` and underlying per-file read/write scopes on canonical
  paths against the exact proposal before prepare, and recheck containment and
  policy at the consequential boundary. No model can self-approve an Ask.
- Use the real M8 MutationEngine, retained preimages/postimages, journal, and
  stale-base guards. Preserve recoverable-not-globally-atomic semantics.
- Persist the task's mutation intent before prepare; persist the batch identity
  before commit. Durable effect disposition is Committed, Compensated,
  ProvenNoEffect, or Unknown. Committed requires matching completed receipts
  plus current postimages and is the only successful mutation outcome.
  Compensated requires rollback receipts and all fresh preimages; ProvenNoEffect
  requires proof no preparation/commit occurred. Both resolve uncertainty but
  do NOT count as successful work: continue only with a fresh proposal under
  unchanged acceptance/baseline. Partial/error/timeout remains Unknown until
  explicitly reconciled, never successful merely because a future ended.
- Extract the M9 canonical process-wide workspace lease into a shared lower-level
  module in `tachyon-tools`. ALL workspace stages participate: baseline capture,
  evidence acquisition, mutation, verification and final completion rehash/commit.
  An exclusive lease around an evidence stage is sufficient; independent reads
  WITHIN that stage still run concurrently. Canonical aliases share the same key.
  The actual effect worker owns the lease/owner guard, not its scheduler-facing
  future. Check cancellation and revision again after acquiring the lease and
  immediately before the effect barrier. A synchronous mutation is owned and
  drained to a safe per-file boundary; timeout/abort of its wrapper cannot release
  the lease or authorize the next conflicting run while it still writes.
  Acquire per stage and release before invoking M9 verification (no reentrant
  locking). After the verifier returns, acquire a fresh lease for final authorized
  rehash, retain it through the supervisor's durable completion transaction, then
  release. Revision/cancellation is rechecked before that transaction. A competing
  mutation either waits through the commit or invalidates the rehash; it cannot
  enter between them. This is process-wide, not host-wide/hostile-filesystem safety.

## Durable steering and recovery

- Persist a defaulted debugging state for legacy compatibility: run identity,
  revision/stage, authoritative graph, terminal node records, workspace/mutation
  binding, artifact references, and unresolved mutation/recovery disposition.
  Do not store source blobs or secret-bearing provider configuration in events.
- Before M9 completion starts, require every current work node terminal and
  successful, no active worker, no stale revision, and no unresolved effect.
  Replace M9's blanket non-verifier-graph guard with this evidence-backed check;
  do not simply delete it. Serialized or user-supplied "succeeded" data is not
  fresh authority. Recovery explicitly re-establishes the relevant disk evidence.
- On restart, an in-flight stage becomes Recovering, not Completed. Rerun only
  safe evidence/model work after explicit continuation; do not blindly reapply a
  proposal. Reconcile the task's own M8 journal under the same policy/lease.
- For a valid partially committed mutation, explicit recovery can finish or
  compensate as selected by the trusted caller. Divergence, journal gaps, unknown
  batches, unexpected siblings, or policy denial stay blocked; no automatic
  overwrite. Recovered successful mutations still require fresh M9 verification.
- Do NOT adopt M8's bulk `recover`/workspace sweep as the runtime entry. Add a
  strict read-only preflight followed by exact task/batch-scoped reconciliation.
  Before ANY workspace mutation, validate journal integrity, the expected task's
  batch set/plan, every current pre/postimage, needed artifacts, and exact policy
  and hard-constraint authorization for reads, writes AND deletions. Corruption,
  gaps, unknown/sibling batches, divergence or denied scope cause zero workspace
  changes (including compensation/cleanup). Revalidate before consequential use.
  Remove blanket marker-name sweeping from the legacy path too: only exact
  journal-owned temp paths with the expected content identity may be cleaned.
  Unknown/orphan/foreign marker files are retained, not treated as garbage. Test
  another task's prepared temps, protected `.operator.tachyon-tmp-keep` files,
  corrupt-journal compensation, denied cleanup and rollback/preimage disposition.
- Steering while a delayed model is active must be promptly accepted, persist a
  revision increment, cancel/discard the old proposal, and cause zero stale writes.
  Pause/cancel must leave no worker capable of committing after acknowledgement.
  A resume with fresh runtime dependencies uses the persisted immutable contract;
  it does not replace acceptance or reset the pre-mutation baseline.
- Selective verification must resolve renamed/path/workspace/target-specific
  dependency forms accurately OR conservatively broaden unsupported forms to
  workspace verification. It must never silently ignore dependency aliases.
  Add the reproduced renamed `alpha` -> `client` counterexample to a production
  supervisor test: selected tests cannot allow Completed while the dependent
  fails. Pin the canonical ordinary-dependency fixture's selected auth+dependent
  checks; omit unrelated crates unless Full risk/explicit acceptance or an
  honestly reported conservative fallback requires broadening. Keep M9 command
  verifiers serialized under their current write claims; parallel-verifier speed
  is not claimed. Native/read-only acceptance clauses may run in parallel.

## Canonical fixture and benchmark

Create `fixtures/auth-refresh/` as a small dependency-free nested Cargo workspace,
excluded from root members. Include:

- auth/session implementation with an intentionally incorrect stale-response
  overwrite: two refresh attempts complete out of order and an older token
  replaces the newest generation, causing authentication failure;
- a correct comparison/reference implementation, multiple call sites, focused
  auth regression tests, an unrelated crate, and protected `migrations/`;
- deterministic event ordering/barriers, NOT probabilistic sleeps to reproduce
  the race; old code MUST fail the real regression, repaired code MUST pass;
- fixtures are synthetic, contain no real credentials, and are copied to a fresh
  scratch directory for each run; never patch the checked-in broken fixture;
- Cargo generates the fixture lockfile offline before capturing source baselines.

The runtime-host example lives under `tachyon-core/examples/` (or an equally thin
benchmark host) and uses the production supervisor path, NOT a second orchestration
implementation. Output machine-readable measurements from real execution:
verified outcome, node start/end times and maximum independent evidence concurrency,
model/Jev/tool counts, estimated/billed tokens distinguished, changed paths,
selected checks, durable task ID/revision, recovery outcome, wall/first-evidence/
first-edit/final-verification times. Unknown/unmeasured metrics are null.

Full and serial runs use the SAME provider script, fixture and acceptance. Since
there is no speculative or Jev stage, no-speculation/no-judgment configurations
explicitly coincide with full in this slice. A conventional reference-loop run
uses the same provider/operations/acceptance but serial model-tool control; any
unsupported benchmark mode is clearly marked unimplemented and cannot support a
performance claim. Report sample count; p50/p95 only for a real repeated sample,
without claiming an M13 speed win from this integration milestone.

## Acceptance gates (all required)

G1. Clean baseline retained; red-green tests precede each production behavior.
G2. Broken fixture regression fails for stale-refresh behavior, not build/setup;
    same tests pass after the runtime applies the model-proposed correct repair.
    Checked-in tests/manifests/reference/migrations remain unchanged.
G3. Real scheduled evidence has overlapping execution intervals (>=2 outstanding
    independent reads), compact model context with source/hash/trust provenance,
    validated graphs and explicit access/effect/resource declarations. Serial
    mode records maximum concurrency 1; no fabricated timing/counter data.
G4. The production supervisor path applies a typed proposal via M8, runs selected
    M9 commands and reaches durable Completed only on correct repair. Wrong patch,
    provider Complete alone, malformed/unknown capability, denied/escaped path,
    migration write and stale evidence all refuse success/forbidden writes.
G5. Delayed-provider steering/pause/cancel tests prove the actor responds and no
    late write occurs. Add full-channel, pending-ack and stalled-scan barriers;
    active mutation abort/timeout must hold ownership until actual drain. A new
    unbound hard constraint fails before mutation. Duplicate create/recover/start
    admission cannot lose acknowledged constraints, acceptance, revision or status;
    only an awaited owner shutdown permits recovery.
G6. Actual subprocess death after a journaled partial mutation; fresh process
    opens the same state, reconciles without reapplying stale proposals, verifies
    and recovers the same task identity/contract. Diverged sources remain untouched.
    Strict scoped preflight proves zero workspace mutation on denied/corrupt/unknown
    recovery. Foreign temps/protected marker files survive. Compensation resolves
    uncertainty but never counts as a completed repair.
G7. Same-workspace baseline/evidence reads, mutation and verification cannot
    overlap conflicting work across independent runs. Cover alias spellings,
    cancellation/drain, complete evidence-version rechecks and a barrier forcing
    a competing mutation against final rehash + durable completion commit.
    No leaked process/worker remains after tests.
G8. Runnable example and new focused tests plus all-feature and default full
    workspace fmt/check/test/strict-Clippy pass. Independent five-seat code board
    reaches BUILD after parent-verified blocker closure. README/CHANGELOG/
    PROGRESS/docs-freshness tests and a measured M10 report are updated. Commit
    only the reviewed scope; no push/deploy. M11 remains next, not started.

## Ownership and sequence

1. Five independent plan seats: architecture/spec, safety/effects, async/steering,
   verification/benchmark honesty, adversarial/cold read. Read-only, no implementation.
2. Adjudicate every material finding against live code, revise this document, and
   obtain unanimous BUILD with verified closures. Freeze while a round reads it.
3. Implement M10 prerequisites with red-green tests: shared leases/task ownership,
   scoped recovery, safe dependent selection and usage provenance. Pin interfaces
   before dependent builders consume them; assign one writer per file family.
4. One writer owns core runtime/integration. A separate writer may own only the
   fixture/benchmark data; interfaces are pinned before parallel implementation.
   Parent integrates, exercises the production path and owns final docs/commit.
5. Independent code board, corrective tests/fixes, final canonical rebuild and
   benchmark. Never measure against a sibling's mutable build directory.

## Board record

- Plan R1: 4 CONDITIONAL, 1 REJECT; all material findings accepted after parent
  adjudication. The five seats used isolated contexts on the inherited model;
  this is tool/evidence independence, not a heterogeneous-model panel.
- Plan R2: unanimous BUILD from architecture, safety, async/durability,
  verification/benchmark, and adversarial seats; each quoted the r2 resolving
  rules and checked source feasibility. Parent checked those quotes against the
  edited contract. All ten material plan findings have explicit executable gates.
  Approval batch: `deleg_474534b9`. This authorizes implementation, not milestone
  completion. Code/evidence review remains required.
- Code R1: not started.

## Adjudication log

Parent reproduced the ownership, safety, selective-check and compensation probes
against current sources with Cargo (offline), all probe assertions exit 0.
Evidence: `$TMPDIR/tachyon-m10/adjudication-r1/{ownership,safety,selection,async-compensation}.log`
and `results.json`. An initial raw-rustc link attempt hit duplicate crate metadata;
the Cargo-based rerun above is the valid executed evidence, not that failed build.

1. **CONFIRMED: duplicate supervisor loses acknowledged state.** Parent observed
   original revision 1 / one constraint, duplicate Cancelled revision 0 / none,
   recovered Cancelled revision 0 / none despite journal `constraint` event.
   Source: core/lib.rs 490-523; store/lib.rs 257-270. Closed in r2 ownership guard
   contract and G5; implementation regression still required.
2. **CONFIRMED: evidence/baseline arbitration omitted.** Independent scheduler
   grants are local; verifier runner 331-338 acknowledges this. Closed in r2's
   ALL-stages lease rule and G7. Source-traced, not a dynamic overlap proof yet.
3. **CONFIRMED: bulk recovery deletes foreign/protected files and writes before
   reporting journal gaps.** Parent probe: foreign temp deleted=true, unrelated
   marker deleted=true, swept=2; journal gaps=[3], compensation still occurred.
   Source: mutation/engine.rs 344-360,546-579,652-655. Closed in strict scoped
   preflight/proven-owned cleanup contract and G6; regression pending.
4. **CONFIRMED: target-only freshness misses other reasoning inputs.** Parent
   changed only dependency.rs; target commit still completed. M8 checks targets
   at engine.rs 159-176, not the model's evidence set. Closed in complete
   supplied-evidence manifest recheck and G7; delayed-provider regression pending.
5. **CONFIRMED: inline scans/drain can block mailbox or ack progress.** Existing
   core verification.rs 103,167-173,259-282 awaits scans/drain from actor handlers.
   New acknowledgement channel makes this a concrete dependency to avoid, not
   an executed M10 deadlock (M10 is not built). Closed in owned-job/cancellable
   acknowledgement/drain protocol and G5 barrier tests.
6. **CONFIRMED: scheduler terminal status precedes effect drain.** Scheduler
   cancellation releases its grants while synchronous mutation may still run;
   mutation/engine.rs 222-294 is not cancellable. Closed in actual worker-owned
   lease/owner lifetime, safe per-file boundaries and G5/G7 abort tests.
7. **CONFIRMED: compensation was excluded from uncertainty resolution.** Parent
   rollback restored all preimages, with `completed=false` and RolledBack states.
   Closed in explicit Committed/Compensated/ProvenNoEffect/Unknown outcomes;
   compensation resolves effects but requires a fresh repair attempt.
8. **CONFIRMED: aliased dependent omitted.** Parent baseline workspace exit 0;
   changed alpha alone selected one passing alpha check; full workspace exit 101
   on client's `preserves_client_contract`. Source: verify/project.rs 154,200-205.
   Closed in resolve-or-broaden rule, pinned selected set and G4/G8 regression.
9. **CONFIRMED: missing usage becomes numeric zero.** Source:
   models/openai_compat.rs 294-320; provider.rs 77-85; fake.rs 23-31.
   Closed in optional per-result usage/provenance contract and adapter tests.
10. **CONFIRMED by source: final completion lacks a continuous lease.** Verifier
    runner drops at 389; core rehash at verification.rs 259 and commit at 233 are
    separate. Closed in lease-through-rehash-and-completion transaction rule,
    with G7 competing-mutation barrier proof required.

These are plan-level resolutions, NOT claims that the code defects are fixed.
R2 must quote the resolving rules; the later code board must execute the gates.
