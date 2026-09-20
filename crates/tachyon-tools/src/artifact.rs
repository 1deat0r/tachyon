//! Content-addressed artifact store (spec §21).
//!
//! Layout: `$root/<first-two-hash-chars>/<blake3>`. Payloads above
//! [`COMPRESSION_THRESHOLD`] are zstd-compressed on disk (an optimization,
//! not an invariant — [`fetch`] detects the framing). Large stdout/model
//! blobs live here, never in event rows. Metadata in SQLite arrives with
//! the store milestone; the on-disk layout is already final.

use crate::ToolError;
use std::path::PathBuf;
use tachyon_types::ArtifactId;

/// Payloads at or above this size are compressed. Spec candidate: 64 KiB.
pub const COMPRESSION_THRESHOLD: usize = 64 * 1024;

/// Magic prefix marking a zstd-compressed artifact payload.
const COMPRESSED_MAGIC: &[u8] = b"TACHYON-ZSTD1\n";

/// Content-addressed spool rooted at `$data/artifacts`.
#[derive(Clone, Debug)]
pub struct ArtifactSpool {
    root: PathBuf,
}

impl ArtifactSpool {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path_for(&self, id: &ArtifactId) -> PathBuf {
        self.root.join(&id.0[..2.min(id.0.len())]).join(&id.0)
    }

    /// Stores `bytes`, returning its BLAKE3 id. Idempotent: identical
    /// bytes hash identically and short-circuit the write.
    pub fn store(&self, bytes: &[u8]) -> Result<ArtifactId, ToolError> {
        let id = ArtifactId(blake3::hash(bytes).to_hex().to_string());
        let path = self.path_for(&id);
        if path.exists() {
            return Ok(id);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let payload = if bytes.len() >= COMPRESSION_THRESHOLD {
            let mut framed = COMPRESSED_MAGIC.to_vec();
            framed.extend_from_slice(&zstd::encode_all(bytes, 3).map_err(|error| {
                ToolError::InvalidArgs(format!("zstd compress failed: {error}"))
            })?);
            framed
        } else {
            bytes.to_vec()
        };
        // Write-then-rename so a crash never leaves a partial artifact.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &payload)?;
        std::fs::rename(&tmp, &path)?;
        Ok(id)
    }

    /// Fetches artifact `id`, transparently decompressing.
    pub fn fetch(&self, id: &ArtifactId) -> Result<Vec<u8>, ToolError> {
        let path = self.path_for(id);
        let payload = std::fs::read(&path)
            .map_err(|_| ToolError::InvalidArgs(format!("unknown artifact: {}", id.0)))?;
        if let Some(rest) = payload.strip_prefix(COMPRESSED_MAGIC) {
            return zstd::decode_all(rest)
                .map_err(|error| ToolError::InvalidArgs(format!("zstd decode failed: {error}")));
        }
        Ok(payload)
    }
}
