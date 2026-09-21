# Tachyon

![CI](https://github.com/1deat0r/tachyon/actions/workflows/ci.yml/badge.svg)

A high-performance AI agent harness. Routine work runs through deterministic
code, repository indexes, and bounded judgment — LLM reasoning pays only for
genuine unresolved uncertainty.

> LLMs are reasoning accelerators, not Tachyon's operating system.

**Status (Sep 2026):** early implementation. Milestone 4 (repository
intelligence) is done and gated; see [`PROGRESS.md`](PROGRESS.md). Not yet a daily
driver — watch this repo if the architecture interests you.

## Quickstart

Requires Rust 1.98.1 (`rustup` installs it from `rust-toolchain.toml`).

```bash
cargo fmt --check && cargo check --workspace && cargo test --workspace
cargo run -p tachyon-app -- doctor
```

In one shell, start the runtime; in another, drive it:

```bash
tachyon gateway
tachyon session create
tachyon task create --session <SESSION_ID> "Where is refreshToken defined and used?"
tachyon task list
```

## Architecture

- **Rust-first core** — scheduler, persistence, policy, repo intelligence.
- **Task Supervisor** — single logical writer of canonical task state.
- **Execution IR** — every scheduled operation is validated before it runs.
- **Predictive routing** — cheapest sufficient path first, not a model loop.
- **Verification-gated completion** — done means proven, not self-reported.

Read [`README_FOR_HERMES.md`](README_FOR_HERMES.md) for the implementing-agent
view, then `docs/` in numeric order: charter → architecture freeze →
implementation spec → adversarial review → plan → acceptance → security.

## Contributing

Milestone-by-milestone, per `docs/04_IMPLEMENTATION_PLAN.md`. Keep the
workspace compiling at every milestone boundary, add tests with every core
invariant, and record results in `PROGRESS.md`. Frozen architecture changes
need an ADR in `docs/adr/` first.

## Security

See [`SECURITY.md`](SECURITY.md) for reporting and scope.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at
your option — the Rust ecosystem standard.
