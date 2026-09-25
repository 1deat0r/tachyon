# M13 — Performance campaign report

Date: 2026-09-25 · Issue: #9 · Branch: `feat/m13-performance-campaign`
Gate command (ledger G3): `cargo test --release -p tachyon-router -p
tachyon-scheduler -p tachyon-gateway -p tachyon-repo -p tachyon-store -p
tachyon-models -p tachyon-verify -p tachyon-tools --test perf -- --ignored
--nocapture`

Outcome: **all five spec §43 targets PASS on the first measured baseline.**
Profiling then attributed two real hotspots — a fixed 10 ms poll in the
process runner's exit detection and a fixed 10 ms poll in scheduler
completion waiting — which were fixed and re-measured (both were on the
verification / process-output critical paths named by docs/04 M13). No other
optimization was performed: per the plan's rule, nothing was rewritten
without a measurement, and no other area missed a target.

## Method

- Harness: one ignore-gated `tests/perf.rs` per crate (router, scheduler,
  gateway, repo, store, models, verify, tools). Ignored in default runs so
  CI never depends on wall-clock timing; executed in **release mode** by
  ledger gate G3 with `--nocapture`.
- Percentiles: nearest-rank on a sorted sample vector —
  `p50 = sorted[n*50/100]`, `p95 = sorted[min(n*95/100, n-1)]`.
- Machine: AMD Ryzen 7 5800X (8C/16T), 30 GB RAM, Linux 7.0.0-31,
  workspace on ext4 (`/dev/sda2`, 1.8 T), Rust 1.98.1, release profile,
  local single-user development machine.
- Target definitions (t0/t1 stated so the number is reproducible):
  - T1: `Instant` around `Router::route(request)` (classify + plan +
    telemetry record), rotating 20-request corpus, warmup 500.
  - T2: gap `entry(successor) − exit(predecessor)` across a 120-node chain
    driven by a zero-work executor (30 ns max executor stamp width); the
    gap is outcome handling + readiness + grant + spawn + wakeup.
  - T3: framed `Ping` write to decoded response on one live gateway socket.
  - T4: `SendMessage` request write to the `Journal` event frame read off a
    second, subscribed socket (operator-visible first task event). Spec
    ambiguity, disclosed: §43 fixes neither t0 nor observer — task
    creation and `Subscribe` sit outside the timer because visibility
    presupposes an existing subscriber (creation→first-replayed-event
    via `after_seq` would be a defensible alternative t0; noted as an
    M14 matrix candidate).
  - T5: warm `definition_use(symbol)` + `lexical_search(symbol)` over an
    already-built index of this workspace (441 files, `target/`/`.git`
    pruned); index built and warmed before sampling. This is the
    **repository-layer leg** of §43's "request": routing adds the
    T1-measured cost and gateway transport the T3-measured cost on top;
    no single test composes the full route→repo→gateway path end to
    end (that composition belongs to M14's benchmark matrix).
- Numbers below are transcribed from the final harness transcript
  (G3-equivalent run on the final tree; 8/8 suites `test result: ok`,
  0 failed). Ledger gate G7 reconciles this report against the final G3
  transcript; run-to-run variance is called out where observed.
- Syscall attribution: `strace -f -c` (hardware counters unavailable:
  `perf_event_paranoid = 4`).

## Spec §43 targets

| Id | Definition | n | p50 | p95 | Target | Result |
|----|------------|---|-----|-----|--------|--------|
| T1 | Deterministic router path (`route()`) | 5 000 | 760 ns | 1.65 µs | p95 < 2 ms | **PASS** |
| T2 | Scheduler dispatch overhead (excl. executor) | 119 | 21.1 µs | 34.8 µs | p95 < 1 ms | **PASS** |
| T3 | Local gateway command (Ping RTT) | 200 | 14.0 µs | 16.2 µs | p95 < 5 ms | **PASS** |
| T4 | First visible task event (command → subscribed frame) | 100 | 159.6 µs | 201.6 µs | p95 < 50 ms | **PASS** |
| T5 | Warmed simple symbol/reference request | 100 | 14.43 ms | 15.15 ms | p50 < 250 ms, p95 < 500 ms | **PASS** |

All five passed **before** any optimization; the two fixes below did not
change these results. T5 carries the least headroom (≈17× on p50) and is
the subject of the indexing finding. T1/T3/T4 exclude external waits by
construction (no model, no child process inside the timed window).

## Component baselines

All eight critical-path areas from docs/04 M13, final-run figures
(before → after where the optimization applies):

| Area | Measurement | n | p50 | p95 |
|------|-------------|---|-----|-----|
| routing | T1 (above) | 5 000 | 760 ns | 1.65 µs |
| IPC | `comp[ipc_frame]` encode+decode, no socket | 5 000 | 470 ns | 550 ns |
| IPC | T3 socket round trip (above) | 200 | 14.0 µs | 16.2 µs |
| persistence | `comp[persistence.create_task]` (WAL, `synchronous=FULL`) | 100 | 82.0 µs | 103.0 µs |
| persistence | `comp[persistence.append_event]` journal commit | 200 | 79.7 µs | 99.5 µs |
| indexing | `comp[index_cold]` scan (repeat) / build | — | 5.68 ms / 7.64 ms | — |
| indexing | T5 split: `definition_use` lookup / `lexical_search` | 100 | 6.04 ms / 8.46 ms | 6.43 ms / 8.96 ms |
| scheduler dispatch | T2 (above) | 119 | 21.1 µs | 34.8 µs |
| model wait | `comp[model_wait.assemble]` trusted context assembly | 100 | 2.0 µs | 2.1 µs |
| model wait | `comp[model_wait.invoke_fake]` scripted dispatch, no network | 100 | 650 ns | 4.11 µs |
| verification | `comp[verification.plan]` snapshot + affected-first plan | 20 | 57.8 µs | 111.5 µs |
| verification | `comp[verification.run]` full run incl. child | 20 | **13.04 ms** (was 22.47 ms) | 14.60 ms (was 22.71 ms) |
| process output | `comp[process_output.baseline]` empty child | 20 | **2.85 ms** (was 11.88 ms; 1.91–2.85 ms across runs) | 2.90 ms (was 12.08 ms) |
| process output | `comp[process_output.payload]` +108 894 B stdout | 20 | **4.30 ms** (was 11.44 ms) | 5.28 ms (was 12.24 ms) |

Component tests carry no §43 number; they are report-only baselines that
fail the gate only if the measurement itself breaks (zero samples, failed
command, missing symbols) — except two, which now carry regression
asserts (see Optimizations: empty-child p50 < 6 ms, verification-run
p50 < 18 ms, added by the expert-board fix round so a reverted fix turns
the suite red instead of only moving printed numbers).

## Profiling

**Syscall attribution (`strace -f -c`):**

- Gateway perf run (start + 200 pings + 100 steer/event pairs): dominated
  by `futex` 6 701 calls / 82.6 % (tokio worker parking), `epoll_wait`
  1 952, `recvfrom` 2 415 / `sendto` 806 (framed socket I/O), `fsync` 124,
  `pwrite64` 1 234 (WAL). No unbounded or repeated-open pattern.
- Store perf run: `futex` 70.6 %, `pwrite64` 3 887, **`fsync` 326 for
  ~311 commits ≈ one fsync per commit** — `synchronous=FULL` is honored
  and still costs only ~80 µs/commit on this disk.
- Post-fix poll accounting: verify run issues 206 `waitid` calls for
  21 children (≈10/child — the 1 ms fast window), tools 167. Pre-fix
  accounting would be 1–2 `waitid` per child plus up to 10 ms of blind
  sleep, which is exactly the latency that was removed.
- strace itself inflates timings (T3: 14 µs free-running vs ~198 µs under
  tracing); all reported numbers are untraced.

**Stage timers (in-harness):**

- T2 executor stamp width ≤ 30 ns: the dispatch gap contains no executor
  work, so T2 measures the scheduler alone.
- T5 split: warm symbol/reference request = 6.0 ms lookup + 8.5 ms lexical
  search. Both halves re-read corpus files per call (`references()` reads
  every indexed file to scan lines; `lexical_search` reads candidates),
  i.e. the "warm" number is a full-corpus read+scan over 2.68 MB, not an
  in-memory hit. Index build itself is 7.6 ms.
- Verification decomposition: plan 58 µs + run 13.0 ms ≈ python3 child
  (~10 ms intrinsic) + runner overhead (~2 ms) + snapshot/report work.

## Optimizations

Two measured, precisely attributed hotspots; both were fixed with the
smallest semantics-preserving change (same control flow, same guarantees,
only the sleep granularity varies):

1. **`crates/tachyon-tools/src/process.rs` — `OwnedChild::wait_for_exit`.**
   Exit detection polls `waitid(WNOHANG|WNOWAIT)` (deliberately
   non-reaping: the leader must stay unreaped until the process group is
   signalled — the PID-reuse guard) on a fixed **10 ms** sleep, so every
   short child paid up to 10 ms of pure poll latency. Fixed: 1 ms fast
   window (50 polls) then the original 10 ms cadence for long-lived
   children (steady-state wakeup rate unchanged).
   Evidence: `comp[process_output.baseline]` 11.88 ms → 1.91–2.85 ms p50
   (≈4–6×); payload arm 11.44 ms → 4.30 ms p50. Raw `/bin/sh -c ':'`
   spawn on this machine is ~0.85 ms, so the runner is now within ~1–2 ms
   of the OS floor.

2. **`crates/tachyon-scheduler/src/scheduler.rs` —
   `SchedulerHandle::wait_finished`.** Completion was noticed on a fixed
   **10 ms** status-poll tick; the only production caller is
   `tachyon_verify::run`, which therefore returned up to 10 ms after the
   child was already done. Same fix: 1 ms fast window, then 10 ms.
   Evidence: `comp[verification.run]` 22.47 ms → 13.04 ms p50 (1.7×),
   consistent with python3's ~10 ms intrinsic startup plus ~2 ms runner.

Regression evidence: `cargo test -p tachyon-tools` green (lifecycle,
ownership, approval, redaction gates), `cargo test -p tachyon-verify`
green ×5, full workspace green ×3 after the fixes (487 passed / 0 failed
on the final run). Everything else measured inside its §43 budget with
headroom, so **no other code was changed** — per docs/04 M13's rule, an
unmeasured rewrite would be out of scope.

**Expert-board fix round (post-review hardening):** a 5-seat review
board with an executed disproof round confirmed that neither fix was
pinned by any assertion (reverting both sleeps kept the whole suite
green while the printed numbers moved). Applied: regression asserts
`comp[process_output.baseline] p50 < 6 ms` and
`comp[verification.run] p50 < 18 ms` (thresholds cleanly separate the
pre-fix ~11.9 ms / ~22.5 ms from the post-fix ~2 ms / ~13 ms);
`scripts/perf_gate.sh` replaces G3's raw command so all five
`perf[T*] PASS` markers are required (a zero-test run or dropped
package can no longer satisfy the gate); scheduler poll delays named
(`FINISH_POLL_FAST_MS`/`FINISH_POLL_SLOW_MS`) to match `process.rs`;
GATES `OWNS` narrowed to the files actually touched. Mutation re-run
afterwards: reverted fixes turn the perf gate red (evidence in the
board record).

## Findings and recommendations

- **Indexing re-reads the corpus per query** (largest remaining measured
  component): `references()` + `lexical_search` cost scales with corpus
  size on every request — 14.4 ms warm at 441 files / 2.68 MB. Projection:
  a ~10 k-file workspace would approach the 250 ms p50 budget warm, and a
  cold or slow filesystem could exceed it. Options if M14's fixture breadth
  shows this: hold file text (or identifier postings) in `SymbolIndex` at
  build time — trades memory for latency and gives up the incidental
  always-fresh read (hash-authoritative `verify()`/`refresh()` already
  guard index freshness). Not done now: no target missed, and M14 must
  first measure on representative fixtures (checklist item #6).
- **Process spawn floor** is ~1–3 ms (dash) plus the child's own startup
  (python3 ≈ 10 ms). Verification/acceptance cost is therefore dominated
  by the chosen command interpreter — worth stating in M14's completion
  breakdown rather than "fixing".
- **Model wait is harness-only here** by design: the fake dispatch
  (650 ns–4 µs) proves the harness adds nothing measurable; real
  provider/network wait belongs to M14's benchmark matrix with pinned
  model IDs and dates (AD-015, checklist item #11).
- A future latency pass could replace the poll loop with `pidfd` on Linux
  (event-driven, no reap) — measured benefit ≤1 ms beyond the current
  fix, so it is post-MVP material.

## Observations

1. **Cold-scan outlier (once):** the very first full release run reported
   `comp[index_cold] first pass scan = 97.72 s` for 441 files / 2.68 MB,
   immediately after the release build churned the disk (cold page cache /
   idle drive). Every subsequent scan — 26+ observations across runs —
   measured 5.7–8.2 ms; shell-level `sha256sum` over the same files takes
   0.016 s. Not reproducible warm. The harness now prints first and repeat
   scans separately, and T5 warms before sampling, so the §43 number never
   depends on this.
2. **One-off test failure — root-caused and fixed (was a watch item):**
   the first full-workspace run after the two optimizations failed
   `cancellation_drains_the_owned_process_before_returning` (verify
   runner:301, `immediate child not reaped`) once; the graceful-TERM
   assertion before it passed, and it never reproduced (25 targeted, 5
   package, 3 full-suite reruns). The expert board's disproof round
   **executed** the root cause: a test-side TOCTOU — Python
   `Path.write_text` creates `target/pid` empty before writing, the test
   polled `exists()` then read once, an empty read gave `pid=""`, and
   `reaped("")` checks `/proc/`, which exists → assert false (12/1000
   empty reads in the repro; strace shows `open(O_CREAT|O_TRUNC)` then
   `write`). Production was independently ruled out by two seats via the
   drain-before-reap invariant chain (`close()` → `join_next` →
   `run_cancellable` → `child.wait()`; tokio reaps synchronously in
   wait). Fix: the test now polls for non-empty content before reading
   (`crates/tachyon-verify/tests/runner.rs`), which also removes the
   `reaped("")` hazard. The mechanism predates this PR.

## Limitations

- Single machine, single run series **and a single repository** (this
  workspace, 441 files); spec §43 says "representative repositories and
  hardware" (plural). One repo is enough to catch pathologies (and did)
  but not to generalize: M14's fixture-breadth work (checklist item #6)
  owns the multi-repository matrix. Internal gates only, not public
  claims.
- Model-wait numbers use `FakeModelProvider` (no network, no provider);
  no token/cost figures here. M14's matrix owns real-provider numbers
  under same-model discipline (AD-015).
- Perf tests are `#[ignore]`d and run through the ledger gate, not CI —
  deliberate (wall-clock assertions in shared CI are flake generators);
  default CI still compiles them (`fmt`/`check`/`clippy`/`test` cover
  compilation).
- No hardware performance counters on this host
  (`perf_event_paranoid = 4`); attribution is strace + in-harness stage
  timers.
- The tools process-output component arm is unix-only (`#[cfg(unix)]`,
  `/bin/sh` child); Windows compiles the target but skips the arm.
- Numbers transcribed here are from the final harness transcript; ledger
  gate G7 re-reconciles them against the recorded G3 evidence (small
  run-to-run variance is expected and was observed, e.g. baseline arm
  1.91–2.85 ms).
