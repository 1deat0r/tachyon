# 05 — Acceptance and Benchmark Specification

## Why this exists

Tachyon's claim is architectural speed. That claim is meaningless unless verification success and latency are measured together.

## Primary metrics

For every benchmark task record:

- verified success/failure;
- total wall-clock completion;
- time-to-first-visible event;
- time-to-first-useful-result;
- time-to-first-edit;
- time-to-final-verification;
- critical-path duration;
- LLM calls and durations;
- judgment/Jev calls and durations;
- native/tool calls and durations;
- input/output tokens;
- monetary cost;
- speculative work started/used/discarded;
- user interventions;
- retries;
- provider failures;
- verification failures.

Report median and p95. Include sample count and verified-success rate. Do not rely on mean latency alone.

## Benchmark modes

Every first-party fixture should support:

1. `tachyon-full`
2. `tachyon-no-speculation`
3. `tachyon-no-judgment`
4. `tachyon-serial`
5. reference conventional model→tool loop

External harness comparisons are supplemental and should use identical model/provider/task/environment when possible.

## Benchmark classes

### Class A — deterministic repository queries

Examples:

- locate symbol definition;
- find references;
- show changed files;
- inspect git status;
- list affected project files.

Success criterion: no primary LLM invocation unless the query genuinely asks for semantic explanation.

### Class B — evidence-first questions

Examples:

- why a test is failing;
- what changed around a subsystem;
- identify likely location of a regression.

Measure whether parallel repository evidence reduces the first reasoning call's latency/context needs.

### Class C — small mutation

Examples:

- fix localized implementation bug;
- update a function while preserving public API;
- repair failing test implementation.

Require AcceptanceContract and verifier pass.

### Class D — multi-file work

Examples:

- safe refactor;
- dependency migration;
- multi-file bug repair.

Measure mutation recovery and selective verification.

### Class E — architecture/reasoning

Examples:

- redesign subsystem under explicit constraints;
- investigate intermittent race;
- plan/execute migration.

Do not expect zero LLM. Measure how much non-reasoning work is removed from model turns.

## Canonical first fixture

Create a small test repository containing:

- token refresh implementation;
- multiple references;
- an intentionally incorrect competing implementation;
- focused auth tests;
- unrelated migrations that must remain untouched.

Tasks:

```text
A1: Where is refreshToken defined and used?
A2: Explain why these two implementations behave differently.
A3: Fix the incorrect implementation.
A4: Find why authentication occasionally fails after token refresh and fix it.
```

A1 should exercise native repository intelligence only.

A2 should reuse A1 evidence and require semantic reasoning.

A3 exercises structured mutation and verification.

A4 exercises the complete debugging path.

## Suggested benchmark task format

```yaml
id: auth-refresh-race
workspace: fixtures/auth-app
prompt: >
  Find why authentication occasionally fails after token refresh
  and fix it.

acceptance:
  - command_pass: cargo test
  - path_unchanged: migrations/**
  - no_unrelated_changes: true
```

## Performance engineering targets

Initial targets, not public guarantees:

- deterministic router path: <2 ms p95;
- scheduler dispatch overhead: <1 ms p95 excluding executor work;
- local IPC command: <5 ms p95;
- first visible event: <50 ms p95;
- warmed simple symbol/reference query: <250 ms p50, <500 ms p95.

Record the hardware/runtime/environment for every benchmark report.

## MVP performance claim gate

Tachyon may claim the architecture is faster only if:

- verified success is equal or higher than its serial reference;
- median time-to-first-useful-result is lower;
- p95 does not regress unacceptably;
- completion time is lower for a meaningful share of representative tasks;
- improvement is not explained merely by changing the model/provider;
- verification is not silently weakened.

## Regression budget

Once a benchmark is stable, CI/nightly performance runs should flag substantial regressions. Do not make strict wall-clock microbenchmarks blocking on noisy shared CI unless the environment is controlled; track trends and use dedicated repeatable performance hosts when available.
