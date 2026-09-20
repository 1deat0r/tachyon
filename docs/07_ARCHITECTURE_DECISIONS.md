# 07 — Initial Architecture Decisions

These are pre-approved decisions so the implementing agent does not repeatedly reopen settled questions.

## AD-001 — Rust core

Use Rust for the core runtime, scheduler, persistence coordination, repository intelligence, policy and gateway runtime. TypeScript may be used later for high-level UI/plugin surfaces but is not required for MVP.

## AD-002 — Tokio structured runtime

Use Tokio for async I/O. Task ownership is structured around Task Supervisors, cancellation roots and tracked child tasks. Avoid detached fire-and-forget work.

## AD-003 — SQLite local durability

Use SQLite/WAL for local runtime state and separate per-workspace rebuildable index databases. One logical StoreWriter serializes correctness-critical writes.

## AD-004 — JSON local IPC initially

Use versioned length-prefixed JSON over local socket/pipe. Do not prematurely build a custom binary protocol. Change only if profiling demonstrates serialization is material.

## AD-005 — Direct native internal capabilities

Internal file/repository/Git/process primitives use direct Rust APIs/process calls. MCP is only an external integration adapter.

## AD-006 — Tree-sitter first AST engine

Use Tree-sitter for incremental language-aware symbol/reference extraction where supported. Keep repository interfaces independent so language-specific/alternative parsers can coexist later.

## AD-007 — BLAKE3 content identity

Use BLAKE3 for file/artifact content identity and stale-preimage checks.

## AD-008 — Provider capability negotiation

Do not define core logic around specific provider names. Models expose capabilities; config maps logical roles to provider/model implementations.

## AD-009 — OpenJEV optional adapter

OpenJEV is a valuable bounded-judgment provider but cannot be mandatory for core task execution.

## AD-010 — No workflow compiler in MVP

Collect traces and stable Execution IR first. Workflow compilation begins only after the runtime/verification semantics are proven.

## AD-011 — No multi-agent default

Use native concurrent workers for parallelizable work. Additional LLM agents require evidence that independent reasoning outweighs model overhead.

## AD-012 — Verification owns “done”

The verifier/acceptance contract determines completion. Model self-reported completion is advisory only.

## AD-013 — Recoverable, not fictional atomic, multi-file edits

The mutation engine journals preimages and per-file commit state and supports safe recovery/rollback. Do not market the batch as globally atomic.

## AD-014 — TUI is a client

Ratatui UI communicates through the gateway protocol. No direct database/model/tool ownership in UI.

## AD-015 — Same-model benchmark discipline

When evaluating harness speed, hold the model constant whenever possible. Tachyon does not receive credit for provider/model substitution.
