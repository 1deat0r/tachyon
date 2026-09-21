//! Repository intelligence: inventory, symbols, search, freshness.
//!
//! Spec §30, Milestone 4. Per workspace: normalized file inventory with
//! BLAKE3 hashes, language-aware symbol/reference index, lexical search,
//! and watcher-driven incremental invalidation. Watchers are hints —
//! content hashes are authoritative truth.
//!
//! Parsing goes through [`LanguageBackend`]: the initial backend is a
//! deterministic heuristic extractor (no native deps, no model calls).
//! Tree-sitter grammars can implement the same trait later without
//! changing any caller.

pub mod index;
pub mod inventory;
pub mod language;
pub mod search;
pub mod watch;

pub use index::{DefinitionUse, Location, SymbolIndex};
pub use inventory::{FileRecord, Inventory, Language};
pub use language::{LanguageBackend, Symbol, SymbolKind};
pub use search::{SearchHit, SearchOptions, search as lexical_search};
pub use watch::Watcher;

/// 1-based line number from a 0-based index. Saturates instead of wrapping.
#[must_use]
pub(crate) fn line_no(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

/// Line count saturated to `u32`.
#[must_use]
pub(crate) fn line_count(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}
