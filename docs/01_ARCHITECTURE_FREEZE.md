# 01 — Architecture Freeze

This document defines the responsibilities that are frozen for the first implementation cycle.

## North-star architecture

```text
                              USER
                                │
                         CLI / TUI / API
                                │
                             GATEWAY
                                │
                         TASK SUPERVISOR
                                │
                          FAST ROUTER
                                │
         ┌──────────────────────┼─────────────────────┐
         │                      │                     │
       native                retrieval             semantic
   deterministic         AST/search/graph      Jev/fast model
         │                      │                     │
         └──────────────────────┼─────────────────────┘
                                │
                          evidence/state
                                │
                       unresolved reasoning?
                                │
                           reasoning model
                                │
                         Execution IR compiler
                                │
                         validated typed DAG
                                │
                            SCHEDULER
                                │
          ┌─────────────────────┼────────────────────┐
          │                     │                    │
     native tools          external tools          MCP
          │                     │                    │
          └─────────────────────┼────────────────────┘
                                │
                          commit barriers
                                │
                           verification
                                │
                        durable journal
                                │
                             complete
```

## Frozen responsibilities

### Rust-first core

Latency-critical runtime, scheduler, persistence coordination, policy and repository intelligence are Rust-first.

### Task Supervisor

Each task has one logical owner. The Task Supervisor is the single writer of canonical task state and coordinates routing, planning, scheduling, approvals, steering, recovery and completion.

### Execution IR

Every scheduled operation exists as validated machine-readable IR before it executes. A model tool call is only a proposed operation until Tachyon validates and compiles it.

### DAG scheduler

Execution is dependency-aware and effect-aware. Concurrency occurs only when declared dependencies, read/write sets, effect state and machine/provider resources permit it.

### Predictive routing

The router does not serially attempt code, then Jev, then small model, then large model. It predicts the cheapest sufficient route and may launch complementary cheap work in parallel.

### Native fast paths

Core same-process functionality uses direct Rust calls. MCP is not used simply to make all tools look uniform.

### Provider capability negotiation

Core behavior depends on capabilities, not provider-name conditionals. Provider-specific extensions remain behind adapters.

### Judgment provider abstraction

OpenJEV is an adapter behind `JudgmentProvider`. Tachyon remains functional without OpenJEV.

### Durability

Execution state is backed by an append-only durable journal plus materialized snapshots. Persistent chat alone is not sufficient.

### Effect semantics

Operations explicitly declare effects and idempotency. Irreversible or ambiguous external effects cross commit barriers. Unknown/non-idempotent effects are not blindly retried after crashes.

### Capability-based security

Access to files, processes, credentials, networks and external mutation is policy-controlled through explicit capabilities. Model text cannot grant permissions.

### Verification-gated completion

Task completion is established by acceptance contracts and executable evidence, not model self-report.

### Gateway/client separation

CLI, TUI, desktop/mobile clients and future messaging integrations are clients of one persistent runtime. UI surfaces do not own agent intelligence.

## Core invariants

1. Deterministic truth is not unnecessarily delegated to a model.
2. Independent work is concurrent only when effects/resources permit it.
3. Provider-specific types do not cross into core state/IR.
4. User hard constraints survive model output and task recovery.
5. No running nodes hold conflicting access claims.
6. Every dangerous effect has an explicit recovery policy.
7. Telemetry/export cannot block correctness-critical execution.
8. Remote gateway mode is opt-in.
9. Stale repository evidence cannot authorize blind mutation.
10. Tachyon's benchmark advantage must survive same-model comparisons.

## Architecture changes

If implementation evidence contradicts a frozen responsibility:

1. measure the problem;
2. write an ADR;
3. describe alternatives;
4. state migration impact;
5. preserve a rollback path;
6. only then modify the architecture.
