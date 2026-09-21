//! Deterministic intent rules (router stage 2).
//!
//! Ordered rules vote for classes with weights; the winner and its margin
//! decide the class. No model, no judge, no I/O — a pure function of the
//! request text, so classification itself costs microseconds.

use crate::RouteClass;

/// A classification: winning class, confidence in [0,1], evidence to fetch.
#[derive(Clone, Debug, PartialEq)]
pub struct Classification {
    pub class: RouteClass,
    pub confidence: f64,
    pub rules_fired: Vec<String>,
    /// Symbol or topic candidates extracted for evidence ops.
    pub candidates: Vec<String>,
}

struct Rule {
    name: &'static str,
    class: RouteClass,
    weight: f64,
    /// Lowercase substring alternatives; any match fires the rule.
    any_of: &'static [&'static str],
}

const RULES: &[Rule] = &[
    // DirectNative: definition/reference lookup.
    Rule {
        name: "where-defined",
        class: RouteClass::DirectNative,
        weight: 3.0,
        any_of: &["where is", "where are", "defined", "definition of"],
    },
    Rule {
        name: "find-references",
        class: RouteClass::DirectNative,
        weight: 3.0,
        any_of: &[
            "references to",
            "used by",
            "who calls",
            "callers of",
            "usages of",
        ],
    },
    Rule {
        name: "show-status",
        class: RouteClass::DirectNative,
        weight: 3.0,
        any_of: &["git status", "show status", "what changed in git"],
    },
    Rule {
        name: "run-command",
        class: RouteClass::DirectNative,
        weight: 2.0,
        any_of: &[
            "run the tests",
            "run tests",
            "build the",
            "show git log",
            "git log",
            "git diff",
        ],
    },
    Rule {
        name: "find-pattern",
        class: RouteClass::DirectNative,
        weight: 2.0,
        any_of: &["find ", "search for", "grep for", "look for"],
    },
    // EvidenceFirst: diagnosis questions over existing behavior.
    Rule {
        name: "why-failing",
        class: RouteClass::EvidenceFirst,
        weight: 3.0,
        any_of: &[
            "why is", "why does", "why did", "failing", "broken", "error",
        ],
    },
    Rule {
        name: "what-changed",
        class: RouteClass::EvidenceFirst,
        weight: 3.0,
        any_of: &["what changed", "recently changed", "around ", "regression"],
    },
    Rule {
        name: "how-does",
        class: RouteClass::EvidenceFirst,
        weight: 2.0,
        any_of: &["how does", "how is", "explain why these"],
    },
    // ReasoningFirst: open-ended hard problems.
    Rule {
        name: "redesign",
        class: RouteClass::ReasoningFirst,
        weight: 3.0,
        any_of: &[
            "redesign",
            "rearchitect",
            "rewrite the",
            "root cause",
            "race condition",
            "intermittent",
            "deadlock",
        ],
    },
    Rule {
        name: "fix-it",
        class: RouteClass::ReasoningFirst,
        weight: 2.0,
        any_of: &["fix the", "fix this", "implement ", "add support for"],
    },
    // Hybrid: explicit asks for comparison/choice with evidence.
    Rule {
        name: "compare",
        class: RouteClass::Hybrid,
        weight: 2.0,
        any_of: &[
            "compare",
            "which is better",
            "behave differently",
            "difference between",
        ],
    },
];

/// Scores in [0,1] derived from vote totals.
fn confidence(winner: f64, runner_up: f64) -> f64 {
    if winner <= 0.0 {
        return 0.0;
    }
    (winner - runner_up).clamp(0.0, 4.0) / 4.0 * 0.5 + 0.5
}

/// Classifies `request` with deterministic rules.
#[must_use]
pub fn classify(request: &str) -> Classification {
    let lowered = request.to_lowercase();
    let mut scores: std::collections::HashMap<RouteClass, f64> = std::collections::HashMap::new();
    let mut fired: Vec<(&str, RouteClass)> = Vec::new();
    for rule in RULES {
        if rule.any_of.iter().any(|pattern| lowered.contains(pattern)) {
            *scores.entry(rule.class).or_insert(0.0) += rule.weight;
            fired.push((rule.name, rule.class));
        }
    }
    let mut ranked: Vec<(RouteClass, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let candidates = extract_candidates(request);
    match ranked.as_slice() {
        [] => Classification {
            // No signal: cheap evidence first, never a blind model call.
            class: RouteClass::EvidenceFirst,
            confidence: 0.25,
            rules_fired: vec!["no-signal".to_owned()],
            candidates,
        },
        [(class, winner)] => Classification {
            class: *class,
            confidence: confidence(*winner, 0.0),
            rules_fired: fired
                .iter()
                .filter(|(_, rule_class)| rule_class == class)
                .map(|(name, _)| (*name).to_owned())
                .collect(),
            candidates,
        },
        [(class, winner), (_, runner_up), ..] => {
            let margin = winner - runner_up;
            // Genuinely ambiguous: judgment decides once M7 lands.
            if margin < 1.0 {
                return Classification {
                    class: RouteClass::JudgmentFirst,
                    confidence: 0.5 - margin * 0.1,
                    rules_fired: fired.iter().map(|(name, _)| (*name).to_owned()).collect(),
                    candidates,
                };
            }
            Classification {
                class: *class,
                confidence: confidence(*winner, *runner_up),
                rules_fired: fired
                    .iter()
                    .filter(|(_, rule_class)| rule_class == class)
                    .map(|(name, _)| (*name).to_owned())
                    .collect(),
                candidates,
            }
        }
    }
}

/// Sentence scaffolding that is never an evidence candidate.
const STOPWORDS: &[&str] = &[
    "where", "what", "why", "how", "show", "find", "search", "run", "explain", "compare", "which",
    "who", "the", "this", "that", "these", "those", "with",
];

/// Extracts `CamelCase`/`snake_case` tokens as evidence candidates.
fn extract_candidates(request: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for token in request.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '/'))
    {
        let token = token.trim_matches(|c| c == '.' || c == '/');
        if token.len() < 3 || STOPWORDS.contains(&token.to_lowercase().as_str()) {
            continue;
        }
        if token.len() >= 3
            && token.chars().any(char::is_uppercase)
            && !candidates.contains(&token.to_owned())
        {
            candidates.push(token.to_owned());
        }
        if token.len() >= 4
            && token.contains('_')
            && token
                .chars()
                .all(|c| c.is_lowercase() || c == '_' || c.is_numeric())
            && !candidates.contains(&token.to_owned())
        {
            candidates.push(token.to_owned());
        }
    }
    candidates.truncate(5);
    candidates
}
