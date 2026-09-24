# 10 — Strategic Review: Is Tachyon Worth Doing?

**Date:** 24 September 2026
**Type:** Go / no-go / pivot review (not an architecture document)
**Method:** Five parallel primary-source research streams (landscape, thesis evidence, architectural novelty, dependency reality, demand). Every load-bearing claim carries three sources unless explicitly marked `[2 SOURCES]` or `[1 SOURCE]`.

---

## Executive verdict

**Continue — but reframe the goal.**

| Question | Verdict |
|---|---|
| Is the product thesis sound? | **Yes, directionally.** Serial-latency and exploration-cost problems are well measured in 2025–2026 research. |
| Is the architecture differentiated? | **As a stack, yes.** Individual ingredients are common or incremental; no shipping product combines all eight. |
| Is the market open for another full coding agent? | **No.** CLI mindshare is concentrated; consolidation is aggressive; layer tools outperform generic "another agent" launches. |
| Is the remaining build (M12–M14) worth finishing? | **Yes, as a proof + focused product**, not as a Claude Code competitor. |
| Biggest execution risk outside the code? | **Distribution**, then **OpenJEV dependency opacity**, then **frontier-lab absorption**. |

**Recommended path:** Finish M12–M14 to a verified MVP, then position Tachyon as a *verification-gated execution kernel + local-first harness for power users* — not as a faster Claude Code. Treat OpenJEV as replaceable (already designed that way) and add a distribution wedge before competing for daily-driver status.

---

## Claim ledger (three sources each unless marked)

### C1 — Serial model/tool turns dominate agent wall-clock; cutting unnecessary turns is a real lever

1. [Speculative Macro Commit for Faster Tool-Using Agents](https://arxiv.org/abs/2609.03236) (arXiv, 2 Sep 2026) — measures serial action–observation as the latency term; −18.6% / −44.9% wall time via speculation.
2. [Speculative Actions: A Lossless Framework for Faster Agentic Systems](https://arxiv.org/abs/2510.04371) (arXiv, Oct 2025) — "each action requiring an API call that can incur substantial latency"; up to ~20% end-to-end cut.
3. [CodeGrep](https://arxiv.org/abs/2608.05886) (arXiv, Aug 2026) — OpenHands on SWE-bench Verified averages **23 rounds / 631K tokens** per resolved issue, much of it exploration.

### C2 — Harness structure alone swings cost massively at similar pass rates

1. [The Scaffold Effect in Coding Agents](https://arxiv.org/abs/2607.22585) (arXiv, Jun 2026) — same models, three harnesses: **up to 40× token difference** per solve; pass-rate gaps mostly 0–8pp.
2. [An Empirical Study of Harness Design for Coding Agents](https://arxiv.org/abs/2609.20804) (arXiv, 17 Sep 2026) — 176 matched settings × 4 models: harness components move cost a lot and accuracy little for strong models.
3. [Anthropic: Harness design for long-running application development](https://www.anthropic.com/engineering/harness-design-long-running-apps) (24 Mar 2026) — full harness cost **~20× solo** ($200 vs $9) but produced working software vs broken solo output; components later became "dead weight" as models improved.
   - Supporting: [Polar](https://arxiv.org/abs/2605.24220) — same base model, different harness → **+22.6 vs +0.6** on SWE-bench Verified after identical training.

### C3 — Deterministic repo indexes / structural search beat agentic wandering on cost

1. [Code Isn't Memory: A Structural Codebase Index](https://arxiv.org/abs/2606.22417) (arXiv, Jun 2026) — fixed harness + fixed model: index improves localization and resolve at **lower $/solve**.
2. [CodeGrep](https://arxiv.org/abs/2608.05886) (arXiv, Aug 2026) — RL-trained retrieval preserves resolve with **15% fewer rounds, 19% fewer tokens**.
3. [FastCode](https://arxiv.org/abs/2603.01012) (arXiv, Mar 2026) — structural scouting beats iterative full-text exploration on accuracy and tokens.
   - Supporting (deterministic-before-LLM hygiene): [Harness Design study](https://arxiv.org/abs/2609.20804) — rule-based elision before LLM summarization was the strongest efficiency win; recoverable machinery went unused.

### C4 — Routing / cheap-first escalation saves cost; verifier blind spots are the risk

1. [RouteLLM](https://arxiv.org/abs/2406.18665) (arXiv, 2024) — preference routers cut cost **>2×** without quality loss.
2. [Anthropic: Building effective agents](https://www.anthropic.com/research/building-effective-agents) (19 Dec 2024) — first-class **Routing** workflow: easy → small model, hard → large model; "simplest solution possible."
3. [Cheap Verifiers, Large Blind Spots](https://arxiv.org/abs/2609.01345) (arXiv, Sep 2026) — cascades can hide up to ~32% true error behind ~3% dashboard error if the cheap tier's judge is weak — **direct warning for any Jev-like gate used as verification**.

### C5 — Productized predictive routing already exists; Claude Code still lacks it

1. [Cursor Router](https://cursor.com/docs/cursor-router) (primary docs) — classifier per agent request; Cost / Balance / Intelligence modes.
2. [Gemini CLI model routing](https://geminicli.com/docs/cli/model-routing/) (primary docs) — fallback chains + experimental **local Gemma router**.
3. [claude-code#69530](https://github.com/anthropics/claude-code/issues/69530) and [claude-code#44976](https://github.com/anthropics/claude-code/issues/44976) (primary issue trackers, 2026) — users request automatic cheapest-capable routing; not built in.
   - Note: Tachyon's lattice is wider (native/index/judge/model) than model-only routers — incremental vs research (C4), ahead of Claude Code's surface.

### C6 — Thin model→tool loops won; thick *peripheries* (hooks, skills, budgets) are table stakes

1. [Anthropic: Building effective agents](https://www.anthropic.com/research/building-effective-agents) — "find the simplest solution possible"; frameworks invite unwarranted complexity.
2. [Claude Code: How it works / Agent loop](https://code.claude.com/docs/en/how-claude-code-works) + [Agent SDK loop](https://code.claude.com/docs/en/agent-sdk/agent-loop) — thin gather→act→verify loop; harness = tools + context around the model.
3. [Why Do Multi-Agent LLM Systems Fail?](https://arxiv.org/abs/2503.13657) (arXiv) — 14 failure modes across 7 frameworks; multi-agent gains "often minimal."
   - Supporting: [Anthropic Managed Agents](https://www.anthropic.com/engineering/managed-agents) (8 Apr 2026) — labs productize the harness layer; assumptions "go stale as models improve."

### C7 — Verification-gated completion is best practice but rarely a hard runtime invariant

1. [Claude Code Agent loop](https://code.claude.com/docs/en/agent-sdk/agent-loop) — loop ends when the model returns no tool calls (**model self-report** at loop level); verify is a documented phase, not a mandatory state gate.
2. [Aider linting and testing](https://aider.chat/docs/usage/lint-test.html) — `--auto-lint` / `--auto-test` is among the few **default mechanical gates** in surveyed OSS.
3. [SWE-ABS](https://arxiv.org/abs/2603.00520) (arXiv, 2026) — adversarial test strengthening rejects **~19.7%** of previously passing SWE-bench Verified patches (78.8% → 62.2%); "done" culture is weakly verified.
   - Supporting (user-reported false done): [r/ClaudeAI thread](https://www.reddit.com/r/ClaudeAI/comments/1v2ssw4/) (22 Jul 2026) — completion claims contradicted by disk state.

### C8 — Durable execution for agents is common; effect-safe journals for *local coding agents* are thinner

1. [Temporal Durable AI](https://docs.temporal.io/ai) + [Temporal blog](https://temporal.io/blog/of-course-you-can-build-dynamic-ai-agents-with-temporal) — crash/timeout resume is a productized category.
2. [Restate: Durable Agents](https://docs.restate.dev/ai/patterns/durable-agents) — every LLM call/tool/routing decision persisted.
3. [Claude Code sessions](https://code.claude.com/docs/en/sessions) — process-local JSONL resume, **not** a single-writer journal with effect idempotency classes.
   - Differentiator is not "journaling exists" but journal → **UnknownAfterCrash / never blind-replay** tied to task state (matches [spec §19/§41](02_IMPLEMENTATION_SPEC.md)).

### C9 — Local daemon + SQLite + Unix socket + TUI is a crowded niche shape

1. [dibs](https://github.com/abevz/dibs) — single write authority over SQLite; HTTP+JSON over Unix socket.
2. [muster](https://github.com/codybuell/muster) — local unix socket, state in local SQLite.
3. [canter](https://github.com/jirathip-dev/canter/issues/5) — single-writer per-user Unix-socket daemon; clients never read SQLite.
   - Differentiator is only the *stack* (IR + access sets + verify + journal + router), not the socket/SQLite combo.

### C10 — Declared read/write access sets as IR invariants are near-novel in shipping form

1. [Claim Plane](https://pith.science/paper/2607.21909) (arXiv 2607.21909 review) — fail-closed authorization for repo mutations under parallel agents (research).
2. [When Tool Calls Succeed but Workflows Fail](https://arxiv.org/html/2609.15397v1) (Sep 2026) — commutativity/exclusion must be **declared on resources**.
3. Search for production coding agents requiring declared RW sets on every proposal returned **[NOTHING FOUND]** across Claude Code / Codex / Cursor / OpenHands first-party docs (architecture audit, 24 Sep 2026).

### C11 — Rust harnesses ship; Rust alone is not a moat

1. [openai/codex](https://github.com/openai/codex) — official TS→Rust rewrite ([discussion](https://github.com/openai/codex/discussions/1174), [InfoQ](https://www.infoq.com/news/2025/06/codex-cli-rust-native-rewrite/)).
2. [goose](https://github.com/aaif-goose/goose) — "Built in Rust for performance and portability"; ~54.6k stars.
3. [Devin Local rewritten in Rust](https://devin.ai/blog/windsurf-is-now-devin-desktop) — claims "up to 30% more token efficient."
   - Counterpoint: [Reddit critique](https://www.reddit.com/r/OpenAI/comments/1mozbod/) — bottleneck is model I/O, not harness GC **[1 SOURCE for the critique; no controlled same-model Rust-vs-Node wall-clock benchmark found — UNVERIFIED]**.

### C12 — The market for a *new full coding agent* is crowded and consolidating

1. GitHub stars (platform counts, 24 Sep 2026): OpenCode **~210k**, Claude Code repo **~148k**, openai/codex **~126k**, OpenHands **~89k**, Cline **~69k**, goose **~54.6k**, Aider **~49k**, Codewhale (Rust, local-first) **~41k** — [repos](https://github.com/anomalyco/opencode), [claude-code](https://github.com/anthropics/claude-code), [codex](https://github.com/openai/codex), [OpenHands](https://github.com/OpenHands/OpenHands), [cline](https://github.com/cline/cline), [goose](https://github.com/aaif-goose/goose), [aider](https://github.com/Aider-AI/aider), [Codewhale](https://github.com/Hmbown/Codewhale). **[platform UI counts — single surface each]**.
2. Consolidation: [Cognition acquires Windsurf](https://cognition.com/blog/windsurf) → [Devin Desktop](https://devin.ai/blog/windsurf-is-now-devin-desktop); [Google Gemini CLI → Antigravity CLI](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli) (19 May 2026); [Roo Code archived](https://github.com/RooCodeInc/Roo-Code) (15 May 2026).
3. Focused layers beat generic launches: [cc-switch](https://github.com/farion1231/cc-switch) **~136k** stars; [Morph fast-apply HN](https://news.ycombinator.com/item?id=44490863) **217 pts**; generic agent Show HNs typically **<25 pts** (HN Algolia sweep, 24 Sep 2026).

### C13 — Documented user pain matches Tachyon's pain (verification, cost, wrong edits) — but pain alone doesn't grant distribution

1. [claude-code#42796](https://github.com/anthropics/claude-code/issues/42796) — quality regression, unread-file edits, stop-hook firing 173×/17 days; log-mined analysis.
2. [claude-code#16157](https://github.com/anthropics/claude-code/issues/16157) — Max usage limits; 1,497 comments; official reply heavily downvoted.
3. [openai/codex#8745](https://github.com/openai/codex/issues/8745) — **498 👍** requesting LSP diagnostics in the loop (model ships type-error-prone changes).
   - Supporting founder postmortems: [ToolJet](https://news.ycombinator.com/item?id=49535001) (scrapped 11-month multi-agent system), [Aden on LangChain/AutoGPT](https://news.ycombinator.com/item?id=46979781).

### C14 — Frontier absorption is the main 12–24 month threat to moat (not to the efficiency goal)

1. [Anthropic: Harness design…](https://www.anthropic.com/engineering/harness-design-long-running-apps) — Opus 4.5/4.6 removed the need for specific harness components ("dead weight").
2. [Anthropic: Managed Agents](https://www.anthropic.com/engineering/managed-agents) — labs run harness-as-service; third parties implement *under* lab interfaces.
3. [Artificial Analysis: GPT-6 Sol and Luna](https://artificialanalysis.ai/articles/gpt-6-sol-and-luna-push-the-cost-efficiency-frontier) (22 Sep 2026) — flagship price cuts (~50–60% cost/task) with level-or-better coding scores under fixed harness.
   - Counterweight: [Scaffold Effect](https://arxiv.org/abs/2607.22585) + [Harness Design study](https://arxiv.org/abs/2609.20804) + [HarnessLens/AHE line](https://arxiv.org/abs/2604.25850) show harness still moves tokens/pass/failure fingerprints in 2026 — absorption is partial, not total.

### C15 — OpenJEV (`openjev.sh`) is live but a high-risk dependency; underlying Jev is real and days old

1. **Live probe (24 Sep 2026):** `openjev.sh` HTTP 200 (Vercel/Next.js); `api.openjev.sh/v1/systemone` exists; Let's Encrypt cert **notBefore=19 Sep 2026** (5 days old); docs describe `{model, state, questions:{id:{type: choice|score|noul}}}` wire format — independent gateway, **not** TypeSafe's official API.
2. **Underlying model is real:** [TypeSafe launch post](https://typesafe.ai/blog/introducing-system-one-models-and-jev) (15 Sep 2026); [TechCrunch](https://techcrunch.com/2026/09/18/a-new-kind-of-ai-model-from-a-chatgpt-inventor-is-thrilling-developers/) (18 Sep 2026); [Wikipedia: Jev (AI model)](https://en.wikipedia.org/wiki/Jev_(AI_model)) — $40M seed, ~$200M valuation, proprietary.
3. **Funding/identity risk:** openjev.sh thesis states free access funded by **$JEV memecoin** fees ("proposed funding flow"); micro-cap ~$272k; **openjev.com ≠ openjev.sh** (the famous [HN OpenJev story](https://news.ycombinator.com/item?id=49752041) is the other domain). GitHub `typesafe-ai` org has **no** openjev.sh repo.
   - Design already treats OpenJEV as optional behind `JudgmentProvider` ([AD-009](07_ARCHITECTURE_DECISIONS.md), [spec §24](02_IMPLEMENTATION_SPEC.md)) — **keep that; never harden the dependency.**

### C16 — Toolchain pins check out

1. [Rust 1.98.1 announced 3 Sep 2026](https://blog.rust-lang.org/2026/09/03/Rust-1.98.1/) — exact match to pin.
2. [tokio 1.53.1](https://docs.rs/tokio/1.53.1/tokio/), [sqlx 0.9.0](https://docs.rs/sqlx/0.9.0/sqlx/), [axum 0.8.9](https://docs.rs/axum/0.8.9/axum/), [ratatui 0.30.2](https://docs.rs/ratatui/0.30.2/ratatui/) — all exist as cited.
3. [MCP 2026-07-28 spec](https://blog.modelcontextprotocol.io/posts/2026-07-28/) + [roadmap](https://blog.modelcontextprotocol.io/posts/mcp-roadmap/) — active LF Projects standard; Tachyon's "MCP is boundary not kernel" stance remains correct.

---

## What the evidence says about each Tachyon pillar

| Pillar (charter/freeze) | Grade | Basis |
|---|---|---|
| Serial-latency is the problem to remove | **Strong** | C1, C2 |
| Deterministic native + index fast paths | **Strong** for search/context; **moderate** for planning | C3, C6; Harness Design study (planning ≈ cost-save for strong models) |
| Predictive cheapest-sufficient route | **Moderate–strong** (research + partial productization; exact lattice unproven end-to-end) | C4, C5 |
| Cheap bounded-judgment tier | **Moderate** for routing/cost; **fragile** if used as "done" judge | C4 (RouteLLM vs Cheap Verifiers) |
| Verification-gated completion | **Strong as a differentiator** — still unevenly enforced as a hard gate | C7 |
| Durable journal + effect recovery | **Strong as engineering**; **common category** in durable-execution; niche in local CLIs | C8 |
| Execution IR + access-set scheduler | **Near-novel in shipping form** | C10; novelty audit (LLMCompiler/PlanCompiler = research precedents) |
| Rust-first | **Validated, not distinctive** | C11 |
| Local SQLite gateway + TUI | **Common niche shape** | C9 |
| Stack combination (all eight) | **System-level incremental-to-novel** | Novelty audit: no fetched source ships all eight together |

---

## Language re-evaluation: Rust vs Go vs TypeScript (asked 24 Sep 2026)

Codebase today: **~45.6k lines across 130 Rust files, 478 test functions**, 19 crates, green through M11.

### Who ships what in 2026

| Language | Shipping agents (primary sources) |
|---|---|
| **TypeScript** | Claude Code (shipped as Bun executable — Anthropic [acquired Bun](https://bun.com/blog/bun-joins-anthropic.md), Dec 2025; [Bun-in-Rust](https://bun.com/blog/bun-in-rust.md) cut Claude Code Linux p50 startup 517→464 ms, Jul 2026), OpenCode (~210k★, [bun.lock](https://github.com/anomalyco/opencode)), Cline ([Tauri+Bun sidecar](https://github.com/cline/cline)), Gemini CLI (TS, [repo](https://github.com/google-gemini/gemini-cli)) |
| **Go** | Antigravity CLI — Google: *"Built in Go… snappier and more responsive"* + async multi-agent ([Google Dev Blog](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli), 19 May 2026; [product post](https://antigravity.google/blog/introducing-google-antigravity-cli)) — **no published numeric benchmarks** `[NOTHING FOUND]`; Charm **Crush** full agent TUI on Bubble Tea ([repo](https://github.com/charmbracelet/crush)); long tail (DeepSeek-Reasonix ~36k★) |
| **Rust** | Codex CLI — official TS→Rust: zero-dep install, sandbox FFI, no GC, wire protocol ([discussion #1174](https://github.com/openai/codex/discussions/1174), May 2025; dual-track burndown [#1266](https://github.com/openai/codex/discussions/1266); [InfoQ](https://www.infoq.com/news/2025/06/codex-cli-rust-native-rewrite/)) — **no published before/after numbers** `[NOT FOUND]`; goose (*"performance and portability"*, [repo](https://github.com/aaif-goose/goose)); Codewhale ~41k★; Devin Local Rust rewrite ([Devin blog](https://devin.ai/blog/windsurf-is-now-devin-desktop)) |

**Synthesis:** winners are polyglot — TS owns market share, Rust owns the native-rewrite narrative, Go won a Google-sized bet. No source says language decides agent success `[multi-source observational]`.

### Fit for *this* workload (gateway + SQLite journal + process trees + index + TUI)

| Dimension | Go | TypeScript (Node/Bun) | Rust (current) |
|---|---|---|---|
| Install UX | Single static binary ([Google Go post](https://developers.googleblog.com/en/why-go-is-an-ideal-language-for-ai-assisted-software-engineering/)) | npm/Bun friction was Codex's #1 gripe; `bun build --compile` mitigates ([#1174](https://github.com/openai/codex/discussions/1174); [Bun docs](https://bun.com/docs)) | Static binaries industry default for agents (Codex, goose) |
| SQLite journal | mattn (cgo) / modernc pure-Go ~1.3–2× CPU-bound slower ([modernc](https://pkg.go.dev/modernc.org/sqlite)) | `bun:sqlite` 3–6× better-sqlite3 claim ([Bun](https://bun.com/docs/runtime/sqlite)); OpenCode ships `opencode.db` | sqlx embedded; best native fit (current stack) |
| TUI | Bubble Tea 45k★ + Crush proves agent TUI ([bubbletea](https://github.com/charmbracelet/bubbletea)) | **Ink** = Claude Code / Gemini / Copilot CLI standard ([ink](https://github.com/vadimdemedes/ink)) | Ratatui 22.7k★, Codex/Codewhale class ([ratatui](https://github.com/ratatui/ratatui)) |
| Process/IPC/cancel | context+exec solid; Windows job objects care needed `[UNVERIFIED depth]` | AbortSignal; real failures are **event-loop stalls / RSS** not CPU ([claude-code#96008](https://github.com/anthropics/claude-code/issues/96008), [#95695 class](https://github.com/anthropics/claude-code/issues)) | tokio + process groups; Codex sandbox path proven |
| Perf headroom | Discord: GC caused **~2-min latency spikes** on one service; *“don't rewrite everything in Rust just because”* ([Discord](https://discord.com/blog/why-discord-is-switching-from-go-to-rust)) | LLM API dominates end-to-end `[secondary]`; startup/memory tails still matter (Codex rationale; Claude Code `--version` ≈4.4s HN `[1 SOURCE]`) | No-GC tails; **no controlled 3-way harness benchmark exists** `[UNVERIFIED]` |
| Solo iteration | Fast ship (Antigravity choice) | Fastest UI iteration | Slowest loop; AI-assisted coding helps `[weak/secondary]` |

### Rewrite economics (mid-project switch)

1. **Successful switches named a measured defect and dual-tracked:** Codex kept TS merging until Rust parity (weeks–months, [#1266](https://github.com/openai/codex/discussions/1266)); Discord rewrote **one small service** with p99 GC spikes, footnoted against blanket rewrites ([Discord](https://discord.com/blog/why-discord-is-switching-from-go-to-rust)); Gemini CLI→Antigravity was a **greenfield successor cutover** with deprecation timeline ([Google](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli)).
2. **Cheapest proven pattern is hybrid kernel:** Ruff/Biome/SWC keep the ecosystem language for packaging while the hot path is native ([ruff](https://github.com/astral-sh/ruff), [biome](https://biomejs.dev), [swc](https://swc.rs)) — VS Code keeps TS and isolates hot paths as processes ([extension host](https://code.visualstudio.com/api/advanced-topics/extension-host)).
3. **No measured pathology in Tachyon yet** — M13 is explicitly the performance campaign (`04:232`); inventing a rewrite before profiling violates the plan's own rule: *"Do not optimize via unmeasured rewrites"* (`04:245`) and AD-015 same-model discipline.
4. **Sunk-cost vs switch-fallacy:** finishing in Rust is correct *today* because there is no named defect the current language cannot address; if M13 later shows a concrete GC-free or install gap Go/TS would fix cheaper, extract that subsystem — do not re-open AD-001 on vibes.

### Language verdict

| Question | Answer |
|---|---|
| Should we have *considered* Go/TS? | **Yes — now formally considered and recorded.** |
| Is Go a better default than Rust for this product *at M12*? | **No.** Strong shipping story (Antigravity, Crush) but no numeric evidence it beats the existing green Rust tree; switching re-opens Codex-class parity debt. |
| Is TypeScript a better default? | **No for the kernel** — proven for harnesses (Claude Code, OpenCode) but Anthropic's own fixes were *owning Bun*, not rewriting the agent loop; event-loop/RSS failure modes are real on the exact surfaces Tachyon hardened. **Yes as an optional future UI shell** (Ink/Tauri sidecar = Cline pattern) if distribution needs a React ecosystem. |
| Keep Rust? | **Yes (AD-001 reaffirmed)** until M13 measurements say otherwise. Revisit only with a named subsystem + numbers. |
| Hybrid escape hatch | Rust kernel stays; optional TS/Ink or Tauri client later (AD-014 already makes TUI a pure gateway client — swap is cheap). Go appears only if a *new* constraint (e.g. embedding in a Go fleet) appears. |

---

## Alternatives considered (better / worse / different)

### Alternative A — Stop; contribute the unique parts elsewhere
- **Fit:** IR validation, verification gate, and access-set scheduling are the rare pieces (C10, C7). Hooks/ACP into OpenCode/Codex ride distribution (C12).
- **Cost:** Loses the integrated proof; M0–M11 investment partially stranded.
- **When better:** If distribution capital (time, audience) is near zero.

### Alternative B — Thin layer on incumbents (router / verifier / indexer as products)
- **Fit:** Layer tools win attention (cc-switch, Morph, Weave, stop-hook culture) (C12, C13).
- **Cost:** Abandons the runtime thesis; rent-seeking on lab UX churn.
- **When better:** If the goal is impact-per-week, not building the runtime.

### Alternative C — Finish M12–M14, ship MVP, reframe positioning (**recommended**)
- **Fit:** Remaining work is bounded (recovery suite → performance → freeze). Thesis still supported (C1–C4). Verification gate + effect journal are the sharpest differentiators (C7, C8, C10).
- **Required additions beyond current plan:** distribution wedge; OpenJEV de-risk (already optional — make fallback path always exercised in CI); explicit non-goal: "beat Claude Code on stars."
- **When better:** Default. Sunk cost is real but the *marginal* cost to a defensible MVP is M12–M14, not M0–M11.

### Alternative D — Pivot to durable-execution / verification infrastructure for *other* agents
- **Fit:** Temporal/Restate own generic durable execution (C8); nobody owns "effect-safe, verification-gated task kernel for coding agents" as a library.
- **Cost:** Re-platforming; new buyers (harness authors vs end developers).
- **When better:** If after MVP nobody uses the TUI but the kernel tests impress.

### Alternative E — Stop entirely
- **Fit:** Market crowded (C12); absorption risk (C14); OpenJEV rabbit hole (C15).
- **Cost:** Wastes a coherent, largely green, high-quality codebase (466 tests, 11 gated milestones).
- **When better:** Only if the goal was "ship a popular CLI by 2027" and no distribution plan exists.

---

## Explicit non-claims (evidence gaps)

- **No published A/B** of Tachyon's *exact* policy (deterministic fast path + index + cheap-judge gate → escalate only on residual uncertainty) vs a reactive loop on SWE-bench with wall-clock primary metric. Adjacent evidence only. `[UNVERIFIED as a whole-product claim]`
- **No controlled same-model Rust-vs-Node harness wall-clock benchmark.** Rust rationale rests on install/GC/rewrite narratives. `[UNVERIFIED]`
- **No 3-way Go/TS/Rust SQLite+IPC+file-index benchmark** for this exact workload. `[UNVERIFIED]`
- **No published numeric Go-vs-Node benchmark for Antigravity CLI**, and none for Codex's TS→Rust. Both are qualitative vendor claims. `[NOTHING FOUND]`
- **Star counts are GitHub UI single-surface counts**, not active users. `[platform counts]`
- **Reddit/forums** for local-model demand not fully scraped (anti-bot). `[GAP]`
- **"Jev" as an industry-standard noun** is ~9 days old (TypeSafe coinage); the *pattern* has older names (routing, cascade, zero-shot). Use "bounded judgment" in external writing unless TypeSafe's product is meant.

---

## Decision

**GO — conditional.**

1. **Finish M12–M14** (recovery hardening → performance → MVP freeze) as planned; the grill's M12 scope remains valid.
2. **Reframe success:** not "replace Claude Code," but "the local harness where done means verified and crashes don't lie" — power users, privacy/BYOK teams, and other harness authors.
3. **De-risk OpenJEV:** keep `JudgmentProvider`; ensure fake/evidence fallback is on the default CI path; never document openjev.sh as required.
4. **Design the distribution wedge before M14 exit** (OSS launch story, ACP compatibility, or embedding the kernel in an existing harness). Architecture without distribution loses (C12, C13).
5. **Watch absorption (C14):** measure whether router/index advantages survive each frontier model drop using the in-tree serial reference (already in [acceptance spec](05_ACCEPTANCE_AND_BENCHMARKS.md)).
6. **Language: stay on Rust** (AD-001 reaffirmed after Go/TS review). Profile in M13 before any subsystem language discussion; optional TS/Tauri UI shell later via AD-014 client boundary if distribution needs it.

If any of (1)–(4) is unacceptable, re-open Alternative B or A rather than shipping an undiscoverable full harness.

---

## Appendix — Research stream artifacts

Internal task reports (session summaries, not checked in): landscape · thesis evidence · architectural novelty · dependency reality · demand · Go · TypeScript · rewrite case studies. All ran 24 Sep 2026 against live web sources. Wikipedia used only as a source hub; vendor docs and arXiv preferred.
