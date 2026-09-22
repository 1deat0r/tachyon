# Task/attempt-scoped recovery

Use `MutationEngine::recover_scoped(context, batch_id, action, allowed_paths)`
for runtime recovery. The public types are re-exported at the crate root:

- `RecoveryAction::{Finish, Compensate}`
- `RecoveryDisposition::{Committed, Compensated}`
- `ScopedRecoveryReport { batch_id, disposition, changed, cleaned }`

The caller binds a **trusted, stable state directory to one task/attempt**, passes
its exact durable batch identity and normalized allowed target paths, and holds
the shared workspace lease for the entire synchronous call. There is no task-ID
in this low-level API: directory/task binding and exclusive admission belong to
the supervisor. Source and state directory must not overlap; canonical workspace
identities in the engine and `ToolsContext` must agree. Canonical aliases work.

## Preflight and effects

The preflight is read-only. Before any staging, replacement, deletion or new
receipt it validates:

1. Exactly the named batch, one nonempty prepared plan, known file identities,
   legal receipt transitions, and no sibling/unknown records, unknown fields,
   malformed/blank lines or unterminated tail.
2. Exact normalized target allowlist membership; deterministic batch/target temp
   names; disjoint target/temp paths; no symlink (including dangling links),
   non-regular leaf, traversal or missing parent in any required path.
3. Every source matching its recorded preimage/postimage, with terminal receipts
   agreeing with their required current image.
4. Both retained artifacts, including 64-character lowercase content-addressed
   IDs and matching decoded BLAKE3 contents. Conservatively, retained artifacts
   are required even for an already terminal batch.
5. Every required policy operation below, including cleanup and created-target
   deletion. Denials and unresolved `Ask` decisions fail closed.

Before consequential use it checks the journal and complete source/temp snapshot
again, rechecks the individual source/staging hashes and path kinds, and repeats
operation authorization. It never calls the legacy bulk recovery entry.

| Resource | Required policy capabilities |
| --- | --- |
| Each allowed target | `fs.metadata`, `fs.read`, `mutation.patch`, `fs.write` |
| Exact prepared temp | `fs.metadata`, `fs.read`; `fs.write` if staging/reusing; `fs.delete` if consumed or cleaned |
| Fresh compensation staging path | `fs.metadata`, `fs.read`, `fs.write`, `fs.delete` |
| Created target being compensated | Additionally `fs.delete` on that exact target |
| Each retained artifact | `fs.metadata`, `fs.read` on `external:<canonical artifact path>` |

Workspace scopes use `workspace/<canonical relative path>`. The operation JSON
binds capability, scope, batch ID, action, expected hash and intended hash.
Trusted-workspace defaults alone do **not** grant `mutation.patch`, `fs.delete`
or external artifact reads; the runtime must supply the necessary grants. The
journal is internal engine state, not a workspace capability operation.

## Dispositions and cleanup

- **Committed** requires all fresh postimages, every commit receipt and the
  batch-completion receipt. Repeated finish is safe and adds no duplicate events.
- **Compensated** requires all fresh preimages (including absence for creates)
  and every rollback receipt. It is not a successful repair. Repeated compensation
  is safe; finish cannot resurrect a compensated batch. A completed batch cannot
  be compensated through this API: a deliberate subsequent mutation needs a new
  batch.
- `changed` contains newly recorded transitions only. It includes catching up an
  unreceipted rename or rollback; it is not the entire historical changed-file
  set. `cleaned` contains exact owned postimage temps removed by this invocation,
  not staging renames or removed created targets.
- Cleanup never scans the workspace. Only a journal's exact postimage temp path
  with freshly matching content is removable. Foreign, orphan, protected marker,
  changed-content and unjournaled restore files remain untouched. Compensation
  uses a fresh `create_new` staging path; a late foreign occupant is refused even
  if its bytes happen to match. A crash before that new path is renamed leaves
  an orphan, deliberately retained rather than guessed to be garbage.

An error leaves the caller's effect **Unknown**. Preflight failures cause zero
workspace changes. IO or interference after effects start can leave a partial
batch: do not infer no effect, successful rollback or completion from an error or
from a worker merely ending. Reconcile explicitly and run fresh task verification.

## Normal (authorized) writes

`MutationEngine::prepare_authorized(context, batch_id, specs)` and
`MutationEngine::commit_authorized_up_to(context, prepared, limit)` are the
runtime entries for an ordinary patch. The caller holds the shared workspace
lease, reserves and persists the batch identity before preparation, and binds a
task/attempt state directory that holds exactly one batch. The low-level
`prepare`/`commit`/`commit_up_to` remain trusted, policy-free APIs for tests and
internal resume, and are not runtime entry points.

### Preflight before any effect

`prepare_authorized` is all-or-nothing: every check below happens before any
journal, spool, directory or source write, so a refusal performs **zero
workspace mutation** and writes no journal record.

1. The canonical workspace identity must match `ToolsContext`, and the
   canonical state directory must be disjoint from the workspace **in both
   directions**.
2. Every spec path must already be in normalized journal-key form. `sub/./b.rs`,
   `sub//b.rs`, `sub\b.rs`, `..`, absolute and empty paths are refused; targets
   must be unique, and so must the derived temps.
3. Journal strict truth: this attempt's journal must be free of malformed,
   torn-tail, unknown-field and sibling-batch records, and must not already
   contain the reserved identity. A crash before the complete `BatchStarted`
   record stays `Unknown` and is never blindly replayed; a reused id is refused
   rather than truncating or adopting an existing owned temp.
4. Each target must resolve to its literal path (containment plus an exact
   canonical match): every existing component must be a literal directory and
   the leaf a regular file, or missing for a create. Symlinks — including
   dangling ones and symlinked ancestor directories — are refused.
5. The derived temp must not exist in any form. A foreign occupant is refused
   even when its bytes already match the intended postimage, and staging writes
   create-new files, so no path is ever truncated into ownership.
6. Every recorded preimage must match the file on disk exactly.

### Normal-write grants

| Resource | Required policy capabilities |
| --- | --- |
| Each exact target | `fs.metadata`, `fs.read`, `mutation.patch`, `fs.write` |
| Exact derived temp, before preparation | `fs.metadata`, `fs.write`, `fs.delete` |
| Exact derived temp, before each commit | `fs.metadata`, `fs.read`, `fs.write`, `fs.delete` |
| Retained postimage artifact, only when re-staging a lost temp | `fs.metadata`, `fs.read` on `external:<canonical artifact path>` |

Workspace scopes are `workspace/<canonical relative path>`, so a temp grant is
an exact derived path such as `workspace/sub/.b.rs.tachyon-tmp-<batch id>`,
never a broad pattern. A specific deny or an unresolved `Ask` always beats a
broad `Allow`, and both fail closed. Trusted-workspace defaults do **not** grant
`mutation.patch` or `fs.delete`; the runtime must supply them. Internal engine
state and artifact-spool writes are trusted storage and need no workspace
capability; the journal is not a workspace capability operation.

The exact operation JSON for each step carries `action`, the batch identity,
`op`, the exact scope and the plan (`path`, `pre_hash`, `post_hash` per file in
commit order); commit operations additionally carry `limit`, `path`,
`expected_hash` and `content_hash` for the file at that boundary. A material
change to the batch, paths or hashes changes the operation hash, so an approval
for one patch cannot authorize another. The runtime can read the exact list it
will be asked for — read-only, no policy or filesystem effects — through
`MutationEngine::prepare_authorizations(batch_id, specs)` and
`MutationEngine::commit_authorizations(prepared, limit)`, which return
`AuthorizedOp { capability, scope, operation }` in check order.

### Commit boundaries

`commit_authorized_up_to` validates the descriptor against strict task-scoped
journal truth (identity, plan and every hash; the descriptor's `state` fields
are not authority) and refuses unknown, forged, sibling, malformed, completed
and compensated batches. It derives the pending set from the journal, so the
**original** prepared descriptor is reusable for every boundary: already
committed files are skipped and are not reported again.

For the requested boundary it authorizes every pending operation and inspects
every source and owned temp image before the first rename; immediately before
each real rename it re-authorizes, re-hashes the source and temp, and re-reads
the journal (counting this call's own receipts). `limit = 1` is the runtime's
per-file cancellation/revision/policy recheck point. A stale source, a changed
or foreign temp image, a divergent post-rename image or a foreign journal change
refuses without writing a receipt: only `FileCommitted` and the final
`BatchCompleted` receipts are appended. `completed: true` is a **batch** receipt
and not task completion authority — the runtime still requires fresh M9
verification.

Refusals before effects leave the batch exactly as prepared, so `recover_scoped`
(Finish or Compensate) remains valid for a true partial batch stopped after one
returned boundary; an unknown effect after a crash is still reconciled only
through `recover_scoped`. A completion that was frozen by a crash (all files
committed, no completion record) is receipted by the next boundary, while a
rename that happened without its receipt fails closed as a stale source.

## Legacy compatibility and limits

`recover(bool) -> RecoveryReport` remains available, but is not a task-runtime
entry: it lacks caller policy/allowlist binding and retains legacy per-batch
reconciliation. Its former marker-name sweep has been replaced by exact
journal-owned, content-verified cleanup, and reported corrupt complete lines now
block both compensation and cleanup. Opening a journal never truncates an
existing torn tail. Legacy complete-prefix replay can ignore that tail, and a
legacy append repairs it; strict task recovery rejects and preserves it across
reopens.

This is process-coordinated, recoverable filesystem mutation, not a global
transaction or hostile-filesystem/multi-process isolation boundary. The caller
must exclude concurrent writers. The JSONL journal is trusted state, not an
authenticated or sequence-checksummed log: erasing a syntactically valid receipt
is indistinguishable from a crash before that receipt was written. No arbitrary
state forgery resistance, automatic orphan garbage collection or task-level
completion authority is claimed here.

One state directory holds exactly one batch identity for the authorized path: a
new attempt (or a second proposal) needs a fresh task/attempt directory, and
must not prepare over a sibling or torn record. The authorized path never
repairs a journal, sweeps a workspace temp or adopts an unknown batch; those
remain explicit `recover_scoped` decisions. Each authorized call re-reads every
source (verify, then stage) rather than trusting an earlier read, and the
existing `create_new` staging keeps a late foreign occupant out of the temp
path, so a caller that excludes concurrent writers can still fail closed on a
crafted one. The state/workspace identity check is canonical-path based, so
canonical aliases of the same roots agree while a symlinked workspace spelling
that resolves elsewhere is refused.

Tests: `tests/authorized.rs` (preparation refusals and the operation binding),
`tests/authorized_commit.rs` (per-file boundaries, partial-batch recovery and
denial), `tests/recovery_scoped.rs`, the late-staging collision unit regression
in `src/engine/scoped.rs`, and the retained M8 `tests/mutation_gate.rs` suite.
