# Changelog

## Unreleased

- Milestone 12: recovery hardening (env-gated fault points, effect
  fixture with §19 reconcile, driver re-entry / fresh-id re-ask,
  gateway SIGKILL restart test, six-domain seam gates) with the §42
  seven-point coverage matrix in `M12_REPORT.md`.
- Bare `tachyon` (no subcommand) opens the TUI (`attach`) instead of
  printing help, matching spec §38's first-class `tachyon` command.
- Milestone 11: TUI + live gateway events + run path (protocol v2
  streaming subscriptions, Ratatui client with all nine panes, `attach` +
  run aliases, operator provider config with redaction, supervisor-owned
  `StartRun` on the shared driver, five new journal kinds, approval wait
  with one-shot grants, run-held workspace lease with `workspace_busy`
  refusal, Cargo acceptance detection) with the disconnect/reconnect gate;
  plan board r4 unanimous BUILD, code board pending.
- Parent fix: stale supervisor handles after run completion no longer
  surface transient `supervisor_gone` (recover-once + regression test).

- Milestone 10: full debugging task (provider-neutral core runtime over
  evidence/model/mutation/verification, supervisor ownership with
  ack-after-drain steering, shared workspace lease through durable
  completion, authorized mutation with scoped recovery, measured
  auth-refresh benchmark) with R1/R2 code boards unanimous BUILD.

- Milestone 9: verification-gated completion (typed acceptance contracts,
  authorized source snapshots, affected-first Rust planning with
  reverse-dependent closure, validated verification IR through the
  scheduler, policy-bound commands on the resolved canonical cwd,
  process-wide per-workspace execution lease, supervisor-owned durable
  completion with atomic journal projection) with the wrong-patch/fixed-patch
  gate end to end.

- Milestone 8: mutation engine (hash-guarded patch specs, durable
  batch journal, preimage retention, per-file atomic commits,
  finish-or-compensate recovery, changed-file events) with Slice C
  fixing the incorrect implementation end to end.

- Milestone 7: judgment layer (provider-neutral boolean/choice/score
  batches, certainty policies, outage fallback, fake provider,
  feature-gated OpenJEV adapter, opt-in router bridge) with a synthetic
  A/B showing 14 avoided model calls at equal verified success.

- Milestone 6: model layer (provider-neutral requests/decisions,
  capability negotiation, role mapping, trusted context assembly,
  fake provider, OpenAI-compatible local adapter) and evidence
  structures with deterministic merge; Vertical Slice B answered in
  one reasoning call.

- Milestone 5: predictive router (deterministic classification, EWMA
  estimates, 75 ms evidence grace window, serial mode) and route telemetry.
- Milestone 4: repository intelligence (BLAKE3 inventory, heuristic
  symbol/reference index, lexical search, watcher invalidation) with
  Vertical Slice A answered zero-LLM.
- Milestone 3: capability policy (scope globs, trusted defaults,
  hash-bound approvals, path containment) and native tools (contained fs,
  process runner, read-only git, artifact spool, credential broker).
- Milestone 2: validated execution IR and conflict-aware DAG scheduler
  (readiness, atomic grants, critical-path priority, retries, timeouts,
  cancellation) with property tests; docs-freshness tripwires in CI.
- Milestone 1: durable task kernel (SQLite journal + snapshots, supervisor
  actor, gateway lifecycle, CLI) with live kill-9 recovery gate.
- Milestone 0: foundation types, protocol framing, config precedence,
  `tachyon doctor`.
- Initial Tachyon architecture and implementation handoff scaffold.
