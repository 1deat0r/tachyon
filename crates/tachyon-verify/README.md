# M9 verification crate

`tachyon-verify` owns acceptance interpretation, local source evidence, Rust check
selection and scheduler-backed execution. It does not own task state and never
imports `tachyon-core`. A supervisor must separately reject unresolved effects,
stale task revisions and post-report source drift before committing `Completed`.

## Public boundary

- `AcceptanceContract`, `Clause`, `CommandCheck`, `VerifyError` preserve the shared
  provider-neutral contract API. Every clause is required. Empty contracts fail.
  Legacy strings recover as `Unresolved`, never as success. Hard constraints must
  be unique top-level exact ID/text bindings to the supplied trusted requirements;
  nested hard wrappers are rejected (maximum wrapper depth: one).
- `WorkspaceSnapshot::capture(&Path)` is a **trusted synchronous local** entry
  point. `capture_authorized(&ToolsContext)` checks exact `fs.list`, `fs.metadata`
  and `fs.read` scopes before directory/entry/content reads. Broad grants cannot
  override specific denials. Runtime callers should use the authorized helper.
- `VerificationPlan::build(task_id, revision, contract, baseline, hard, risk)` is
  the corresponding trusted synchronous planner. Prefer
  `build_authorized(task_id, revision, contract, baseline, hard, risk, context)`
  with runtime policy. Both bind canonical roots and validate all contract input.
  Async callers must call either synchronous builder in a blocking pool.
- `ProjectDetector` / `RustProjectDetector` select commands, not results. Ordinary
  member Rust sources select the nearest `Cargo.toml` **plus the
  reverse-dependent closure** (members depending on a changed member, transitively).
  Unparseable dependency metadata broadens conservatively to the workspace check
  instead of omitting tests. Root/shared/unknown impact selects
  `cargo test --offline --workspace`. Full risk adds that broad check
  after focused and explicitly required checks. Cargo itself parses/validates
  manifests: a malformed manifest cannot become invented verification success.
- `run(plan, Arc<ToolsContext>, CancellationToken)` executes fresh checks through
  `tachyon_scheduler::spawn` and the verification `Executor`, using
  `tachyon_tools::process::run_cancellable`, never a private command bypass.
  One canonical workspace runs one verification at a time process-wide (async
  lease held through scheduler shutdown and worker drain), so independent runs
  cannot execute conflicting workspace write claims concurrently and a timed-out
  scheduler cannot hand the workspace to the next run while its process is still
  handling TERM.
- `VerificationReport` binds task/revision, plan-input digest, final snapshot and
  per-node evidence. `passed()`, `failures()`, `snapshot()`, `task_id()`,
  `revision()` and `checks()` are read-only. `CheckEvidence` exposes node ID,
  status and bounded diagnostics; serialization also includes command digests,
  exit status, scheduler status and stdout/stderr artifact IDs.
  **Deserialized reports are historical, not fresh completion authority:**
  `passed()` deliberately returns false after a serialization round trip.

## Capability checklist

### `verify.command`

| Requirement | Implementation |
| --- | --- |
| Why execute a capability? | Project-specific executable tests establish truth that acceptance text or model judgment cannot establish. Dispatch and interpretation remain deterministic. |
| Input schema | Exact object `{binding: string, command: CommandCheck}`. Command fields: nonempty `program`, literal string `args`, normalized relative `cwd` (`.` for root), string map `env`, integer `timeout_ms` in `1..=600000`. Unknown fields and weaker/altered IR are rejected. |
| Output schema | Executor-private evidence: command hash, real exit code, bounded diagnostic, redacted stream artifact IDs. No caller-supplied pass booleans or model outputs are consumed. |
| Access | Conservative `dir:/workspace/**` write claim. A test or build script can mutate sources; it is not a read-only operation. |
| Effect class | `DestructiveLocalMutation`, never speculative. Native execution is not a sandbox; see scope below. |
| Idempotency | `Unknown`. |
| Resource claim | One process slot, 100 CPU units, no provider/GPU/network scheduling claim. Fits scheduler defaults. These are scheduling estimates, not OS limits. |
| Policy | Explicit `verify.command` on the **resolved canonical** workspace cwd scope **and** underlying `process.spawn` on the program are both required. The cwd is contained (traversal rejected, symlinks resolved, inside-workspace verified) before authorization; the scope, approval binding (invocation plus resolved scope) and executed directory all derive from that same resolved target, so a `target/alias -> ../restricted` style symlink cannot bypass a denial. Source reads independently enforce exact file policy, including root metadata. Unknown capability policy blocks/asks; no trusted global defaults are modified. |
| Cancellation | Run owns its scheduler and a separate worker `JoinSet`. Dropped scheduler execution futures cancel owned workers rather than dropping their process cleanup. Explicit cancellation drains the cancellable process runner and workers before return. Aborting the whole run aborts owned tasks; process drop provides the upstream force-kill fallback. |
| Retry | Exactly one attempt; no automatic replay. |
| Verification method | Real process status plus per-node scheduler-success/attempt cross-check. Sources are rehashed before and after commands and after draining the scheduler. |
| Crash recovery | Command effects remain unknown after a crash. The supervisor must block/reconcile; never automatically replay based on a stored report. |
| Latency | Local process latency with an explicit timeout, plus full-source scan costs. No model, paid API or remote test dependency. |

### `verify.clause`

Exact `{binding: string, clause: Clause}` payload; read-only workspace claim, no
process slot, one attempt and no speculation. `FileUnchanged` compares trusted
baseline fingerprints/existence; a directory is not accepted as a covered file.
`ChangedPathsWithin` compares actual added/deleted/modified file paths using
segment boundaries, not raw prefix matching. An empty allowed list permits no
file changes. Unresolved requirements block execution and completion. Deterministic
comparison itself adds no rights: every fresh source read uses the authorized
snapshot helper. The graph and executor independently recompile minimum
capability declarations before accepting a node.

## Honest scope

- Snapshots cover local regular source files, including hidden files, lockfiles
  and Unix mode changes. Only directories named `.git` or `target` are excluded
  anywhere in the tree. No gitignore, caller-provided changed-file list or
  repository-index cache decides coverage. Explicit protected paths inside
  exclusions are rejected, not falsely reported unchanged.
- Symlinks and nonregular entries in coverage are refused, and walk/read errors
  fail closed. Roots are canonicalized and compared. Fingerprints are BLAKE3.
- Scans are observational local filesystem snapshots, **not atomic filesystem
  snapshots or an adversarial filesystem sandbox**. A hostile concurrent writer
  can race path resolution or change-and-restore bytes between observations.
  Runtime should isolate/serialize workspace mutation; the supervisor must
  rehash immediately before completion. Native verifier programs can access
  external resources, contact networks, spawn children or change generated
  directories unless stronger OS isolation/policy is supplied. `--offline`
  prevents Cargo dependency fetching; it is not a network sandbox for test code.
- Policy-aware constructors are provided; the policy-free `capture`/`build`
  compatibility APIs remain trusted-only. Baselines/hard requirements must come
  from the runtime, never from model-authored JSON.
- Process group termination, output spooling/redaction and unsupported-platform
  behavior are owned by `tachyon-tools`. This crate does not claim Windows Job
  Object support or repair upstream process output-memory limits.

## Executed verification

```
cargo fmt -p tachyon-verify
cargo test -p tachyon-verify
cargo clippy -p tachyon-verify --all-targets -- -D warnings
```

Implementation used executed tracer red/green cycles. Observed reds included:
malformed command accepted; traversal/protected path accepted; missing snapshot,
planner and runner APIs; missing shared-input expansion; missing hard binding
accepted; missing affected-first ordering; verification-policy side effect;
stale plan passing; denied source reads bypassed; scheduler cancellation dropping
process cleanup; and forged executor declarations reaching execution. Each was
followed by the corresponding green test. Additional regression coverage checks
legacy strict serde, symlinks/nonregular entries, mode/lockfile changes, real
success/failure exits, both policy boundaries, repeat execution, serialized
report non-authority, unresolved requirements and missing/timeout commands.
