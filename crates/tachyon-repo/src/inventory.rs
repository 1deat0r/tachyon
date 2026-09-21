//! Workspace file inventory: normalized paths, sizes, mtimes, BLAKE3.
//!
//! Hashes establish file identity; watcher events are only invalidation
//! hints. Directories that never carry evidence (`.git`, `target`,
//! `node_modules`, …) are pruned at walk time.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Source language by file extension.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    #[default]
    Unknown,
}

impl Language {
    #[must_use]
    pub fn of(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "rs" => Self::Rust,
            "py" | "pyi" => Self::Python,
            "ts" | "tsx" | "mts" | "cts" => Self::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            _ => Self::Unknown,
        }
    }
}

/// One inventoried file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// Workspace-relative path with `/` separators.
    pub rel: String,
    pub len: u64,
    /// mtime seconds (hint only — identity is the hash).
    pub mtime_secs: i64,
    /// BLAKE3 content hash (hex).
    pub hash: String,
    pub language: Language,
}

/// Inventory failures.
#[derive(Debug, Error)]
pub enum InventoryError {
    #[error("workspace root unreadable: {0}")]
    UnreadableRoot(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Directories pruned at walk time (never evidence carriers).
const PRUNE_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "dist",
    "build",
    ".next",
    ".idea",
    ".vscode",
];

/// Normalized workspace inventory.
#[derive(Clone, Debug, Default)]
pub struct Inventory {
    pub root: PathBuf,
    pub files: Vec<FileRecord>,
}

impl Inventory {
    /// Walks `root`, hashing text files. `limit_files` bounds runaway walks.
    pub fn scan(root: &Path, limit_files: usize) -> Result<Self, InventoryError> {
        if !root.is_dir() {
            return Err(InventoryError::UnreadableRoot(root.display().to_string()));
        }
        let mut files = Vec::new();
        let mut walker = walkdir::WalkDir::new(root).into_iter();
        while let Some(entry) = walker.next() {
            let entry = entry
                .map_err(|error| InventoryError::UnreadableRoot(format!("walk failed: {error}")))?;
            if entry.file_type().is_dir() {
                if entry.depth() > 0
                    && entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| PRUNE_DIRS.contains(&name))
                {
                    walker.skip_current_dir();
                }
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            if files.len() >= limit_files {
                break;
            }
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let metadata = std::fs::metadata(path)?;
            let hash = hash_file(path)?;
            files.push(FileRecord {
                rel,
                len: metadata.len(),
                mtime_secs: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |duration| duration.as_secs().cast_signed()),
                hash,
                language: Language::of(path),
            });
        }
        files.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(Self {
            root: root.to_path_buf(),
            files,
        })
    }

    #[must_use]
    pub fn get(&self, rel: &str) -> Option<&FileRecord> {
        self.files.iter().find(|file| file.rel == rel)
    }
}

/// Hashes a file, streaming so large files never load fully. Binary files
/// hash like any other bytes — callers decide what to index.
fn hash_file(path: &Path) -> Result<String, InventoryError> {
    let mut hasher = blake3::Hasher::new();
    let mut file = std::fs::File::open(path)?;
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// True when the first 8 KiB contain no null byte.
#[must_use]
pub fn is_probably_text(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0_u8; 8192];
    let Ok(read) = file.read(&mut head) else {
        return false;
    };
    !head[..read].contains(&0)
}
