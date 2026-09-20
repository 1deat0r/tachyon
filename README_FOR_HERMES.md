# Tachyon — Hermes Handoff Package

This package is the authoritative starting point for the Tachyon project.

Tachyon is a high-performance AI agent harness intended to feel like a faster, sharper Codex/Hermes-style agent while using a fundamentally different execution architecture: deterministic computation, repository indexes, retrieval and bounded judgment handle work whenever they can; LLM reasoning is used only where genuine unresolved reasoning remains.

## Hermes: start here

Read these files in this order before writing production code:

1. `AGENTS.md`
2. `docs/00_PROJECT_CHARTER.md`
3. `docs/01_ARCHITECTURE_FREEZE.md`
4. `docs/02_IMPLEMENTATION_SPEC.md`
5. `docs/03_ADVERSARIAL_REVIEW.md`
6. `docs/04_IMPLEMENTATION_PLAN.md`
7. `docs/05_ACCEPTANCE_AND_BENCHMARKS.md`
8. `docs/06_SECURITY_AND_RECOVERY.md`
9. `docs/07_ARCHITECTURE_DECISIONS.md`
10. `HERMES_START_PROMPT.md`

The Rust workspace scaffold is intentionally minimal. Its purpose is to establish the crate boundaries and build order without pre-implementing the architecture incorrectly.

## Non-negotiable product goal

A normal user should be able to run:

```bash
cd project
tachyon
```

and use Tachyon like a modern coding agent. The complexity belongs inside the runtime, not in the user's workflow.

## Core rule

> LLMs are reasoning accelerators, not Tachyon's operating system.

Prefer, in order of fitness rather than blindly in sequence:

```text
native deterministic code
repository/index/search
bounded semantic judgment (Jev-compatible)
small/fast model
primary reasoning model
specialist model
```

The fast router predicts the cheapest sufficient route and may start cheap evidence work in parallel. It must not serially try every tier.

## Architecture lock

Do not replace the architecture with a conventional `model -> tool -> model -> tool` loop.

The following are frozen architectural responsibilities unless measurements or correctness evidence demonstrate that they must change:

- Rust-first core runtime.
- Persistent Task Supervisor with single-writer task state.
- Typed Execution IR before scheduler execution.
- Dependency-aware DAG scheduler.
- Explicit read/write sets and resource claims.
- Predictive routing rather than a fixed escalation staircase.
- Native in-process fast paths for core tools.
- Provider capability negotiation rather than provider-name conditionals.
- `JudgmentProvider` abstraction; OpenJEV is an adapter, not a core dependency.
- Durable append-only journal plus snapshots.
- Explicit effect/idempotency model and commit barriers.
- Capability-based policy model.
- Deterministic verification gating completion.
- Gateway/client separation: CLI/TUI are clients of one runtime.
- Recovery after process interruption.
- Performance measurement against a serial reference executor.

If implementation evidence suggests a frozen responsibility should change, write an ADR first in `docs/adr/` describing evidence, alternatives and migration impact.

## First proof

Do not begin by building a giant general agent.

The first important Tachyon proof is:

```text
> Where is refreshToken defined and used?
```

It should be answered through repository intelligence with zero LLM and zero Jev calls.

Then prove:

```text
> Why do these two implementations behave differently?
```

using existing evidence plus at most the reasoning actually required.

Then prove:

```text
> Fix the incorrect implementation.
```

through validated Execution IR, mutation handling and verification.

Only after these vertical slices work should Tachyon expand.

## Implementation policy

- Build milestone-by-milestone.
- Keep the workspace compiling at every completed milestone.
- Add tests with every core invariant.
- Do not hide failures with retries.
- Do not add major subsystems early because they are exciting.
- Do not add multi-agent swarms to work that native concurrency can perform.
- Do not make OpenJEV mandatory for basic execution.
- Do not route same-process tools through MCP.
- Do not put telemetry exports on the synchronous execution critical path.

## Current baseline

This package was prepared on 20 September 2026. The dependency baseline was checked against current upstream release information before packaging. See `docs/08_REFERENCE_BASELINE.md`.

## Expected initial commands

After the scaffold has been implemented sufficiently:

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Then:

```bash
tachyon doctor
tachyon gateway
tachyon
tachyon run "Where is refreshToken defined and used?"
```

## Definition of a good implementation

Tachyon should become faster because its execution architecture is faster, not because benchmarks quietly substitute a faster model or skip verification.
