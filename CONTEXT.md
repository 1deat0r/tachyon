# Tachyon — Shared Language

Domain glossary for issues, PRs, tests, and code names. Use these terms exactly; do not drift to synonyms.

Authoritative deeper contracts: `docs/01_ARCHITECTURE_FREEZE.md`, `docs/02_IMPLEMENTATION_SPEC.md`, `AGENTS.md`.

## System shape

| Term | Meaning |
|------|---------|
| **Gateway** | Local IPC (and optional remote) boundary. Clients (CLI/TUI) talk only to the gateway; no client owns agent decisions. |
| **Task Supervisor** | Single logical writer of canonical task state for one task. Owns routing, planning, steering, recovery, completion coordination. |
| **Session** | Persistent user interaction context. Owns zero or more tasks. |
| **Task** | Executable unit of work with a durable status machine. |
| **CLI** | `tachyon` binary (`tachyon-app`). Argument parsing, config, output. No decision logic. |
| **TUI** | Ratatui client (`tachyon-tui`). Pure gateway client (AD-014): display + input only. |
| **Shared driver** | The ONE run path spawned by the gateway (`StartRun` → `drive`). CLI/TUI never spawn their own. |

## Execution and state

| Term | Meaning |
|------|---------|
| **Execution IR / ExecutionGraph / ExecutionNode** | Validated machine-readable plan before anything runs. Model tool calls are proposals until validated into IR. |
| **Access set** | Declared read/write (and related) footprint of a node. No two running nodes may hold conflicting access sets. |
| **Effect class** | Declared consequence class of an operation (e.g. pure read, local process, external keyed effect). Paired with **idempotency**. |
| **Commit barrier** | Point where irreversible/ambiguous effects become durable under policy. |
| **Durable journal** | Append-only event log; source of truth for recovery. Snapshots are materializations, not the sole truth. |
| **TaskStatus** | Canonical enum: `Created`, `Routing`, `Planning`, `Executing`, `Verifying`, `WaitingApproval`, `Paused`, `Recovering`, `Completed`, `Failed`, `Cancelled`. |
| **Recovering** | Status while a gateway/supervisor rebuilds state after restart; not a silent resume of unknown effects. |
| **Workspace pin / canonical root** | One durable canonical filesystem root for a run; policy, evidence, and mutation all read that same value (no second resolution). |
| **Workspace lease** | Exclusive claim on a canonical workspace root for the life of a run (`workspace_busy` when contended). |

## Security and judgment

| Term | Meaning |
|------|---------|
| **Capability** | Explicit policy-controlled permission (path globs, process, network, credentials). Model text cannot grant capabilities. |
| **Access set** | See above; also the thing policy checks against capabilities. |
| **JudgmentProvider** | Abstraction over OpenJEV (and fakes). OpenJEV is replaceable; core works without it. |
| **Acceptance contract / verification gate** | Machine-checkable definition of done. Completion never comes from model self-report. |
| **Approval** | Human (or policy) grant for a parked operation; one-shot, durable, never silently replayed when unknown. |

## Routing and cost

| Term | Meaning |
|------|---------|
| **Fast router** | Predictive cheapest-sufficient route (not serial trial of models). |
| **Jev** | Cheap judgment/scoring path behind `JudgmentProvider` (and related tools). |
| **Evidence** | Deterministic facts collected for routing/planning (repo intelligence, hashes, search), before or alongside model calls. |

## Repo / crates (shorthand)

Use crate names when the boundary matters: `tachyon-core` (supervisor/state), `tachyon-gateway` (IPC), `tachyon-ir`, `tachyon-store`, `tachyon-policy`, `tachyon-tools`, `tachyon-scheduler`, `tachyon-verify`, `tachyon-models`, `tachyon-judgment`, `tachyon-router`, `tachyon-tui`, `tachyon-app`.

**Dependency rule:** lower-level crates never import `tachyon-core`, gateway, or UI. Provider types do not leak into core/IR.

## Words to avoid (use the glossary term instead)

| Don't say | Say |
|-----------|-----|
| "agent loop" for the supervisor | **Task Supervisor** |
| "plan" for the validated DAG | **Execution IR** / **ExecutionGraph** |
| "permission" | **capability** |
| "done" without evidence | **verification gate passed** / **Completed** |
| "restart resume" for unknown effects | **Recovering** + reconcile (never blind replay) |
| "MCP call" for in-process tools | **native tool** (MCP is external boundary only) |
