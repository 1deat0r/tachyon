# M12 — Recovery hardening report

**Date:** 2026-09-24 · **Issue:** [#8](https://github.com/1deat0r/tachyon/issues/8) · **Branch:** `feat/m12-recovery-hardening`

## What shipped

| Deliverable | Where |
|---|---|
| Env-gated fault points (ADR 0001) | `crates/tachyon-tools/src/fault.rs` — `TACHYON_FAULT_POINT`, `OnceLock` cached, no-op unless armed; documented in `docs/06_SECURITY_AND_RECOVERY.md` |
| Fault seams at §42-adjacent boundaries | `evidence.read` (runtime), `model.enter` (driver), `mutation.commit` (engine), `verify.command` (runner, **before** child spawn), `approval.park` (supervisor) |
| Effect table API + §19 reconcile | `tachyon-store` `insert_effect_prepared` / `commit_effect` / `mark_effect_unknown_after_crash` / `load_effects_for_task`; `recover_task` classifies prepared rows — **unrecognized idempotency defaults to `unknown_after_crash` (fail-safe)** |
| Effect fixture tests | `crates/tachyon-core/tests/effect_fixture.rs` (7 tests incl. unrecognized-class fail-safe) |
| Driver re-entry (ADR 0002) | `Loop::resume`: `Recovering→Paused` (no pin); gateway `resume_task`: `Recovering`+pin → `start_run` re-entry. **Pin is the durable in-flight proxy** (running map is memory-only after restart) |
| Re-entry tests | `crates/tachyon-gateway/tests/reentry.rs` (2 tests) — old approval id stays `expired`; continuation count may grow (fresh id minted by policy `Ask` on next park) |
| Gateway SIGKILL restart | `crates/tachyon-app/tests/kill_restart.rs` — real binary via `CARGO_BIN_EXE`, portable `transport::connect`, kill, restart same dir, recover objective, steering accepted (`revision` non-decreasing) |
| Armed fault-point kill | `crates/tachyon-core/tests/fault_kill.rs` — child armed with `TACHYON_FAULT_POINT=evidence.read`, parks, parent `Child::kill()`, parent continues |
| Six-domain seam gates | `crates/tachyon-core/tests/fault_seam_gates.rs` |
| Fault-point unit tests | `tachyon-tools` `fault::tests` (3 tests) |
| M11 test flip | `restart_approval.rs` — no-run Resume now lands `Paused` (was 400) |

## Spec §42 coverage matrix

| # | §42 point | Nearest real seam | Test | Result |
|---|---|---|---|---|
| 1 | journal commit | Supervisor journalled transitions (WAL/FULL); kill pattern proven via `fault_kill` + existing `exit(137)` child | `fault_kill` + `runtime_recovery` store tests | pass (mapped) |
| 2 | EffectPrepared | `effects.state=prepared` via `insert_effect_prepared` | `effect_fixture_gates_*` (Keyed/Queryable/NonId/Unknown/unrecognized) | pass |
| 3 | remote effect return | Prepared row + crash before `commit_effect` → §19 reconcile | `effect_fixture_gates_keyed_prepared_stays_prepared_for_reentry` | pass |
| 4 | EffectCommitted | `commit_effect` flip prepared→committed | committed-untouched + double-commit typed error | pass |
| 5 | each multi-file mutation commit | `mutation.commit` fault + settled k=0..3 | `mutation_gates_*` + existing `mutation_gate::crash_between_every_commit_recovers_coherent` | pass (mapped to settled coverage) |
| 6 | verification | `verify.command` fault **before** `run_cancellable` | `verifier_gates_*` + existing `verification_gate` interrupted-tail | pass (kill-before-spawn + settled interruption) |
| 7 | approval wait | `approval.park` fault + restart-during-wait | `approval_gates_*` + `restart_during_approval_wait_*` + `reentry_no_run_*` | pass |

## Fault-point kill pattern (ADR 0001)

```
arm TACHYON_FAULT_POINT=<seam> → child hits reach() → parent Child::kill() → restart → assert reconcile
```

Portable transport (`tachyon_gateway::transport::connect`) and `Child::kill`. Default env leaves every seam inert (`fault_seam_gates`). One end-to-end armed kill is checked in (`fault_kill`); remaining domain kills reuse settled crash suites (mutation k=0..3, exit-137, restart-during-wait) plus the gateway SIGKILL test.

## Six domains (plan M12)

| Domain | Seam / coverage | Gate test |
|---|---|---|
| Native reads | `evidence.read` (+ armed `fault_kill`) | `native_read_gates_*` |
| Model/Jev | `model.enter` | `model_call_gates_*` |
| Mutation | `mutation.commit` + settled crash suite | `mutation_gates_*` + `mutation_gate.rs` |
| Verifier | `verify.command` | `verifier_gates_*` |
| Approval wait | `approval.park` + restart/reentry | `approval_gates_*` + `reentry_*` |
| Keyed/queryable effect | prepared→commit + §19 matrix | `effect_fixture_gates_*` |

## Explicit non-claims / follow-ups

- **Deferred (issue #26):** general effect-barrier journal protocol; node-level `UnknownAfterCrash` assignment.
- **Pin-as-run proxy:** `resume_task` treats `Recovering`+`workspace_root` as an in-flight run. The in-memory `running` map is empty after restart; a crash between pin and spawn could theoretically re-enter a never-started run — accepted residual (prepare_run order makes the window tiny).
- **Fresh-id evidence:** tests prove the pre-restart approval id stays `expired` and is never granted; a *new* id appears when the re-entered driver parks again under `Ask` (policy mints `ApprovalId::generate()`). Not asserted as a fixed literal in the no-run test (no re-park there by design).
- Mapped §42 rows that reuse settled coverage are cited, not re-proven with new kill tests.
- No paid APIs: FakeModelProvider / local fakes only.

## Gates executed

`GATES.md` (11/11 met, unlazy automatic evidence) · `PROGRESS.md` Milestone 12 entry · workspace `fmt` / `check` / `test` / strict `clippy` green.

