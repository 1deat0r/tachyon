//! Bounded judgment items, batches, and certainty policies (spec §24).
//!
//! Judgment answers tiny closed questions — boolean, choice, score — over
//! shared context. It never plans, never executes, and never substitutes
//! for verification. Every item carries a [`CertaintyPolicy`]; outcomes
//! below threshold become explicit [`PolicyAction::Fallback`]s (usually back
//! to evidence), never silent guesses.
//!
//! Confidence is fail-closed: `NaN` sorts as zero, values clamp to
//! `[0, 1]`. A provider that cannot decide must abstain through low
//! confidence, not through a confident wrong answer.

use serde::{Deserialize, Serialize};
use tachyon_models::ContextBlock;
use uuid::Uuid;

/// One closed question a judge can answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JudgmentKind {
    /// Yes/no over the shared context.
    Boolean {
        /// The question to answer.
        question: String,
    },
    /// Pick one of the named options.
    Choice {
        /// The question to answer.
        question: String,
        /// Candidate answers; outcome indexes into this list.
        options: Vec<String>,
    },
    /// Rate within `[min, max]`.
    Score {
        /// What to rate.
        question: String,
        /// Inclusive lower bound.
        min: f32,
        /// Inclusive upper bound; must exceed `min`.
        max: f32,
    },
}

/// What to do when confidence falls short.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertainAction {
    /// Go back to cheap evidence (the default: never spend more on doubt).
    #[default]
    RouteToEvidence,
    /// Spend one model call to resolve the doubt.
    EscalateModel,
    /// Stop and ask the user.
    AskUser,
}

/// Per-item certainty contract.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CertaintyPolicy {
    /// Minimum confidence to accept, in `[0, 1]`.
    pub min_confidence: f32,
    /// Where doubt goes.
    pub on_uncertain: UncertainAction,
}

impl Default for CertaintyPolicy {
    fn default() -> Self {
        Self {
            min_confidence: 0.5,
            on_uncertain: UncertainAction::RouteToEvidence,
        }
    }
}

/// One item in a batch: the question plus its certainty contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JudgmentItem {
    /// Stable identity pairing requests to outcomes.
    pub id: Uuid,
    /// The closed question.
    pub kind: JudgmentKind,
    /// When to accept the answer.
    pub policy: CertaintyPolicy,
}

impl JudgmentItem {
    /// Creates an item with a fresh id and default policy.
    #[must_use]
    pub fn new(kind: JudgmentKind) -> Self {
        Self {
            id: Uuid::now_v7(),
            kind,
            policy: CertaintyPolicy::default(),
        }
    }

    /// Sets a custom certainty policy.
    #[must_use]
    pub fn with_policy(mut self, policy: CertaintyPolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// A batch of judgments sharing one context (spec §24). Batching amortizes
/// the round trip; the shared context is assembled once, not per item.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JudgmentBatch {
    /// Common trusted context for every item.
    pub shared_context: Vec<ContextBlock>,
    /// The closed questions to answer.
    pub items: Vec<JudgmentItem>,
}

/// A judged value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgmentValue {
    /// Answer to a [`JudgmentKind::Boolean`].
    Boolean(bool),
    /// Index into [`JudgmentKind::Choice`] options.
    Choice(usize),
    /// Rating for [`JudgmentKind::Score`].
    Score(f32),
}

/// One provider answer, paired to its item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JudgmentOutcome {
    /// The item this answers.
    pub item_id: Uuid,
    /// The judged value.
    pub value: JudgmentValue,
    /// Provider confidence, clamped to `[0, 1]` on read (`NaN` → 0).
    pub confidence: f32,
}

/// What the harness does with an outcome.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PolicyAction {
    /// Confident enough: carry the value forward.
    Accept {
        /// The judged value.
        value: JudgmentValue,
        /// Clamped confidence.
        confidence: f32,
    },
    /// Not confident enough: route around judgment.
    Fallback {
        /// Where the doubt goes.
        action: UncertainAction,
        /// Clamped confidence that fell short.
        confidence: f32,
    },
}

/// Applies an item's certainty policy to an outcome. Fail-closed on every
/// axis: `NaN` or infinite confidence counts as zero, a non-finite policy
/// threshold accepts nothing, shape mismatches and out-of-range values
/// fall back rather than coercing.
#[must_use]
pub fn apply_policy(item: &JudgmentItem, outcome: &JudgmentOutcome) -> PolicyAction {
    let confidence = effective_confidence(outcome);
    if !shapes_match(&item.kind, &outcome.value) {
        return PolicyAction::Fallback {
            action: item.policy.on_uncertain,
            confidence,
        };
    }
    if confidence >= accept_threshold(item) {
        PolicyAction::Accept {
            value: outcome.value.clone(),
            confidence,
        }
    } else {
        PolicyAction::Fallback {
            action: item.policy.on_uncertain,
            confidence,
        }
    }
}

/// Usable confidence for an outcome: finite values clamp to `[0, 1]`; `NaN`
/// and infinities count as zero. A provider claiming infinite confidence
/// is broken, not certain.
pub(crate) fn effective_confidence(outcome: &JudgmentOutcome) -> f32 {
    if outcome.confidence.is_finite() {
        outcome.confidence.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Acceptance threshold for an item: the policy minimum clamped to
/// `[0, 1]`, or infinity when the policy itself is non-finite (a broken
/// threshold accepts nothing).
pub(crate) fn accept_threshold(item: &JudgmentItem) -> f32 {
    if item.policy.min_confidence.is_finite() {
        item.policy.min_confidence.clamp(0.0, 1.0)
    } else {
        f32::INFINITY
    }
}

/// Whether a value answers its question in range. No coercion: a `Choice`
/// index outside `options`, or a `Score` outside `[min, max]`, is a
/// provider bug, surfaced as fallback. `NaN` scores never match.
fn shapes_match(kind: &JudgmentKind, value: &JudgmentValue) -> bool {
    match (kind, value) {
        (JudgmentKind::Boolean { .. }, JudgmentValue::Boolean(_)) => true,
        (JudgmentKind::Choice { options, .. }, JudgmentValue::Choice(index)) => {
            options.get(*index).is_some()
        }
        (JudgmentKind::Score { min, max, .. }, JudgmentValue::Score(score)) => {
            (*min..=*max).contains(score)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boolean_item() -> JudgmentItem {
        JudgmentItem::new(JudgmentKind::Boolean {
            question: "is it broken?".to_owned(),
        })
    }

    #[test]
    fn confident_match_accepts() {
        let item = boolean_item();
        let outcome = JudgmentOutcome {
            item_id: item.id,
            value: JudgmentValue::Boolean(true),
            confidence: 0.9,
        };
        assert!(matches!(
            apply_policy(&item, &outcome),
            PolicyAction::Accept { .. }
        ));
    }

    #[test]
    fn doubt_and_mismatch_fall_back() {
        let item = boolean_item();
        let unsure = JudgmentOutcome {
            item_id: item.id,
            value: JudgmentValue::Boolean(true),
            confidence: 0.1,
        };
        assert!(matches!(
            apply_policy(&item, &unsure),
            PolicyAction::Fallback {
                action: UncertainAction::RouteToEvidence,
                ..
            }
        ));
        let mismatched = JudgmentOutcome {
            item_id: item.id,
            value: JudgmentValue::Choice(0),
            confidence: 0.99,
        };
        assert!(matches!(
            apply_policy(&item, &mismatched),
            PolicyAction::Fallback { .. }
        ));
        let nan = JudgmentOutcome {
            item_id: item.id,
            value: JudgmentValue::Boolean(true),
            confidence: f32::NAN,
        };
        assert!(matches!(
            apply_policy(&item, &nan),
            PolicyAction::Fallback { .. }
        ));
    }

    #[test]
    fn non_finite_policy_accepts_nothing() {
        let item = JudgmentItem::new(JudgmentKind::Boolean {
            question: "q".to_owned(),
        })
        .with_policy(crate::CertaintyPolicy {
            min_confidence: f32::NAN,
            on_uncertain: crate::UncertainAction::RouteToEvidence,
        });
        let outcome = JudgmentOutcome {
            item_id: item.id,
            value: JudgmentValue::Boolean(true),
            confidence: 1.0,
        };
        assert!(matches!(
            apply_policy(&item, &outcome),
            PolicyAction::Fallback { .. }
        ));
    }

    #[test]
    fn out_of_range_values_fall_back() {
        let choice = JudgmentItem::new(JudgmentKind::Choice {
            question: "q".to_owned(),
            options: vec!["a".to_owned(), "b".to_owned()],
        });
        let oob = JudgmentOutcome {
            item_id: choice.id,
            value: JudgmentValue::Choice(9),
            confidence: 0.99,
        };
        assert!(matches!(
            apply_policy(&choice, &oob),
            PolicyAction::Fallback { .. }
        ));
        let score = JudgmentItem::new(JudgmentKind::Score {
            question: "q".to_owned(),
            min: 0.0,
            max: 1.0,
        });
        let wild = JudgmentOutcome {
            item_id: score.id,
            value: JudgmentValue::Score(1e30),
            confidence: 0.99,
        };
        assert!(matches!(
            apply_policy(&score, &wild),
            PolicyAction::Fallback { .. }
        ));
    }
}
