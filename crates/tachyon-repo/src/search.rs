//! Lexical search abstraction over indexed text files.
//!
//! Deterministic substring search with structured hits. Semantic search is
//! a later milestone; unsupported languages fall back to exactly this.

use crate::inventory::{Inventory, is_probably_text};
use crate::projection::TextProjection;
use serde::{Deserialize, Serialize};

/// One search hit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub excerpt: String,
}

/// Search controls.
#[derive(Clone, Debug)]
pub struct SearchOptions {
    /// Case-sensitive matching.
    pub case_sensitive: bool,
    /// Only files with these extensions (e.g. `["rs"]`); empty = all.
    pub extensions: Vec<String>,
    /// Maximum hits returned.
    pub limit: usize,
    /// Excerpt context lines around the hit.
    pub context: u32,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: true,
            extensions: Vec::new(),
            limit: 100,
            context: 0,
        }
    }
}

/// Searches indexed text files for `pattern`. File order then line order —
/// fully deterministic. Returns at most `options.limit` hits. File text is
/// served through `projection`, so warm queries read no corpus bytes;
/// files whose text cannot be read are skipped.
#[must_use]
pub fn search(
    root: &std::path::Path,
    inventory: &Inventory,
    projection: &TextProjection,
    pattern: &str,
    options: &SearchOptions,
) -> Vec<SearchHit> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let needle = if options.case_sensitive {
        pattern.to_owned()
    } else {
        pattern.to_lowercase()
    };
    let mut hits = Vec::new();
    let mut records: Vec<&crate::inventory::FileRecord> = inventory.files.iter().collect();
    records.sort_by(|a, b| a.rel.cmp(&b.rel));
    for record in records {
        if hits.len() >= options.limit {
            break;
        }
        if !options.extensions.is_empty() {
            let extension = std::path::Path::new(&record.rel)
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("");
            if !options
                .extensions
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case(extension))
            {
                continue;
            }
        }
        // Projection first: text already served from memory means the file
        // was text at index time, so the on-disk probe is skipped too and
        // warm queries perform no I/O at all. Only a projection miss pays
        // the 8 KiB text probe and the counted read.
        let path = root.join(&record.rel);
        let projected = projection
            .get_cached(&record.rel, &record.hash)
            .or_else(|| {
                if !is_probably_text(&path) {
                    return None;
                }
                projection.get_or_read(root, &record.rel, &record.hash)
            });
        let Some(text) = projected else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if hits.len() >= options.limit {
                break;
            }
            let haystack = if options.case_sensitive {
                (*line).to_owned()
            } else {
                line.to_lowercase()
            };
            let Some(column) = haystack.find(needle.as_str()) else {
                continue;
            };
            hits.push(SearchHit {
                file: record.rel.clone(),
                line: crate::line_no(index),
                column: crate::line_no(column),
                excerpt: excerpt_with_context(&lines, index, options.context),
            });
        }
    }
    hits
}

fn excerpt_with_context(lines: &[&str], index: usize, context: u32) -> String {
    let start = index.saturating_sub(context as usize);
    let end = (index + context as usize + 1).min(lines.len());
    lines[start..end].join("\n")
}
