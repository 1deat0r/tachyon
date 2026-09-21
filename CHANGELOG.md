# Changelog

## Unreleased

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
