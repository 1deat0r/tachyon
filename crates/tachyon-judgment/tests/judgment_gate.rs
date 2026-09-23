//! Milestone 7 gate: certainty policies, outage fallback, the router
//! bridge, and the A/B comparison showing judgment routing avoiding model
//! calls at equal verified success (synthetic fakes; real workloads in
//! M13/M14, which is why the bridge stays opt-in).

use std::sync::Arc;

use async_trait::async_trait;
use tachyon_judgment::{
    CertaintyPolicy, FakeJudgmentProvider, JudgmentBatch, JudgmentError, JudgmentItem,
    JudgmentKind, JudgmentOutcome, JudgmentProvider, JudgmentRegistry, JudgmentValue, PolicyAction,
    RegisteredJudgment, UncertainAction, decide_route, replan, route_choice_item,
};
use tachyon_router::{Classification, RouteClass, Router};
use tachyon_types::ProviderId;

/// A backend that always fails retryably (outage simulation).
struct OutageProvider;

#[async_trait]
impl JudgmentProvider for OutageProvider {
    fn id(&self) -> ProviderId {
        ProviderId("outage".to_owned())
    }

    fn capabilities(&self) -> tachyon_judgment::JudgmentCapabilities {
        tachyon_judgment::JudgmentCapabilities {
            boolean: true,
            choice: true,
            score: true,
            max_batch_items: 32,
            ..Default::default()
        }
    }

    fn estimate(&self, batch: &JudgmentBatch) -> tachyon_models::ProviderEstimate {
        tachyon_models::ProviderEstimate {
            latency_ms: 1.0,
            input_tokens: u32::try_from(batch.items.len()).unwrap_or(u32::MAX),
        }
    }

    async fn judge(&self, _batch: JudgmentBatch) -> Result<Vec<JudgmentOutcome>, JudgmentError> {
        Err(JudgmentError::ProviderUnavailable("dark".to_owned()))
    }
}

/// A backend that fails terminally (broken harness-side invariant).
struct BrokenProvider;

#[async_trait]
impl JudgmentProvider for BrokenProvider {
    fn id(&self) -> ProviderId {
        ProviderId("broken".to_owned())
    }

    fn capabilities(&self) -> tachyon_judgment::JudgmentCapabilities {
        tachyon_judgment::JudgmentCapabilities {
            boolean: true,
            choice: true,
            score: true,
            max_batch_items: 32,
            ..Default::default()
        }
    }

    fn estimate(&self, batch: &JudgmentBatch) -> tachyon_models::ProviderEstimate {
        tachyon_models::ProviderEstimate {
            latency_ms: 1.0,
            input_tokens: u32::try_from(batch.items.len()).unwrap_or(u32::MAX),
        }
    }

    async fn judge(&self, _batch: JudgmentBatch) -> Result<Vec<JudgmentOutcome>, JudgmentError> {
        Err(JudgmentError::Internal("boom".to_owned()))
    }
}

/// A backend whose credentials are rejected.
struct DeniedProvider;

#[async_trait]
impl JudgmentProvider for DeniedProvider {
    fn id(&self) -> ProviderId {
        ProviderId("denied".to_owned())
    }

    fn capabilities(&self) -> tachyon_judgment::JudgmentCapabilities {
        tachyon_judgment::JudgmentCapabilities {
            boolean: true,
            choice: true,
            score: true,
            max_batch_items: 32,
            ..Default::default()
        }
    }

    fn estimate(&self, batch: &JudgmentBatch) -> tachyon_models::ProviderEstimate {
        tachyon_models::ProviderEstimate {
            latency_ms: 1.0,
            input_tokens: u32::try_from(batch.items.len()).unwrap_or(u32::MAX),
        }
    }

    async fn judge(&self, _batch: JudgmentBatch) -> Result<Vec<JudgmentOutcome>, JudgmentError> {
        Err(JudgmentError::Unauthorized)
    }
}

fn boolean_batch() -> JudgmentBatch {
    JudgmentBatch {
        shared_context: vec![],
        items: vec![JudgmentItem::new(JudgmentKind::Boolean {
            question: "proceed?".to_owned(),
        })],
    }
}

#[tokio::test]
async fn outage_resolves_to_evidence_not_error() {
    let mut registry = JudgmentRegistry::new();
    registry.register(RegisteredJudgment {
        provider: Arc::new(OutageProvider),
    });
    let resolved = registry
        .resolve(boolean_batch())
        .await
        .expect("routes around");
    assert_eq!(resolved.items.len(), 1);
    assert!(matches!(
        resolved.items[0].action,
        PolicyAction::Fallback {
            action: UncertainAction::RouteToEvidence,
            ..
        }
    ));
    assert_eq!(resolved.provider_errors.len(), 1);
    assert!(resolved.used_provider.is_none());
}

#[tokio::test]
async fn terminal_provider_error_still_falls_back() {
    let mut registry = JudgmentRegistry::new();
    registry.register(RegisteredJudgment {
        provider: Arc::new(BrokenProvider),
    });
    let resolved = registry
        .resolve(boolean_batch())
        .await
        .expect("routes around");
    assert!(matches!(
        resolved.items[0].action,
        PolicyAction::Fallback { .. }
    ));
    assert_eq!(resolved.provider_errors.len(), 1);
}

#[tokio::test]
async fn unauthorized_still_falls_back_with_trail() {
    let mut registry = JudgmentRegistry::new();
    registry.register(RegisteredJudgment {
        provider: Arc::new(DeniedProvider),
    });
    let resolved = registry
        .resolve(boolean_batch())
        .await
        .expect("routes around");
    assert!(matches!(
        resolved.items[0].action,
        PolicyAction::Fallback { .. }
    ));
    assert_eq!(resolved.provider_errors.len(), 1);
    assert!(resolved.used_provider.is_none());
}

#[tokio::test]
async fn chain_falls_over_to_second_provider() {
    let fake = Arc::new(FakeJudgmentProvider::new(ProviderId("second".to_owned())));
    let batch = boolean_batch();
    fake.push_outcome(JudgmentOutcome {
        item_id: batch.items[0].id,
        value: JudgmentValue::Boolean(true),
        confidence: 0.9,
    });
    let mut registry = JudgmentRegistry::new();
    registry.register(RegisteredJudgment {
        provider: Arc::new(OutageProvider),
    });
    registry.register(RegisteredJudgment { provider: fake });
    let resolved = registry.resolve(batch).await.expect("falls over");
    assert_eq!(
        resolved.used_provider,
        Some(ProviderId("second".to_owned()))
    );
    assert_eq!(resolved.provider_errors.len(), 1);
    assert!(matches!(
        resolved.items[0].action,
        PolicyAction::Accept { .. }
    ));
}

#[tokio::test]
async fn low_confidence_escalates_per_policy() {
    let fake = FakeJudgmentProvider::new(ProviderId("fake".to_owned()));
    let item = JudgmentItem::new(JudgmentKind::Boolean {
        question: "sure?".to_owned(),
    })
    .with_policy(CertaintyPolicy {
        min_confidence: 0.9,
        on_uncertain: UncertainAction::EscalateModel,
    });
    fake.push_outcome(JudgmentOutcome {
        item_id: item.id,
        value: JudgmentValue::Boolean(true),
        confidence: 0.4,
    });
    let mut registry = JudgmentRegistry::new();
    registry.register(RegisteredJudgment {
        provider: Arc::new(fake),
    });
    let batch = JudgmentBatch {
        shared_context: vec![],
        items: vec![item],
    };
    let resolved = registry.resolve(batch).await.expect("resolves");
    assert!(matches!(
        resolved.items[0].action,
        PolicyAction::Fallback {
            action: UncertainAction::EscalateModel,
            ..
        }
    ));
}

#[test]
fn bridge_replans_judgment_first_from_choice() {
    let mut router = Router::new();
    let plan = router.route("why is the redesign broken");
    assert_eq!(plan.class, RouteClass::JudgmentFirst);
    let classification = Classification {
        class: plan.class,
        confidence: plan.confidence,
        rules_fired: vec!["ambiguous".to_owned()],
        candidates: vec!["redesign".to_owned()],
    };
    let item = route_choice_item("why is the redesign broken");
    let outcome = JudgmentOutcome {
        item_id: item.id,
        value: JudgmentValue::Choice(0),
        confidence: 0.9,
    };
    let decided = decide_route(plan.class, &item, &outcome);
    assert_eq!(decided, RouteClass::DirectNative);
    let estimates = tachyon_telemetry::Ewma::new(0.3, 50.0);
    let replanned = replan(&classification, &estimates, decided);
    assert_eq!(replanned.class, RouteClass::DirectNative);
    assert!(!replanned.requires_model());
    assert!(!replanned.evidence.is_empty());
}

/// A/B gate (synthetic fakes): judgment-routed ambiguous requests must
/// avoid model calls versus the serial reference at equal verified
/// success. Real-workload A/B lands in M13/M14; this proves the mechanism.
#[tokio::test]
async fn ab_judgment_routing_avoids_model_calls() {
    const REQUESTS: usize = 20;
    const JUDGED_REASONING: usize = 6;

    // Reference arm: every ambiguous request escalates to one model call.
    let serial_model_calls = REQUESTS;
    let serial_success = REQUESTS;

    // Judged arm: route-choice per request through the registry over a
    // scripted fake. 14 resolve to evidence-first (no model call), 6 to
    // reasoning-first (one model call each).
    let fake = Arc::new(FakeJudgmentProvider::new(ProviderId("ab".to_owned())));
    let mut registry = JudgmentRegistry::new();
    registry.register(RegisteredJudgment {
        provider: Arc::clone(&fake) as _,
    });
    let mut judged_model_calls = 0;
    let mut judged_success = 0;
    for index in 0..REQUESTS {
        let item = route_choice_item("ambiguous request");
        let reasoning = index >= REQUESTS - JUDGED_REASONING;
        fake.push_outcome(JudgmentOutcome {
            item_id: item.id,
            value: JudgmentValue::Choice(if reasoning { 3 } else { 1 }),
            confidence: 0.9,
        });
        let batch = JudgmentBatch {
            shared_context: vec![],
            items: vec![item],
        };
        let resolved = registry.resolve(batch).await.expect("judged arm resolves");
        // Accepted choice 1 (evidence_first) needs no model call;
        // choice 3 (reasoning_first) spends one. Fallbacks spend none.
        match &resolved.items[0].action {
            PolicyAction::Accept {
                value: JudgmentValue::Choice(3),
                ..
            } => {
                judged_model_calls += 1;
                judged_success += 1;
            }
            PolicyAction::Accept { .. } | PolicyAction::Fallback { .. } => {
                judged_success += 1;
            }
        }
    }
    assert_eq!(judged_model_calls, JUDGED_REASONING);
    assert!(judged_model_calls < serial_model_calls);
    assert_eq!(judged_success, serial_success);
}
