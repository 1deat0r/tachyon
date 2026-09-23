# M10 benchmark slice — thin runtime host + measured report

Read AGENTS.md and docs/milestones/M10_PLAN.md r2 (APPROVED FOR IMPLEMENTATION,
"Canonical fixture and benchmark" section is your contract). Owner requested
continuation; parent owns milestone docs, final full gates, code board and
checkpoint. Do not claim milestone complete.

Signature: Hermes Agent; parent muse-spark / meta-ai. Skills: TDD, Rust
baseline, subagent-driven development. No paid/model HTTP calls, no
push/deploy, no commits. Scratch only under $TMPDIR. In every cargo command,
REMOVE inherited CARGO_TARGET_DIR and pass `--target-dir
"$TMPDIR/tachyon-m10/bench-target"`. Format only your files, never the
workspace.

## Ownership (one writer)

Write ONLY `crates/tachyon-core/examples/` (new; a thin runtime host, e.g.
`auth_refresh.rs`). Do NOT touch `src/`, other crates, Cargo manifests,
`fixtures/` (read-only: copy to fresh scratch per run, never patch the
checked-in broken fixture), or docs. The example must build under the
existing dev-dependencies; if it needs a new dependency, STOP and report
instead of editing manifests.

## Delivered dependencies (reuse — do not reinvent)

- `tachyon-core::runtime` (new): `RuntimeBounds`, `collect_evidence`,
  `compile_*`, `parse_proposal` (`ModelProposal` JSON), `gate_proposal_writes`,
  `persist_intent`/`load_intent`, `MutationOutcome`, `RunMeasurements`
  (unknown serializes null), `max_overlap`, `SteeringState`,
  `mark_recovering`, `resolve_check_selection`. Full API in
  `crates/tachyon-core/src/runtime.rs`.
- Production supervisor path: `create_task` / `recover_task` /
  `SupervisorHandle`. No second orchestration implementation: the example
  constructs trusted inputs/provider, starts and observes the supervisor,
  optionally steers it, serializes real outcomes.
- Fixture `fixtures/auth-refresh/` (separate dependency-free Cargo workspace,
  deliberately broken stale-refresh `auth-session`, correct reference,
  protected `migrations/`, unrelated crate, synthetic event log with
  untrusted instruction text, offline-generated Cargo.lock). Known passing
  control: scratch-only manual repair passes all six fixture tests with ONLY
  `auth-session/src/session.rs` differing (see M10_WORK_LOG.md).
- Scripted model responses are test/replay providers, never evidence of
  genuine reasoning. M10 claims no live-model quality evaluation.
- Known runtime-slice limits: evidence is `fs.read` only (`repo.lexical`
  compiles but has no executor — do not use it); no repeated-sample timing
  stats yet (report sample count; p50/p95 only for a real repeated sample).

## Build

Per the plan's benchmark contract:

1. Scripted provider sequence driving the full run (evidence -> model ->
   patch -> verification) against a scratch fixture copy: broken regression
   MUST fail first, model-proposed correct repair applies via real M8
   authorized mutation + M9 verification, durable Completed only on correct
   repair. Same provider script, same fixture, same acceptance for full and
   serial runs; no-speculation/no-judgment configs coincide with full in this
   slice (no speculative/Jev stage exists) — report that explicitly, do not
   fabricate a difference. Unsupported modes are marked unimplemented.
2. A conventional reference-loop run (same provider/operations/acceptance,
   serial model-tool control, NOT the supervisor path) for honest comparison.
   No M13 speed claim: report wall/first-evidence/first-edit/final-
   verification times and state the sample count.
3. Machine-readable output (JSON to stdout): verified outcome, node
   start/end times + maximum independent evidence concurrency from real
   overlapping intervals (`max_overlap`; serial mode records 1),
   model/tool counts, estimated vs billed tokens distinguished via
   `TokenProvenance` (unknown=null, never zero), changed paths, selected
   checks (note any conservative broadening), durable task ID/revision,
   recovery outcome, timing milestones. Unknown/unmeasured = null.
4. Deterministic ordering barriers for the race reproduction, NOT
   probabilistic sleeps. Checked-in tests/manifests/reference/migrations
   remain unchanged after every run (assert this in an example self-check
   where cheap).

## Gates

`cargo run --offline --example <name>` full + serial + reference runs with
real measured JSON; `cargo test --offline -p tachyon-core` still green;
`cargo check --offline --workspace`; strict all-target core Clippy; all with
`--target-dir` above. Report <=800 words: exact files, executed runs with
real measured numbers (no fabricated timing/counter data), gates with
pass/fail outputs, remaining limitations. Parent integrates and runs final
gates.
