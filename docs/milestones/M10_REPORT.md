# M10 report — full debugging task (measured)

Status: GATED 2026-09-22. Plan r2 unanimous BUILD (`deleg_474534b9`);
code R1 1 BUILD + 4 CONDITIONAL → parent fixes → code R2 unanimous BUILD
(`deleg_5157da5d`). No live-model quality claim; scripted replay proves
integration, not diagnostic quality. No M13 speed claim (n=1 per mode).

## What was built

- Debugging stage compiler + pre-mutation gates (`tachyon-core::runtime`):
  bounded `fs.read` evidence, validated IR with lowered router placeholders,
  typed proposals, conjunctive hard-binding enforcement, `base_hash`
  freshness, typed `MutationIntent`, `Committed`-only success, null-honest
  `RunMeasurements`, FNV-1a freshness tokens with a pinned BLAKE3 boundary.
- Supervisor ownership + responsive actor: one owner per (db, TaskId),
  owned jobs/drains, ack-after-drain, Recovering-not-Completed, lease
  through rehash→durable-completion with alias-spelling barriers.
- Shared `WorkspaceLease` (`tachyon-tools`) across baseline, evidence,
  mutation, verification, completion; verifier `run_with_lifetime`.
- Authorized mutation (`prepare_authorized`/`commit_authorized_up_to`) +
  strict task-scoped recovery; aliased dependency selection (resolve or
  broaden); per-result usage provenance (null, never zero).
- Thin benchmark host (`examples/auth_refresh.rs`) + `fixtures/auth-refresh/`
  (checked-in broken, scratch-only repair, protected `migrations/`).

## Measured runs (`cargo run --offline --example auth_refresh -- MODE`)

> M14 note: the host was generalized into `examples/bench_matrix.rs`
> (descriptor-driven, five §44 modes + `fixture-check`); the numbers
> below are as measured in M10 and were not re-run under the new name.

| mode | outcome | max evidence concurrency | wall | revision | recovery | fixture unchanged |
|---|---|---|---|---|---|---|
| full | completed | 4 | ~600ms | 1 | recovered_completed | true |
| serial | completed | 1 | ~560ms | ~1 | recovered_completed | true |
| reference | completed_reference | 1 | ~344ms | n/a | n/a | true |

Each: model_calls 1 (scripted), tool_calls 7, estimated/billed tokens null,
changed `auth-session/src/session.rs` only, selected auth-session + client
checks without broadening, `sample_count: 1`, p50/p95 null. Wall gaps at
n=1 are harness overhead, not a benchmark. Unsupported modes report
`unimplemented`.

## Gates (executed, all exit 0)

- `cargo fmt --check`; `cargo check --workspace` (default + `--all-features`).
- `cargo test --workspace`: 330 passed / 0 failed, 65 suites (default);
  `--all-features`: 338 / 0.
- `cargo clippy --workspace --all-targets [--all-features] -- -D warnings`.
- G2: broken fixture fails first, repaired passes, checked-in tree unchanged.
- G4 refusals (wrong patch, Complete-alone, unknown capability, denied/
  escaped path, migration write, stale evidence, unbound constraint):
  zero writes, all green. G5/G6/G7 barriers green, incl. 600-sender
  burst, alias-spelling rehash-window exclusion, real `exit(137)` recovery.

## Board record

- Code R1: architecture/safety/async/adversarial CONDITIONAL (first-binding
  bypass, scopeless-write pass, untyped intent, missing burst barrier,
  FNV pin, driver note), verification BUILD. All accepted as real.
- Fixes: conjunctive binding + 2 regressions, scopeless deny, typed intent,
  foreign-grant regression, boundary pin, orchestration note, burst test,
  portable EOF liveness (no `/proc`). Competing-mutation + pending-ack
  barriers already existed as unit tests.
- Code R2: unanimous BUILD with quoted source + rerun outputs.

M11 is next, not started.
