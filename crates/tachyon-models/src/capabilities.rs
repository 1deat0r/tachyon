//! Capability negotiation: core selects capabilities and roles, never
//! provider names (spec §25).
//!
//! A [`ModelProvider`](crate::ModelProvider) advertises
//! [`ModelCapabilities`]; callers state [`CapabilityRequirements`] and the
//! registry only routes to a provider whose capabilities satisfy them.
//! Provider-specific extensions stay behind the adapter.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Coarse latency band, used for cheapest-sufficient selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyClass {
    /// Local or edge inference, sub-second typical.
    Low,
    /// Hosted API, seconds typical.
    #[default]
    Medium,
    /// Largest reasoning models.
    High,
}

/// Coarse cost band, used for cheapest-sufficient selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostClass {
    /// Free local inference.
    #[default]
    Low,
    /// Cheap hosted tier.
    Medium,
    /// Flagship pricing.
    High,
}

/// One composable backend feature (spec §25). A set — not a bool row — so
/// new features extend negotiation without reshaping the struct.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFeature {
    /// Incremental `delta` events through the event sink.
    Streaming,
    /// Native constrained/`json` output mode.
    StructuredOutput,
    /// Native function/tool-call encoding.
    ToolCallEncoding,
    /// Image-bearing context blocks.
    Vision,
    /// Prompt-caching support for repeated prefixes.
    PromptCaching,
    /// Provider reasoning-effort controls.
    ReasoningControls,
}

/// What a model backend can do (spec §25).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    /// Supported backend features.
    pub features: BTreeSet<ModelFeature>,
    /// Hard context window, tokens.
    pub context_window_tokens: u32,
    /// Latency band for cheapest-sufficient routing.
    pub latency_class: LatencyClass,
    /// Cost band for cheapest-sufficient routing.
    pub cost_class: CostClass,
}

impl ModelCapabilities {
    /// Whether the backend offers `feature`.
    #[must_use]
    pub fn supports(&self, feature: ModelFeature) -> bool {
        self.features.contains(&feature)
    }
}

/// What a task needs from whichever provider serves it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRequirements {
    /// Caller will set `require_structured_output` on the request.
    pub need_structured_output: bool,
    /// Context carries image blocks.
    pub need_vision: bool,
    /// Caller consumes incremental `delta` events.
    pub need_streaming: bool,
    /// Smallest acceptable context window, tokens.
    pub min_context_window_tokens: u32,
}

impl CapabilityRequirements {
    /// Whether `capabilities` can serve a request with these requirements.
    /// Pure capability check — no provider-name conditionals (spec §25).
    #[must_use]
    pub fn satisfied_by(self, capabilities: &ModelCapabilities) -> bool {
        (!self.need_structured_output || capabilities.supports(ModelFeature::StructuredOutput))
            && (!self.need_vision || capabilities.supports(ModelFeature::Vision))
            && (!self.need_streaming || capabilities.supports(ModelFeature::Streaming))
            && capabilities.context_window_tokens >= self.min_context_window_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capable() -> ModelCapabilities {
        ModelCapabilities {
            features: BTreeSet::from([ModelFeature::StructuredOutput]),
            context_window_tokens: 32_768,
            ..ModelCapabilities::default()
        }
    }

    #[test]
    fn negotiation_gates_on_capabilities_not_names() {
        let caps = capable();
        let needs = CapabilityRequirements {
            need_structured_output: true,
            min_context_window_tokens: 16_384,
            ..CapabilityRequirements::default()
        };
        assert!(needs.satisfied_by(&caps));
        assert!(
            !CapabilityRequirements {
                need_vision: true,
                ..needs
            }
            .satisfied_by(&caps)
        );
        assert!(!needs.satisfied_by(&ModelCapabilities {
            context_window_tokens: 4_096,
            ..caps
        }));
    }
}
