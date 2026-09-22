//! Process-wide workspace exclusion shared by evidence, mutation and verification.
//!
//! A lease is coordination, NOT authorization: callers still perform exact policy
//! and containment checks before every read/effect. Canonical aliases share a key.
//! Stage owners may clone a lease into actual workers so cancellation of an outer
//! scheduler future cannot unlock the workspace while blocking work still runs.
//! Never acquire recursively; drop all clones before starting another leased stage.
//! This protects cooperating runs in this process, not hostile filesystem writers.

use crate::ToolError;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct HeldLease {
    root: PathBuf,
    _guard: OwnedMutexGuard<()>,
}

/// Exclusive workspace stage admission. Released only after the last owner drops.
#[derive(Clone, Debug)]
pub struct WorkspaceLease(Arc<HeldLease>);

impl WorkspaceLease {
    /// Waits without blocking the runtime; cancellation never grants authority.
    ///
    /// Move/clone this into the actual effect worker, not just its waiting wrapper.
    /// Cancellation/revision and policy must be rechecked at the effect barrier.
    pub async fn acquire(root: &Path, cancel: &CancellationToken) -> Result<Self, ToolError> {
        let root = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ToolError::WorkspaceLeaseCancelled),
            root = tokio::fs::canonicalize(root) => root?,
        };
        let lock = workspace_lock(&root)?;
        let guard = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ToolError::WorkspaceLeaseCancelled),
            guard = lock.lock_owned() => guard,
        };
        if cancel.is_cancelled() {
            return Err(ToolError::WorkspaceLeaseCancelled);
        }
        Ok(Self(Arc::new(HeldLease {
            root,
            _guard: guard,
        })))
    }

    /// The canonical root protected by this lease.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.0.root
    }
}

type Registry = Mutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>;

fn workspace_lock(root: &Path) -> Result<Arc<AsyncMutex<()>>, ToolError> {
    static LOCKS: OnceLock<Registry> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| ToolError::InvalidArgs("workspace lease registry poisoned".into()))?;
    // Owned guards and waiting futures keep their mutex alive. Remove only keys
    // with no such owners, avoiding an unbounded registry of historical roots.
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(root).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(AsyncMutex::new(()));
    locks.insert(root.to_path_buf(), Arc::downgrade(&lock));
    Ok(lock)
}
