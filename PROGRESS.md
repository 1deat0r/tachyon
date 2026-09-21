# Tachyon Progress

This file is updated by the implementing agent after every milestone.

## Current milestone

Milestone 10 — Full debugging task (Milestone 9 complete, see gates below)

## Completed gates

- 2026-09-22 Milestone 9 — Verification-gated completion: `tachyon-verify`
  (typed `AcceptanceContract` with legacy fail-closed recovery, authorized
  source snapshots, Rust affected-first planning with reverse-dependent
  closure, validated verification IR through the scheduler, policy-bound
  `verify.command` on the resolved canonical cwd, fresh private evidence,
  serialized reports never re-authorize) plus supervisor-owned durable
  completion in `tachyon-core` (immutable baseline/contract binding,
  hard-constraint executable bindings, atomic journal/status/snapshot
  transitions, interruption without blind replay, post-report rehash) and
  the `tachyon-tools` process-ownership prerequisite (owned process groups,
  TERM grace then KILL, inherited-pipe deadlines, contained cwd, bound
  approvals, redact-before-spool). Gate: model-proposed wrong patch fails
  the real regression (`command exited Some(101)`) and refuses `Completed`;
  the corrected patch passes, persists `Completed`, and recovers it.
  Tests: 32 verifier (2 unit + 4 acceptance + 6 planner + 15 runner +
  5 snapshot), core 4 unit + 6 integration, store 4, tools 29;
  full workspace 53 suites green; `fmt`, `check`, `clippy -D warnings` clean.
- 2026-09-22 R1 board (5 seats): all REVISE with executed proof — cwd
  symlink alias bypassing the `verify.command` denial, cross-run workspace
  write-claim overlap via independent schedulers, scheduler-timeout grant
  release before worker cleanup, snapshot cadence reset across recovery,
  root metadata read without authorization, affected selection missing
  reverse dependents. Adjudication: all accepted as real. Fixes: canonical
  cwd resolution before authorization with invocation+resolved-scope approval
  binding and same-target execution; process-wide per-workspace async lease
  held through scheduler shutdown and worker drain; `snapshot_base`
  threaded separately from replayed journal position; root `fs.metadata`
  authorization before `is_dir`; reverse-dependency closure with
  conservative workspace broadening. 6 new regression tests.
  R2 verify-by-quote dispatched on the fixed tree.
- 2026-09-22 R2 board (5 seats): unanimous BUILD — each fix verified by
  quoted source plus executed regression and a novel scratch probe
  (nested alias chain, aliased-spelling lease contention, double recovery
  cadence, broad metadata/list denials, transitive a<-b<-c closure and
  dependency cycles); M9 wrong-patch/correct-patch gate re-confirmed
  6/6. Milestone 9 GATED.

- 2026-09-21 Milestone 8 — Mutation engine: `tachyon-mutation`
  (PatchSpec + base-hash guard, fsync'd batch journal with torn-tail
  repair, preimage retention in the artifact spool, temp staging beside
  targets, per-file rename with commit-time re-verification, finish or
  compensate recovery with divergence freeze, changed-file events).
  Vertical Slice C ("Fix the incorrect implementation") replaces the
  `==` token compare with constant-time equality end to end. Gate:
  crash injection between every file commit (k=0..=3) recovers coherent;
  stale preimages refused at prepare and at commit; 19 tests (8 unit +
  11 gate); `fmt`, `check`, `clippy -D warnings` clean.
- 2026-09-21 R1 board (5 seats): spec BUILD, security BUILD, API
  conditional BUILD, correctness HOLD, adversarial HOLD — both HOLDs
  with executed proof (symlink-cycle sweep hang, `contains` sweep
  deleting user files, poisoned batch denying siblings, aliasing dup
  paths, torn restore, vacuous empty completion, commit check-then-act).
  Adjudication: all findings accepted except race preventability
  (inherent to recoverable-not-atomic; post-rename detection added).
  Fixes: symlink-blind exact-pattern sweep keyed by relative temp path,
  per-batch isolation with `batch_errors`, canonical normalization,
  single-read prepare, retained postimage spool with temp re-staging,
  post-rename verify (`Diverged` abort), atomic restore via temp+rename,
  post-action sweep, lenient replay with `journal_gaps`,
  `UnknownBatch` plan cross-check, containment into `InvalidPath`,
  serde-stable reports, `MutationBatch` struct removed. R2 verify
  dispatched on the fixed tree.
- 2026-09-21 R2 board (5 seats): 4 BUILD (spec-fixes, security,
  API, adversarial) + 1 HOLD (correctness, 8 fresh executed bugs in
  the new code). Adjudication: 7 accepted as real (P1 same-ms temp
  collision, P2 compensated batch finished, P3 vacuous empty
  completion, P5 dotless restore temps, P6 rmdir of user dirs, P7
  unbound `post_artifact`, P8 sweep IO aborting recovery); P4
  (corrupt journal bricks finish) stays fail-closed per security +
  adversarial seats, docs rescoped to name the hand-repair path.
  Fixes: full-id temp names, finish refuses compensated batches and
  completes only non-empty all-`Committed`, `post_artifact` bound in
  plan check, dot-prefixed restore temps, rmdir removed, best-effort
  sweep with `sweep_errors`. 2 new gate tests (same-ms temps,
  no-resurrect); 21 tests (8 unit + 13 gate), full workspace gates
  green (46 suites). R3 verify dispatched.
- 2026-09-21 R3 board (3 seats): 2 BUILD (correctness re-probe of all
  7 Ps with executed proof, spec) + 1 narrow HOLD (adversarial:
  stale-descriptor resurrection via commit after compensate —
  re-applies and seals against future recovery, executed). Fix:
  `commit_up_to` refuses journaled-rolled-back batches with new
  non-retryable `MutationError::Compensated`; resume is fresh prepare
  only. Gate test `stale_descriptor_commit_after_compensate_refused`;
  22 tests (8 unit + 14 gate), full workspace gates green
  (46 suites). R4 verify dispatched.
- 2026-09-21 R4 single seat (correctness + spec): BUILD — resurrection
  fix re-probed with independent executed checks (stale commit refused
  `Compensated`, non-retryable, disk untouched, future recovery clean;
  fresh prepare on the same path still completes; no false
  `Compensated` on normal/partial flows), M8 Build items + Slice C +
  crash gate all green. Milestone 8 GATED: 22 tests (8 unit + 14
  gate), 46 workspace suites green.

- 2026-09-21 Milestone 7 — Judgment/OpenJEV: `tachyon-judgment`
  (provider-neutral `JudgmentProvider`, boolean/choice/score items with
  per-item certainty policies, batched resolution, capability-routed
  registry with outage fallback to evidence, `FakeJudgmentProvider`,
  `OpenJEV` adapter behind the `openjev` feature over plain-HTTP JSON,
  opt-in router bridge closing the M5 `JudgmentFirst` loop).
  A/B gate (synthetic fakes): 20 ambiguous requests avoid 14 model
  calls (6 vs 20) at equal 20/20 verified success. Default
  `JudgmentFirst` still resolves to evidence — real-workload A/B lands
  in M13/M14. Gate: 22 tests default (15 unit + 7 gate) + 30 with
  `openjev`; `fmt`, `check`, `clippy -D warnings` clean both ways.
  Review: R1 1 BUILD/4 HOLD (auth flags dismissed as redaction
  phantoms, one retracted after TCP capture); fixes — fail-closed
  certainty, OOB bounds, resolve fallback for all provider errors,
  policy-threaded bridge, source factory, 408/timeout mapping, bounded
  reads; R2 4 BUILD/1 HOLD (wire-confidence laundering); fix —
  non-finite confidence rejected at parse; R3 unanimous BUILD.

- 2026-09-21 Milestone 6 — Model layer: `tachyon-retrieval`
  (evidence structures with provenance, deterministic merge/rank),
  `tachyon-models` (provider-neutral `ModelProvider`, capability
  negotiation, role mapping, trusted context assembly with
  `WorkspaceData`-never-authority, structured `AgentDecision` with
  boundary repair, streaming event sink, `FakeModelProvider`,
  OpenAI-compatible HTTP adapter for local inference, plain-`http`
  only). Vertical Slice B ("Explain why these two implementations
  behave differently") answered in one reasoning call over repo
  evidence, citing both provenances. Gate: 34 models tests (29 unit +
  5 gate incl. Slice B) + 7 retrieval tests green; `fmt`, `check`,
  `clippy -D warnings` clean. R1 board (1 BUILD / 4 HOLD) adjudicated:
  auth-key flag dismissed as display-redaction phantom (byte-verified
  `{key}` interpolation, `grep -c` = 1); real findings fixed —
  converging marker-aware budget fitter, pinned-never-dropped, newest
  history survives, fail-closed trust, provenance-aware merge, NaN-safe
  ranking, `ContextOverflow` taxonomy, usage accounting, `Retry-After`
  support, CRLF rejection, role-carrying selection with request
  constructor. R2 verify-by-quote: 2 BUILD (spec, adversarial) / 3 HOLD.
  Real hang confirmed by 2 seats (fitter spun at marker floor when
  allowance saturated to 0 with non-empty pinned content) — fixed by
  making `truncate_block_chars` return `false` at identical-content
  floor plus `output_exceeding_total_terminates_with_pinned` regression
  test. API-seat auth HOLD dismissed: self-refuting (it reports 40/40
  green including the header-carries-key test, which can only pass if
  the key hits the wire) plus independent `od` verification. R2: 2
  BUILD / 3 HOLD; hang fix + regression test landed. R3 unanimous
  BUILD: all 3 HOLD seats flipped with quotes + live runs (API seat
  confirmed the `***` was display redaction via `od` comparison).

- 2026-09-21 Milestone 5 — Predictive router: `tachyon-router`
  (deterministic rule classification DirectNative/EvidenceFirst/
  JudgmentFirst/ReasoningFirst/Hybrid, candidate extraction with stoplist,
  EWMA-priced evidence plans, 75 ms grace window, serial mode,
  JudgmentFirst resolving to evidence until M7), `tachyon-telemetry`
  (bounded recorder, EWMA, route audit records). Gate: simple
  repo/search/git routes plan zero model calls; complex routes still
  launch evidence first; 7 router-gate + 2 telemetry tests green.

- 2026-09-21 Milestone 4 — Repository intelligence: `tachyon-repo`
  (walkdir inventory with BLAKE3 identity + prune rules, heuristic
  `LanguageBackend` for Rust/Python/TS/JS symbols, word-boundary reference
  index, deterministic lexical search, notify watcher as invalidation hints,
  hash-authoritative verify/refresh with generations). Vertical Slice A
  ("where is refreshToken defined and used?") answered with zero LLM calls
  in structured locations. Gate: 7 repo-gate tests green, full workspace
  gate green.

- 2026-09-21 Milestone 3 — Policy + native tools: `tachyon-policy`
  (capability/scope globs, trusted-workspace defaults, denials-first
  decisions, BLAKE3 canonical operation hashes, hash-bound approvals,
  real path containment with traversal/symlink rejection), `tachyon-tools`
  (capability registry, contained fs read/list/metadata/write, concurrent
  process runner with bounded inline + artifact spool + redaction,
  allowlisted read-only git, content-addressed artifact store with zstd
  above 64 KiB, credential-handle broker with output redaction). Gate:
  `fmt --check`, `check`, `test` (all suites green incl. 7 policy tests +
  11 tools-gate tests: local auto-allow, outside-write approval flow,
  deny posture, traversal/symlink escape, git allowlist, artifact
  roundtrip, secret redaction), `clippy -D warnings`.

- 2026-09-21 Milestone 2 — Execution IR + scheduler: `tachyon-ir`
  (validated DAG: identity/invocation/dataflow/bindings/purity/effects,
  conditional deps, cardinality, resource-key grammar + segment-based
  overlap, critical-path estimates), `tachyon-scheduler` (loop owns
  readiness, atomic conflict/resource grants, CP-priority scoring,
  retries/backoff, timeouts, structured cancellation, duration EWMA;
  `FakeExecutor` + `Tracker` for order/violation assertions). Gate:
  `fmt --check`, `check`, `test` (all suites green incl. 8 scheduler
  tests + 24-case proptest of conflict-freedom and dependency order),
  `clippy -D warnings`. Two real bugs found by testing and fixed:
  `JoinSet::join_next` on an empty set never pends (busy-spun the loop
  and starved commands — now guarded by `is_empty`); proptest spawned
  the loop outside a runtime (moved inside `block_on`).

- 2026-09-20 Milestone 1 — Durable task kernel: SQLite `state.db`
  (WAL/FULL/FKs/busy-timeout, single-writer `StoreWriter`, migrations),
  append-only journal + snapshots (every 100 events, terminal states),
  `TaskState`/supervisor actor (mailbox 256, journal-before-state,
  revision bumps, terminal discipline), gateway lifecycle (0700 dir,
  endpoint file, stale eviction, Unix socket, framed JSON dispatch),
  CLI client (`gateway`, `session create`, `task create/list/get/send/
  pause/resume/cancel`). Gate: `fmt --check`, `check`, `test` (21 passed,
  0 failed incl. gateway restart-recovery test), `clippy -D warnings`,
  plus live `kill -9` gate against the gateway binary (verified dead,
  stale endpoint evicted on restart): task recovered at rev 1 with same
  objective/status and continued to rev 2. (First attempt mistakenly
  killed the wrapper shell, leaving the gateway alive and the restart
  correctly refused with AlreadyRunning; redone against the binary.)

- 2026-09-20 Milestone 0 — Foundation: `tachyon-types` (UUIDv7 ids,
  RFC 3339 timestamps), `tachyon-protocol` skeleton (versioned
  request/event envelopes, LE length-prefixed JSON framing, 15-command set),
  config loading with precedence defaults < file < `TACHYON_*` env < CLI,
  tracing bootstrap (stderr, `--json`-safe stdout), `tachyon --version`,
  `tachyon doctor` (6 checks), `tachyon config`. Gate: `cargo fmt --check`,
  `cargo check`, `cargo test` (15 passed, 0 failed), `cargo clippy -D
  warnings`, `tachyon doctor` exit 0 — all pass on Rust 1.98.1.

- 2026-09-20 scaffold baseline: `cargo fmt --check`, `cargo check --workspace`,
  `cargo test --workspace` (35 suites, 0 tests — stubs), and
  `cargo clippy --workspace --all-targets -- -D warnings` all pass on Rust 1.98.1.
  One scaffold fix required: workspace `clippy::all`/`pedantic` lints needed
  explicit `priority = -1` for Rust 1.98 `lint_groups_priority`. Package
  SHA-256 manifest verified 61/61 files OK; zip sha256 matched.

## Measurements

No benchmark measurements yet.

## Blockers

None recorded.

## Architecture deviations

None.
