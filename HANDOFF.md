# Handoff — 2026-09-26 sessions: scaffold research → Skeleton v2 → issue #31 fix

Fresh-session entry point. Nothing below is in-progress mid-edit; everything landed is committed/green, everything open is listed under **Open work**.

## Latest session (2026-09-26 follow-up): issue #31 fixed

1. **Fix landed**: PR https://github.com/1deat0r/tachyon/pull/33 (`test(core): pin cancel-ack-after-reap with delay-robust probes`) squash-merged → `f476721` on main; issue #31 closed by the merge; tree clean, main == origin.
2. **What changed**: the two simultaneity assertions in `crates/tachyon-core/tests/responsive_actor.rs` compared two observation times with unbounded scheduler delay between them (the reported panic at :351, plus the `ack still pending` beat after it — same defect, next failure on a fast runner). Replaced by a single-instant monotone read: at the moment the ack is observed the child must already be gone (socket EOF / process handle). Code ordering guarantees it (terminate+reap → job completion → `settle_if_drained` → ack), so observation delay cannot false-fail it, while a live child there is a real defect. Reaped-before-durable is now tolerated as legitimate, exactly as issue #31 authorised; no production code changed; `still_pending` is now `#[cfg(unix)]` (its last Windows use was removed).
3. **Verification**: ledger `/tmp/opencode/tachyon-issue31/GATES.md` **8/8 gates met** — local suite green, cancel test 30/30 consecutive runs, `cargo check --target x86_64-pc-windows-gnu -p tachyon-core --tests` clean (cfg path compiles on Windows), PR 3-OS CI green on both heads, merged + issue closed. Two-seat `/code-review` (Standards/Spec) ran before merge; its two wording findings fixed in `87cea28`, three declined with rationale recorded in the G7 evidence. Post-merge main CI green ×3 OS (`run 36237587461`) — the exact spot where the flake fired.

## What the first session established (condensed)

1. **Muse-spark scaffold audit**: re-fetched all 15 of its sources — 13/15 clean, one 404 (real Codex doc: `https://developers.openai.com/codex/agent-configuration/agents-md`), one miscite, vendor-family independence problem, two unsourced tree elements.
2. **Method decision (user-confirmed direction)**: `project-skeleton` (canonical, `VERSION` 0.1.0, methodology 2026-09-24) **stays the method**; the 2026-09-26 sources pass is its scheduled re-research. Method delta for the offered-but-not-yet-started **v0.2.0 merge**: (a) live-URL + claim-in-text check, (b) 3 sources must span ≥2 vendor families, (c) every tree element sourced (local convention counts locally), (d) scarcity ≠ irrelevance (evals + permissions re-admitted with 3 vendors each).
3. **Tachyon audit** (`/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent`): full skeleton v1 compliance-plus; gaps were permission layer, acceptance-not-in-CI, skills-lock ignored, and GitHub required-review absent.
4. **Delivered Skeleton v2** (7 paths, one squash PR, all gates green):
   - PR https://github.com/1deat0r/tachyon/pull/30 merged → `e1c1384` on main (tree clean, == origin).
   - `.claude/settings.json` (env-read deny), `opencode.json` (git-push ask, env deny), `.github/workflows/acceptance.yml` (nightly `17 3 * * *` + dispatch), `skills-lock.json` tracked, `.gitignore` `.claude/*` + `!.claude/settings.json`.
   - Governance: `docs/adr/0003-skeleton-v2-permission-and-acceptance-gates.md` + `AGENTS.md` stamped `Skeleton: v2 — 2026-09-26`.
5. **Verification**: 3 unlazy ledgers, **16/16 gates met** (`/tmp/opencode/tachyon-v020/GATES*.md`; approvals under `~/.unlazy/approved`). Pre-push local gate suite (fmt/check/test/clippy `-D warnings`) green; PR 3-OS CI green; acceptance first live run green (all 5 steps).
6. **Flake caught post-merge**: Windows `cancel_acknowledges_after_real_reap_while_the_mailbox_serves` (responsive_actor.rs:351) failed on main, same tree passed Windows ×2 on the PR and on rerun → pre-existing timing race, not the change. Stop-the-line issue filed: https://github.com/1deat0r/tachyon/issues/31.

## Open work (priority order)

1. **project-skeleton v0.2.0 merge** (offered, not yet approved) — edit canonical source at `/run/media/its1deat0r/Projects/Skills/canonical/project-skeleton/` (never the opencode view), bump `VERSION` 0.1.0 → 0.2.0 + CHANGELOG, add: `.claude/skills/` frozen path, permission-layer element, lazy `evals/` element, the four method rules above, refreshed source citations. Then re-export views (`export.py`) and run `scripts/validate-all.sh` per home `AGENTS.md`. **Input**: Hindsight document titled `Skeleton v2 sources pass (2026-09-26) — verified scaffold findings` (retrieve via `hindsight_search_knowledge_pages`).
2. **Review-gate revisit trigger** — when a second identity/bot exists, enable `required_approving_review_count` on main (solo self-approval deadlock recorded in ADR 0003).
3. Optional: glance at the nightly acceptance run (03:17 UTC) after the first schedule fires; and watch the next few `windows-latest` runs for any further `responsive_actor` flakes (issue #31's fix is unproven against time until 2–3 clean main runs).

## Key decisions — do not re-litigate

- Branch protection stays **checks-only** (user's explicit choice; rationale in ADR 0003).
- Frozen paths untouched; v2 was additive only (project-skeleton §4 honored: ADR + stamp).
- 3-source standard for this work = 3 sources **and ≥2 vendor families** (or labeled GitHub-authoritative for platform mechanics).
- Test-ordering assertions must be delay-robust: read current state at a single instant, never compare two observation times across a scheduler gap (issue #31 / PR #33). "Reaped before the durable row was observed" is a legitimate interleaving — re-asserting it re-opens the flake.

## Artifacts (reference, not duplicated)

- Repo: ADR 0003, `AGENTS.md` (delivery rules + stamp), `acceptance.yml`, permission configs — all at `github.com/1deat0r/tachyon`, commit `e1c1384`. Issue #31 fix at `f476721` (PR #33).
- Ledgers/evidence: `/tmp/opencode/tachyon-v020/` (GATES.md, GATES-skeleton-v2.md, GATES-delivery.md + pr-body.md + flake-issue.md) and `/tmp/opencode/tachyon-issue31/` (GATES.md 8/8 + pr-body.md + run-cancel-loop.mjs). **`/tmp` may be wiped on reboot** — ledgers are evidence only; the work itself is landed in git/GitHub.
- Research findings: Hindsight doc `Skeleton v2 sources pass (2026-09-26) — verified scaffold findings`.
- This file: repo-root `HANDOFF.md` at project root, tracked in git — matching the convention in pi-rust, Hermes-Agent-Rust, TIDE (and Research's lowercase `handoff.md`). A transient copy also sits at `/tmp/opencode/handoff.md`.

## Suggested skills (Skill tool)

- `unlazy` — write gates before any non-trivial continuation (3-ledger pattern in session 1, 1-ledger in session 2).
- `project-skeleton` — needed for item 1 (and its §4 versioning rules).
- `diagnosing-bugs` — only if a further `responsive_actor` flake appears (see Open work item 3).

## Environment notes

- Host: opencode/T3 Code; run gates from `/home/its1deat0r/.config/opencode/skills/unlazy/scripts/` with `--cwd "<tachyon project path>"`; ledgers outside the repo need an explicit path argument.
- No secrets in this document; git author PII intentionally omitted.
