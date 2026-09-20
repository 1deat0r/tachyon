# 00 — Project Charter

## Product

**Tachyon** is a high-performance AI agent harness designed to feel like a faster, better Codex/Hermes-style agent across CLI, TUI and gateway interfaces.

Its defining technical idea is not a faster model. It is an execution runtime that removes LLM inference from the critical path whenever intelligence is not genuinely required.

## User experience

The intended default experience is:

```bash
cd some-project
tachyon
```

Then:

```text
> fix the failing authentication tests
```

The user should not need to choose routers, model tiers, retrieval backends or execution graphs. Advanced controls may exist, but the default product is a normal agent interface.

## Product thesis

Conventional agent loops commonly pay serial latency for model decisions between operations that software could have planned, searched, executed or verified directly.

Tachyon instead treats the model as one capability within a larger runtime:

```text
intent
  ↓
predictive route
  ↓
cheap native/retrieval evidence in parallel
  ↓
bounded semantic judgment where useful
  ↓
LLM reasoning only for unresolved uncertainty
  ↓
validated Execution IR
  ↓
dependency-aware parallel execution
  ↓
deterministic verification
```

## North-star metrics

Primary:

- verified task success / wall-clock time;
- p50 and p95 time-to-first-useful-result;
- p50 and p95 verified task completion time.

Secondary:

- LLM calls per task;
- input/output tokens;
- monetary cost;
- user interventions;
- cache hit rate;
- critical-path duration;
- discarded speculative work;
- verification failures and retries.

## Non-goals for MVP

Tachyon MVP is not:

- a multi-agent swarm framework;
- a workflow automation marketplace;
- a browser automation product;
- a model-provider-specific CLI;
- an MCP wrapper;
- a Jev wrapper;
- a distributed execution cluster;
- a self-modifying autonomous runtime.

## Definition of success

A user familiar with modern coding agents should understand Tachyon immediately, while routine operations feel noticeably faster because Tachyon answers them through native/indexed execution rather than repeated model calls.

Complex work may still require powerful models. Tachyon's advantage is that only the irreducibly intelligent portions of the task should pay that cost.
