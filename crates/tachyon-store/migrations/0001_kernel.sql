-- Tachyon kernel schema, Milestone 1 (spec docs/02 §17-§18).
--
-- state.db holds correctness-critical runtime data. Index databases
-- (Milestone 4) live separately and are rebuildable; they are not here.

-- Persistent user interaction contexts.
CREATE TABLE sessions (
    id          TEXT    NOT NULL PRIMARY KEY,
    created_at  INTEGER NOT NULL
) STRICT;

-- Executable work. snapshot_json is an opaque TaskState document owned by
-- tachyon-core; the store never interprets it.
CREATE TABLE tasks (
    id              TEXT    NOT NULL PRIMARY KEY,
    session_id      TEXT    NOT NULL REFERENCES sessions (id),
    workspace_id    TEXT    NOT NULL,
    objective       TEXT    NOT NULL,
    status          TEXT    NOT NULL,
    revision        INTEGER NOT NULL,
    snapshot_json   TEXT,
    snapshot_seq    INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
) STRICT;

CREATE INDEX tasks_by_session ON tasks (session_id);

-- Append-only journal of state transitions. (task_id, seq) is the
-- reconnect/replay cursor; rows are never updated or deleted.
CREATE TABLE task_events (
    id              INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    task_id         TEXT    NOT NULL REFERENCES tasks (id),
    seq             INTEGER NOT NULL,
    event_id        TEXT    NOT NULL,
    schema_version  INTEGER NOT NULL,
    kind            TEXT    NOT NULL,
    payload         TEXT    NOT NULL,
    created_at      INTEGER NOT NULL,
    UNIQUE (task_id, seq)
) STRICT;

CREATE INDEX task_events_by_task ON task_events (task_id, seq);

-- Scheduler state (Milestone 2+). Tables exist so later milestones migrate
-- forward instead of re-baselining; Milestone 1 writes no rows here.
CREATE TABLE nodes (
    id          TEXT    NOT NULL PRIMARY KEY,
    task_id     TEXT    NOT NULL REFERENCES tasks (id),
    status      TEXT    NOT NULL,
    node_json   TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

CREATE TABLE node_dependencies (
    task_id     TEXT    NOT NULL REFERENCES tasks (id),
    from_id     TEXT    NOT NULL,
    to_id       TEXT    NOT NULL,
    PRIMARY KEY (task_id, from_id, to_id)
) STRICT;

-- Effect receipts for crash reconciliation (Milestone 8+).
CREATE TABLE effects (
    id              TEXT    NOT NULL PRIMARY KEY,
    task_id         TEXT    NOT NULL REFERENCES tasks (id),
    effect_class    TEXT    NOT NULL,
    idempotency     TEXT    NOT NULL,
    state           TEXT    NOT NULL,
    receipt         TEXT,
    updated_at      INTEGER NOT NULL
) STRICT;

-- Policy approvals bound to operation hashes (Milestone 3+).
CREATE TABLE approvals (
    id              TEXT    NOT NULL PRIMARY KEY,
    task_id         TEXT    NOT NULL REFERENCES tasks (id),
    operation_hash  TEXT    NOT NULL,
    decision        TEXT    NOT NULL,
    decided_at      INTEGER NOT NULL
) STRICT;

-- Content-addressed artifact metadata (Milestone 3+).
CREATE TABLE artifacts (
    id          TEXT    NOT NULL PRIMARY KEY,
    size_bytes  INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
) STRICT;

-- Aggregate routing/provider measurements (Milestone 5+).
CREATE TABLE provider_stats (
    provider_id TEXT    NOT NULL PRIMARY KEY,
    stats_json  TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;
