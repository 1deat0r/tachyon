# 03 — Adversarial Specialist Review

This review is a simulated specialist fleet: separate review lenses were applied to the implementation specification as if each reviewer were accountable for failure in their domain. It is not represented as review by external human experts.

## Disposition

**Proceed to implementation.** No remaining issue justifies another broad architecture redesign before code exists.

The remaining risks are implementation risks and should now be tested through milestones, property tests, crash injection and benchmarks.

## Review lenses

### 1. Rust/API design

**Finding:** Avoid forcing provider/runtime APIs into a lowest-common-denominator interface. Use stable core request/result structures plus capability negotiation and adapter-specific extension maps only at adapter boundaries.

**Resolution:** Incorporated. Provider names must not appear in core routing logic.

### 2. Structured concurrency

**Finding:** A global event bus must not own task lifecycle. Detached tasks would make cancellation and recovery unreliable.

**Resolution:** Each Task Supervisor owns a structured task tree and cancellation root. Event streams are observation/persistence channels, not control ownership.

### 3. Scheduler correctness

**Finding:** “Parallelize everything” is unsafe without formal conflicts. Path hierarchy also matters: a write to `src/**` must conflict with `src/auth.rs`.

**Resolution:** Execution nodes declare read/write sets. Resource keys use hierarchical overlap checks. Claims are acquired atomically before dispatch to eliminate ordinary hold-and-wait deadlock.

### 4. Scheduler performance

**Finding:** Maximum concurrency can be slower than selective concurrency, especially for builds/tests competing for CPU/disk.

**Resolution:** Resource claims and critical-path priority are first-class. Verification concurrency is measured rather than assumed beneficial.

### 5. Crash consistency/database

**Finding:** Storing only a serialized `TaskState` is insufficient for effect recovery; storing every tiny telemetry event synchronously would make SQLite a latency bottleneck.

**Resolution:** Append-only journal + periodic snapshots for correctness. Separate rebuildable index DBs. One logical state writer. High-volume telemetry stays outside the critical journal.

### 6. SQLite concurrency

**Finding:** WAL does not make SQLite a multi-writer database. Pretending otherwise would create contention.

**Resolution:** Correctness-critical state writes flow through one asynchronous StoreWriter. Microbatch normal journal operations; force immediate commits around effect barriers.

### 7. External effects

**Finding:** There is an unavoidable crash window after a remote side effect succeeds but before local state records success.

**Resolution:** Every effect declares idempotency/query/compensation semantics. Unknown or non-idempotent effects enter an explicit `UnknownAfterCrash` state rather than being replayed.

### 8. Multi-file mutation

**Finding:** A group of filesystem renames is not globally atomic across files.

**Resolution:** MutationBatch is recoverable, not falsely described as atomic. Persist preimages/postimages and per-file commit state; recovery finishes or rolls back according to journal state.

### 9. Security

**Finding:** Command deny-lists are not a security boundary. Model-generated commands and repository prompt injection need a stronger boundary.

**Resolution:** Capabilities and normalized resource scopes are the policy unit. Repository/external content is untrusted data. Arbitrary shell execution is intentionally broad and less parallelizable than direct capabilities.

### 10. Path security

**Finding:** Workspace checks are vulnerable if lexical path checks ignore `..` or symlink escapes.

**Resolution:** Canonicalization/symlink-aware containment is mandatory before policy decisions; security tests cover escape attempts.

### 11. Credential handling

**Finding:** Passing secrets through task state, logs or model context creates avoidable exposure.

**Resolution:** Credential broker uses handles. Raw secrets are injected only at the execution boundary and registered with output redaction.

### 12. Gateway/IPC

**Finding:** Local and remote transport should not share identical security assumptions. A local daemon accidentally binding a TCP port is unacceptable.

**Resolution:** Local IPC defaults to Unix socket / Windows named pipe. Remote HTTP/WebSocket is separate, authenticated and disabled by default.

### 13. Slow subscribers

**Finding:** A slow TUI or remote client must not backpressure the runtime.

**Resolution:** Bounded per-subscriber queues. Ephemeral progress may be dropped; durable state is replayed by sequence cursor after `ResyncRequired`.

### 14. Repository indexing

**Finding:** Filesystem watcher events can be lost/coalesced and mtimes are not authoritative.

**Resolution:** BLAKE3 content hashes define content identity. Watchers are hints for incremental refresh; mutation paths verify current hashes before write.

### 15. Model interface

**Finding:** Provider-native tool calling can accidentally bypass policy if treated as immediate execution.

**Resolution:** Native tool calls are parsed as proposals and pass through schema validation, IR compilation and policy like any other operation.

### 16. Jev integration

**Finding:** A remote bounded-judgment call can be slower than a local deterministic/classifier path. Jev must not become a mandatory tax.

**Resolution:** `JudgmentProvider` is optional and routable. Batched Jev is used only when expected to improve cost/latency/success.

### 17. Verification

**Finding:** Tests can dominate total task duration. Blindly running the entire suite after each edit defeats the latency goal.

**Resolution:** Acceptance contracts plus project-aware affected verification first, expanding only as risk dictates. Full verification remains available when required.

### 18. Benchmark science

**Finding:** Comparing Tachyon using a faster model than the baseline would measure models, not harnesses. Mean latency can hide tail failures.

**Resolution:** Same-model comparisons where possible; report median/p95 plus verified success and critical-path time. Keep a serial reference executor in-tree.

### 19. Cross-platform execution

**Finding:** Signals, PTYs, process trees, paths and IPC differ substantially between Unix and Windows.

**Resolution:** Platform abstraction owns process groups/Job Objects, local IPC, path handling and terminal behavior. Unix assumptions must not leak across the core.

### 20. TUI architecture

**Finding:** Letting the TUI read SQLite or own task logic would couple UX to execution and make remote control awkward.

**Resolution:** TUI is a gateway client only. It consumes state/events and emits commands.

### 21. Panic/fault isolation

**Finding:** A single panicking capability must not destroy unrelated tasks or corrupt the gateway.

**Resolution:** Executors report typed completion/failure to supervisors; task boundaries isolate failures. Critical core panics still terminate when invariants may be corrupt, followed by journal recovery.

### 22. Scope/product

**Finding:** Workflow compilation, browser use, multi-agent collaboration and distributed workers are attractive but would obscure whether the low-latency kernel actually works.

**Resolution:** Deferred beyond MVP exit gates.

## Final implementation review verdict

The specification is ready to hand to a coding agent because the remaining unknowns are empirical:

- exact scheduler heuristics;
- evidence grace-window values;
- index backend crossover points;
- which Jev decisions actually save latency/cost;
- resource-claim calibration;
- affected-test discovery quality;
- model provider performance.

Those should be learned through instrumentation and benchmark fixtures rather than another paper redesign.
