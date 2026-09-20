# Prompt to give Hermes Agent

You are implementing **Tachyon**, a high-performance Rust-first AI agent harness.

The repository/package you have received is authoritative. Begin by reading `README_FOR_HERMES.md` and `AGENTS.md`, then read every file under `docs/` in numeric order.

Your job is to implement Tachyon methodically from Milestone 0 onward. Do not redesign the frozen architecture unless concrete implementation or benchmark evidence requires it; if that occurs, create an ADR before changing the architecture.

Important constraints:

- Keep the workspace buildable at milestone boundaries.
- Do not jump directly to LLM integration.
- Prove the native fast path first.
- Use the Execution IR and scheduler rather than creating an alternate direct execution path.
- Preserve provider-agnostic and JudgmentProvider-agnostic core boundaries.
- Treat security, idempotency, durability and recovery as core runtime concerns rather than cleanup work.
- Add tests for invariants before expanding features.
- Build only the current milestone plus prerequisites; do not implement deferred features early.
- Commit coherent milestones with descriptive messages.
- Keep a `PROGRESS.md` file recording completed gates, measurements, unresolved blockers and any approved deviations.

Start with Milestone 0 from `docs/04_IMPLEMENTATION_PLAN.md`. Inspect the provided scaffold, correct anything that fails against the pinned toolchain, then proceed.

At the end of each milestone:

1. run formatting/check/tests/clippy;
2. run that milestone's acceptance gate;
3. record actual results in `PROGRESS.md`;
4. only then begin the next milestone.

Do not ask the user to make low-level implementation decisions that are already resolved by the specification. Escalate only genuine architectural conflicts, unsafe irreversible operations, or blocked external credentials/services.
