//! Evidence structures and deterministic merge/rank (spec §31, M6).
//!
//! [`EvidenceItem`] is the unit of retrieved truth: content plus the
//! [`Provenance`] needed to trace it back to a source file, search result, or
//! diagnostic. [`EvidencePackage`] groups findings about one question so the
//! model layer can assemble bounded reasoning context from it.
//!
//! Merge and ranking are pure and deterministic: same inputs always produce
//! the same package, duplicates collapse to their first occurrence (so
//! provenance survives), and ordering is by relevance with a content
//! tie-break — never by hash-map iteration order.

#![warn(unsafe_code)]

use serde::{Deserialize, Serialize};
use tachyon_types::{CapabilityId, Timestamp};
use uuid::Uuid;

/// What kind of retrieved material an [`EvidenceItem`] carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// A symbol definition site (`repo.symbol.search`).
    SymbolDefinition,
    /// A symbol use/reference site (`repo.symbol.references`).
    SymbolReference,
    /// A lexical search hit (`repo.lexical.search`).
    LexicalHit,
    /// A workspace file excerpt (`fs.read`).
    FileExcerpt,
    /// Read-only version-control facts (`git.status`/`diff`/`log`).
    GitInfo,
    /// Compiler, test, or linter output.
    Diagnostic,
    /// A human- or tool-supplied note.
    Note,
}

/// Where an [`EvidenceItem`] came from. Survives merge and ranking so model
/// output stays traceable to source files and results (spec §31).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Provenance {
    /// Capability or adapter that produced the item (`repo.symbol.search`).
    pub source: String,
    /// Workspace-relative path, when the item is file-backed.
    pub path: Option<String>,
    /// BLAKE3 content hash of the source at retrieval time, when known.
    /// Stale hashes must not authorize mutation (frozen invariant 9); the
    /// mutation engine re-checks them before writing (M8).
    pub hash: Option<String>,
    /// Index or workspace generation, when known.
    pub generation: Option<u64>,
}

impl Provenance {
    /// Builds file-backed provenance for repository evidence.
    #[must_use]
    pub fn repo(source: &str, path: &str) -> Self {
        Self {
            source: source.to_owned(),
            path: Some(path.to_owned()),
            hash: None,
            generation: None,
        }
    }

    /// Attaches the source content hash observed at retrieval time.
    #[must_use]
    pub fn with_hash(mut self, hash: &str) -> Self {
        self.hash = Some(hash.to_owned());
        self
    }
}

/// One retrieved fact with its provenance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceItem {
    /// Stable identity for dedupe-safe transport (not an ordering key).
    pub id: Uuid,
    /// What kind of material this is.
    pub kind: EvidenceKind,
    /// The retrieved text itself.
    pub content: String,
    /// Trace-back to the source; never dropped by merge or rank.
    pub provenance: Provenance,
    /// Source version string (commit, index generation label), if known.
    pub source_version: Option<String>,
    /// Higher ranks first; `None` sorts after every scored item.
    pub relevance: Option<f32>,
    /// When the item was retrieved.
    pub created_at: Timestamp,
}

impl EvidenceItem {
    /// Creates an item with a fresh id and current timestamp.
    #[must_use]
    pub fn new(kind: EvidenceKind, content: &str, provenance: Provenance) -> Self {
        Self {
            id: Uuid::now_v7(),
            kind,
            content: content.to_owned(),
            provenance,
            source_version: None,
            relevance: None,
            created_at: Timestamp::now(),
        }
    }

    /// Sets the relevance score used by ranking.
    #[must_use]
    pub fn with_relevance(mut self, relevance: f32) -> Self {
        self.relevance = Some(relevance);
        self
    }
}

/// A known hole in the evidence: what is missing and how to fill it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceGap {
    /// What is missing, in plain language.
    pub description: String,
    /// Native capability that could fill the gap, if one is known.
    pub suggested_capability: Option<CapabilityId>,
}

impl EvidenceGap {
    /// Describes a gap with no known filler.
    #[must_use]
    pub fn new(description: &str) -> Self {
        Self {
            description: description.to_owned(),
            suggested_capability: None,
        }
    }
}

/// All evidence assembled for one question (spec §31).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EvidencePackage {
    /// The question the evidence answers.
    pub question: String,
    /// Supporting items, best first after [`EvidencePackage::ranked`].
    pub findings: Vec<EvidenceItem>,
    /// Items that contradict each other or the findings.
    pub contradictions: Vec<EvidenceItem>,
    /// Known holes a reasoning call should weigh.
    pub gaps: Vec<EvidenceGap>,
}

impl EvidencePackage {
    /// Creates an empty package for `question`.
    #[must_use]
    pub fn new(question: &str) -> Self {
        Self {
            question: question.to_owned(),
            findings: Vec::new(),
            contradictions: Vec::new(),
            gaps: Vec::new(),
        }
    }

    /// Merges packages about the same question into one.
    ///
    /// Deterministic: packages fold in order; duplicate findings (same kind,
    /// content, and source path) collapse to their first occurrence, keeping
    /// the highest relevance seen; gaps dedupe by description. Provenance
    /// always comes from the surviving first occurrence. Identical content
    /// from a different source path is a separate location, not a duplicate,
    /// and survives the merge.
    #[must_use]
    pub fn merge(question: &str, packages: Vec<EvidencePackage>) -> Self {
        let mut merged = Self::new(question);
        for package in packages {
            for item in package.findings {
                merged.push_dedup(item, false);
            }
            for item in package.contradictions {
                merged.push_dedup(item, true);
            }
            for gap in package.gaps {
                if !merged
                    .gaps
                    .iter()
                    .any(|known| known.description == gap.description)
                {
                    merged.gaps.push(gap);
                }
            }
        }
        merged.ranked()
    }

    /// Sorts findings and contradictions by relevance (highest first,
    /// unscored last) with a content tie-break. Stable and deterministic.
    #[must_use]
    pub fn ranked(mut self) -> Self {
        rank_items(&mut self.findings);
        rank_items(&mut self.contradictions);
        self
    }

    /// Total item count across findings and contradictions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.findings.len() + self.contradictions.len()
    }

    /// Whether the package holds no items and no gaps.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.findings.is_empty() && self.contradictions.is_empty() && self.gaps.is_empty()
    }

    fn push_dedup(&mut self, item: EvidenceItem, contradiction: bool) {
        let bucket = if contradiction {
            &mut self.contradictions
        } else {
            &mut self.findings
        };
        if let Some(known) = bucket.iter_mut().find(|known| {
            known.kind == item.kind
                && known.content == item.content
                && known.provenance.source == item.provenance.source
                && known.provenance.path == item.provenance.path
        }) {
            // Same material seen again: keep the first provenance, keep the
            // strongest relevance signal.
            if bump_relevance(known.relevance, item.relevance) {
                known.relevance = item.relevance;
            }
        } else {
            bucket.push(item);
        }
    }
}

/// Sorts items in place: relevance descending (`None` and `NaN` last), then
/// content, kind, and provenance — fully deterministic for any input order,
/// including separately reconstructed logical duplicates. The fresh `id` is
/// only a final tie-break within one run.
fn rank_items(items: &mut [EvidenceItem]) {
    items.sort_by(|left, right| {
        relevance_score(right.relevance)
            .total_cmp(&relevance_score(left.relevance))
            .then_with(|| left.content.cmp(&right.content))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.provenance.cmp(&right.provenance))
            .then_with(|| left.id.cmp(&right.id))
    });
}

/// Sortable relevance: `NaN` cannot hijack ordering and sinks with the
/// unscored items instead.
fn relevance_score(relevance: Option<f32>) -> f32 {
    relevance
        .filter(|value| !value.is_nan())
        .unwrap_or(f32::NEG_INFINITY)
}

/// Whether `candidate` is a stronger relevance signal than `current`.
fn bump_relevance(current: Option<f32>, candidate: Option<f32>) -> bool {
    match (current, candidate) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(known), Some(seen)) => seen > known,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: EvidenceKind, content: &str, relevance: Option<f32>) -> EvidenceItem {
        let mut item = EvidenceItem::new(
            kind,
            content,
            Provenance::repo("repo.symbol.search", "a.rs"),
        );
        item.relevance = relevance;
        item
    }

    #[test]
    fn merge_dedupes_and_keeps_first_provenance() {
        let first = item(EvidenceKind::SymbolDefinition, "fn f() {}", Some(0.2));
        let second = item(EvidenceKind::SymbolDefinition, "fn f() {}", Some(0.9));
        let package_a = EvidencePackage {
            question: "q".to_owned(),
            findings: vec![first.clone()],
            contradictions: vec![],
            gaps: vec![],
        };
        let package_b = EvidencePackage {
            question: "q".to_owned(),
            findings: vec![second],
            contradictions: vec![],
            gaps: vec![],
        };
        let merged = EvidencePackage::merge("q", vec![package_a, package_b]);
        assert_eq!(merged.findings.len(), 1);
        let kept = &merged.findings[0];
        assert_eq!(kept.id, first.id);
        assert_eq!(kept.relevance, Some(0.9));
    }

    #[test]
    fn ranking_orders_by_relevance_then_content() {
        let package = EvidencePackage {
            question: "q".to_owned(),
            findings: vec![
                item(EvidenceKind::Note, "b", None),
                item(EvidenceKind::Note, "a", Some(0.1)),
                item(EvidenceKind::Note, "c", Some(0.9)),
            ],
            contradictions: vec![],
            gaps: vec![],
        }
        .ranked();
        let order: Vec<&str> = package
            .findings
            .iter()
            .map(|item| item.content.as_str())
            .collect();
        assert_eq!(order, vec!["c", "a", "b"]);
    }

    #[test]
    fn gaps_dedupe_by_description() {
        let package = EvidencePackage::merge(
            "q",
            vec![
                EvidencePackage {
                    question: "q".to_owned(),
                    findings: vec![],
                    contradictions: vec![],
                    gaps: vec![EvidenceGap::new("missing caller list")],
                },
                EvidencePackage {
                    question: "q".to_owned(),
                    findings: vec![],
                    contradictions: vec![],
                    gaps: vec![EvidenceGap::new("missing caller list")],
                },
            ],
        );
        assert_eq!(package.gaps.len(), 1);
        assert!(package.is_empty() || !package.gaps.is_empty());
    }
}
