//! Durable batch journal: append-only JSONL with per-record fsync.
//!
//! The journal is the source of truth across crashes — not memory, not the
//! filesystem. Every record is one JSON object per line, synced before the
//! call returns. On open, a torn tail (crash mid-write leaves a final line
//! without `\n`) is truncated away; any other corrupt line fails closed
//! with [`MutationError::JournalCorrupt`].

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tachyon_types::{MutationBatchId, Timestamp};

use crate::{FileMutation, FileState, MutationError};

/// Journal file name inside the state directory.
pub const JOURNAL_FILE: &str = "mutation.log";

/// One durable record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum JournalRecord {
    /// File plan persisted: pre/post hashes, preimage artifacts, temp names.
    BatchStarted {
        /// Batch identity.
        batch_id: MutationBatchId,
        /// Planned files in commit order.
        files: Vec<FileMutation>,
        /// When preparation finished.
        at: Timestamp,
    },
    /// One file renamed into place.
    FileCommitted {
        /// Batch identity.
        batch_id: MutationBatchId,
        /// File in journal-key form.
        path: String,
        /// When the rename was journaled.
        at: Timestamp,
    },
    /// Batch finished: every file committed.
    BatchCompleted {
        /// Batch identity.
        batch_id: MutationBatchId,
        /// When completion was journaled.
        at: Timestamp,
    },
    /// Batch aborted before completion (stale preimage or caller abort).
    /// Already-committed files stay committed; the rest is recovery's job.
    BatchAborted {
        /// Batch identity.
        batch_id: MutationBatchId,
        /// Why the batch stopped.
        reason: String,
        /// When the abort was journaled.
        at: Timestamp,
    },
    /// Preimage restored (or created file removed) by recovery.
    FileRolledBack {
        /// Batch identity.
        batch_id: MutationBatchId,
        /// File in journal-key form.
        path: String,
        /// When the rollback was journaled.
        at: Timestamp,
    },
}

/// In-memory rebuild of one batch's file states from the journal.
#[derive(Clone, Debug)]
pub struct ReplayedBatch {
    /// Batch identity.
    pub id: MutationBatchId,
    /// Files with journaled states applied.
    pub files: Vec<FileMutation>,
    /// Whether `BatchCompleted` was seen.
    pub completed: bool,
    /// Whether `BatchAborted` was seen.
    pub aborted: bool,
}

/// Append-only journal rooted at a state directory.
pub struct BatchJournal {
    path: PathBuf,
}

impl BatchJournal {
    /// Opens (creating) the journal, repairing a torn tail from a crashed
    /// write. Repair truncates the partial last line; anything else corrupt
    /// fails on replay, not silently.
    pub fn open(state_dir: &Path) -> Result<Self, MutationError> {
        std::fs::create_dir_all(state_dir)?;
        let path = state_dir.join(JOURNAL_FILE);
        if path.exists() {
            repair_torn_tail(&path)?;
        } else {
            std::fs::write(&path, "")?;
        }
        Ok(Self { path })
    }

    /// Appends one record and syncs before returning.
    pub fn append(&self, record: &JournalRecord) -> Result<(), MutationError> {
        let mut line = serde_json::to_string(record)
            .map_err(|error| MutationError::Io(format!("journal encode: {error}")))?;
        line.push('\n');
        let mut file = std::fs::OpenOptions::new().append(true).open(&self.path)?;
        file.write_all(line.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    /// Replays all records into per-batch states, in journal order.
    /// Fails closed on any corrupt line.
    pub fn replay(&self) -> Result<BTreeMap<MutationBatchId, ReplayedBatch>, MutationError> {
        let (batches, gaps) = self.replay_lenient()?;
        if let Some(first) = gaps.first() {
            return Err(MutationError::JournalCorrupt(*first));
        }
        Ok(batches)
    }

    /// Replays leniently: corrupt lines are skipped and their 1-based line
    /// numbers returned. Compensation runs on lenient truth, so one
    /// damaged record cannot deny rollback to sibling batches. Finishing
    /// and new commits require a clean journal (strict replay): a corrupt
    /// line fails those closed, with the gap numbers telling the operator
    /// exactly which lines to excise.
    pub fn replay_lenient(
        &self,
    ) -> Result<(BTreeMap<MutationBatchId, ReplayedBatch>, Vec<usize>), MutationError> {
        let content = std::fs::read_to_string(&self.path)?;
        let mut batches: BTreeMap<MutationBatchId, ReplayedBatch> = BTreeMap::new();
        let mut gaps = Vec::new();
        for (index, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(line) {
                Ok(record) => apply_record(&mut batches, record),
                Err(_) => gaps.push(index.saturating_add(1)),
            }
        }
        Ok((batches, gaps))
    }
}

/// Drops a trailing partial line (no terminating newline = torn write) and
/// truncates the file to the repair point.
fn repair_torn_tail(path: &Path) -> Result<(), MutationError> {
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() || bytes.ends_with(b"\n") {
        return Ok(());
    }
    let cut = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |pos| pos + 1);
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.set_len(cut as u64)?;
    file.sync_all()?;
    Ok(())
}

/// Folds one record into the replay map.
fn apply_record(batches: &mut BTreeMap<MutationBatchId, ReplayedBatch>, record: JournalRecord) {
    match record {
        JournalRecord::BatchStarted {
            batch_id, files, ..
        } => {
            batches.insert(
                batch_id,
                ReplayedBatch {
                    id: batch_id,
                    files,
                    completed: false,
                    aborted: false,
                },
            );
        }
        JournalRecord::FileCommitted { batch_id, path, .. } => {
            if let Some(batch) = batches.get_mut(&batch_id)
                && let Some(file) = batch.files.iter_mut().find(|file| file.path == path)
            {
                file.state = FileState::Committed;
            }
        }
        JournalRecord::BatchCompleted { batch_id, .. } => {
            if let Some(batch) = batches.get_mut(&batch_id) {
                batch.completed = true;
            }
        }
        JournalRecord::BatchAborted { batch_id, .. } => {
            if let Some(batch) = batches.get_mut(&batch_id) {
                batch.aborted = true;
            }
        }
        JournalRecord::FileRolledBack { batch_id, path, .. } => {
            if let Some(batch) = batches.get_mut(&batch_id)
                && let Some(file) = batch.files.iter_mut().find(|file| file.path == path)
            {
                file.state = FileState::RolledBack;
            }
        }
    }
}

/// Test helper: the preimage hash recorded for `path`, flattened.
#[cfg(test)]
pub(crate) fn recorded_pre(
    batches: &BTreeMap<MutationBatchId, ReplayedBatch>,
    id: &MutationBatchId,
    path: &str,
) -> Option<String> {
    batches
        .get(id)
        .and_then(|batch| batch.files.iter().find(|file| file.path == path))
        .and_then(|file| file.pre_hash.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(id: MutationBatchId) -> JournalRecord {
        JournalRecord::BatchStarted {
            batch_id: id,
            files: vec![],
            at: Timestamp::from_micros(0),
        }
    }

    #[test]
    fn torn_tail_repairs_and_replays() {
        let dir = std::env::temp_dir().join(format!("tachyon-mj-{}", Timestamp::now().as_micros()));
        let id = MutationBatchId::generate();
        {
            let journal = BatchJournal::open(&dir).expect("open");
            journal.append(&started(id)).expect("append");
            // Simulate a torn write: partial line, no newline, no sync.
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(dir.join(JOURNAL_FILE))
                .expect("raw");
            file.write_all(b"{\"record\": \"batch_complet")
                .expect("tear");
            file.sync_all().expect("sync");
        }
        let journal = BatchJournal::open(&dir).expect("reopen repairs");
        let batches = journal.replay().expect("replay");
        assert!(batches.contains_key(&id));
        assert!(!batches[&id].completed);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_body_line_fails_closed() {
        let dir =
            std::env::temp_dir().join(format!("tachyon-mj2-{}", Timestamp::now().as_micros()));
        let journal = BatchJournal::open(&dir).expect("open");
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(JOURNAL_FILE))
            .expect("raw");
        writeln!(file, "{{\"record\": \"bogus\"}}").expect("write");
        file.sync_all().expect("sync");
        let error = journal.replay().expect_err("corrupt line");
        assert!(matches!(error, MutationError::JournalCorrupt(1)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn records_fold_in_order() {
        let dir =
            std::env::temp_dir().join(format!("tachyon-mj3-{}", Timestamp::now().as_micros()));
        let journal = BatchJournal::open(&dir).expect("open");
        let id = MutationBatchId::generate();
        journal.append(&started(id)).expect("append");
        journal
            .append(&JournalRecord::BatchCompleted {
                batch_id: id,
                at: Timestamp::from_micros(1),
            })
            .expect("append");
        let batches = journal.replay().expect("replay");
        assert!(batches[&id].completed);
        assert!(recorded_pre(&batches, &id, "missing").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
