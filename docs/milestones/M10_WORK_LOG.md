# M10 implementation evidence — in progress

Authority: approved M10_PLAN.md r2; independent plan batch `deleg_474534b9`
returned five BUILD verdicts. This is NOT implementation/milestone sign-off.
Date: 2026-09-22. Signature: Hermes Agent, parent gpt-6-astra/openai-codex,
configured reasoning ultra (live resolution recorded in the plan).
Tools: file operations, terminal, execute_code, delegate_task.
Skills: TDD, Rust workspace baseline, Tokio runtime, independent board,
subagent-driven development. No paid/model HTTP calls, push or deployment.

## Parent: shared workspace lease

- Added cloneable `tachyon_tools::workspace::WorkspaceLease`, keyed by canonical
  root process-wide; cancellation-aware acquisition and weak registry entries.
  This is exclusion, not policy authority or a hostile-filesystem sandbox.
- Actual blocking workers can own a clone; exclusion lasts until all clones drop.
  Aliased roots conflict; independent roots do not. Six primitive tests pass.
- M9 verifier now uses this same lease, not a private verifier-only mutex.
- Additional reproduced abort defect: cancelling the whole verifier discarded
  workers and released exclusion before process cleanup. Regression failed with
  `workspace admitted a conflicting owner before actual process cleanup`.
- Fixed actual-worker lease lifetime and drain ownership. Aborting a run or its
  close waiter no longer aborts TERM/KILL/reap. Blocking scans and artifact writes
  retain their own guard clones. Orderly cleanup requires a live Tokio runtime.
- New `tachyon_verify::run_with_lifetime(plan, context, cancel,
  Arc<dyn Send + Sync>)` retains a caller's opaque task-owner guard in these actual
  workers. Existing `run` stays compatible. The ownership builder was directed
  to use this entry; that integration awaits parent verification.

Executed parent evidence (under `$TMPDIR/tachyon-m10/`):
- `lease-red.log`: new primitive API initially missing (compile 101).
- `lease-green.log`: six primitive tests pass.
- `verifier-shared-lease-red.log`: real verifier bypassed shared lease (test 101).
- `verifier-shared-lease-green.log`: same real verifier waits (pass).
- `verifier-abort-lease-red.log`: process cleanup/reap boundary regression (101).
- `verifier-abort-lease-green.log`: both new integration tests pass.
- `tools-lease-regression.log`: 35 passed / zero failed.
- `verifier-lease-regression.log`: 17 passed / zero failed (unit and runner tests).
- `lease-clippy-green.log`: tools+verify strict all-target Clippy, exit 0.
  Initial Clippy caught an underscore-prefixed test binding; renamed, rerun green.

These are prerequisite tests. Final all-feature/default workspace gates must be
rerun after all builders finish. No final benchmark number is taken from a shared
mutable child build directory; parent commands explicitly choose their target dir.

## Parent: canonical auth fixture

Created `fixtures/auth-refresh/`, a separate dependency-free Cargo workspace:
auth-session, reverse-dependent client, unrelated metrics, correct reference,
protected migrations, and synthetic event log including untrusted instruction text.
The checked-in implementation deliberately remains broken.

- `fixture-broken.log`: actual workspace run exits 101. Failures are stale-response
  generation regression, duplicate completion overwrite, and the client's two
  affected call sites. Reference/normal-newer/unrelated checks pass.
- Cargo.lock was generated offline before source baselines/copying.
- `fixture-green-control.log`: a scratch-only manual repair passes all six tests.
  SHA-256 comparison proves only `auth-session/src/session.rs` differs from the
  checked-in fixture. Tests/reference/manifests/log/migrations remain unchanged.
- This control establishes fixture validity, NOT production runtime integration.
  G2/G4 are still pending until the supervisor applies the proposal via real M8
  mutation and M9 verification. No model diagnostic-quality claim is made.

## Parallel prerequisites

Batch `deleg_d3dfad9b`: four isolated writer scopes, pinned by M10_BUILD_BRIEF.md:
exclusive supervisor ownership, strict scoped recovery, aliased dependency
selection, optional usage provenance. Their reports and parent verification are
pending. Do not infer successful integration from files appearing mid-build.

## Parent verification of phase-2 slices (2026-09-22, post re-dispatch)

Parent reran all phase-2 prerequisite suites offline, default features, exit 0:
core responsive_actor 4/4, supervisor_ownership 7/7; mutation authorized 9/9,
authorized_commit 9/9, recovery_scoped 19/19; tools workspace_lease 6/6;
verify selection 7/7, workspace_lease 2/2; models usage 7/7. Full workspace
gate also green at parent: fmt 0, check 0, test 300/0 over 62 suites, strict
Clippy 0. Pinned APIs confirmed present: `prepare_authorized` /
`commit_authorized_up_to`, `run_with_lifetime`, `TaskOwnership`/`OwnedWorkers`,
`WorkspaceLease`. Phase-2 builder reports still to be filed; test evidence
above stands on its own. Runtime integration dispatched as `deleg_bf925e10`
per M10_RUNTIME_BRIEF.md; benchmark host + code board + final gates pending.

## Parent verification of runtime slice (2026-09-22)

Batch `deleg_bf925e10` returned COMPLETED. Parent verified: new
`crates/tachyon-core/src/runtime.rs` (~1190 lines) +
`tests/runtime_{stages,repair,recovery}.rs` exist, owned paths only;
`cargo test -p tachyon-core --test runtime_stages --test runtime_repair
--test runtime_recovery` rerun by parent: 13+5+4 = 22/22 pass, exit 0.
Pinned surface confirmed: `RuntimeBounds`, `collect_evidence`,
`compile_operation`/`compile_evidence_graph`/`lower_router_placeholders`,
`parse_proposal`, `gate_proposal_writes`, `persist_intent`/`load_intent`,
`MutationOutcome`, `RunMeasurements`, `max_overlap`, `SteeringState`,
`mark_recovering`, `resolve_check_selection`. Noted limits: fs.read
evidence only, FNV-1a evidence hashes, no full-channel barrier tests, fixture
via synthetic analog. Benchmark slice dispatched as `deleg_23252973` per
M10_BENCH_BRIEF.md. Remaining: bench return, full gates, five-seat code
board, final report + checkpoint.
- Bench (`deleg_23252973`) COMPLETED and parent-verified: all three modes run
  green (full concurrency 4, serial 1, reference completed_reference; nulls
  for unknown; fixture unchanged). Final gates green: fmt, all-features
  check, 334/0 tests over 65 suites, strict all-targets+all-features Clippy.
- Code R1 dispatched as `deleg_8f7633e4` (5 seats). Remaining: adjudicate R1,
  fix, R2, docs/report, commit.
- Code R1 returned 1 BUILD (verification) + 4 CONDITIONAL. Parent adjudicated
  all findings CONFIRMED and fixed: conjunctive extra_hard enforcement +
  2 gate regressions, scopeless-write deny, typed `MutationIntent`,
  exact-hash foreign-grant regression (policy), FNV/BLAKE3 pin (doc+test),
  example orchestration note, 600-sender burst barrier, portable EOF
  liveness probe (no /proc). Competing-mutation + pending-ack barriers were
  already present as unit tests (R1 seats missed them). Full gates green:
  fmt, default + all-features check/test (330/338, 0 failed), strict Clippy.
- Code R2 dispatched as `deleg_5157da5d` (arch/safety/async verify-by-quote).
  Remaining: R2 verdicts, docs/report, commit.

## Still required

Responsive durable core runtime/stage compiler, acceptance-before-effects,
complete evidence freshness, final lease-through-completion, bounded worker
ack/drain protocol, runtime example, canonical happy/negative cases, steering and
crash/reopen proof, five-seat code board, full gates, final report and checkpoint.
M11 is not started. This file records progress, not a completed milestone.

## Interruption and re-dispatch (2026-09-22 14:2x +12:00)

- Batch `deleg_a9980d66` (responsive actor + authorized mutation) FAILED on
  provider capacity, not task difficulty: `HTTP 429 The usage limit has been
  reached` on the configured parent provider (gpt-6-astra/openai-codex) after
  3 attempts, ~262s in. Owner switched the model to deepseek-flash.
- Those two children had already written PARTIAL edits, so the tree is mid-flight
  and currently does NOT build: `cargo check --offline --workspace --all-features`
  exits 101 with 6 errors in tachyon-core (`no field 'revision' on
  &ActiveVerification` at src/verification.rs:302; `JobResult` vs
  `VerificationReport` mismatch at src/lib.rs:756; `cannot find function
  'capture_leased'` at src/verification.rs:134). Untracked partial artifacts:
  `crates/tachyon-core/tests/responsive_actor.rs` (73 lines).
- The tachyon-mutation partial side is coherent and does build: `cargo check
  --offline -p tachyon-mutation --all-targets` exits 0, with new
  `src/engine/authorized.rs` (65 lines, `prepare_authorized` + private
  `authorize_write`), `mod authorized;` in engine.rs, and
  `tests/authorized.rs` (133 lines). `commit_authorized_up_to` is not yet present.
- Re-dispatched both slices as `deleg_77ca54a6` on deepseek-flash, each child
  told to reconcile (not trust) the partial edits, with the same crate-isolated
  ownership as before: ACTOR owns tachyon-core, MUTATION owns tachyon-mutation.
- No partial edit above is verified work. Nothing in this section is evidence of
  a passing test, and no green result should be attributed to the failed batch.
