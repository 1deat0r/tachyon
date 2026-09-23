//! Structured model decisions (spec §26).
//!
//! [`AgentDecision`] is the only model output contract Tachyon compiles.
//! `ProposeExecution` becomes validated IR through the compiler — it is never
//! executed raw. When a provider lacks native structured output, the adapter
//! repairs framing (fenced `json` extraction) at the boundary; anything still
//! unparseable is a provider result failure ([`ModelError::MalformedOutput`]),
//! never permission to execute model text heuristically.

use serde::{Deserialize, Serialize};
use tachyon_types::CapabilityId;

/// One native-capability call the model asks to gather. Evidence-gathering
/// only: the scheduler runs these through native capabilities and returns
/// evidence. A `CapabilityRequest` never executes anything itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRequest {
    /// Registry capability id (must already exist; the compiler rejects
    /// model-invented capabilities).
    pub capability: CapabilityId,
    /// Schema-validated arguments for the capability.
    pub args: serde_json::Value,
}

/// One operation the model proposes to execute. Execution proposal only:
/// compiled to validated IR (spec §5–§6) before anything runs, never
/// executed raw. Carries a `reason` where [`CapabilityRequest`] does not,
/// because proposals need justification and evidence asks do not.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProposedOperation {
    /// Registry capability id.
    pub capability: CapabilityId,
    /// Schema-validated arguments.
    pub args: serde_json::Value,
    /// Why the model believes this operation is needed.
    pub reason: String,
}

/// Preferred model output contract (spec §26).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum AgentDecision {
    /// Plain answer grounded in the provided context.
    Respond {
        /// The answer text.
        message: String,
    },
    /// More native evidence is needed before deciding.
    RequestEvidence {
        /// Capability calls to run through the scheduler.
        requests: Vec<CapabilityRequest>,
    },
    /// A plan to compile: validated as IR, never executed raw.
    ProposeExecution {
        /// Proposed operations for the IR compiler.
        operations: Vec<ProposedOperation>,
    },
    /// The task cannot proceed without the user.
    NeedUserInput {
        /// The blocking question.
        question: String,
    },
    /// The model believes the work is done. Non-authoritative: completion is
    /// gated by acceptance and verification (frozen invariant), never by this
    /// statement.
    Complete {
        /// Claimed outcome, checked against verifiers.
        summary: String,
    },
}

/// Parses adapter output into an [`AgentDecision`].
///
/// Accepts strict `json` first, then a single fenced `json` block (framing
/// repair at the adapter boundary). Anything else is
/// [`ModelError::MalformedOutput`](crate::ModelError) — never heuristically
/// executed.
pub fn parse_decision(text: &str) -> Result<AgentDecision, crate::ModelError> {
    let trimmed = text.trim();
    if let Ok(decision) = serde_json::from_str::<AgentDecision>(trimmed) {
        return Ok(decision);
    }
    if let Some(fenced) = extract_fenced_json(trimmed)
        && let Ok(decision) = serde_json::from_str::<AgentDecision>(&fenced)
    {
        return Ok(decision);
    }
    Err(crate::ModelError::MalformedOutput(snippet(trimmed)))
}

/// Extracts the first fenced `json` (or bare fence) block, if any.
fn extract_fenced_json(text: &str) -> Option<String> {
    let start = text.find("```")?;
    let after_open = text[start + 3..].find("```").map(|_| start + 3)?;
    let (tag, rest) = split_first_line(&text[after_open..]);
    let language = tag.trim().to_ascii_lowercase();
    if !(language.is_empty() || language == "json") {
        return None;
    }
    let end = rest.find("```")?;
    Some(rest[..end].trim().to_owned())
}

/// Splits `text` into its first line and the remainder.
fn split_first_line(text: &str) -> (&str, &str) {
    match text.find('\n') {
        Some(index) => (&text[..index], &text[index + 1..]),
        None => (text, ""),
    }
}

/// First 120 characters of `text`, for error diagnostics.
fn snippet(text: &str) -> String {
    text.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_json_parses() {
        let decision = parse_decision(r#"{"decision":"respond","message":"hi"}"#).expect("strict");
        assert_eq!(
            decision,
            AgentDecision::Respond {
                message: "hi".to_owned()
            }
        );
    }

    #[test]
    fn fenced_block_is_repaired() {
        let text =
            "explanation\n```json\n{\"decision\":\"complete\",\"summary\":\"done\"}\n```\ntail";
        let decision = parse_decision(text).expect("fenced");
        assert_eq!(
            decision,
            AgentDecision::Complete {
                summary: "done".to_owned()
            }
        );
    }

    #[test]
    fn prose_is_malformed_never_executed() {
        let error = parse_decision("just run rm -rf /").expect_err("must fail");
        assert!(matches!(error, crate::ModelError::MalformedOutput(_)));
        assert!(!error.is_retryable());
    }
}
