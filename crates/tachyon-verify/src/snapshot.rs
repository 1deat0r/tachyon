//! Complete local source coverage, excluding only `.git` and `target` directories.
use crate::{VerifyError, contract::validate_path};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSnapshot {
    root: PathBuf,
    pub(crate) directories: BTreeSet<String>,
    pub(crate) files: BTreeMap<String, FileFingerprint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileFingerprint {
    hash: String,
    mode: u32,
}

impl WorkspaceSnapshot {
    /// Synchronous full scan. Async callers must use a blocking pool.
    pub fn capture(root: &Path) -> Result<Self, VerifyError> {
        Self::capture_with(root, None)
    }

    /// Enforce exact path policy before every metadata, directory or file read.
    /// A workspace-wide grant cannot hide a specific denied source file.
    pub fn capture_authorized(context: &tachyon_tools::ToolsContext) -> Result<Self, VerifyError> {
        Self::capture_with(&context.workspace_root, Some(context))
    }

    fn capture_with(
        root: &Path,
        context: Option<&tachyon_tools::ToolsContext>,
    ) -> Result<Self, VerifyError> {
        let root = root.canonicalize()?;
        // The canonicalization above stats the root; authorize that metadata
        // read before trusting `is_dir`, like every other filesystem read.
        authorize_path(context, &root, &root, "fs.metadata")?;
        if !root.is_dir() {
            return Err(VerifyError::Blocked(
                "snapshot root is not a directory".into(),
            ));
        }
        let mut files = BTreeMap::new();
        let mut directories = BTreeSet::new();
        let mut dirs = vec![root.clone()];
        while let Some(dir) = dirs.pop() {
            authorize_path(context, &root, &dir, "fs.list")?;
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                authorize_path(context, &root, &path, "fs.metadata")?;
                let metadata = fs::symlink_metadata(&path)?;
                let relative = path
                    .strip_prefix(&root)
                    .map_err(|_| VerifyError::Blocked("source escaped root".into()))?;
                let name = relative
                    .to_str()
                    .ok_or_else(|| VerifyError::Blocked("non-UTF8 source path".into()))?
                    .replace(std::path::MAIN_SEPARATOR, "/");
                validate_path(&name, false)?;
                if metadata.file_type().is_symlink() {
                    return Err(VerifyError::Blocked(format!(
                        "symlink is not covered: {name}"
                    )));
                }
                if metadata.is_dir() {
                    if !matches!(entry.file_name().to_str(), Some(".git" | "target")) {
                        directories.insert(name);
                        dirs.push(path);
                    }
                } else if metadata.is_file() {
                    authorize_path(context, &root, &path, "fs.read")?;
                    files.insert(name, fingerprint(&path)?);
                } else {
                    return Err(VerifyError::Blocked(format!("nonregular source: {name}")));
                }
            }
        }
        Ok(Self {
            root,
            directories,
            files,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns a stable, sorted union. A foreign root is never equal.
    #[must_use]
    pub fn changed_paths(&self, other: &Self) -> Vec<String> {
        self.files
            .keys()
            .chain(other.files.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|path| {
                self.root != other.root || self.files.get(*path) != other.files.get(*path)
            })
            .cloned()
            .collect()
    }

    #[must_use]
    pub fn same_sources(&self, other: &Self) -> bool {
        self == other
    }
}

fn authorize_path(
    context: Option<&tachyon_tools::ToolsContext>,
    root: &Path,
    path: &Path,
    capability: &str,
) -> Result<(), VerifyError> {
    if let Some(context) = context {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| VerifyError::Blocked("snapshot scope escaped".into()))?;
        let path = relative
            .to_str()
            .ok_or_else(|| VerifyError::Blocked("non-UTF8 snapshot path".into()))?
            .replace(std::path::MAIN_SEPARATOR, "/");
        let scope = format!("workspace/{path}");
        tachyon_tools::authorize(
            &context.policy,
            &context.approvals,
            capability,
            &scope,
            &serde_json::json!({"op": capability, "path": path, "root": root}),
            "capture verification source evidence",
        )
        .map_err(|error| VerifyError::Blocked(error.to_string()))?;
    }
    Ok(())
}

fn fingerprint(path: &Path) -> Result<FileFingerprint, VerifyError> {
    let mut file = fs::File::open(path)?;
    let before = file.metadata()?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0; 65_536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let after = fs::symlink_metadata(path)?;
    if !after.is_file() || before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(VerifyError::Blocked(
            "source changed during snapshot".into(),
        ));
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        after.permissions().mode()
    };
    #[cfg(not(unix))]
    let mode = u32::from(after.permissions().readonly());
    Ok(FileFingerprint {
        hash: hasher.finalize().to_hex().to_string(),
        mode,
    })
}
