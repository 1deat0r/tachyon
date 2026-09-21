# Tachyon Progress

This file is updated by the implementing agent after every milestone.

## Current milestone

Milestone 6 — Model layer (Milestone 5 complete, see gates below)

## Completed gates

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
