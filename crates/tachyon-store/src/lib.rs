//! Tachyon Store.
//!
//! SQLite durability for the runtime: sessions, tasks, the append-only
//! event journal, and snapshots (spec §17–§18).
//!
//! All correctness-critical writes flow through [`StoreWriter`], the single
//! logical writer. It serializes writers with a mutex over a
//! single-connection pool and commits every operation immediately:
//! durability first, microbatching later (a Milestone 13 tuning item, not
//! a Milestone 1 behavior). Snapshots are opaque [`String`] documents owned
//! by `tachyon-core`; the store never interprets them.

#![warn(unsafe_code)]

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use tachyon_types::Timestamp;
use thiserror::Error;
use tokio::sync::Mutex;

/// Errors produced by the durability layer.
#[derive(Debug, Error)]
pub enum StoreError {
    /// SQLite failure.
    #[error("sqlite error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// Migration failure.
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// Stored data does not parse (ids, JSON shapes).
    #[error("corrupt stored data: {detail}")]
    Corrupt {
        /// What failed to parse.
        detail: String,
    },
    /// No task with this id exists.
    #[error("task not found: {task_id}")]
    TaskNotFound {
        /// Requested task id.
        task_id: String,
    },
}

/// One row of `tasks`, including the optional opaque snapshot.
#[derive(Clone, Debug, FromRow)]
pub struct TaskRow {
    /// Task id (hyphenated UUID).
    pub id: String,
    /// Owning session id.
    pub session_id: String,
    /// Workspace id.
    pub workspace_id: String,
    /// User's objective text.
    pub objective: String,
    /// Status name.
    pub status: String,
    /// State revision.
    pub revision: i64,
    /// Opaque snapshot document, if any.
    pub snapshot_json: Option<String>,
    /// Journal sequence the snapshot covers, if any.
    pub snapshot_seq: Option<i64>,
    /// Creation time (micros since epoch).
    pub created_at: i64,
    /// Last update time (micros since epoch).
    pub updated_at: i64,
}

/// One row of `task_events`.
#[derive(Clone, Debug, Serialize, Deserialize, FromRow)]
pub struct JournalEvent {
    /// Per-task sequence cursor.
    pub seq: i64,
    /// Event id (hyphenated UUID).
    pub event_id: String,
    /// Envelope schema version.
    pub schema_version: i64,
    /// Transition kind (`created`, `message`, `constraint`, `status`, …).
    pub kind: String,
    /// Transition payload (JSON).
    pub payload: String,
    /// Journal time (micros since epoch).
    pub created_at: i64,
}

/// Task list entry for clients.
#[derive(Clone, Debug, Serialize, Deserialize, FromRow)]
pub struct TaskSummary {
    /// Task id.
    pub id: String,
    /// Owning session id.
    pub session_id: String,
    /// User's objective text.
    pub objective: String,
    /// Status name.
    pub status: String,
    /// State revision.
    pub revision: i64,
    /// Last update time.
    pub updated_at: i64,
}

/// Materialized task metadata committed atomically with a journal event.
pub struct TransitionState<'a> {
    pub status: &'a str,
    pub revision: i64,
    pub snapshot_json: Option<&'a str>,
}

/// The single logical writer of correctness-critical state.
pub struct StoreWriter {
    pool: sqlx::SqlitePool,
    write: Mutex<()>,
}

impl StoreWriter {
    /// Opens (creating) `state.db` under `data_dir` and runs migrations.
    pub async fn open(data_dir: &Path) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(data_dir.join("state.db"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self {
            pool,
            write: Mutex::new(()),
        })
    }

    /// Inserts a session row.
    pub async fn create_session(&self, session_id: &str) -> Result<(), StoreError> {
        let _guard = self.write.lock().await;
        let now = Timestamp::now().as_micros();
        sqlx::query("INSERT INTO sessions (id, created_at) VALUES (?, ?)")
            .bind(session_id)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Inserts a task row plus its `created` journal event (seq 0) atomically.
    /// `snapshot_json` is the initial full-state document; `created_payload`
    /// is the journal payload for seq 0 (a `Created` transition document
    /// owned by `tachyon-core`, opaque here).
    #[allow(clippy::too_many_arguments)]
    pub async fn create_task(
        &self,
        task_id: &str,
        session_id: &str,
        workspace_id: &str,
        objective: &str,
        status: &str,
        snapshot_json: &str,
        created_payload: &str,
    ) -> Result<(), StoreError> {
        let _guard = self.write.lock().await;
        let now = Timestamp::now().as_micros();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO tasks (id, session_id, workspace_id, objective, status,
             revision, snapshot_json, snapshot_seq, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 0, ?, 0, ?, ?)",
        )
        .bind(task_id)
        .bind(session_id)
        .bind(workspace_id)
        .bind(objective)
        .bind(status)
        .bind(snapshot_json)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO task_events (task_id, seq, event_id, schema_version,
             kind, payload, created_at)
             VALUES (?, 0, ?, 1, 'created', ?, ?)",
        )
        .bind(task_id)
        .bind(tachyon_types::EventId::generate().to_string())
        .bind(created_payload)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Appends one journal event; returns its per-task sequence number.
    pub async fn append_event(
        &self,
        task_id: &str,
        kind: &str,
        payload: &str,
    ) -> Result<i64, StoreError> {
        self.append(task_id, kind, payload, None).await
    }

    /// Journal and projected status/revision/snapshot share one SQLite commit.
    /// A crash cannot leave a terminal task row without its acceptance event.
    pub async fn append_transition(
        &self,
        task_id: &str,
        kind: &str,
        payload: &str,
        state: TransitionState<'_>,
    ) -> Result<i64, StoreError> {
        self.append(task_id, kind, payload, Some(state)).await
    }

    async fn append(
        &self,
        task_id: &str,
        kind: &str,
        payload: &str,
        state: Option<TransitionState<'_>>,
    ) -> Result<i64, StoreError> {
        let _guard = self.write.lock().await;
        let mut tx = self.pool.begin().await?;
        let next: Option<i64> =
            sqlx::query_scalar("SELECT MAX(seq) + 1 FROM task_events WHERE task_id = ?")
                .bind(task_id)
                .fetch_one(&mut *tx)
                .await?;
        let seq = next.unwrap_or(0);
        let now = Timestamp::now().as_micros();
        sqlx::query(
            "INSERT INTO task_events (task_id, seq, event_id, schema_version,
             kind, payload, created_at)
             VALUES (?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(task_id)
        .bind(seq)
        .bind(tachyon_types::EventId::generate().to_string())
        .bind(kind)
        .bind(payload)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE tasks SET updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        if let Some(state) = state {
            sqlx::query(
                "UPDATE tasks SET status = ?, revision = ?,
                 snapshot_seq = CASE WHEN ? IS NULL THEN snapshot_seq ELSE ? END,
                 snapshot_json = COALESCE(?, snapshot_json) WHERE id = ?",
            )
            .bind(state.status)
            .bind(state.revision)
            .bind(state.snapshot_json)
            .bind(seq)
            .bind(state.snapshot_json)
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(seq)
    }

    /// Replaces the task snapshot and its metadata after a transition.
    pub async fn save_snapshot(
        &self,
        task_id: &str,
        snapshot_seq: i64,
        snapshot_json: &str,
        status: &str,
        revision: i64,
    ) -> Result<(), StoreError> {
        let _guard = self.write.lock().await;
        let now = Timestamp::now().as_micros();
        let rows = sqlx::query(
            "UPDATE tasks SET snapshot_json = ?, snapshot_seq = ?, status = ?,
             revision = ?, updated_at = ? WHERE id = ?",
        )
        .bind(snapshot_json)
        .bind(snapshot_seq)
        .bind(status)
        .bind(revision)
        .bind(now)
        .bind(task_id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if rows == 0 {
            return Err(StoreError::TaskNotFound {
                task_id: task_id.to_owned(),
            });
        }
        Ok(())
    }

    /// Closes the pool, waiting for checked-out connections to return.
    /// Call before deleting the data directory (mandatory on Windows,
    /// where open files cannot be removed).
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Loads a task row, or `None` when absent.
    pub async fn load_task(&self, task_id: &str) -> Result<Option<TaskRow>, StoreError> {
        sqlx::query_as::<_, TaskRow>("SELECT * FROM tasks WHERE id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(StoreError::from)
    }

    /// Loads journal events strictly after `after_seq`, in order.
    pub async fn load_events_since(
        &self,
        task_id: &str,
        after_seq: i64,
    ) -> Result<Vec<JournalEvent>, StoreError> {
        sqlx::query_as::<_, JournalEvent>(
            "SELECT seq, event_id, schema_version, kind, payload, created_at
             FROM task_events WHERE task_id = ? AND seq > ? ORDER BY seq",
        )
        .bind(task_id)
        .bind(after_seq)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::from)
    }

    /// Lists tasks, optionally restricted to one session, newest first.
    pub async fn list_tasks(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<TaskSummary>, StoreError> {
        if let Some(session) = session_id {
            sqlx::query_as::<_, TaskSummary>(
                "SELECT id, session_id, objective, status, revision, updated_at
                 FROM tasks WHERE session_id = ? ORDER BY updated_at DESC",
            )
            .bind(session)
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::from)
        } else {
            sqlx::query_as::<_, TaskSummary>(
                "SELECT id, session_id, objective, status, revision, updated_at
                 FROM tasks ORDER BY updated_at DESC",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::from)
        }
    }

    /// True when a session row exists.
    pub async fn session_exists(&self, session_id: &str) -> Result<bool, StoreError> {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_one(&self.pool)
            .await
            .map(|count| count > 0)
            .map_err(StoreError::from)
    }

    /// Ids of tasks that did not reach a terminal state.
    pub async fn incomplete_tasks(&self) -> Result<Vec<String>, StoreError> {
        sqlx::query_scalar::<_, String>(
            "SELECT id FROM tasks WHERE status NOT IN ('Completed', 'Failed', 'Cancelled')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::StoreWriter;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    async fn open_test_store() -> (StoreWriter, PathBuf) {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("tachyon-store-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = StoreWriter::open(&dir).await.unwrap();
        (store, dir)
    }

    #[tokio::test]
    async fn task_lifecycle_round_trips() {
        let (store, dir) = open_test_store().await;
        store.create_session("session-1").await.unwrap();
        store
            .create_task(
                "task-1",
                "session-1",
                "ws-1",
                "do a thing",
                "Created",
                "{}",
                "{}",
            )
            .await
            .unwrap();

        let seq = store
            .append_event("task-1", "message", "{\"text\":\"hi\"}")
            .await
            .unwrap();
        assert_eq!(seq, 1);
        store
            .save_snapshot("task-1", seq, "{\"rev\":1}", "Created", 1)
            .await
            .unwrap();

        let row = store.load_task("task-1").await.unwrap().unwrap();
        assert_eq!(row.status, "Created");
        assert_eq!(row.revision, 1);
        assert_eq!(row.snapshot_seq, Some(1));

        let tail = store.load_events_since("task-1", 0).await.unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].kind, "message");

        let all = store.load_events_since("task-1", -1).await.unwrap();
        assert_eq!(all.len(), 2);

        let tasks = store.list_tasks(None).await.unwrap();
        assert_eq!(tasks.len(), 1);
        let filtered = store.list_tasks(Some("session-1")).await.unwrap();
        assert_eq!(filtered.len(), 1);
        let empty = store.list_tasks(Some("nope")).await.unwrap();
        assert!(empty.is_empty());

        let incomplete = store.incomplete_tasks().await.unwrap();
        assert_eq!(incomplete, vec!["task-1".to_owned()]);

        assert!(store.load_task("missing").await.unwrap().is_none());
        let err = store
            .save_snapshot("missing", 0, "{}", "Created", 0)
            .await
            .unwrap_err();
        assert!(matches!(err, super::StoreError::TaskNotFound { .. }));

        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn cancelled_tasks_leave_the_incomplete_set() {
        let (store, dir) = open_test_store().await;
        store.create_session("s").await.unwrap();
        store
            .create_task("t", "s", "w", "obj", "Created", "{}", "{}")
            .await
            .unwrap();
        store
            .save_snapshot("t", 0, "{}", "Cancelled", 0)
            .await
            .unwrap();
        assert!(store.incomplete_tasks().await.unwrap().is_empty());
        store.close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn journal_and_completion_projection_commit_together() {
        let (store, dir) = open_test_store().await;
        store.create_session("s").await.unwrap();
        store
            .create_task("t", "s", "w", "obj", "Created", "{}", "{}")
            .await
            .unwrap();
        let seq = store
            .append_transition(
                "t",
                "verification_finished",
                "{}",
                super::TransitionState {
                    status: "Completed",
                    revision: 3,
                    snapshot_json: Some("{\"verified\":true}"),
                },
            )
            .await
            .unwrap();
        let row = store.load_task("t").await.unwrap().unwrap();
        assert_eq!(row.status, "Completed");
        assert_eq!(row.revision, 3);
        assert_eq!(row.snapshot_seq, Some(seq));
        assert_eq!(row.snapshot_json.as_deref(), Some("{\"verified\":true}"));
        assert_eq!(store.load_events_since("t", 0).await.unwrap().len(), 1);
        assert!(store.incomplete_tasks().await.unwrap().is_empty());
        store.close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn projection_failure_rolls_back_the_acceptance_event() {
        let (store, dir) = open_test_store().await;
        store.create_session("s").await.unwrap();
        store
            .create_task("t", "s", "w", "obj", "Created", "{}", "{}")
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER fault_projection BEFORE UPDATE OF status ON tasks BEGIN SELECT RAISE(ABORT, 'injected projection failure'); END")
            .execute(&store.pool).await.unwrap();
        assert!(
            store
                .append_transition(
                    "t",
                    "verification_finished",
                    "{}",
                    super::TransitionState {
                        status: "Completed",
                        revision: 1,
                        snapshot_json: Some("{\"verified\":true}"),
                    }
                )
                .await
                .is_err()
        );
        let row = store.load_task("t").await.unwrap().unwrap();
        assert_eq!(row.status, "Created");
        assert_eq!(row.revision, 0);
        assert_eq!(row.snapshot_seq, Some(0));
        assert!(store.load_events_since("t", 0).await.unwrap().is_empty());
        assert_eq!(store.incomplete_tasks().await.unwrap(), vec!["t"]);
        store.close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}
