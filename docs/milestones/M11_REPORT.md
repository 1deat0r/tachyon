# M11 report — TUI + live gateway events + run path (measured)

Status: IMPLEMENTED 2026-09-23. Plan r4 unanimous
BUILD (`docs/milestones/M11_PLAN.md`). Code G8: R1 returned 2×BUILD +
2×CONDITIONAL + 1×REJECT (ten blockers — all fixed or amended below);
R2 returned 2×BUILD + 2×CONDITIONAL with seat5 pending (closures for the
R2 conditionals landed same day — see Board record). No speed
claim; no live-model
quality claim (scripted providers prove integration, not judgment quality).

## What was built

- Protocol v2: tagged `ServerFrame` (response/event discriminator),
  `GatewayEvent::Journal` opaque passthrough, subscription ack
  (`subscribed/task_id/after_seq/last_seq`), task-scoped Approve/Deny.
- Gateway push subscriptions: ack, then live `EventEnvelope` frames fanned
  out from `StoreWriter` commit broadcast; bounded 256-frame queue with
  last-written-cursor `ResyncRequired`; re-`Subscribe` switches task with
  previous-task frames flushed before the re-ack.
- `tachyon-tui` (Ratatui 0.30.2 + crossterm 0.29, AD-014 pure client):
  conversation, task status/list, live operations feed, changed files,
  approval display/decision, steering input, pause/resume/cancel, optional
  execution-graph inspection; producer-dependent panes render honest empty
  states; render on state change or bounded ticks (≤30 FPS).
- CLI: `tachyon attach` (+ picker), `run`, `ps`, `pause`, `resume`,
  `cancel` aliases; existing `task …` subcommands unchanged. `trace` stays out.
- Operator provider config (`openai_compat`/`fake`, `api_key_env` names
  only); key value registered with the redaction registry at load; provider
  error text filtered before events/logs/TUI. No provider → `run` refuses.
- Supervisor-owned `StartRun`: workspace validation + canonicalization
  before policy/lease, canonical root pinned durably, detected Cargo
  acceptance (explicit file otherwise), M10 proposal/ack pattern through
  the ONE shared driver; `auth_refresh` example refactored onto it.
- Run-held workspace lease: prepare takes it non-blocking on the
  canonical root BEFORE the durable pin (a `workspace_busy` refusal
  leaves no pin — R1 B1), attaches it to the
  run's `ToolsContext`; every drive-reachable inner acquisition
  (verification capture/plan/runner) reuses the guard instead of
  re-acquiring the non-reentrant lock. `WorkspaceLease::try_acquire`
  added as the probe primitive.
- Five new supervisor-journalled kinds (`stage`, `evidence_summary`,
  `changed_files`, `agent_message`, `approval_request`) with typed
  `StateEvent`/`apply_journal`/`event_kind` arms; TUI renders all five plus
  a safe placeholder for unknown kinds.
- Approval wait: park → `WaitingApproval` + pending row + event; grant is
  one-shot and durable (`pending→granted→applied` written BEFORE the
  decision is journalled — row truth first, R1 B4);
  deny/double-decide/cancel-during-wait typed; restart → `Recovering` +
  expired rows (stale pendings AND granted-never-applied orphans, R1
  B3) + typed `Approve` refusal + typed `Resume` refusal; applied rows
  survive as history and stay undecidable — never a silent auto re-run
  (spec §19). Driver re-entry / fresh-id re-ask amended to M12 (see
  blockers section).
- Parent-found fix (reviewable inside G8): stale supervisor handles after
  run completion surfaced transient `supervisor_gone` to status polls —
  `with_live_supervisor` evicts and recovers once (GetTask/mutate/decide)
  plus a deterministic regression test. `GetArtifact` stale comment fixed.

## Measured runs

- G7 commit→frame latency (streaming `g7_reports_…`, n=200 synthetic
  events, t0=commit return, t1=frame read): p50 ~259µs, p95 ~311µs —
  §43 p95<50ms PASS (reporting-only gate; values vary run to run; the
  prior R1-fix run measured ~272/~362, same verdict).
- G5 e2e: scratch `fixtures/auth-refresh` copy driven by the scripted
  provider to durable `Completed` with live `stage`/`changed_files`/
  `agent_message`/`verification_finished` observed; checked-in tree
  byte-identical afterwards.

## Gates (executed, exit 0)

- `cargo fmt --check`; `cargo check --workspace` (default + `--all-features`).
- `cargo test --workspace`: 86 suites, 466 passed / 0 failed / 0 ignored
  (default); `--all-features`: 474 / 0 / 0.
- `cargo clippy --workspace --all-targets [--all-features] -- -D warnings`.
- G2 streaming (11 tests incl. overflow/lag/teardown/switch + G7
  measurement); G3a/G3b TestBackend rendering (existing + five new kinds +
  unknown-kind placeholder); G4 disconnect/reconnect gapless replay
  (protocol + TUI-client legs); G5 run e2e; G6 approval wait observables;
  redaction test; `auth_refresh` example + M10 runtime gates green after
  the driver refactor; docs-freshness green with `tachyon-tui` IMPLEMENTED.
- Transients observed (green on immediate rerun, noted not hidden): one
  `tachyon-verify` runner timing failure; two `g5_e2e` `supervisor_gone`
  hits before the parent fix (root-caused and fixed, see above).

## Board record

- Plan R1–R4: 5×CONDITIONAL → 5×CONDITIONAL → 4×BUILD+1×CONDITIONAL →
  5×BUILD unanimous; plan APPROVED at r4 (record in `M11_PLAN.md`).
- Code G8 R1 (2026-09-23): seat1 architecture BUILD, seat2 safety
  CONDITIONAL, seat3 async BUILD, seat4 honesty CONDITIONAL, seat5
  adversarial REJECT — ten blockers raised; all fixed or amended in the
  section below.
- Code G8 R2 (2026-09-23): seat2 safety **BUILD** (amendment blessed),
  seat4 honesty **BUILD** (gates reproduced exactly: 466/474/86, G7
  PASS inside the disclosed envelope), seat1 architecture CONDITIONAL
  (stated the old pin→lease safety order in `start_run`'s doc + plan
  item 5 / adjudicated #6), seat3 async CONDITIONAL (single-flight
  election cleared the flag before publishing the handle; a waiter
  could clear a flag it did not own), seat5 adversarial still running
  (its probe already forced the amendment-wording tightening above).
  Closures landed: `supervisor_for` restructured — election loop +
  `finish_recovery` (publish-before-clear, owner-only flag clear, bound
  5 s then an honest direct attempt); order claims amended marker-stamped
  in `start_run`'s doc, plan item 5 and adjudicated #6; `take_asked`
  comment corrected to held-whole-run; amendment premise tightened to
  "no GATEWAY-PROTOCOL run can park". R3 re-verification pending
  (unanimous BUILD required before commit).

## R1 blockers: fixes and one amendment

Fixed, each with a regression test (RED→GREEN noted where the red was
observed):

- **Seat5-B5 Subscribe loaded the whole journal per attach** → cursor
  pushdown: `load_events_since(after_seq)` filters in the store; new
  `latest_seq` MAX aggregate answers `last_seq` (`server.rs::subscribe`,
  `store/lib.rs::latest_seq`); streaming suite 11/11 green (overflow /
  replay / teardown assertions unchanged).
- **Seat2-B3 granted-never-applied orphan survived recovery** → new
  `expire_granted` + `load_granted_for_task`; recovery now expires stale
  pendings AND these orphans (`recover_task`); test
  `granted_without_applied_expires_on_recovery` (RED before the fix).
- **Seat5-B3 blind evict raced recoveries into `task_already_owned`** →
  single-flight recovery election in `supervisor_for` (losers wait for
  the winner's handle); test
  `concurrent_get_task_recovers_once_with_no_already_owned` (RED with
  the election disabled, GREEN with it).
- **Seat5-B4 the journal could LEAD the durable row on decide** →
  `decide_approval` moves the row (and `applied` on a grant) BEFORE
  journalling; tests assert the crash-window row carries no forged
  `approval` event and that recovery never invents one.
- **Seat5-B1 pin-before-lease wedged a refused run's task** → the
  workspace lease is drawn BEFORE the pin; a `workspace_busy` refusal
  leaves no pin; test `busy_workspace_leaves_no_pin_and_the_retry_admits`
  (refusal → `workspace_root` null → free → retry pins).
- **Seat5-B6 bare ^a/^d decided approvals on one keystroke** →
  approve/deny are confirm-gated like cancel: the armed id is captured,
  confirmed `y` re-checks the pending ask before anything is sent, the
  footer shows the gated command + id; three input tests cover arm /
  confirm / stale-pending.
- **Seat5-B2 a command after cancel resurrected a terminal task** →
  verified the central guard already makes recovered actors write-proof
  (`transition_journalled` refuses any event from a terminal state;
  `start_run`/`prepare_run` re-check explicitly); test
  `commands_after_cancel_never_revive_the_terminal_task` pins every
  mutator typed-refused + revision unchanged. Closure variant: reads
  need the recovered actor, so the guarantee is "cannot mutate", not
  "never spawns" — flagged for R2 to judge equivalence.
- **Seat4-B1 counts/G7 unverified** → rerun at final state: 466/474
  passed, 86 suites, G7 p50 ~259–272µs / p95 ~311–362µs PASS (above).

Amended — a plan change, not code:

- **Seat2-B1/B2 Resume-from-Recovering driver re-entry + crash-after-
  applied fresh-id re-ask** → deferred to M12, amended in place in
  `M11_PLAN.md` (item 8, G6, adjudicated-finding #4 — all marker-stamped
  "Amended 2026-09-23"). The precise premise (tightened at R2 after
  seat5's probe): the GATEWAY's `run_policy()` allow-list never `Ask`s,
  so no gateway-protocol run can park — library `drive()` with an
  asking `ToolsContext` CAN (seat5 demonstrated it), but M11 ships no
  gateway surface that runs or re-enters such a run; directly-parked
  tasks have no driver to re-enter; re-parking without a live waiter
  would manufacture an `Executing` zombie that executed nothing
  (spec §19). M11's contract stays test-pinned: expired /
  refused / undecidable rows, typed `Resume` refusal, recovery never
  re-arms the registry. M12 lands driver re-entry + an ask-capable run
  policy + the fresh-id assertion together. R2 must bless the amendment.

## Handoff note (2026-09-23 session)

Mid-close-out the tree received edits from the previous session's writer
slice (lease tests, `ToolsContext` carrier, verification reuse,
`workspace_busy`, drive cancellation) with no live local process; owner
confirmed it was last-session work and put this session in charge. The
slice was absorbed (its `run_lease`/lease tests pass unmodified except a
one-line clippy fix in `driver_run.rs`), the earlier quarantine of the
gateway lease test lifted by the writer's own rework note, and the G8
board brief names the lease rework as a focus area. M12 is next, not started.
