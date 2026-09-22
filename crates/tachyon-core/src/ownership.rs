//! Process-local admission for the canonical database/task pair.
//!
//! Clients never own this guard. The actor (and each actual effect worker)
//! retains a clone; only the last drop permits another actor to recover.
use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tachyon_types::TaskId;
use tokio_util::sync::CancellationToken;

use crate::CoreError;

type Key = (PathBuf, TaskId);

fn owners() -> &'static Mutex<HashSet<Key>> {
    static OWNERS: OnceLock<Mutex<HashSet<Key>>> = OnceLock::new();
    OWNERS.get_or_init(|| Mutex::new(HashSet::new()))
}

#[derive(Clone, Debug)]
pub(crate) struct TaskOwnership(Arc<Lease>);

/// Observation/control only: a retained client cannot keep ownership alive.
#[derive(Clone, Debug, Default)]
pub(super) struct TaskLifecycle {
    pub(super) shutdown: CancellationToken,
    pub(super) released: CancellationToken,
}

#[derive(Debug)]
struct Lease {
    key: Key,
    lifecycle: TaskLifecycle,
}

impl TaskOwnership {
    pub(super) fn acquire(database: &Path, task: TaskId) -> Result<Self, CoreError> {
        let key = (database.to_path_buf(), task);
        let mut owners = owners()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !owners.insert(key.clone()) {
            return Err(CoreError::TaskAlreadyOwned(task));
        }
        Ok(Self(Arc::new(Lease {
            key,
            lifecycle: TaskLifecycle::default(),
        })))
    }

    pub(super) fn lifecycle(&self) -> TaskLifecycle {
        self.0.lifecycle.clone()
    }

    /// Opaque admission anchor for lower-level workers and blocking closures.
    pub(crate) fn lifetime(&self) -> Arc<dyn Send + Sync> {
        self.0.clone()
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        owners()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
        self.lifecycle.released.cancel();
    }
}

/// Effect workers are cooperatively cancelled, never aborted with their actor.
/// The drop fallback transfers their join handles to a drain task. That task and
/// each worker retain admission until the actual futures (including awaited
/// blocking work) finish; clients await the same `released` notification.
///
/// New effect workers must await their blocking children, not detach them. An
/// internally panicking worker that abandons such children needs to transfer a
/// guard into those children too. Runtime/process destruction is not an async
/// drain and is deliberately not advertised as awaited shutdown.
pub(super) struct OwnedWorkers<T: Send + 'static> {
    tasks: tokio::task::JoinSet<T>,
    ownership: TaskOwnership,
}

impl<T: Send + 'static> OwnedWorkers<T> {
    pub(super) fn new(ownership: TaskOwnership) -> Self {
        Self {
            tasks: tokio::task::JoinSet::new(),
            ownership,
        }
    }

    pub(super) fn spawn(&mut self, future: impl Future<Output = T> + Send + 'static) {
        let ownership = self.ownership.clone();
        self.tasks.spawn(async move {
            let _ownership = ownership;
            future.await
        });
    }

    pub(super) fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub(super) async fn join_next(&mut self) -> Option<Result<T, tokio::task::JoinError>> {
        self.tasks.join_next().await
    }
}

impl<T: Send + 'static> Drop for OwnedWorkers<T> {
    fn drop(&mut self) {
        if self.tasks.is_empty() {
            return;
        }
        self.ownership.lifecycle().shutdown.cancel();
        let ownership = self.ownership.clone();
        let mut tasks = std::mem::take(&mut self.tasks);
        tokio::spawn(async move {
            let _ownership = ownership;
            while tasks.join_next().await.is_some() {}
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn opaque_lifetime_anchor_survives_aborted_blocking_wrapper() {
        let database = std::env::temp_dir().join("tachyon-ownership-anchor.db");
        let task = TaskId::generate();
        let owner = TaskOwnership::acquire(&database, task).unwrap();
        let lifecycle = owner.lifecycle();
        let lifetime: Arc<dyn Send + Sync> = owner.lifetime();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, blocked) = tokio::sync::oneshot::channel();
        let wrapper = tokio::spawn(async move {
            tokio::task::spawn_blocking(move || {
                let _lifetime = lifetime;
                entered.send(()).unwrap();
                blocked.blocking_recv().unwrap();
            })
            .await
            .unwrap();
        });
        started.await.unwrap();
        drop(owner);
        wrapper.abort();
        assert!(wrapper.await.unwrap_err().is_cancelled());
        let duplicate = TaskOwnership::acquire(&database, task);
        let admitted_early = duplicate.is_ok();
        drop(duplicate);
        release.send(()).unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            lifecycle.released.cancelled(),
        )
        .await
        .unwrap();
        assert!(!admitted_early, "blocking effect outlived task admission");
        drop(TaskOwnership::acquire(&database, task).unwrap());
    }
}
