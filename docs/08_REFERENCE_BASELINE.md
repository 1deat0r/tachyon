# 08 — Reference Baseline (20 September 2026)

This file records the dependency/research baseline used while preparing the handoff. It is not a requirement to remain permanently pinned to these versions; update intentionally and keep the lockfile.

## Rust

Pinned toolchain in this package: **Rust 1.98.1**.

Official release announcement:
https://blog.rust-lang.org/releases/latest/

Rust 1.98.1 was released 3 September 2026 and fixes a vtable-generation miscompilation present in 1.98.0. Do not downgrade the project to 1.98.0 without a specific reason.

## Tokio

Baseline: **Tokio 1.53.1**.

https://docs.rs/crate/tokio/latest

## Axum

Baseline: **Axum 0.8.9**.

https://docs.rs/crate/axum/latest

## SQLx

Baseline: **SQLx 0.9.0**.

https://docs.rs/crate/sqlx/latest

## Ratatui

Baseline: **Ratatui 0.30.2**.

https://docs.rs/crate/ratatui/latest

## Clap

Baseline: **Clap 4.6.7**.

https://docs.rs/crate/clap/latest

## SQLite WAL

SQLite WAL documentation:
https://www.sqlite.org/wal.html

Important architectural assumption: WAL allows readers alongside a writer but does not turn SQLite into a true multi-writer store. Tachyon deliberately uses one logical correctness-state writer.

## OpenJEV

OpenJEV documentation:
https://openjev.sh/docs

Tachyon uses bounded judgments only behind `JudgmentProvider` and must remain operational without OpenJEV.

## MCP

Model Context Protocol updates/specification material:
https://blog.modelcontextprotocol.io/

MCP is an integration boundary, not the internal Tachyon kernel protocol.

## WASI / Wasmtime

Deferred untrusted plugin sandbox references:
https://wasi.dev/
https://docs.wasmtime.dev/

Not required for initial MVP milestones.

## OpenTelemetry

https://opentelemetry.io/docs/

Tachyon uses `tracing` internally; OTLP/OpenTelemetry export is optional and cannot be correctness-critical.
