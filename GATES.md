# Gates: M14 MVP freeze

OWNS: GATES.md, PROGRESS.md, CHANGELOG.md, README.md, MVP_REPORT.md, fixtures/**, docs/milestones/M14_MATRIX.md, scripts/m14_*, crates/tachyon-core/Cargo.toml, crates/tachyon-core/examples/bench_matrix.rs, crates/tachyon-repo/src/**, crates/tachyon-repo/tests/projection.rs, crates/tachyon-router/tests/perf.rs, crates/tachyon-repo/tests/perf.rs, crates/tachyon-models/tests/perf.rs

Scope: Ship issue #10 — benchmark fixture breadth (Class D multi-file + Class E architecture alongside auth-refresh), a descriptor-driven spec §44 matrix host running every fixture under all five modes with docs/05 primary metrics and p50/p95 reporting, the §42 security escape and recovery fault-injection suites, the M13 index-corpus-re-read watch item resolved with a measured projection, and `MVP_REPORT.md` with verified-success comparison, median/p95 TTFR and completion, critical-path breakdown, model/Jev/tool calls, known limitations, deferred work, and the §45 MVP exit checklist.

- [x] G1: Workspace fmt/check/strict clippy green after M14 changes
  CHECK: cargo fmt --check && cargo check --workspace && cargo clippy --workspace --all-targets -- -D warnings
  EXPECT: Finished
  EVIDENCE: automatic-evidence=v1; definition-sha256=9360f658ea530a568a3b47f799f99c4197d17aa83de86ceb1f50e4489e3b62fb; exit=0; EXPECT=matched; output-sha256=b7d665728d6241db55b9f07980c6dc8979b0d244c939f243fead0d639469e621; output-bytes=365; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=dd5253ad1dae/21 entries

- [x] G2: Default workspace test suite still green (matrix/perf tests are ignore-gated)
  CHECK: cargo test --workspace
  EXPECT: test result: ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=f24768e759995db91aea9a258dec670e6a9fb2056fa8f6df831c15694bc12246; exit=0; EXPECT=matched; output-sha256=5e37b8a7e3f16a410d73d32521ae8608a30fc3201ddf6f1089ca447df3e571c5; output-bytes=52242; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=dd5253ad1dae/21 entries

- [x] G3: Three benchmark fixtures ship with task descriptors; each fixture's broken-first regression fails before the scripted fix and passes after (fixture self-check, never patching the checked-in tree)
  CHECK: sh scripts/m14_fixture_gate.sh
  EXPECT: fixture gate ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=bc41b914d771e6b448ddecbeded7f08e1847e6c9b13f3fc488331b6d99f9f2e1; exit=0; EXPECT=matched; output-sha256=405471270a6c3693763afa5cdfedff5a2aea5915e2bb276d6c5550c00a76e588; output-bytes=200; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=dd5253ad1dae/21 entries

- [x] G4: Full spec §44 matrix runs in release mode and emits a raw transcript covering every fixture × mode cell plus the Class A (zero-LLM) and Class B (evidence-first) legs, each carrying the docs/05 primary metrics
  CHECK: sh scripts/m14_matrix.sh
  EXPECT: m14 matrix run ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=12a9838019245e0a43ce582b48502955ba4cc2a8ce184c689be1ac825524094c; exit=0; EXPECT=matched; output-sha256=6e9e3e71d421aa16bb01e330d0ec2ffe145d125ce5e06da66b6c559062663bc7; output-bytes=103; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=dd5253ad1dae/21 entries

- [x] G5: Matrix artifact validated — all expected cells present, required metrics non-null, verified-success comparison computed against the in-tree reference, pinned scripted-provider configuration recorded
  CHECK: node scripts/m14_matrix_check.mjs
  EXPECT: m14 matrix ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=8dd73a1d20180cb810780ca0e478955f81e38e621125f1bfb884b2790c480edb; exit=0; EXPECT=matched; output-sha256=e25c2fb55758917d5e8936bb683f9011a3904eed917fb1b122c27f18008194ba; output-bytes=14; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=dd5253ad1dae/21 entries

- [x] G6: §42 security escape suite and recovery fault-injection suite both pass on the M14 tree
  CHECK: sh scripts/m14_suites.sh
  EXPECT: security and recovery suites ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=05f7626336d1a8c0f761cf1477b1b1cfd85c31be70fd0f65a2c3a12001da1253; exit=0; EXPECT=matched; output-sha256=cb7b78067e4c5fb94edf7ad42c1c05ecadbd3ec61d8640735d35b4c1e20841e7; output-bytes=18656; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=dd5253ad1dae/21 entries

- [x] G7: Index projection: a warm definition/reference query re-reads zero corpus bytes after the index build (M13 watch item resolved with instrumentation, not a claim)
  CHECK: cargo test -p tachyon-repo --test projection --release -- --ignored --nocapture
  EXPECT: projection ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=44c0ccc8d3645a04edbdb57641786611922104ed2844468337def9dc53aecaa1; exit=0; EXPECT=matched; output-sha256=a603f70f745ddd16c9b2504041d9cd99537cda821773cb7c4a0847d72837ce31; output-bytes=460; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=dd5253ad1dae/21 entries

- [x] G8: `MVP_REPORT.md` exists with verified-success comparison, median/p95 TTFR and completion, critical-path breakdown, model/Jev/tool calls, known limitations, deferred work, kill criteria, and a §45 exit-condition disposition for every bullet
  CHECK: node scripts/m14_report_check.mjs
  EXPECT: m14 report ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=1a50f48cd9cd309f428a050d6f22f92faffaf7e6271cc9e20eb0bf9a1603d162; exit=0; EXPECT=matched; output-sha256=5cdf523589c7ad328f1e53a76c9bf990f9203cfac2489ea2d9e4d750d2968513; output-bytes=14; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=dd5253ad1dae/21 entries

- [x] G9: PROGRESS.md has a Milestone 14 completed-gates entry and the README status line matches it
  CHECK: node scripts/m14_progress_check.mjs
  EXPECT: progress ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=a97e3d3648ae84a1f37e159cbb98878590a28d0a1e0aad0ae54f6394902ef6e0; exit=0; EXPECT=matched; output-sha256=e500d7c1ac8f12963e6ab03611c408e9d733a01734b2486d268517781e38d974; output-bytes=12; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=dd5253ad1dae/21 entries

- [x] G10: CHANGELOG.md has an M14 MVP-freeze entry
  CHECK: node -e 'const t=require("fs").readFileSync("CHANGELOG.md","utf8");if(!/- Milestone 14:/.test(t)){process.exit(1)};console.log("changelog ok")'
  EXPECT: changelog ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=25de5762eea9b759a8b178e213cf48b2b69e522fb3ebb3af1888470cfa32ca3c; exit=0; EXPECT=matched; output-sha256=44a0b09485c8ddccf2059b31b1e493ed850a2a1e3f996f42022f89d6fceaf057; output-bytes=13; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=dd5253ad1dae/21 entries

- [x] G11: MVP_REPORT.md numbers reconcile against the final matrix artifact (no copied or hand-invented figures)
  EVIDENCE: Reconciled 2026-09-25 against docs/milestones/M14_MATRIX.json (sha 84d02e3, n=10/cell, 150/150 verified): all nine full-cell p50/p95 pairs (completion 364/393, 585/699, 505/3432; TTFR 32/48, 108/120, 115/1836; first-evidence 5/14, 14/31, 20/222), both leg pairs (33/35, 38/47), all twelve serial/reference p50s, 150/150, n=10, alias p50s 369/384, T5 post-projection 10.30/11.01 ms (release transcript `perf[T5] n=100 p50=10.298244ms p95=11.005575ms PASS`) vs M13 baseline 14.43/15.15, and model_ms p50 range 0.007–0.018 all verified present and equal; derived values re-computed from the artifact (TTFR/completion ratios 3.38x/1.38x/6.15x rounded to 3.4x/1.4x/6.1x; verification shares 91/82/77% stated as 77–91%; stage shares are completion/first-edit/first-evidence p50 differences, noted as derived). Two errors found by this reconciliation (verification share 65–80%, model floor 0.009) were corrected before recording this evidence. Every figure traces to the artifact or an M13/M14 transcript; none invented.
