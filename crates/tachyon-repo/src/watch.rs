//! Filesystem watcher: incremental invalidation hints (spec §30).
//!
//! Events mark index entries stale; content hashes decide truth. The
//! watcher never mutates the index itself — callers drain [`Watcher`]
//! events and run [`SymbolIndex::refresh`](crate::SymbolIndex::refresh).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use thiserror::Error;

/// Watcher failures.
#[derive(Debug, Error)]
pub enum WatchError {
    #[error("notify backend: {0}")]
    Backend(String),
    #[error("watch target missing: {0}")]
    MissingTarget(String),
}

use notify::Watcher as _;

/// Deduplicated change notifications as workspace-relative paths.
#[derive(Debug)]
pub struct Watcher {
    root: PathBuf,
    receiver: Option<Receiver<Vec<PathBuf>>>,
    _backend: Option<notify::RecommendedWatcher>,
}

impl Watcher {
    /// Starts watching `root` recursively. Events are debounced 100 ms and
    /// delivered as batches of absolute paths.
    pub fn watch(root: &Path) -> Result<Self, WatchError> {
        // Canonicalize: event paths arrive resolved (macOS TMPDIR is a
        // symlink), so prefix-stripping must use the resolved root.
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        if !root.is_dir() {
            return Err(WatchError::MissingTarget(root.display().to_string()));
        }
        let (batch_tx, batch_rx) = mpsc::channel::<Vec<PathBuf>>();
        let (event_tx, event_rx) = mpsc::channel::<PathBuf>();
        let mut watcher = notify::RecommendedWatcher::new(
            move |result: Result<notify::Event, notify::Error>| {
                if let Ok(event) = result {
                    for path in event.paths {
                        let _ = event_tx.send(path);
                    }
                }
            },
            notify::Config::default(),
        )
        .map_err(|error| WatchError::Backend(error.to_string()))?;
        watcher
            .watch(&root, notify::RecursiveMode::Recursive)
            .map_err(|error| WatchError::Backend(error.to_string()))?;
        // Debounce thread: batches events separated by quiet windows.
        std::thread::spawn(move || {
            let mut pending: Vec<PathBuf> = Vec::new();
            loop {
                match event_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(path) => {
                        if !pending.contains(&path) {
                            pending.push(path);
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if !pending.is_empty() {
                            let batch = std::mem::take(&mut pending);
                            if batch_tx.send(batch).is_err() {
                                return;
                            }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        Ok(Self {
            root: root.to_path_buf(),
            receiver: Some(batch_rx),
            _backend: Some(watcher),
        })
    }

    /// Drains pending batches as workspace-relative path strings.
    /// Non-blocking: returns what has arrived so far.
    #[must_use]
    pub fn pending(&self) -> Vec<String> {
        let mut rels = Vec::new();
        if let Some(receiver) = &self.receiver {
            while let Ok(batch) = receiver.try_recv() {
                for path in batch {
                    if let Ok(rel) = path.strip_prefix(&self.root) {
                        rels.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
        rels.sort();
        rels.dedup();
        rels
    }
}
