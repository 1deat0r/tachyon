//! Provider-neutral judgment interface (spec §24).
//!
//! Core selects capabilities, never provider names. `OpenJEV` is one adapter
//! behind the `openjev` feature; this crate compiles and runs without it.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tachyon_models::{CostClass, LatencyClass, ProviderEstimate};
use tachyon_types::ProviderId;

use crate::{JudgmentBatch, JudgmentKind, JudgmentOutcome};

/// What a judgment backend can answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgmentCapabilities {
    /// Boolean questions.
    pub boolean: bool,
    /// Choice questions.
    pub choice: bool,
    /// Score questions.
    pub score: bool,
    /// Largest batch accepted per call.
    pub max_batch_items: u32,
    /// Latency band for cheapest-sufficient routing.
    pub latency_class: LatencyClass,
    /// Cost band for cheapest-sufficient routing.
    pub cost_class: CostClass,
}

impl JudgmentCapabilities {
    /// Whether the backend answers this question shape.
    #[must_use]
    pub fn supports(&self, kind: &JudgmentKind) -> bool {
        match kind {
            JudgmentKind::Boolean { .. } => self.boolean,
            JudgmentKind::Choice { .. } => self.choice,
            JudgmentKind::Score { .. } => self.score,
        }
    }

    /// Whether the backend answers every item in the batch within its size
    /// limit.
    #[must_use]
    pub fn serves(&self, batch: &JudgmentBatch) -> bool {
        u32::try_from(batch.items.len()).unwrap_or(u32::MAX) <= self.max_batch_items
            && batch.items.iter().all(|item| self.supports(&item.kind))
    }
}

/// Judgment failure taxonomy (spec §40). Retryability mirrors the model
/// layer: the registry owns retry/fallback policy, providers never retry
/// internally.
#[derive(Clone, Debug, PartialEq, thiserror::Error, Serialize, Deserialize)]
pub enum JudgmentError {
    /// Bad batch shape or configuration. Fix the caller.
    #[error("invalid judgment batch: {0}")]
    InvalidRequest(String),
    /// Rejected credentials.
    #[error("judgment provider unauthorized")]
    Unauthorized,
    /// Throttled. Retry after the given milliseconds.
    #[error("judgment provider rate limited, retry after {retry_after_ms}ms")]
    RateLimited {
        /// Server-advised or default backoff.
        retry_after_ms: u64,
    },
    /// Provider unreachable or erroring. Fall back, don't fail the harness.
    #[error("judgment provider unavailable: {0}")]
    ProviderUnavailable(String),
    /// The call exceeded its deadline.
    #[error("judgment call timed out after {timeout_ms}ms")]
    Timeout {
        /// Deadline that fired.
        timeout_ms: u64,
    },
    /// Response framing unusable. A provider result failure, handled as
    /// fallback — never as a confident answer. Retryable (unlike the
    /// model layer's `MalformedOutput`) because the registry retries
    /// across providers: the next backend may parse what this one
    /// garbled.
    #[error("malformed judgment response: {0}")]
    MalformedResponse(String),
    /// Caller cancelled before outcomes committed.
    #[error("judgment call cancelled")]
    Cancelled,
    /// Broken harness-side invariant (missing adapter, empty registry use).
    #[error("internal judgment error: {0}")]
    Internal(String),
}

impl JudgmentError {
    /// Whether the registry may try the next provider. `Cancelled`
    /// propagates; everything operational becomes per-item fallback.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. }
            | Self::ProviderUnavailable(_)
            | Self::Timeout { .. }
            | Self::MalformedResponse(_) => true,
            Self::InvalidRequest(_) | Self::Unauthorized | Self::Cancelled | Self::Internal(_) => {
                false
            }
        }
    }
}

/// Provider-neutral judgment interface (spec §24).
#[async_trait]
pub trait JudgmentProvider: Send + Sync {
    /// Stable provider identity for telemetry.
    fn id(&self) -> ProviderId;

    /// Which question shapes this backend answers.
    fn capabilities(&self) -> JudgmentCapabilities;

    /// Latency/cost guess for one batch. Uncalibrated seed, like models.
    fn estimate(&self, batch: &JudgmentBatch) -> ProviderEstimate;

    /// Judges one batch, returning one outcome per item (by `item_id`).
    /// A provider failure routes around the provider rather than failing
    /// the harness — the registry converts errors to fallback actions.
    async fn judge(&self, batch: JudgmentBatch) -> Result<Vec<JudgmentOutcome>, JudgmentError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JudgmentItem;

    #[test]
    fn capabilities_gate_shapes_and_size() {
        let caps = JudgmentCapabilities {
            boolean: true,
            max_batch_items: 1,
            ..JudgmentCapabilities::default()
        };
        assert!(caps.supports(&JudgmentKind::Boolean {
            question: "q".to_owned()
        }));
        assert!(!caps.supports(&JudgmentKind::Choice {
            question: "q".to_owned(),
            options: vec!["a".to_owned()]
        }));
        let two = JudgmentBatch {
            shared_context: vec![],
            items: vec![
                JudgmentItem::new(JudgmentKind::Boolean {
                    question: "q".to_owned(),
                }),
                JudgmentItem::new(JudgmentKind::Boolean {
                    question: "q".to_owned(),
                }),
            ],
        };
        assert!(!caps.serves(&two), "max_batch_items is 1");
    }

    #[test]
    fn taxonomy_routes_around_operational_failures() {
        assert!(JudgmentError::ProviderUnavailable("x".to_owned()).is_retryable());
        assert!(JudgmentError::MalformedResponse("x".to_owned()).is_retryable());
        assert!(!JudgmentError::Cancelled.is_retryable());
    }
}
