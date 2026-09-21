//! `OpenJEV` adapter, behind the `openjev` feature (spec §24).
//!
//! Speaks a minimal documented JSON judgments protocol to a configurable
//! endpoint (default: local). The protocol binding is provisional: it
//! covers the three internal judgment forms over one round trip, and a
//! later ADR pins the real `OpenJEV` wire spec when one exists. Without the
//! feature this module — and only this module — is absent; the crate,
//! registry, and fake all compile and run.
//!
//! Wire protocol (provisional, versioned by path):
//!
//! ```text
//! POST {base}/v1/judgments
//! {"model": "...", "context": "...", "items": [
//!   {"id": "<uuid>", "kind": "boolean", "question": "..."},
//!   {"id": "<uuid>", "kind": "choice", "question": "...",
//!    "options": ["a", "b"]},
//!   {"id": "<uuid>", "kind": "score", "question": "...",
//!    "min": 0.0, "max": 1.0}]}
//!
//! {"outcomes": [
//!   {"item_id": "<uuid>", "value": {"boolean": true},
//!    "confidence": 0.9}]}
//! ```
//!
//! Value shapes: `{"boolean": bool}`, `{"choice": index}`,
//! `{"score": number}`. Anything else is `MalformedResponse`.

use async_trait::async_trait;
use tachyon_models::ProviderEstimate;
use tachyon_types::ProviderId;

use crate::RemoteJudgmentConfig;
use crate::{
    JudgmentBatch, JudgmentCapabilities, JudgmentError, JudgmentItem, JudgmentKind,
    JudgmentOutcome, JudgmentProvider, JudgmentValue,
};

/// Pluggable HTTP layer. Production uses [`TcpJudgmentTransport`]; tests
/// inject canned responses without sockets.
#[async_trait]
pub trait JudgmentTransport: Send + Sync {
    /// POSTs `body` as `json` to `url` and returns the response body.
    async fn post_json(
        &self,
        url: &str,
        api_key: Option<&str>,
        body: &str,
        timeout_ms: u64,
    ) -> Result<String, JudgmentError>;
}

/// Minimal `http://` POST transport over `TcpStream`. Same scope contract
/// as the model adapter: `Connection: close`, read to EOF, plain HTTP only
/// (no TLS dependency in M7), config control characters rejected.
pub struct TcpJudgmentTransport;

#[async_trait]
impl JudgmentTransport for TcpJudgmentTransport {
    async fn post_json(
        &self,
        url: &str,
        api_key: Option<&str>,
        body: &str,
        timeout_ms: u64,
    ) -> Result<String, JudgmentError> {
        let (host, port, path) = parse_http_url(url)?;
        let request = build_http_request(&host, port, &path, api_key, body);
        let address = format!("{host}:{port}");
        let raw = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            round_trip(&address, &request),
        )
        .await
        .map_err(|_| JudgmentError::Timeout { timeout_ms })??;
        let (status, headers, response_body) = split_http_response(&raw)?;
        status_to_result(status, &headers, &response_body)
    }
}

/// Transport failure below the provider API.
fn transport_failure(detail: &str) -> JudgmentError {
    JudgmentError::ProviderUnavailable(format!("judgment transport: {detail}"))
}

/// Runs one TCP exchange: connect, write, read to EOF.
async fn round_trip(address: &str, request: &str) -> Result<String, JudgmentError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .map_err(|error| transport_failure(&format!("connect {address}: {error}")))?;
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| transport_failure(&format!("write: {error}")))?;
    let mut raw = Vec::new();
    // Bounded read: a hostile or broken server must not OOM the harness.
    let mut capped = stream.take(256 * 1024);
    capped
        .read_to_end(&mut raw)
        .await
        .map_err(|error| transport_failure(&format!("read: {error}")))?;
    String::from_utf8(raw)
        .map_err(|error| transport_failure(&format!("response is not UTF-8: {error}")))
}

/// Accepts only plain `http://` URLs; control characters fail closed.
fn parse_http_url(url: &str) -> Result<(String, u16, String), JudgmentError> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        JudgmentError::InvalidRequest(format!("openjev adapter supports http:// URLs only: {url}"))
    })?;
    if rest.chars().any(|char| char == '\r' || char == '\n') {
        return Err(JudgmentError::InvalidRequest(
            "base URL contains control characters".to_owned(),
        ));
    }
    let (authority, path) = match rest.find('/') {
        Some(index) => (rest[..index].to_owned(), rest[index..].to_owned()),
        None => (rest.to_owned(), "/".to_owned()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => {
            let port = port.parse::<u16>().map_err(|_| {
                JudgmentError::InvalidRequest(format!("bad port in base URL: {url}"))
            })?;
            (host.to_owned(), port)
        }
        None => (authority, 80),
    };
    if host.is_empty() {
        return Err(JudgmentError::InvalidRequest(format!(
            "empty host in base URL: {url}"
        )));
    }
    Ok((host, port, path))
}

/// Renders a minimal HTTP/1.1 POST. Non-default ports ride the `Host`
/// header so name-based local servers route correctly.
fn build_http_request(
    host: &str,
    port: u16,
    path: &str,
    api_key: Option<&str>,
    body: &str,
) -> String {
    use std::fmt::Write as _;
    let authority = if port == 80 {
        host.to_owned()
    } else {
        format!("{host}:{port}")
    };
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(key) = api_key {
        let _ignored = write!(request, "Authorization: Bearer {key}\r\n");
    }
    request.push_str("\r\n");
    request.push_str(body);
    request
}

/// Parsed headers: lowercased names with trimmed values.
type Headers = Vec<(String, String)>;

/// Splits a raw HTTP/1.x response into status, headers, and body.
fn split_http_response(raw: &str) -> Result<(u16, Headers, String), JudgmentError> {
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| JudgmentError::ProviderUnavailable("malformed HTTP response".to_owned()))?;
    let mut lines = head.lines();
    let status_line = lines.next().ok_or_else(|| {
        JudgmentError::ProviderUnavailable("malformed HTTP status line".to_owned())
    })?;
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| {
            JudgmentError::ProviderUnavailable(format!("bad HTTP status line: {status_line}"))
        })?;
    let mut headers = Headers::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(|| {
            JudgmentError::ProviderUnavailable(format!("bad HTTP header: {line}"))
        })?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    Ok((code, headers, body.to_owned()))
}

/// Maps HTTP status to the judgment taxonomy. 408 is a retryable timeout;
/// anything unrecognized becomes `ProviderUnavailable` (fail over, don't
/// fail the harness). Bodies embedded in errors are truncated — full
/// payloads never reach logs through these strings.
fn status_to_result(status: u16, headers: &Headers, body: &str) -> Result<String, JudgmentError> {
    match status {
        200..=299 => Ok(body.to_owned()),
        401 | 403 => Err(JudgmentError::Unauthorized),
        408 => Err(JudgmentError::Timeout { timeout_ms: 0 }),
        429 => Err(JudgmentError::RateLimited {
            retry_after_ms: headers
                .iter()
                .find(|(name, _)| name == "retry-after")
                .and_then(|(_, value)| value.parse::<u64>().ok())
                .map_or(1_000, |seconds| seconds.saturating_mul(1_000)),
        }),
        400..=499 => Err(JudgmentError::InvalidRequest(format!(
            "HTTP {status}: {}",
            snippet(body)
        ))),
        _ => Err(JudgmentError::ProviderUnavailable(format!(
            "HTTP {status}: {}",
            snippet(body)
        ))),
    }
}

/// First 200 characters of `text`, for error diagnostics.
fn snippet(text: &str) -> String {
    text.chars().take(200).collect()
}

/// Joins the endpoint URL, tolerating a trailing slash on the base.
fn judgments_url(base_url: &str) -> String {
    format!("{}/v1/judgments", base_url.trim_end_matches('/'))
}

/// Renders one item in the provisional wire format.
fn wire_item(item: &JudgmentItem) -> serde_json::Value {
    let mut value = serde_json::json!({"id": item.id.to_string()});
    match &item.kind {
        JudgmentKind::Boolean { question } => {
            value["kind"] = "boolean".into();
            value["question"] = question.clone().into();
        }
        JudgmentKind::Choice { question, options } => {
            value["kind"] = "choice".into();
            value["question"] = question.clone().into();
            value["options"] = options.clone().into();
        }
        JudgmentKind::Score { question, min, max } => {
            value["kind"] = "score".into();
            value["question"] = question.clone().into();
            value["min"] = (*min).into();
            value["max"] = (*max).into();
        }
    }
    value
}

/// Builds the judgments request body. Pure function, unit tested.
fn build_request_body(config: &RemoteJudgmentConfig, batch: &JudgmentBatch) -> String {
    let context = batch
        .shared_context
        .iter()
        .map(|block| block.content.as_str())
        .collect::<Vec<_>>()
        .join("\n---\n");
    serde_json::json!({
        "model": config.model,
        "context": context,
        "items": batch.items.iter().map(wire_item).collect::<Vec<_>>(),
    })
    .to_string()
}

/// Parses one outcome value in the provisional shapes.
fn parse_value(value: &serde_json::Value) -> Option<JudgmentValue> {
    if let Some(flag) = value.get("boolean").and_then(serde_json::Value::as_bool) {
        return Some(JudgmentValue::Boolean(flag));
    }
    if let Some(index) = value.get("choice").and_then(serde_json::Value::as_u64) {
        return Some(JudgmentValue::Choice(
            usize::try_from(index).unwrap_or(usize::MAX),
        ));
    }
    if let Some(score) = value.get("score").and_then(serde_json::Value::as_f64)
        && score.is_finite()
    {
        return Some(JudgmentValue::Score(narrow_float(score)));
    }
    None
}

/// Narrows a wire float to `f32`. Scores and confidences are small
/// magnitudes; out-of-range values saturate at the `f32` extremes rather
/// than wrapping or erroring.
fn narrow_float(value: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    let narrowed = value.clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32;
    narrowed
}

/// Parses the outcomes array. Pure and tested: unknown shapes are
/// `MalformedResponse`, never confident answers.
fn parse_outcomes(body: &str) -> Result<Vec<JudgmentOutcome>, JudgmentError> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|error| {
        JudgmentError::MalformedResponse(format!("response is not JSON: {error}"))
    })?;
    let outcomes = value
        .get("outcomes")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            JudgmentError::MalformedResponse("response has no outcomes array".to_owned())
        })?;
    outcomes
        .iter()
        .map(|outcome| {
            let item_id = outcome
                .get("item_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| id.parse().ok())
                .ok_or_else(|| {
                    JudgmentError::MalformedResponse("outcome has no item_id".to_owned())
                })?;
            let value = outcome.get("value").and_then(parse_value).ok_or_else(|| {
                JudgmentError::MalformedResponse("outcome has unknown value shape".to_owned())
            })?;
            let confidence = match outcome
                .get("confidence")
                .and_then(serde_json::Value::as_f64)
            {
                // Absent confidence means zero, never a guess. Non-finite
                // confidence is malformed (matching the score path): the
                // policy layer must never see an infinity laundered into
                // `f32::MAX` and clamped to a confident 1.0.
                None => 0.0,
                Some(value) if value.is_finite() => narrow_float(value),
                Some(_) => {
                    return Err(JudgmentError::MalformedResponse(
                        "outcome confidence is not finite".to_owned(),
                    ));
                }
            };
            Ok(JudgmentOutcome {
                item_id,
                value,
                confidence,
            })
        })
        .collect()
}

/// `OpenJEV` judgments provider over any [`JudgmentTransport`].
pub struct OpenJevProvider<T = TcpJudgmentTransport> {
    id: ProviderId,
    config: RemoteJudgmentConfig,
    capabilities: JudgmentCapabilities,
    transport: T,
}

impl<T> OpenJevProvider<T> {
    /// Creates the adapter with an explicit transport (stub in tests).
    #[must_use]
    pub fn new(id: ProviderId, config: RemoteJudgmentConfig, transport: T) -> Self {
        let capabilities = JudgmentCapabilities {
            boolean: true,
            choice: true,
            score: true,
            max_batch_items: config.max_batch_items,
            ..JudgmentCapabilities::default()
        };
        Self {
            id,
            config,
            capabilities,
            transport,
        }
    }
}

impl OpenJevProvider<TcpJudgmentTransport> {
    /// Creates the production adapter speaking plain HTTP.
    #[must_use]
    pub fn local(id: ProviderId, config: RemoteJudgmentConfig) -> Self {
        Self::new(id, config, TcpJudgmentTransport)
    }
}

#[async_trait]
impl<T: JudgmentTransport> JudgmentProvider for OpenJevProvider<T> {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn capabilities(&self) -> JudgmentCapabilities {
        self.capabilities.clone()
    }

    fn estimate(&self, batch: &JudgmentBatch) -> ProviderEstimate {
        let items = f64::from(u32::try_from(batch.items.len()).unwrap_or(u32::MAX));
        ProviderEstimate {
            latency_ms: 60.0 + 10.0 * items,
            input_tokens: u32::try_from(batch.items.len()).unwrap_or(u32::MAX),
        }
    }

    async fn judge(&self, batch: JudgmentBatch) -> Result<Vec<JudgmentOutcome>, JudgmentError> {
        let api_key = self
            .config
            .api_key_env
            .as_deref()
            .and_then(|name| std::env::var(name).ok());
        let url = judgments_url(&self.config.base_url);
        parse_http_url(&url).map(|_| ())?;
        let body = build_request_body(&self.config, &batch);
        let raw = self
            .transport
            .post_json(
                &url,
                api_key.as_deref(),
                &body,
                self.config.request_timeout_ms,
            )
            .await?;
        parse_outcomes(&raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JudgmentItem;

    struct StubTransport {
        response: Result<String, JudgmentError>,
    }

    #[async_trait]
    impl JudgmentTransport for StubTransport {
        async fn post_json(
            &self,
            _url: &str,
            _api_key: Option<&str>,
            _body: &str,
            _timeout_ms: u64,
        ) -> Result<String, JudgmentError> {
            match &self.response {
                Ok(body) => Ok(body.clone()),
                Err(error) => Err(error.clone()),
            }
        }
    }

    fn batch() -> JudgmentBatch {
        JudgmentBatch {
            shared_context: vec![],
            items: vec![JudgmentItem::new(JudgmentKind::Boolean {
                question: "go?".to_owned(),
            })],
        }
    }

    #[test]
    fn body_carries_all_three_shapes() {
        let provider = OpenJevProvider::new(
            ProviderId("stub".to_owned()),
            RemoteJudgmentConfig::default(),
            StubTransport {
                response: Ok(String::new()),
            },
        );
        let batch = JudgmentBatch {
            shared_context: vec![],
            items: vec![
                JudgmentItem::new(JudgmentKind::Boolean {
                    question: "b".to_owned(),
                }),
                JudgmentItem::new(JudgmentKind::Choice {
                    question: "c".to_owned(),
                    options: vec!["x".to_owned(), "y".to_owned()],
                }),
                JudgmentItem::new(JudgmentKind::Score {
                    question: "s".to_owned(),
                    min: 0.0,
                    max: 1.0,
                }),
            ],
        };
        let body: serde_json::Value =
            serde_json::from_str(&build_request_body(&provider.config, &batch)).expect("JSON");
        assert_eq!(body["items"][0]["kind"], "boolean");
        assert_eq!(body["items"][1]["options"][1], "y");
        assert_eq!(body["items"][2]["max"], 1.0);
    }

    #[test]
    fn unknown_value_shapes_are_malformed() {
        let bad = serde_json::json!({"outcomes": [
            {"item_id": uuid::Uuid::now_v7().to_string(),
             "value": {"verdict": "maybe"}, "confidence": 0.99}
        ]})
        .to_string();
        let error = parse_outcomes(&bad).expect_err("unknown shape");
        assert!(matches!(error, JudgmentError::MalformedResponse(_)));
        assert!(error.is_retryable());
    }

    #[test]
    fn non_finite_wire_confidence_is_malformed() {
        // 1e999 overflows f64 to infinity on parse: it must fail here,
        // never launder through narrowing into a confident 1.0.
        let id = uuid::Uuid::now_v7();
        let bad = format!(
            "{{\"outcomes\": [{{\"item_id\": \"{id}\", \
              \"value\": {{\"boolean\": true}}, \"confidence\": 1e999}}]}}"
        );
        let error = parse_outcomes(&bad).expect_err("infinite confidence");
        assert!(matches!(error, JudgmentError::MalformedResponse(_)));
    }

    #[test]
    fn plain_http_only() {
        assert!(parse_http_url("https://example.com/v1").is_err());
        assert!(parse_http_url("http://h/x\r\nbad").is_err());
    }

    #[test]
    fn request_timeout_is_retryable_and_bodies_truncate() {
        let headers = Headers::new();
        let error = status_to_result(408, &headers, "slow").expect_err("408");
        assert!(matches!(error, JudgmentError::Timeout { .. }));
        assert!(error.is_retryable());
        let long = "x".repeat(10_000);
        let error = status_to_result(500, &headers, &long).expect_err("500");
        match error {
            JudgmentError::ProviderUnavailable(detail) => {
                assert!(detail.chars().count() < long.chars().count());
            }
            other => panic!("expected unavailable, got {other:?}"),
        }
    }

    #[test]
    fn endpoint_url_tolerates_trailing_slash() {
        assert_eq!(
            judgments_url("http://h:8811/"),
            "http://h:8811/v1/judgments"
        );
        assert_eq!(judgments_url("http://h:8811"), "http://h:8811/v1/judgments");
    }

    #[test]
    fn host_header_carries_non_default_port() {
        let wire = build_http_request("h", 8811, "/v1", None, "{}");
        assert!(wire.contains("Host: h:8811\r\n"));
        let plain = build_http_request("h", 80, "/v1", None, "{}");
        assert!(plain.contains("Host: h\r\n"));
    }

    #[tokio::test]
    async fn stub_response_parses_to_outcomes() {
        let batch = batch();
        let completion = serde_json::json!({"outcomes": [
            {"item_id": batch.items[0].id.to_string(),
             "value": {"boolean": true}, "confidence": 0.7}
        ]})
        .to_string();
        let provider = OpenJevProvider::new(
            ProviderId("stub".to_owned()),
            RemoteJudgmentConfig::default(),
            StubTransport {
                response: Ok(completion),
            },
        );
        let (outcomes, confidence) = {
            let outcomes = provider.judge(batch).await.expect("stub");
            let confidence = outcomes[0].confidence;
            (outcomes, confidence)
        };
        assert_eq!(outcomes[0].value, JudgmentValue::Boolean(true));
        assert!((confidence - 0.7).abs() < f32::EPSILON);
    }
}
