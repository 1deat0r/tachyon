//! Deterministic fake provider for CI (spec §42).
//!
//! [`FakeJudgmentProvider`] serves a scripted queue of [`JudgmentOutcome`]s
//! — one per requested item, re-stamped with the item's id — and records
//! every batch for assertions. Core tests must not require live judges;
//! this is the provider they use.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use tachyon_models::ProviderEstimate;
use tachyon_types::ProviderId;

use crate::{
    JudgmentBatch, JudgmentCapabilities, JudgmentError, JudgmentOutcome, JudgmentProvider,
};

/// Deterministic scripted provider.
pub struct FakeJudgmentProvider {
    id: ProviderId,
    capabilities: JudgmentCapabilities,
    script: Mutex<VecDeque<JudgmentOutcome>>,
    batches: Mutex<Vec<JudgmentBatch>>,
}

impl FakeJudgmentProvider {
    /// Creates a fake answering all shapes, batches up to 32 items.
    #[must_use]
    pub fn new(id: ProviderId) -> Self {
        Self {
            id,
            capabilities: JudgmentCapabilities {
                boolean: true,
                choice: true,
                score: true,
                max_batch_items: 32,
                ..JudgmentCapabilities::default()
            },
            script: Mutex::new(VecDeque::new()),
            batches: Mutex::new(Vec::new()),
        }
    }

    /// Queues one scripted outcome (consumed per requested item).
    pub fn push_outcome(&self, outcome: JudgmentOutcome) {
        self.script
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(outcome);
    }

    /// How many batches have been judged.
    #[must_use]
    pub fn batch_count(&self) -> usize {
        self.batches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

#[async_trait]
impl JudgmentProvider for FakeJudgmentProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn capabilities(&self) -> JudgmentCapabilities {
        self.capabilities.clone()
    }

    fn estimate(&self, batch: &JudgmentBatch) -> ProviderEstimate {
        ProviderEstimate {
            latency_ms: 5.0,
            input_tokens: u32::try_from(batch.items.len()).unwrap_or(u32::MAX),
        }
    }

    async fn judge(&self, batch: JudgmentBatch) -> Result<Vec<JudgmentOutcome>, JudgmentError> {
        self.batches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(batch.clone());
        let mut script = self
            .script
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut outcomes = Vec::with_capacity(batch.items.len());
        for item in &batch.items {
            let mut outcome = script.pop_front().ok_or_else(|| {
                JudgmentError::Internal("fake judgment script exhausted".to_owned())
            })?;
            // Re-stamp with the requested id so pairing is deterministic
            // regardless of how the script was built.
            outcome.item_id = item.id;
            outcomes.push(outcome);
        }
        drop(script);
        Ok(outcomes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{JudgmentItem, JudgmentKind, JudgmentValue};

    #[tokio::test]
    async fn script_answers_in_order_and_records() {
        let fake = FakeJudgmentProvider::new(ProviderId("fake".to_owned()));
        let item = JudgmentItem::new(JudgmentKind::Boolean {
            question: "go?".to_owned(),
        });
        fake.push_outcome(JudgmentOutcome {
            item_id: item.id,
            value: JudgmentValue::Boolean(true),
            confidence: 0.8,
        });
        let batch = JudgmentBatch {
            shared_context: vec![],
            items: vec![item],
        };
        let outcomes = fake.judge(batch).await.expect("scripted");
        assert_eq!(fake.batch_count(), 1);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].value, JudgmentValue::Boolean(true));
    }

    #[tokio::test]
    async fn exhausted_script_is_internal() {
        let fake = FakeJudgmentProvider::new(ProviderId("fake".to_owned()));
        let batch = JudgmentBatch {
            shared_context: vec![],
            items: vec![JudgmentItem::new(JudgmentKind::Boolean {
                question: "q".to_owned(),
            })],
        };
        let error = fake.judge(batch).await.expect_err("empty script");
        assert!(matches!(error, JudgmentError::Internal(_)));
        assert!(!error.is_retryable());
    }
}
