# M11 — TUI (Ratatui) + live gateway events + run path

Revision: r4. Status: APPROVED — plan board unanimous BUILD at R4
(R1 5× CONDITIONAL → 19 findings closed; R2 5× CONDITIONAL → 7 residuals
closed; R3 4× BUILD + 1× CONDITIONAL → 3 citation defects closed;
R4 5× BUILD). Implementation authorized per Ownership and sequence.

## Authority and baseline

- Owner request: "continue development where you left off on tachyon agent" (2026-09-23).
- Dependency gate: M10 GATED (see `PROGRESS.md`, `docs/milestones/M10_REPORT.md`);
  branch `feat/m6-m9-milestones` at `e48f2a6`, tree clean.
- Parent independently reran the pinned Rust 1.98.1 baseline on 2026-09-23:
  `cargo fmt --check` exit 0; `cargo check --workspace` exit 0;
  `cargo test --workspace` 65 suites / 332 passed / 0 failed;
  `cargo clippy --workspace --all-targets -- -D warnings` exit 0.
- Governing contracts: `AGENTS.md`, `docs/01_ARCHITECTURE_FREEZE.md`,
  `docs/02_IMPLEMENTATION_SPEC.md` §16/§35/§36/§37/§38/§41/§43/§45,
  `docs/04_IMPLEMENTATION_PLAN.md` M11, `docs/07_ARCHITECTURE_DECISIONS.md` AD-014,
  `docs/08_REFERENCE_BASELINE.md` (Ratatui 0.30.2, crossterm 0.29 — already pinned
  in the workspace `Cargo.toml`, unused).
- Execution signature: Hermes Agent; parent model `mimo-v2.6-flash`, provider
  `xiaomi`; reasoning effort `minimal` resolved live from the default profile's
  `~/.hermes/config.yaml` on 2026-09-23 (`agent.reasoning_effort`; no
  `mimo-v2.6-flash` entry in `agent.reasoning_overrides`).
  Tools: file read/search/patch, terminal, delegate_task.
  Skills: board-of-expert-agents-review, subagent-driven-development;
  test-driven-development and rust-workspace-setup before implementation.

## Deliverable and non-goals

Deliver a TUI that is a pure gateway client (AD-014) over a gateway that can
stream live task events, plus the minimum run path needed for those events to
be real. M11's plan gate (docs/04): **disconnect/close TUI without cancelling
the task; reconnect/attach and replay state.**

### Traceability (R1 seat-5 closure)

docs/04 M11 requires nine UX surfaces (docs/04:207-215: conversation; live
operations; changed files; approval UI; streaming output; task status;
pause/resume/cancel; steering; optional execution graph inspection). Today no
gateway-reachable code produces execution events at all
(`configure_verification` / `verify_and_complete` are called only from tests
and the `auth_refresh` example), so the four producer-dependent panes (live
operations, changed files, approval UI, streaming output) would be
permanently empty without items 5–9 below. Items 5–9 and G5/G6 are
therefore **prerequisites of docs/04's own M11 pane list** (runtime scope rule,
HERMES_START_PROMPT.md:18: "Build only the current milestone plus
prerequisites; do not implement deferred features early."), plus two
named anchors: MVP exit (§45) requires a usable `tachyon run` (spec §38) and
M12's fault list names "approval wait", which must exist before M12. If a
board still rules this scope inflation, the only acceptable response is
escalation to the owner for a docs/04 amendment (Phase-A-only M11; items
5–9 and G5/G6 moved behind a new milestone line — item 8 rides item 6, so
approval wait moves with the run path) — never a silent plan edit.

Phase A — TUI + live event streaming (implements docs/04's gate sentence):

1. Gateway push subscriptions: `Subscribe` becomes a streaming mode —
   acknowledgement response, then `EventEnvelope` frames as events are
   journalled.
2. TUI crate (`tachyon-tui`): Ratatui 0.30.2 + crossterm 0.29, three
   independent paths (input task, gateway-event reader task, render loop),
   render only on state change or bounded ticks (≤30 FPS, spec §38).
3. Panes: conversation (objective, steering, durable outcomes), task
   status/list, live operations feed, changed files, approval display and
   decision, input line for steering, pause/resume/cancel, optional execution
   graph inspection (toggle). Panes whose producers land in Phase B render an
   explicit honest empty state until those events exist.
4. CLI: `tachyon attach` (open TUI for a task, picker when no id), plus the
   spec §38 aliases `run`, `ps`, `pause`, `resume`, `cancel` (existing
   `task …` subcommands keep working). `trace` stays out (deferred below).

Phase B — run path + approval wait (prerequisites per Traceability above):

5. Operator-owned model provider configuration (config file/env; kind
   `openai_compat` or explicit `fake` for scripted tests; none configured →
   `run` refuses honestly). `api_key_env` only. **Workspace validation at
   StartRun**: the `--workspace` root must exist and canonicalize *before*
   policy and lease init (ToolsContext canonicalizes best-effort —
   `tachyon-tools/src/lib.rs:74-75` "stays as given until it does" — so the
   gateway must reject roots that fail canonicalization; no create-through-
   symlink window), and the canonical root is pinned into the task's durable
   state in the same pre-spawn sequence as the lease — **[Amended
   2026-09-23, code R1 board B1: the workspace lease is now drawn BEFORE
   the pin, so a `workspace_busy` (or any pre-spawn) refusal leaves no
   pin from a run that never started; pin/lease/policy/evidence identity
   stays one canonical value — the original "pin before any lease" order
   was the wedge]** (docs/06:58
   "canonicalize existing parents and validate containment immediately
   before consequential write"; spec §34 containment).
   **Secrets**: the resolved `api_key_env` *value* is registered with the
   existing `CredentialBroker` redaction registry (`tachyon-tools/
   src/credential.rs`) at config load — spec §35 "Known secret material is
   registered with redaction filters for process/provider output" — and all
   provider error text (`ModelError` bodies) passes the filter before it can
   reach events, logs, or the TUI.
6. `Command::StartRun` gateway command + `tachyon run [--workspace PATH]
   OBJECTIVE`: a **new supervisor-owned run path** — `SupervisorCommand::StartRun`
   executing the M10 slice through the M10 plan §2 proposal/ack pattern
   (worker proposes via run-ID/task/revision-bound message, supervisor
   acknowledges and journals). The `auth_refresh` example is **refactored onto
   the same path** to prove single orchestration (not a fork); its script
   inputs stay example-owned. This is declared the largest single item in
   M11 (writer C); the example's M10 gates must stay green after the
   refactor (G7). If implementation finds this larger than plan-sized, stop
   and escalate rather than silently forking a second orchestration.
7. New durable journal vocabulary, journalled only by the supervisor —
   `stage` (evidence/model/mutation/verify transitions), `evidence_summary`
   (paths + hashes, never source blobs), `changed_files` receipts,
   `agent_message` (durable model answers), `approval_request` (parked ask).
   **`StateEvent`, `apply_journal` and `event_kind` gain typed arms for all
   five kinds before any of them is journalled** (today
   `crates/tachyon-core/src/lib.rs:612-616` parses every payload to
   `StateEvent` and `:768-780` knows only nine kinds — an unknown kind would
   hit `Corrupt` at recovery, failing the task instead of the event).
   Fail-closed `Corrupt` stays reserved for kinds the running build genuinely
   does not know; the TUI's unknown-kind placeholder covers client-side
   forward-compat via the opaque payload passthrough (D2).
   Existing kinds (`created/message/constraint/status/approval/verification_*`)
   unchanged; journal `schema_version` stays 1 (journal format, distinct from
   `PROTOCOL_VERSION`).
8. Approval wait end-to-end:
   - A supervisor-owned job hitting `ToolError::ApprovalRequired { request }`
     parks: status → `WaitingApproval`, supervisor journals
     `approval_request`, inserts a pending row in the existing `approvals`
     table (`decision='pending'`, `decided_at=0`; STRICT schema has exactly
     `id/task_id/operation_hash/decision/decided_at` — no rewrite needed),
     and holds the job.
   - Gateway `Approve`/`Deny` resolve `approval_id → task_id` by store
     **read** (reads do not violate the single-writer rule; every approval
     **write** remains supervisor-owned) and route
     `Command::Approve/Deny { task_id, approval_id }` (task-scoped fields are
     new in protocol v2) to that task's supervisor.
   - **One-shot grant, durably enforced**: the row state machine is
     `pending → granted | denied`, and a granted re-run first flips the row
     to `applied` *before* execution starts (supervisor writes only).
     `tachyon-policy::Approvals` gains consume-on-first-use semantics
     (`resolve` is `&self` today, `crates/tachyon-policy/src/lib.rs:296-319` —
     a granted hash authorizes exactly **one** `authorize()` success;
     afterwards the map entry is consumed), so a model re-emitting the same
     operation after its grant was used must park again under a **fresh**
     approval id — it can never mint or replay its own grant (M3 foreign-grant
     regression stays authoritative).
   - **Idempotency gate (§19)**: the re-run is the *first* execution of the
     previously blocked operation (the original attempt never left
     `authorize()`), so it is permitted under the single grant; a crash after
     `applied` but before an effect receipt lands the task in `Recovering`
     (spec §41) and continuation must **re-ask** — the human re-decides under
     a fresh request before anything runs again (spec §19 as written: "mark
     UnknownAfterCrash and require safe reconciliation; never blindly
     replay" — the recorded-human-decision re-ask is this plan's stricter
     rule on top of it). The parked operation's
     effect class/idempotency are declared per capability (checklist row),
     typically `Unknown → UnknownAfterCrash`.
   - **Restart-during-wait semantics (R1 seats 1+2 closure)**: gateway start
     runs the existing `recover_incomplete` (`server.rs:132`) / spec §41 flow
     → non-terminal task marked `Recovering`; stale `pending` rows are
     expired by the supervisor (`decision='expired'`); a subsequent `Approve`
     on an expired/foreign/unknown id is a **typed error**; the explicit
     continuation command is the **existing `Command::Resume` /
     `tachyon resume <task>`**, which re-enters the driver, re-hits the same
     policy `Ask`, and parks again with a **fresh** `approval_request` id —
     decision still required after every restart. G6 asserts all of it.
     **[Amended 2026-09-23, code R1 board; wording tightened at R2
     seat5's probe: the gateway's `run_policy()` is an allow-list that
     never `Ask`, so no GATEWAY-PROTOCOL run can park — library
     `drive()` with an asking `ToolsContext` CAN park, but M11 ships no
     gateway config or surface that runs it, and either way a
     directly-parked task has no run to re-enter. The driver-re-entry /
     fresh-id leg therefore moves to M12: re-parking without a live
     waiter would manufacture an `Executing` zombie that executed
     nothing (spec §19). The M11 contract is what
     the test pins: expired rows + typed `Approve` refusal + typed
     `Resume` refusal — nothing mints ids, nothing executes, the
     decision (or the run) stays required. M12 lands driver re-entry,
     the ask-capable run policy, and the fresh-id assertion together.]**
   - **Cancel wins**: cancel during `WaitingApproval` → terminal `Cancelled`,
     pending row expired, parked operation never runs, later `Approve` typed
     error (observable assertions in G6).
9. Acceptance source for `run`: detected default for Cargo projects
   (`CommandPasses cargo test --offline --locked`, `ChangedPathsWithin`
   workspace, `FileUnchanged` for `Cargo.toml`/`Cargo.lock`/`migrations/**`),
   overridable by an explicit `--acceptance FILE`; non-Cargo workspaces
   require the explicit file (fail closed). Never model-influenced.

Non-goals (explicit):

- No provider token/SSE streaming — the M6 deferral stands
  (`openai_compat.rs` has no `Streaming` feature). **`GatewayEvent::Progress`
  has no M11 producer**: "streaming output" (docs/04) is delivered as
  durable, ordered journal events pushed live; the `Progress` variant stays in
  the protocol but renders as an honest empty state until the provider-token
  milestone lands its producer (R1 seat-3 closure — Progress has no journal
  commit and therefore no seq, so it cannot ride D2's commit-triggered path).
- No automated restart of `Recovering` runs after a gateway crash (the deep
  recovery sweep is M12); the continuation surface for M11 is exactly
  `Command::Resume` (named above), nothing else.
- No kill/fault sweep (M12), no performance optimization or speed claims
  (M13), no new providers, no workflow/browser/distributed/swarm features
  (AGENTS scope list), remote gateway stays off by default.
- No artifact retrieval UI (`GetArtifact` keeps its stub; its stale
  "arrives in Milestone 3" comment gets a wording fix only).
- No constraint-adding gateway command (steering v1 = messages); no mouse
  support; no `tachyon trace`.

## Capability checklist (AGENTS.md)

| capability | why not already deterministic | schema | access set | effect class | idempotency | resource claim | cancellation | retry | verification | crash recovery | latency |
|---|---|---|---|---|---|---|---|---|---|---|---|
| push subscription | n/a — deterministic plumbing; exists only because clients must observe a durable journal they cannot poll efficiently | `Command::Subscribe{task_id, after_seq}` → ack + `EventEnvelope` frames; bounded per-connection queue | read `task_events` for subscribed task | read-only local | replay by cursor is idempotent (client drops `seq ≤ last_seen`) | bounded queue per subscription connection (256 frames); overflow → `ResyncRequired{after_seq}` | connection teardown drops both split halves; task untouched | client re-`Subscribe`s from its own last-parsed seq; no server retry loop | G2 gapless/overflow/lag tests | journal is source of truth; reconnect replays | first visible event target p95 <50 ms (§43), measured in G7 |
| StartRun driver | replaces ad-hoc host orchestration with one supervisor-acknowledged path | `Command::StartRun{workspace_root, …}` → status/`stage`/`agent_message` events | fs.read/lexical under pinned canonical root; mutation.patch gated by M10 gates; verify commands policy-bound | `ReversibleLocalMutation` (patch) + `DestructiveLocalMutation`/`Unknown` idempotency for verify commands (`tachyon-verify/README.md:64-65`) | attempt re-run only via `Recovering` + explicit `Resume`; proposal replay revision-gated; post-grant crash → re-ask (§19) | workspace lease (existing M9/M10 lease) after canonicalization | supervisor cancel drains workers (existing M10 barriers) | existing `RetryBudget` (one bounded retry) | M10 G2/G4 gates re-run through the shared path (G7) | `Recovering`, never silent `Completed`; M12 deepens | first event <50 ms p95; run duration not claimed |
| approval wait | sync `authorize` fails closed today; a *wait* needs a durable park point a human can decide | `approval_request` event + `approvals(decision='pending', decided_at=0)` row + `WaitingApproval`; row machine `pending→granted→applied` / `→denied` / `→expired` | task-scoped `approvals` rows: gateway reads, supervisor writes | decision recording is read-only; the granted re-run carries its operation's own class (typically Unknown → UnknownAfterCrash) | one-shot: one `authorize()` success per grant, durable `applied` marker before execution; double decide typed error | one pending approval per parked job | cancel/terminal beats wait (row expired, op never runs) | re-run exactly once per unused grant; post-`applied` crash re-asks via fresh request | G6 park/grant/deny/cancel/restart/crash-after-grant | spec §41 `Recovering` + expired pending row; decision required again | interactive (no §43 target) |
| operator provider configuration | operator-owned settings are data; deterministic code already reads them | config/env: `kind ∈ {openai_compat, fake}`, `base_url`, `model`, `api_key_env` (names only) | env read for named key; **no secret values** in config, journal, events, TUI (§35) + CredentialBroker redaction registration | none (configuration) | reload idempotent | none | n/a | n/a | redaction test + `run` refuses when absent | reload on restart re-registers redaction | n/a |
| attach / TUI client | pure display+input client (AD-014) — no deterministic gap, but the capability is new surface | protocol v2 frames only; no direct DB/model/tool access (AD-014) | read-only via gateway commands; steering/pause/resume/cancel/approve/deny only | all user effects go through existing commands (none originate in the TUI) | detach/reconnect idempotent by seq cursor | 1 terminal, 1 subscription connection | detach **never** cancels the task (G4) | bounded reconnect backoff, cursor replay | G3/G4 + input-mapping test | state rebuilt from journal replay | render ≤30 FPS bounded ticks |

## Design decisions (board rules on these)

- **D1 — protocol bump**: introduce a **tagged server frame** wrapper —
  `enum ServerFrame { response | event }` with an explicit discriminator field
  around `ResponseEnvelope` / `EventEnvelope` — plus the new
  `GatewayEvent::Journal` variant, the subscription ack, and the task-scoped
  `Approve`/`Deny` fields. All are breaking for third-party decoders → bump
  `PROTOCOL_VERSION` 1→2 (the existing equality check rejects skew cleanly at
  the request boundary; inbound check stays **per request frame**, unchanged).
- **D2 — streaming architecture, two connections per attach**:
  - *Command connection*: existing strict request/response (unchanged
    semantics), now wrapped in `ServerFrame`.
  - *Subscription connection*: client's first frame is
    `Subscribe{task_id, after_seq}`; server replies
    `ServerFrame::response` with ack
    `{"subscribed":true,"task_id":"…","after_seq":N,"last_seq":M}` (echo of
    the cursor + current max seq), then emits `ServerFrame::event` frames
    only; a later `Subscribe` on the same connection re-acks and switches
    the task/cursor (D3) — on re-subscribe the writer **flushes queued
    previous-task frames and emits the re-ack before any new-task frame**
    (no previous-task frame follows the re-ack), and the client additionally
    drops frames whose `task_id` ≠ the attached task. Discrimination is the
    explicit `frame` tag — no untagged guessing.
  - *Event payload mapping*: `EventEnvelope` carries
    `GatewayEvent::Journal { kind: String, payload: serde_json::Value }` —
    an opaque passthrough of the journalled `StateEvent` JSON (seq, event_id,
    task_id, schema_version from the journal row). The TUI matches on `kind`
    with a safe placeholder for unknown kinds (G3b); future kinds are
    additive and require no version bump.
  - *Commit fan-out*: `StoreWriter` gains a broadcast notification
    `(task_id, seq)` fired **after** each successful journal commit;
    per-connection forwarder subscribes, then pulls from the store by cursor.
    Broadcast lag is safe — the pull catches up from the journal (G2 lag
    test). No commits → no notifications → no busy loop.
  - *Writes*: one writer task per connection owns the write half
    (`tokio::io::split`, valid for Unix stream and Windows named pipe alike);
    reader loop keeps serving commands. Forwarder/writer failure tears down
    **both halves** — a connection can never sit event-dead while still
    answering commands.
  - *Overflow (R1 seats 3+5 closure)*: bounded queue (256 frames). On
    overflow the forwarder stops advancing and clears queued frames, and the
    writer emits `ResyncRequired{after_seq: X}` where **X = the last seq
    whose frame write actually completed on the socket** (delivery recorded
    at write time — never "last queued", which would skip the never-written
    tail). The client always resumes from **its own last-parsed seq** and
    drops `seq ≤ last_seen` on replay, making replay idempotent; X is the
    server's consistency hint, validated but never trusted over the client's
    own cursor. G2 tests the unwritten-tail case explicitly.
- **D3 — one subscription per subscription connection** (re-`Subscribe`
  switches task); the task-list pane refreshes by bounded `ListTasks` poll
  (≥1 s interval while visible), not by subscription.
- **D4 — approval rows**: reuse the existing `approvals` table
  (`decision ∈ {pending, granted, denied, applied, expired}`,
  `decided_at=0` while pending). Row writes: supervisor only (single logical
  writer). Gateway: read-only id→task resolution. Grant consumption is
  durable (`applied` before execution) and registry consumption is one-shot
  (item 8).
- **D5 — closing set**: **G1–G8 are all required to close M11**; G4 is the
  gate text docs/04 names verbatim, the others are additional. There is no
  plan-internal split: if Phase B proves over-sized during implementation,
  stop and escalate — splitting means a docs/04 amendment (new milestone
  line, renumbering M12+), which is architecture-level and owner-approved
  only.

## Acceptance gates (all required — G1–G8 close M11)

- G1. Clean baseline retained (rerun above), red-green tests precede each
  production behavior.
- G2. Streaming: replay from `after_seq` is gapless and ordered; live events
  arrive after `Subscribe`; queue overflow emits `ResyncRequired` with the
  **last-written** cursor and a subsequent `Subscribe` replays exactly the
  missed suffix including the never-written tail; broadcast lag (forced
  `Lagged` receiver) loses nothing after catch-up; a subscribed command
  connection still serves requests; forwarder/writer teardown drops both
  split halves (no event-dead-but-answering connection); re-`Subscribe`
  switches task on the same subscription connection **with previous-task
  frames flushed before the re-ack (no stale-task frame after it; client
  drops `task_id` ≠ attached)**; closing either side leaves tasks untouched.
- G3a (gates Phase A). Headless `ratatui::backend::TestBackend` rendering
  over the **existing nine kinds**, fixture fed through
  `ServerFrame::event` decode → state → buffer (not hand-built state):
  conversation, status, list and approval-display panes render;
  **stated scope: unit-value rendering only — no crossterm input loop or
  live-socket coverage is claimed here** (those live in G2/G4);
  input-mapping unit test proves the detach key issues no `CancelTask`.
- G3b (gates Phase B). Same harness over the full vocabulary — all five new
  kinds render: `stage`, `evidence_summary` (live-operations feed lines),
  `changed_files` receipts, `agent_message`, `approval_request` panes; a
  synthetic unknown `kind` renders the safe placeholder without panic.
- G4. **M11 gate (docs/04 verbatim: "disconnect/close TUI without cancelling
  task; reconnect/attach and replay state")** — automated:
  (i) protocol level: attach (command + subscription connections), receive
  events, disconnect **both**, assert the task still exists with unchanged
  status (no cancel), reconnect the subscription with the client's last-parsed
  seq, assert exact gapless replay of the missed suffix;
  (ii) TUI-client level: the real `tachyon-tui` reader/decoder/state stack
  (terminal-independent by construction — the input task is separate) is
  spawned against a live gateway, torn down by dropping the client, asserted
  to leave the task untouched, then respawned with cursor replay. The
  interactive pty demo of `tachyon attach` recorded in the report is
  **corroborative only, never load-bearing**.
- G5. Run path e2e: scratch copy of `fixtures/auth-refresh` (verified
  present at repo root) copied to a temp dir; `tachyon run` with the scripted
  provider — `tachyon_models::fake::FakeModelProvider`
  (`ProviderId("bench-script")`, `scripted-replay-1`, the same one
  `auth_refresh.rs:39,298` uses) — drives the shared supervisor path to
  durable `Completed`, with `stage`/`changed_files`/`agent_message`/
  verification events visible to a live subscriber; the run is labeled
  "scripted test/replay provider" in CLI/TUI output; **the checked-in fixture
  tree is asserted byte-identical afterwards** (`git diff --exit-code`).
- G6. Approval wait e2e, all observable: park → `WaitingApproval` + pending
  row + `approval_request` event; grant → row `applied` before execution,
  exactly one re-run, proceeds; deny → operation fails with the recorded
  reason; double decide → typed error; **cancel during wait → terminal
  `Cancelled`, row expired, parked op never runs, later `Approve` typed
  error**; **gateway restart with pending row → `Recovering` (spec §41) +
  row expired + `Approve` typed error + `Resume` typed refusal (fresh-id
  leg amended to M12 per item 8, 2026-09-23), decision still
  required**; **crash between grant and `applied` → row expired by
  recovery (re-ask semantics ride M12's re-entry; M11 asserts the row
  truth + typed refusals); crash after `applied` → row survives as
  history and stays undecidable, never a silent auto re-run (spec §19
  "never blindly replay"; recovery never re-arms the registry — the
  human re-ask is this plan's stricter rule and lands with M12
  re-entry)**; grant used once → same
  operation hash parks again under a fresh id (one-shot registry).
- G7. Full workspace: `fmt`, `check` (default + `--all-features`), `test`,
  `clippy -D warnings` (both feature sets) green; the `auth_refresh` example
  and its M10 gates stay green after the driver refactor; docs-freshness green
  with `tachyon-tui` moved into `IMPLEMENTED`, README status + CHANGELOG +
  PROGRESS gate entries updated; a **redaction test** asserting a registered
  key is scrubbed from a provider error body before it can reach events or
  the TUI; **measurement artifact required**: event
  commit (t0, `StoreWriter` return) → `EventEnvelope` frame write completion
  (t1) over n≥100 synthetic events, p50/p95 reported with §43's <50 ms p95
  pass/miss noted (reporting required; a miss fails nothing but must be
  stated).
- G8. Independent five-seat code board reaches unanimous BUILD after
  parent-verified blocker closure; commit only the reviewed scope, no push.

## Ownership and sequence

1. Five independent plan seats: architecture/spec, safety/effects,
   async/durability, verification/honesty, adversarial/cold read. Read-only,
   isolated, no implementation. Plan rounds run to unanimous BUILD
   (R1 → adjudicate → R2 → adjudicate → R3 → …); the parent freezes the
   plan while a round reads it.
2. Parent adjudicates every material finding against live sources, revises
   this document, dispatches the next round; freeze while a round reads it.
3. Implementation phases: writer A = protocol + gateway streaming; writer B =
   TUI crate; writer C = supervisor `StartRun` path + config/CLI (Phase B,
   largest item); parent integrates, runs gates, owns docs and the commit.
   Interfaces pinned in this plan before parallel work opens.
4. Independent code board (its own R1→fix→R2 rounds), final canonical rebuild, gates, docs,
   commit. No push.

## Board record

- Plan R1 dispatched 2026-09-23: **5/5 CONDITIONAL** (architecture,
  safety, async, verification, adversarial), ~19 findings total
  (architecture 2, safety 5, async 4, verification 5, adversarial 7 — heavy
  overlap: durable grant consumption ×3, overflow cursor ×2, G3 phase split
  ×2, gate identity ×2).
- Plan R2 (verify-by-quote) dispatched 2026-09-23: **5/5 CONDITIONAL** —
every R1 finding LANDED (Q1/Q2 quoted), no authority hole (seat 2), no
vacuous gate (seat 4); 7 residual defects: two invented quotations, two
citation errors, redaction test named-but-absent from G7, re-Subscribe
flush rule missing, Traceability trigger/remedy gap + G3b four-of-five kinds.
- Plan R3 (final verify) dispatched 2026-09-23: **4× BUILD + 1×
  CONDITIONAL** (adversarial seat); all 7 fixes LANDED by every seat;
  3 residual citation defects — stale "R2 in flight" ownership line,
  escalation trigger keyed to closed round R2, docs/04 pane quote truncated
  without ellipsis.
- Plan R4 (confirm) dispatched 2026-09-23: **5/5 BUILD — unanimous; plan
  APPROVED at r4.**

## Adjudication log (R1 → r2)

Every cite was re-verified against the live source by the parent before
accepting. All findings accepted (merged where overlapping):

1. **[accepted]** Architecture #1 — unknown journal kinds hit `Corrupt` at
   recovery (`core lib.rs:612-616`, `:768-780` verified). Closure landed in
   item 7 (typed arms before any kind is journalled) and G3b (unknown-kind
   placeholder).
   Variant chosen: restart lands the task in spec §41 `Recovering` with stale
   pending rows expired + re-ask on continuation, rather than recovering
   straight into `WaitingApproval` — §41's existing flow is the canonical
   path and satisfies both seats' "decision still required" requirement.
2. **[accepted]** Architecture #2 — checklist rows added for operator
   provider configuration and the attach/TUI client.
3. **[accepted]** Safety #1 / Async #3 — grant replay forever (schema has no
   consumption column; `Approvals::resolve(&self)` verified). Closure:
   row machine `pending→granted→applied`, `applied` written before
   execution, consume-on-first-use registry (policy API change declared),
   crash-after-grant re-ask case in G6.
4. **[accepted]** Safety #2 / Adversarial #2 — restart-during-wait undefined;
   closure: `Recovering` + expired rows + typed-error `Approve` + named
   continuation command (`Command::Resume`), all asserted in G6.
   [Amended 2026-09-23, code R1 board: `Command::Resume` asserts a typed
   refusal in M11; driver re-entry moves to M12 per item 8.]
5. **[accepted]** Safety #3 — §35 redaction registration omitted
   (spec:983, `credential.rs` verified). Closure: item 5 registers the
   resolved key with `CredentialBroker`, filters provider error text; redaction
   test added to G7's workspace gates.
6. **[accepted]** Safety #4 — `--workspace` validation unspecified
   (best-effort canonicalize verified at `tachyon-tools lib.rs:74-75`;
   docs/06:58 verified). Closure: item 5 rejects non-canonicalizing roots
   before policy+lease, pins the canonical root durably. [Order amended
   2026-09-23 (R1 board B1): lease before pin — see item 5.]
7. **[accepted]** Safety #5 — checklist effect/idempotency wrong (verify
   README:64-65 = `DestructiveLocalMutation`/`Unknown` verified). Closure:
   corrected in the StartRun row; approval row now declares the re-run's own
   class.
8. **[accepted]** Async #1 / Adversarial #7 — overflow cursor skips
   never-written tail. Closure: D2 `after_seq` = last frame whose write
   completed; client resumes from its own last-parsed seq, drops
   `seq ≤ last_seen`; G2 unwritten-tail test; both-halves teardown.
9. **[accepted]** Async #2 — `Progress` has no commit path under D2
   (verified: broadcast fires only on journal commit). Closure: `Progress`
   cut from M11 producers (non-goal) with explicit honest empty state;
   producer arrives with the M6 token-streaming deferral.
10. **[accepted]** Async #4 — G3 untestable at Phase-A order. Closure: G3a
    (existing kinds) / G3b (full vocabulary) split; D5 rewritten (all gates
    required, no plan-internal split).
11. **[accepted]** Verification #1 — G4 leaned on a manual demo. Closure:
    automated TUI-client teardown/respawn leg added; demo explicitly
    corroborative only.
12. **[accepted]** Verification #2 — G3 scope honesty + fixture injection
    point. Closure: decode→state→buffer declared, unit-value limits stated,
    live coverage assigned to G2/G4.
13. **[accepted]** Verification #3 — "cancel wins" had no assertions.
    Closure: full observable list in G6.
14. **[accepted]** Verification #4 — G5 starting state unpinned. Closure:
    named fixture (verified present), scratch procedure,
    `FakeModelProvider` identified (`auth_refresh.rs:39,298` verified),
    checked-in-tree-unchanged assertion added.
15. **[accepted]** Verification #5 — measurement outside required gates.
    Closure: folded into G7 as a required artifact (t0/t1 defined).
16. **[accepted]** Adversarial #1 — gate-identity contradiction. Closure: D5
    states G1–G8 all required; G4 = docs/04's verbatim gate.
17. **[accepted]** Adversarial #3 — scope inflation vs docs/04. Closure:
    Traceability section added with the escalation-only fallback (owner-level
    docs/04 amendment; never a silent plan edit).
18. **[accepted]** Adversarial #4/#5 — connection roles, ack payload,
    version-check timing, new-kind carrier. Closure: D1 tagged `ServerFrame`
    + D2 two-connection model + named ack JSON + `GatewayEvent::Journal`
    opaque passthrough + per-request inbound check retained (facts verified:
    spec §41 recovery, `recover_incomplete server.rs:132`, `Approvals`
    registry `policy lib.rs:296-319`).
19. **[accepted]** Adversarial #6 — "extract, not fork" was a silent
    rewrite risk. Closure: item 6 reworded to the supervisor-owned
    `StartRun` path (M10 §2 proposal/ack pattern), declared largest item,
    example refactored onto it with its gates staying green (G7), escalation
    instead of silent forking.

## Adjudication log (R3 → r4)

All R3 residuals accepted; three citation-only fixes applied (5 patches):

1. **[accepted, seat 5]** Ownership item 1 no longer claims a specific round
   is "in flight" — replaced with the evergreen round sequence (cannot go
   stale again); item 4 clarified as the code board's own rounds.
2. **[accepted, seat 5]** Traceability escalation trigger is now
   round-agnostic ("If a board still rules…").
3. **[accepted, seat 5]** The docs/04 pane list is quoted in full (nine
   surfaces, docs/04:207-215) and "those four panes" replaced by the
   explicit list of producer-dependent panes (live operations, changed
   files, approval UI, streaming output).

## Adjudication log (R2 → r3)

All R2 residuals accepted; seven surgical fixes applied (11 patches):

1. **[accepted, seats 1+4+5]** Invented quotation "("Unknown after crash"
   allows re-run only "after a recorded human decision")" removed; item 8
   now quotes spec §19 as written (docs/02:561 "mark UnknownAfterCrash and
   require safe reconciliation; never blindly replay") and declares the
   recorded-decision re-ask as this plan's stricter rule; G6's "(§19)"
   parenthetical aligned to the same wording.
2. **[accepted, seats 2+5]** G7 now names the redaction test (registered key
   scrubbed from a provider error body before it can reach events or the
   TUI) — the adjudication log claim is now true of the gate text.
3. **[accepted, seat 3]** D2 re-Subscribe flush rule: previous-task frames
   cleared before the re-ack, none after it, client drops `task_id` ≠
   attached task; asserted in G2 (no stale-task frame can inflate
   `last_seen` and gap the new task's replay).
4. **[accepted, seat 5]** Traceability trigger/remedy now cover items 5–9 +
   G5/G6 consistently; item 8 explicitly rides item 6 (approval wait moves
   with the run path).
5. **[accepted, seat 5]** G3b names all five kinds; `evidence_summary`'s
   render host declared (live-operations feed lines).
6. **[accepted, seats 1+5]** Scope-rule attribution corrected to
   HERMES_START_PROMPT.md:18 — AGENTS.md contains no such sentence.
7. **[accepted, seat 5]** Adjudication log item 1 now cites G3b (not G6) for
   the unknown-kind placeholder; G6 asserts restart semantics, G3b asserts
   the placeholder.
