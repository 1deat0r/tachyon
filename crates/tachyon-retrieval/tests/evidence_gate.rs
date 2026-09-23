//! Milestone 6 gate for `tachyon-retrieval`: merges are deterministic,
//! duplicates collapse to first provenance, ranking is relevance-ordered.

use tachyon_retrieval::{EvidenceGap, EvidenceItem, EvidenceKind, EvidencePackage, Provenance};

fn item(content: &str, relevance: Option<f32>) -> EvidenceItem {
    let mut item = EvidenceItem::new(
        EvidenceKind::SymbolReference,
        content,
        Provenance::repo("repo.symbol.references", "src/auth.rs").with_hash("h1"),
    );
    item.relevance = relevance;
    item
}

#[test]
fn merge_is_deterministic_and_provenance_survives() {
    let package = |relevance| EvidencePackage {
        question: "q".to_owned(),
        findings: vec![item("use auth::refresh;", Some(relevance))],
        contradictions: vec![],
        gaps: vec![EvidenceGap::new("caller list")],
    };
    let low = package(0.2);
    let high = package(0.8);
    let first = EvidencePackage::merge("q", vec![low.clone(), high.clone()]);
    let second = EvidencePackage::merge("q", vec![low, high]);
    assert_eq!(first, second);
    assert_eq!(first.findings.len(), 1);
    assert_eq!(first.findings[0].relevance, Some(0.8));
    assert_eq!(first.findings[0].provenance.hash.as_deref(), Some("h1"));
    assert_eq!(first.gaps.len(), 1);
}

#[test]
fn contradictions_are_preserved_not_merged_into_findings() {
    let package = EvidencePackage {
        question: "q".to_owned(),
        findings: vec![item("a", Some(0.5))],
        contradictions: vec![item("b", Some(0.4))],
        gaps: vec![],
    };
    let merged = EvidencePackage::merge("q", vec![package]);
    assert_eq!(merged.findings.len(), 1);
    assert_eq!(merged.contradictions.len(), 1);
    assert_eq!(merged.len(), 2);
    assert!(!merged.is_empty());
    assert!(EvidencePackage::new("empty").is_empty());
}

#[test]
fn identical_content_at_different_paths_survives() {
    let package = EvidencePackage {
        question: "q".to_owned(),
        findings: vec![
            EvidenceItem::new(
                EvidenceKind::SymbolDefinition,
                "fn f() {}",
                Provenance::repo("repo.symbol.search", "one.rs"),
            ),
            EvidenceItem::new(
                EvidenceKind::SymbolDefinition,
                "fn f() {}",
                Provenance::repo("repo.symbol.search", "two.rs"),
            ),
        ],
        contradictions: vec![],
        gaps: vec![],
    };
    let merged = EvidencePackage::merge("q", vec![package]);
    assert_eq!(
        merged.findings.len(),
        2,
        "distinct locations are not duplicates"
    );
}

#[test]
fn nan_relevance_sinks_with_unscored() {
    let package = EvidencePackage {
        question: "q".to_owned(),
        findings: vec![
            item("scored", Some(0.99)),
            item("poisoned", Some(f32::NAN)),
            item("unscored", None),
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
    assert_eq!(order[0], "scored");
    assert!(order[1..].contains(&"poisoned"));
    assert!(order[1..].contains(&"unscored"));
}
