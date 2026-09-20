# Tachyon Progress

This file is updated by the implementing agent after every milestone.

## Current milestone

Milestone 2 — Execution IR + scheduler (Milestone 1 complete, see gates below)

## Completed gates

- 2026-09-20 Milestone 1 — Durable task kernel: SQLite `state.db`
  (WAL/FULL/FKs/busy-timeout, single-writer `StoreWriter`, migrations),
  append-only journal + snapshots (every 100 events, terminal states),
  `TaskState`/supervisor actor (mailbox 256, journal-before-state,
  revision bumps, terminal discipline), gateway lifecycle (0700 dir,
  endpoint file, stale eviction, Unix socket, framed JSON dispatch),
  CLI client (`gateway`, `session create`, `task create/list/get/send/
  pause/resume/cancel`). Gate: `fmt --check`, `check`, `test` (21 passed,
  0 failed incl. gateway restart-recovery test), `clippy -D warnings`,
  plus live `kill -9` gate: task recovered at rev 1 with same
  objective/status and continued to rev 2.

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
