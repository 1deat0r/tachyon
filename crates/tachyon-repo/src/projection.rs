//! In-memory text projection over a workspace corpus.
//!
//! Index-time writes ([`TextProjection::put`]) cache file text under its
//! BLAKE3 hash; queries ([`TextProjection::text`]) serve from memory when
//! the stored hash matches and count the bytes read on a miss, so warm
//! queries are measurable as reading zero corpus bytes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Projected file text keyed by workspace-relative path, plus the corpus
/// bytes read while serving query misses.
pub struct TextProjection {
    entries: Mutex<HashMap<String, (String, Arc<str>)>>,
    query_bytes_read: AtomicU64,
}

impl TextProjection {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            query_bytes_read: AtomicU64::new(0),
        }
    }

    /// Stores `text` for `rel` under `hash` at index time. Index-time
    /// writes never count toward [`TextProjection::query_bytes_read`].
    pub fn put(&self, rel: &str, hash: &str, text: String) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.insert(rel.to_owned(), (hash.to_owned(), Arc::from(text)));
    }

    /// Serves `rel` from memory only, when the stored hash equals `hash`.
    /// Never touches the disk, so callers can skip disk probes entirely
    /// for text that is already projected.
    #[must_use]
    pub fn get(&self, rel: &str, hash: &str) -> Option<Arc<str>> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries
            .get(rel)
            .filter(|(stored, _)| stored == hash)
            .map(|(_, text)| Arc::clone(text))
    }

    /// Returns the projected text for `rel` as of `hash`.
    ///
    /// A stored entry whose hash equals `hash` is served from memory. On a
    /// miss the file is read from disk and its byte length added to
    /// [`TextProjection::query_bytes_read`]: content hashing to `hash` is
    /// stored for later queries, content that drifted is returned without
    /// being stored (live reads under drift stay live), and read failures
    /// return `None`.
    #[must_use]
    pub fn text(&self, root: &Path, rel: &str, hash: &str) -> Option<Arc<str>> {
        {
            let entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((stored, text)) = entries.get(rel)
                && stored == hash
            {
                return Some(Arc::clone(text));
            }
        }
        let text = std::fs::read_to_string(root.join(rel)).ok()?;
        self.query_bytes_read
            .fetch_add(text.len() as u64, Ordering::SeqCst);
        let digest = blake3::hash(text.as_bytes()).to_hex();
        let projected: Arc<str> = Arc::from(text);
        if digest.as_str() == hash {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            entries.insert(rel.to_owned(), (hash.to_owned(), Arc::clone(&projected)));
        }
        Some(projected)
    }

    /// Corpus bytes read while serving [`TextProjection::text`] misses.
    #[must_use]
    pub fn query_bytes_read(&self) -> u64 {
        self.query_bytes_read.load(Ordering::SeqCst)
    }
}

impl Default for TextProjection {
    fn default() -> Self {
        Self::new()
    }
}
