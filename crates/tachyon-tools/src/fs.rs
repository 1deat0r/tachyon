//! Filesystem tools: read, list, metadata, write (spec §34, M3).
//!
//! All paths resolve through [`crate::resolve_scope`]: traversal is a hard
//! error, inside-workspace maps to `workspace/<rel>`, absolute-outside maps
//! to `external:<abs>` and therefore needs an explicit grant or approval.

use crate::{ToolError, ToolsContext, authorize, resolve_scope};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// File read cap: 64 MiB (larger blobs belong in the artifact store).
const READ_CAP: u64 = 64 * 1024 * 1024;

/// File metadata snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMetadata {
    pub path: String,
    pub len: u64,
    pub is_dir: bool,
    pub is_file: bool,
    pub readonly: bool,
}

/// One directory entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub len: u64,
}

/// Reads a file (policy `fs.read`). Caps at 64 MiB.
pub fn read(context: &ToolsContext, path: &Path) -> Result<Vec<u8>, ToolError> {
    let (resolved, scope) = resolve_scope(&context.workspace_root, path)?;
    let operation = serde_json::json!({"op": "fs.read", "scope": scope});
    authorize(
        &context.policy,
        &context.approvals,
        "fs.read",
        &scope,
        &operation,
        &format!("read {}", resolved.display()),
    )?;
    let metadata = std::fs::metadata(&resolved)?;
    if metadata.len() > READ_CAP {
        return Err(ToolError::InvalidArgs(format!(
            "file exceeds 64 MiB read cap: {}",
            resolved.display()
        )));
    }
    Ok(std::fs::read(&resolved)?)
}

/// Lists a directory (policy `fs.list`).
pub fn list(context: &ToolsContext, path: &Path) -> Result<Vec<DirEntry>, ToolError> {
    let (resolved, scope) = resolve_scope(&context.workspace_root, path)?;
    let operation = serde_json::json!({"op": "fs.list", "scope": scope});
    authorize(
        &context.policy,
        &context.approvals,
        "fs.list",
        &scope,
        &operation,
        &format!("list {}", resolved.display()),
    )?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&resolved)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let len = if file_type.is_file() {
            entry.metadata().map_or(0, |metadata| metadata.len())
        } else {
            0
        };
        entries.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: file_type.is_dir(),
            len,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Stats a path (policy `fs.metadata`).
pub fn metadata(context: &ToolsContext, path: &Path) -> Result<FileMetadata, ToolError> {
    let (resolved, scope) = resolve_scope(&context.workspace_root, path)?;
    let operation = serde_json::json!({"op": "fs.metadata", "scope": scope});
    authorize(
        &context.policy,
        &context.approvals,
        "fs.metadata",
        &scope,
        &operation,
        &format!("stat {}", resolved.display()),
    )?;
    let stat = std::fs::metadata(&resolved)?;
    Ok(FileMetadata {
        path: resolved.to_string_lossy().into_owned(),
        len: stat.len(),
        is_dir: stat.is_dir(),
        is_file: stat.is_file(),
        readonly: stat.permissions().readonly(),
    })
}

/// Writes a file, creating parent directories (policy `fs.write`).
pub fn write(context: &ToolsContext, path: &Path, bytes: &[u8]) -> Result<u64, ToolError> {
    let (resolved, scope) = resolve_scope(&context.workspace_root, path)?;
    let operation = serde_json::json!({
        "op": "fs.write",
        "scope": scope,
        "bytes": bytes.len(),
        "content_hash": blake3::hash(bytes).to_hex().to_string(),
    });
    authorize(
        &context.policy,
        &context.approvals,
        "fs.write",
        &scope,
        &operation,
        &format!("write {} ({} bytes)", resolved.display(), bytes.len()),
    )?;
    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&resolved, bytes)?;
    Ok(bytes.len() as u64)
}
