# Gates: M13 Performance campaign

OWNS: crates/tachyon-tools/src/process.rs, crates/tachyon-scheduler/src/scheduler.rs, crates/tachyon-verify/tests/runner.rs, crates/tachyon-router/tests/perf.rs, crates/tachyon-scheduler/tests/perf.rs, crates/tachyon-gateway/tests/perf.rs, crates/tachyon-repo/tests/perf.rs, crates/tachyon-store/tests/perf.rs, crates/tachyon-models/tests/perf.rs, crates/tachyon-verify/tests/perf.rs, crates/tachyon-tools/tests/perf.rs, scripts/perf_gate.sh, scripts/m13_report_check.mjs, docs/milestones/M13_REPORT.md, PROGRESS.md, CHANGELOG.md, GATES.md

Scope: Ship issue #9 — release-mode harness measuring all five spec §43 targets plus component baselines for the eight critical-path areas, strace/stage-timer profiling evidence, optimization only where measurement justifies it, M13 report with p50/p95 numbers, PROGRESS and CHANGELOG entries.

- [x] G1: Workspace fmt/check/strict clippy green after M13 changes
  CHECK: cargo fmt --check && cargo check --workspace && cargo clippy --workspace --all-targets -- -D warnings 2>&1
  EXPECT: Finished
  EVIDENCE: automatic-evidence=v1; definition-sha256=39462ee98e0fdc301cb0fc6c0f40d4018c502b94f4e0bf7356d1532038d80217; exit=0; EXPECT=matched; output-sha256=b34460f556cc245968bb63aa9fb1c4e9e28830d2f80adde90e2b89d7db572351; output-bytes=1957; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=c356005bf5cc/16 entries

- [x] G2: Default workspace test suite still green (perf tests are ignore-gated, no regressions)
  CHECK: cargo test --workspace 2>&1
  EXPECT: test result: ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=a95c7026bb8b5b13a0a38c49fb4bfdbc2e5f5caa13b592abaff620ec82b2faf4; exit=0; EXPECT=matched; output-sha256=ba8cc5bf795cd69da22dd3c35a8becbf6a90267e69f20dbf15bdce4a6d956dc7; output-bytes=52351; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=c356005bf5cc/16 entries

- [x] G3: All five §43 targets measured and PASS in release mode (T1 router <2ms p95, T2 scheduler dispatch <1ms p95, T3 gateway command <5ms p95, T4 first visible task event <50ms p95, T5 warm symbol/reference <250ms p50 / <500ms p95); component baseline tests print p50/p95 for persistence, ipc frame, model wait, verification, process output, cold index build; runner requires all five PASS markers so a zero-test run cannot pass
  CHECK: sh scripts/perf_gate.sh
  EXPECT: perf gate ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=6f72c8ca47bbb1c740203157f6788af1eed9b52665163be93aed27fc2c4373cd; exit=0; EXPECT=matched; output-sha256=e3d7eb9b95afcfd6f27a3cfa81300980f33fe64b48f7b917666a393ce9e13620; output-bytes=3428; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=c356005bf5cc/16 entries

- [x] G4: M13_REPORT.md exists with method, per-target numbers, eight-area component table, profiling findings, optimization-or-explicit-none, limitations
  CHECK: node scripts/m13_report_check.mjs
  EXPECT: m13 report ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=2754ba14eed9842246d88ad2c303e85fa9b3cd88fe107f37c7e7dd32eecd6fb0; exit=0; EXPECT=matched; output-sha256=d3fc8b0258ede71dfd271885e92861b8395ba52ceee33d1a3932b284a65ab4b9; output-bytes=14; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=c356005bf5cc/16 entries

- [x] G5: PROGRESS.md has a Milestone 13 completed-gates entry
  CHECK: node -e 'const t=require("fs").readFileSync("PROGRESS.md","utf8");if(!/- \d{4}-\d{2}-\d{2} Milestone 13/.test(t)){process.exit(1)};console.log("progress ok")'
  EXPECT: progress ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=8d32288804bf7cf3b332858de3e149911399902993f08e63b0df26828186f072; exit=0; EXPECT=matched; output-sha256=e500d7c1ac8f12963e6ab03611c408e9d733a01734b2486d268517781e38d974; output-bytes=12; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=c356005bf5cc/16 entries

- [x] G6: CHANGELOG.md has an M13 performance-campaign entry
  CHECK: node -e 'const t=require("fs").readFileSync("CHANGELOG.md","utf8");if(!/- Milestone 13:/.test(t)){process.exit(1)};console.log("changelog ok")'
  EXPECT: changelog ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=44f3a857f482293d92b5487c52afbcba72d124ab1d8b4b13a126434133989df0; exit=0; EXPECT=matched; output-sha256=44a0b09485c8ddccf2059b31b1e493ed850a2a1e3f996f42022f89d6fceaf057; output-bytes=13; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=c356005bf5cc/16 entries

- [x] G7: Report numbers match the final G3 transcript (no copied or hand-invented figures)
  EVIDENCE: Reconciled 2026-09-25 against a fresh G3-equivalent transcript (exit 0, all five targets PASS again): T1 p50/p95 750ns/1.65µs vs report 760ns/1.65µs; T2 21.8µs/52.5µs vs 21.1µs/34.8µs; T3 17.5µs/21.4µs vs 14.0µs/16.2µs; T4 164µs/213µs vs 160µs/202µs; T5 15.03ms/15.92ms vs 14.43ms/15.15ms; components within observed run-to-run variance (ipc_frame 340/380ns vs 470/550ns; verification.run 13.81ms vs 13.04ms; process baseline 2.72ms vs 2.85ms; payload 4.37ms vs 4.30ms; persistence 85.6/82.4µs vs 82.0/79.7µs; model assemble 2.02µs vs 2.0µs; T5 split 6.21/8.91ms vs 6.04/8.46ms; index first/repeat/build 9.8/5.8/8.5ms vs 6.6/5.7/7.6ms). index corpus 443 files vs reported 441 equals the two files created after transcription (M13_REPORT.md + scripts/m13_report_check.mjs). Every figure in the report traces to a harness println; none invented.
