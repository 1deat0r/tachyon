//! Symbol/reference index over an inventory (spec §30).
//!
//! Definitions are extracted per file through the [`LanguageBackend`];
//! references are word-boundary occurrences of the name in indexed text
//! files, excluding the definition site. Before acting on evidence,
//! callers re-check hashes via [`SymbolIndex::verify`].

use crate::inventory::{Inventory, is_probably_text};
use crate::language::{LanguageBackend, Symbol};
use crate::projection::TextProjection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A structured source location.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// Workspace-relative path (`/` separators).
    pub file: String,
    /// 1-based line.
    pub line: u32,
    /// 1-based end line (definitions only; references use `line`).
    pub end_line: u32,
    /// The full source line (definitions) or excerpt (references).
    pub excerpt: String,
}

/// A definition site plus its symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub symbol: Symbol,
    pub location: Location,
}

/// Slice-A answer: where a name is defined and where it is used.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionUse {
    pub name: String,
    pub definitions: Vec<Location>,
    pub references: Vec<Location>,
}

/// Per-file index state.
struct IndexedFile {
    hash: String,
    symbols: Vec<(Symbol, String)>,
}

/// Symbol index with freshness tracking.
pub struct SymbolIndex {
    root: std::path::PathBuf,
    backend: Box<dyn LanguageBackend>,
    files: HashMap<String, IndexedFile>,
    projection: TextProjection,
    /// Index generation: bumped by every build/refresh.
    pub generation: u64,
}

impl SymbolIndex {
    #[must_use]
    pub fn new(root: &std::path::Path, backend: impl LanguageBackend + 'static) -> Self {
        Self {
            root: root.to_path_buf(),
            backend: Box::new(backend),
            files: HashMap::new(),
            projection: TextProjection::new(),
            generation: 0,
        }
    }

    /// The text projection fed at index time: pass it to
    /// [`crate::search::search`] so warm queries serve corpus text from
    /// memory instead of re-reading files.
    #[must_use]
    pub fn projection(&self) -> &TextProjection {
        &self.projection
    }

    /// (Re)builds the index over `inventory`, extracting symbols from text
    /// files with a known language. Clears the text projection first so no
    /// prior-corpus text survives into the new generation.
    pub fn build(&mut self, inventory: &Inventory) {
        self.files.clear();
        self.projection.clear();
        for record in &inventory.files {
            self.index_record(record);
        }
        self.generation += 1;
    }

    /// Re-indexes `rels` (watcher invalidation path). Unknown rels are
    /// dropped from the index and from the projection together.
    pub fn refresh(&mut self, inventory: &Inventory, rels: &[&str]) {
        for rel in rels {
            if let Some(record) = inventory.get(rel) {
                self.index_record(record);
            } else {
                self.files.remove(*rel);
                self.projection.remove(rel);
            }
        }
        self.generation += 1;
    }

    fn index_record(&mut self, record: &crate::inventory::FileRecord) {
        if matches!(record.language, crate::inventory::Language::Unknown) {
            return;
        }
        let path = self.root.join(&record.rel);
        if !is_probably_text(&path) {
            return;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        self.projection.put(&record.rel, &record.hash, text.clone());
        let symbols = self.backend.extract(record.language, &text);
        let lines: Vec<&str> = text.lines().collect();
        let with_excerpts = symbols
            .into_iter()
            .map(|symbol| {
                let excerpt = lines
                    .get(symbol.start_line.saturating_sub(1) as usize)
                    .unwrap_or(&"")
                    .to_string();
                (symbol, excerpt)
            })
            .collect();
        self.files.insert(
            record.rel.clone(),
            IndexedFile {
                hash: record.hash.clone(),
                symbols: with_excerpts,
            },
        );
    }

    /// All definition sites for `name`.
    #[must_use]
    pub fn definitions(&self, name: &str) -> Vec<Definition> {
        let mut found = Vec::new();
        for (rel, indexed) in &self.files {
            for (symbol, excerpt) in &indexed.symbols {
                if symbol.name == name {
                    found.push(Definition {
                        symbol: symbol.clone(),
                        location: Location {
                            file: rel.clone(),
                            line: symbol.start_line,
                            end_line: symbol.end_line,
                            excerpt: excerpt.clone(),
                        },
                    });
                }
            }
        }
        found.sort_by(|a, b| {
            a.location
                .file
                .cmp(&b.location.file)
                .then(a.location.line.cmp(&b.location.line))
        });
        found
    }

    /// Word-boundary references to `name` across indexed files, excluding
    /// the definition lines themselves. Deterministic file/line order.
    /// File text comes from the indexed generation via the projection, so
    /// it stays consistent with the symbols filtered above; callers repair
    /// staleness against disk through [`Self::verify`] and [`Self::refresh`].
    #[must_use]
    pub fn references(&self, name: &str) -> Vec<Location> {
        let definition_lines: std::collections::HashSet<(&str, u32)> = self
            .files
            .iter()
            .flat_map(|(rel, indexed)| {
                indexed
                    .symbols
                    .iter()
                    .filter(|(symbol, _)| symbol.name == name)
                    .map(|(symbol, _)| (rel.as_str(), symbol.start_line))
            })
            .collect();
        let mut hits = Vec::new();
        let mut rels: Vec<&String> = self.files.keys().collect();
        rels.sort();
        for rel in rels {
            let Some(indexed) = self.files.get(rel.as_str()) else {
                continue;
            };
            let Some(text) = self.projection.get_or_read(&self.root, rel, &indexed.hash) else {
                continue;
            };
            for (index, line) in text.lines().enumerate() {
                let line_number = crate::line_no(index);
                if definition_lines.contains(&(rel.as_str(), line_number)) {
                    continue;
                }
                if !identifier_occurrences(line, name).is_empty() {
                    hits.push(Location {
                        file: (*rel).clone(),
                        line: line_number,
                        end_line: line_number,
                        excerpt: line.trim().to_owned(),
                    });
                }
            }
        }
        hits
    }

    /// Slice A: definitions + references for `name`. Zero LLM, zero Jev.
    #[must_use]
    pub fn definition_use(&self, name: &str) -> DefinitionUse {
        DefinitionUse {
            name: name.to_owned(),
            definitions: self
                .definitions(name)
                .into_iter()
                .map(|definition| definition.location)
                .collect(),
            references: self.references(name),
        }
    }

    /// Re-hashes every indexed file: returns rels whose content drifted
    /// from the indexed hash (stale index entries), and drops deleted files
    /// from the index and the projection together. Hashes are authoritative;
    /// the index is repaired by `refresh`.
    pub fn verify(&mut self, inventory: &Inventory) -> Vec<String> {
        let mut stale = Vec::new();
        let indexed: Vec<String> = self.files.keys().cloned().collect();
        for rel in indexed {
            match inventory.get(&rel) {
                None => {
                    self.files.remove(&rel);
                    self.projection.remove(&rel);
                    stale.push(rel);
                }
                Some(record) => {
                    let hash = &self.files.get(&rel).map(|file| file.hash.clone());
                    if hash.as_ref() != Some(&record.hash) {
                        self.files.remove(&rel);
                        self.projection.remove(&rel);
                        stale.push(rel);
                    }
                }
            }
        }
        stale.sort();
        stale
    }

    /// Reads a file relative to the index root, straight from disk.
    /// Unlike [`SymbolIndex::read_projected`]/[`SymbolIndex::references`]
    /// this is intentionally live: use it for explicit evidence reads,
    /// never as a query path.
    #[must_use]
    pub fn read_rel(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(rel)).ok()
    }

    /// Reads a file from the indexed generation via the projection
    /// (memory when the indexed hash still matches, one counted disk
    /// read otherwise). Query and evidence paths that must see the same
    /// generation as the symbols use this instead of [`SymbolIndex::read_rel`].
    #[must_use]
    pub fn read_projected(&self, rel: &str) -> Option<std::sync::Arc<str>> {
        let indexed = self.files.get(rel)?;
        self.projection.get_or_read(&self.root, rel, &indexed.hash)
    }

    /// True when `path` is inside the index root (for watch filtering).
    #[must_use]
    pub fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.root)
    }
}

/// Byte columns of whole-identifier occurrences of `name` in `line`.
fn identifier_occurrences(line: &str, name: &str) -> Vec<usize> {
    if name.is_empty() {
        return Vec::new();
    }
    let bytes = line.as_bytes();
    let needle = name.as_bytes();
    let mut columns = Vec::new();
    let mut start = 0;
    while start + needle.len() <= bytes.len() {
        match bytes[start..]
            .windows(needle.len())
            .position(|window| window == needle)
        {
            Some(offset) => {
                let at = start + offset;
                let before_ok = at == 0 || !is_ident_byte(bytes[at - 1]);
                let after = at + needle.len();
                let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
                if before_ok && after_ok {
                    columns.push(at);
                }
                start = at + needle.len().max(1);
            }
            None => break,
        }
    }
    columns
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
