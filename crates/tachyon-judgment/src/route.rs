//! Router bridge: bounded judgment resolves `JudgmentFirst` (spec §22–§24).
//!
//! The M5 router labels ambiguous requests `JudgmentFirst` but resolves
//! them to evidence because no judge existed. This bridge closes that loop
//! as explicit opt-in: build a route-choice item, judge it, and replan from
//! the decided class. Default router paths are untouched — until judgment
//! proves itself on real workloads (M7 gate), `JudgmentFirst` still means
//! evidence-first unless the caller invokes this bridge.

use tachyon_router::{Classification, RouteClass, RoutePlan, plan};
use tachyon_telemetry::Ewma;

use crate::{CertaintyPolicy, JudgmentItem, JudgmentKind, JudgmentOutcome, JudgmentValue};

/// Route-class options in a fixed order. The index IS the contract:
/// outcomes carry an index into this list.
pub const ROUTE_OPTIONS: [&str; 5] = [
    "direct_native",
    "evidence_first",
    "judgment_first",
    "reasoning_first",
    "hybrid",
];

/// Builds the route-choice item for `request`: which of the five classes
/// serves it cheapest. Doubt falls back to evidence (the M5 default), so a
/// weak judge can never spend more than the status quo.
#[must_use]
pub fn route_choice_item(request: &str) -> JudgmentItem {
    JudgmentItem::new(JudgmentKind::Choice {
        question: format!("Which route serves this request cheapest? {request}"),
        options: ROUTE_OPTIONS.iter().map(ToString::to_string).collect(),
    })
    .with_policy(CertaintyPolicy {
        min_confidence: 0.5,
        on_uncertain: crate::UncertainAction::RouteToEvidence,
    })
}

/// Decides a [`RouteClass`] from a judged outcome against the item's own
/// certainty policy. Mismatched ids, non-finite confidence, doubt below
/// threshold, wrong shapes, and out-of-range indexes all keep the original
/// class — a judge that cannot decide must not reroute.
#[must_use]
pub fn decide_route(
    original: RouteClass,
    item: &JudgmentItem,
    outcome: &JudgmentOutcome,
) -> RouteClass {
    if item.id != outcome.item_id {
        return original;
    }
    if crate::batch::effective_confidence(outcome) < crate::batch::accept_threshold(item) {
        return original;
    }
    match &outcome.value {
        JudgmentValue::Choice(index) => index_to_class(*index).unwrap_or(original),
        JudgmentValue::Boolean(_) | JudgmentValue::Score(_) => original,
    }
}

/// Replans from the judged class, reusing router pricing and escalation.
/// Confidence carries the classification's confidence — the outcome's
/// confidence already gated the decision in [`decide_route`]. The
/// fired-rules trail gains `judgment-route` so telemetry shows the judge
/// ran.
#[must_use]
pub fn replan(classification: &Classification, estimates: &Ewma, decided: RouteClass) -> RoutePlan {
    let reclassified = Classification {
        class: decided,
        confidence: classification.confidence,
        rules_fired: classification
            .rules_fired
            .iter()
            .cloned()
            .chain(std::iter::once("judgment-route".to_owned()))
            .collect(),
        candidates: classification.candidates.clone(),
    };
    plan::build(&reclassified, estimates)
}

/// Maps a choice index to its class. `judgment_first` as an outcome keeps
/// the `JudgmentFirst` label, which the router already lowers to
/// evidence-only execution — the bridge is one-shot per caller, so this
/// cannot loop by itself.
fn index_to_class(index: usize) -> Option<RouteClass> {
    match index {
        0 => Some(RouteClass::DirectNative),
        1 => Some(RouteClass::EvidenceFirst),
        2 => Some(RouteClass::JudgmentFirst),
        3 => Some(RouteClass::ReasoningFirst),
        4 => Some(RouteClass::Hybrid),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_match_class_names() {
        let classes = [
            RouteClass::DirectNative,
            RouteClass::EvidenceFirst,
            RouteClass::JudgmentFirst,
            RouteClass::ReasoningFirst,
            RouteClass::Hybrid,
        ];
        let names: Vec<&str> = classes.iter().map(|class| class.name()).collect();
        assert_eq!(ROUTE_OPTIONS.to_vec(), names);
    }

    #[test]
    fn weak_or_wrong_outcomes_keep_original() {
        let item = route_choice_item("ambiguous");
        let weak = JudgmentOutcome {
            item_id: item.id,
            value: JudgmentValue::Choice(0),
            confidence: 0.1,
        };
        assert_eq!(
            decide_route(RouteClass::JudgmentFirst, &item, &weak),
            RouteClass::JudgmentFirst
        );
        let stray = JudgmentOutcome {
            item_id: item.id,
            value: JudgmentValue::Choice(9),
            confidence: 0.9,
        };
        assert_eq!(
            decide_route(RouteClass::JudgmentFirst, &item, &stray),
            RouteClass::JudgmentFirst
        );
    }

    #[test]
    fn infinite_confidence_never_reroutes() {
        let item = route_choice_item("ambiguous");
        for confidence in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            let outcome = JudgmentOutcome {
                item_id: item.id,
                value: JudgmentValue::Choice(0),
                confidence,
            };
            assert_eq!(
                decide_route(RouteClass::JudgmentFirst, &item, &outcome),
                RouteClass::JudgmentFirst,
                "confidence {confidence} must not reroute"
            );
        }
    }
}
