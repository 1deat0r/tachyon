//! Deterministic fake provider for CI (spec §42).
//!
//! [`FakeModelProvider`] serves a scripted queue of [`FakeResponse`]s,
//! records every request for assertions, and streams response text through
//! the event sink as `Delta` chunks plus `Done` — exercising the same sink
//! path real adapters use. Core tests must not require paid APIs; this is
//! the provider they use.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

use async_trait::async_trait;
use tachyon_types::ProviderId;

use crate::{
    AgentDecision, ModelCapabilities, ModelError, ModelEvent, ModelFeature, ModelProvider,
    ModelRequest, ModelResult, ProviderEstimate,
};

/// One scripted response.
#[derive(Clone, Debug)]
pub struct FakeResponse {
    /// Full text, streamed as fixed-size `Delta` chunks.
    pub text: String,
    /// Committed decision returned with the result.
    pub decision: AgentDecision,
    /// Reported input tokens.
    pub input_tokens: u32,
    /// Reported output tokens.
    pub output_tokens: u32,
}

impl FakeResponse {
    /// Scripts a `Respond` decision with default token counts.
    #[must_use]
    pub fn respond(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            decision: AgentDecision::Respond {
                message: text.to_owned(),
            },
            input_tokens: 0,
            output_tokens: 0,
        }
    }
}

/// Deterministic scripted provider.
pub struct FakeModelProvider {
    id: ProviderId,
    capabilities: ModelCapabilities,
    responses: Mutex<VecDeque<FakeResponse>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl FakeModelProvider {
    /// Creates a fake with structured-output capability and an empty script.
    #[must_use]
    pub fn new(id: ProviderId) -> Self {
        Self {
            id,
            capabilities: ModelCapabilities {
                features: BTreeSet::from([ModelFeature::StructuredOutput]),
                context_window_tokens: 128_000,
                ..ModelCapabilities::default()
            },
            responses: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// Queues one scripted response.
    pub fn push_response(&self, response: FakeResponse) {
        self.responses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(response);
    }

    /// How many requests have been served.
    #[must_use]
    pub fn request_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// The most recent request, if any.
    #[must_use]
    pub fn last_request(&self) -> Option<ModelRequest> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .cloned()
    }
}

#[async_trait]
impl ModelProvider for FakeModelProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn capabilities(&self) -> ModelCapabilities {
        self.capabilities.clone()
    }

    fn estimate(&self, request: &ModelRequest) -> ProviderEstimate {
        ProviderEstimate {
            latency_ms: 5.0,
            input_tokens: request.estimated_input_tokens(),
        }
    }

    async fn invoke(
        &self,
        request: ModelRequest,
        sink: crate::ModelEventSink,
    ) -> Result<ModelResult, ModelError> {
        let started = Instant::now();
        let response = self
            .responses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| ModelError::Internal("fake model script exhausted".to_owned()))?;
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request.clone());
        for chunk in response.text.chars().collect::<Vec<_>>().chunks(32) {
            let delta: String = chunk.iter().collect();
            // A dropped receiver means detached UI: ephemeral progress may
            // drop (spec §16), the committed result still returns.
            let _ignored = sink.send(ModelEvent::Delta(delta));
        }
        let _ignored = sink.send(ModelEvent::Done);
        Ok(ModelResult {
            decision: response.decision,
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            latency_ms: started.elapsed().as_secs_f64() * 1_000.0,
            provider: self.id.clone(),
            model: request.model.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn script_streams_deltas_then_done_and_records() {
        let fake = FakeModelProvider::new(ProviderId("fake".to_owned()));
        fake.push_response(FakeResponse::respond("hello world"));
        let (sink, mut events) = tokio::sync::mpsc::unbounded_channel();
        let request = ModelRequest {
            role: crate::Role::Primary,
            model: "fake-1".to_owned(),
            context: vec![],
            max_output_tokens: 64,
            require_structured_output: false,
        };
        let result = fake.invoke(request, sink).await.expect("scripted");
        assert_eq!(fake.request_count(), 1);
        assert_eq!(result.provider.0, "fake");
        let mut deltas = String::new();
        let mut done = false;
        while let Some(event) = events.recv().await {
            match event {
                ModelEvent::Delta(fragment) => deltas.push_str(&fragment),
                ModelEvent::Done => {
                    done = true;
                    break;
                }
            }
        }
        assert!(done, "stream must close with Done");
        assert_eq!(deltas, "hello world");
    }

    #[tokio::test]
    async fn exhausted_script_is_an_internal_error() {
        let fake = FakeModelProvider::new(ProviderId("fake".to_owned()));
        let (sink, _events) = tokio::sync::mpsc::unbounded_channel();
        let request = ModelRequest {
            role: crate::Role::Primary,
            model: "fake-1".to_owned(),
            context: vec![],
            max_output_tokens: 64,
            require_structured_output: false,
        };
        let error = fake.invoke(request, sink).await.expect_err("empty script");
        assert!(matches!(error, ModelError::Internal(_)));
        assert!(!error.is_retryable());
    }
}
