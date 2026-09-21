//! Judgment registry: capability routing, outage fallback, config selection.
//!
//! Resolution never fails the harness for operational reasons. Batch-shape
//! bugs (`InvalidRequest`) and caller cancellation (`Cancelled`) propagate
//! as errors; every provider outage, timeout, or malformed response becomes
//! per-item [`PolicyAction::Fallback`]s, and an empty or incapable registry
//! resolves everything to evidence. A Jev failure routes around the
//! provider — the task continues on deterministic paths.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tachyon_types::ProviderId;
use uuid::Uuid;

use crate::{
    JudgmentBatch, JudgmentError, JudgmentOutcome, JudgmentProvider, PolicyAction, apply_policy,
};

/// One registered backend. Order is preference order for fallback.
pub struct RegisteredJudgment {
    /// The serving backend.
    pub provider: Arc<dyn JudgmentProvider>,
}

/// One resolved item: its policy action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedItem {
    /// The judged item.
    pub item_id: Uuid,
    /// Accept the value or route around judgment.
    pub action: PolicyAction,
}

/// A resolved batch. Always complete: one entry per requested item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedBatch {
    /// One entry per requested item, in request order.
    pub items: Vec<ResolvedItem>,
    /// Backend that served the batch, if any answered.
    pub used_provider: Option<ProviderId>,
    /// Provider failures folded into fallbacks, oldest first.
    pub provider_errors: Vec<String>,
}

/// Registry of judgment backends with fallback resolution.
#[derive(Default)]
pub struct JudgmentRegistry {
    providers: Vec<RegisteredJudgment>,
}

impl JudgmentRegistry {
    /// Creates an empty registry. Empty resolves everything to evidence —
    /// Tachyon runs without any judgment provider (spec §24).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a backend. Order is fallback preference.
    pub fn register(&mut self, provider: RegisteredJudgment) {
        self.providers.push(provider);
    }

    /// Resolves a batch: first capable backend wins; failures walk the
    /// chain; anything still unresolved becomes per-item fallback. Returns
    /// `Err` only for caller bugs (`InvalidRequest` from [`validate`]) and
    /// caller cancellation (`Cancelled`): every provider-side failure —
    /// outage, timeout, auth, malformed, internal — routes around the
    /// provider rather than failing the harness (spec §24).
    pub async fn resolve(&self, batch: JudgmentBatch) -> Result<ResolvedBatch, JudgmentError> {
        validate(&batch)?;
        let mut errors = Vec::new();
        for entry in self
            .providers
            .iter()
            .filter(|entry| entry.provider.capabilities().serves(&batch))
        {
            match entry.provider.judge(batch.clone()).await {
                Ok(outcomes) => {
                    return Ok(pair_outcomes(
                        &batch,
                        &outcomes,
                        Some(entry.provider.id()),
                        errors,
                    ));
                }
                Err(JudgmentError::Cancelled) => return Err(JudgmentError::Cancelled),
                Err(error) => {
                    errors.push(format!("{}: {error}", entry.provider.id()));
                }
            }
        }
        // No capable provider, or every backend failed: evidence fallback
        // per item, with the failure trail attached.
        Ok(pair_outcomes(&batch, &[], None, errors))
    }
}

/// Builds a provider from operator-owned configuration. `Fake` always
/// works; `Remote` needs the `openjev` feature and fails closed with
/// `Internal` without it — never with a half-wired provider.
pub fn provider_from_source(
    source: &JudgmentSource,
) -> Result<Arc<dyn JudgmentProvider>, JudgmentError> {
    match source {
        JudgmentSource::Fake => Ok(Arc::new(crate::FakeJudgmentProvider::new(ProviderId(
            "fake".to_owned(),
        ))) as _),
        #[cfg(feature = "openjev")]
        JudgmentSource::Remote(config) => Ok(provider_from_openjev(config)),
        #[cfg(not(feature = "openjev"))]
        JudgmentSource::Remote(_) => Err(JudgmentError::Internal(
            "remote judgment needs the openjev feature".to_owned(),
        )),
    }
}

/// Builds the remote provider. Only compiled with the `openjev` feature;
/// without it the `Remote` arm fails closed in [`provider_from_source`].
#[cfg(feature = "openjev")]
fn provider_from_openjev(config: &RemoteJudgmentConfig) -> Arc<dyn JudgmentProvider> {
    Arc::new(crate::OpenJevProvider::local(
        ProviderId("openjev".to_owned()),
        config.clone(),
    )) as _
}

/// Validates batch shape before any provider sees it: non-empty items,
/// unique ids, non-empty choice options, finite score bounds with
/// `min < max`, and finite policy thresholds.
fn validate(batch: &JudgmentBatch) -> Result<(), JudgmentError> {
    if batch.items.is_empty() {
        return Err(JudgmentError::InvalidRequest(
            "batch holds no items".to_owned(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for item in &batch.items {
        if !seen.insert(item.id) {
            return Err(JudgmentError::InvalidRequest(
                "batch holds duplicate item ids".to_owned(),
            ));
        }
        if !item.policy.min_confidence.is_finite() {
            return Err(JudgmentError::InvalidRequest(
                "policy min_confidence must be finite".to_owned(),
            ));
        }
        match &item.kind {
            crate::JudgmentKind::Choice { options, .. } if options.is_empty() => {
                return Err(JudgmentError::InvalidRequest(
                    "choice item holds no options".to_owned(),
                ));
            }
            crate::JudgmentKind::Score { min, max, .. }
                if !(min.is_finite() && max.is_finite() && min < max) =>
            {
                return Err(JudgmentError::InvalidRequest(
                    "score item needs finite min < max".to_owned(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Pairs provider outcomes to requested items in request order, applying
/// certainty policies. Missing outcomes fall back per the item's own
/// policy (not a hardcoded default); surplus or unknown ids are ignored.
fn pair_outcomes(
    batch: &JudgmentBatch,
    outcomes: &[JudgmentOutcome],
    used_provider: Option<ProviderId>,
    provider_errors: Vec<String>,
) -> ResolvedBatch {
    let items = batch
        .items
        .iter()
        .map(|item| {
            let action = outcomes
                .iter()
                .find(|outcome| outcome.item_id == item.id)
                .map_or(
                    PolicyAction::Fallback {
                        action: item.policy.on_uncertain,
                        confidence: 0.0,
                    },
                    |outcome| apply_policy(item, outcome),
                );
            ResolvedItem {
                item_id: item.id,
                action,
            }
        })
        .collect();
    ResolvedBatch {
        items,
        used_provider,
        provider_errors,
    }
}

/// Operator-owned source selection. `Remote` holds plain data and always
/// compiles; actual remote use needs the `openjev` feature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum JudgmentSource {
    /// Deterministic scripted backend (tests, CI, offline use).
    Fake,
    /// Remote judgments endpoint (needs `openjev` feature to serve).
    Remote(RemoteJudgmentConfig),
}

/// Remote endpoint configuration (operator-owned, never model-chosen).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoteJudgmentConfig {
    /// Base URL, e.g. `http://localhost:8811`. Plain HTTP only in M7.
    pub base_url: String,
    /// Model or judge name sent in every request.
    pub model: String,
    /// Environment variable holding the API key (`None` for local servers).
    pub api_key_env: Option<String>,
    /// Per-request deadline, milliseconds.
    pub request_timeout_ms: u64,
    /// Largest batch accepted per call.
    pub max_batch_items: u32,
}

impl Default for RemoteJudgmentConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8811".to_owned(),
            model: "default".to_owned(),
            api_key_env: None,
            request_timeout_ms: 30_000,
            max_batch_items: 16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UncertainAction;
    use crate::fake::FakeJudgmentProvider;

    fn provider() -> Arc<dyn JudgmentProvider> {
        Arc::new(FakeJudgmentProvider::new(ProviderId("fake".to_owned())))
    }

    #[test]
    fn empty_registry_resolves_to_evidence() {
        let batch = JudgmentBatch {
            shared_context: vec![],
            items: vec![crate::JudgmentItem::new(crate::JudgmentKind::Boolean {
                question: "q".to_owned(),
            })],
        };
        let resolved = pair_outcomes(&batch, &[], None, vec![]);
        assert_eq!(resolved.items.len(), 1);
        assert!(matches!(
            resolved.items[0].action,
            PolicyAction::Fallback {
                action: UncertainAction::RouteToEvidence,
                ..
            }
        ));
        assert!(resolved.used_provider.is_none());
    }

    #[test]
    fn validation_rejects_bad_shapes() {
        assert!(matches!(
            validate(&JudgmentBatch::default()),
            Err(JudgmentError::InvalidRequest(_))
        ));
        let _provider = provider();
    }

    #[test]
    fn source_factory_serves_fake() {
        let provider = provider_from_source(&JudgmentSource::Fake).expect("fake always builds");
        assert_eq!(provider.id().0, "fake");
    }

    #[test]
    fn remote_source_needs_the_feature() {
        let source = JudgmentSource::Remote(RemoteJudgmentConfig::default());
        #[cfg(not(feature = "openjev"))]
        assert!(matches!(
            provider_from_source(&source),
            Err(JudgmentError::Internal(_))
        ));
        #[cfg(feature = "openjev")]
        assert!(provider_from_source(&source).is_ok());
    }
}
