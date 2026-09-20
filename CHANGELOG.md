# Changelog

## Unreleased

- Milestone 2: validated execution IR and conflict-aware DAG scheduler
  (readiness, atomic grants, critical-path priority, retries, timeouts,
  cancellation) with property tests; docs-freshness tripwires in CI.
- Milestone 1: durable task kernel (SQLite journal + snapshots, supervisor
  actor, gateway lifecycle, CLI) with live kill-9 recovery gate.
- Milestone 0: foundation types, protocol framing, config precedence,
  `tachyon doctor`.
- Initial Tachyon architecture and implementation handoff scaffold.
