//! Credential broker skeleton (spec §35).
//!
//! Runtime state stores handles, never secret values. Secrets are injected
//! only at the execution boundary; known secret material is registered for
//! redaction of process/provider output. Raw values must not enter model
//! prompts, durable events, TUI messages, telemetry, or artifact logs.

use std::collections::HashMap;

/// Opaque handle to registered secret material.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialHandle(pub String);

/// Redaction placeholder embedded in scrubbed output.
#[must_use]
pub fn redaction_for(handle: &CredentialHandle) -> String {
    format!("[redacted:{}]", handle.0)
}

/// Registers secrets under handles and redacts them from output.
#[derive(Clone, Debug, Default)]
pub struct CredentialBroker {
    secrets: HashMap<CredentialHandle, Vec<u8>>,
}

impl CredentialBroker {
    /// Registers secret material, returning its handle. The same bytes
    /// registered twice return the existing handle.
    pub fn register(&mut self, secret: &[u8], label: &str) -> CredentialHandle {
        if let Some((handle, _)) = self.secrets.iter().find(|(_, known)| known == &secret) {
            return handle.clone();
        }
        let handle = CredentialHandle(format!("{label}-{}", self.secrets.len() + 1));
        self.secrets.insert(handle.clone(), secret.to_vec());
        handle
    }

    /// Injects the secret at the execution boundary. Callers must never
    /// persist or log the return value.
    #[must_use]
    pub fn use_handle(&self, handle: &CredentialHandle) -> Option<Vec<u8>> {
        self.secrets.get(handle).cloned()
    }

    /// Redacts all registered secret material from `text`.
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        let mut scrubbed = text.to_owned();
        for (handle, secret) in &self.secrets {
            let Ok(needle) = std::str::from_utf8(secret) else {
                continue;
            };
            if !needle.is_empty() {
                scrubbed = scrubbed.replace(needle, &redaction_for(handle));
            }
        }
        scrubbed
    }

    /// Byte-level redaction for process output (may be non-UTF8).
    #[must_use]
    pub fn redact_bytes(&self, bytes: &[u8]) -> Vec<u8> {
        let mut scrubbed = bytes.to_vec();
        for (handle, secret) in &self.secrets {
            if secret.is_empty() {
                continue;
            }
            let replacement = redaction_for(handle).into_bytes();
            scrubbed = replace_bytes(&scrubbed, secret, &replacement);
        }
        scrubbed
    }
}

fn replace_bytes(haystack: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    if needle.is_empty() {
        return haystack.to_vec();
    }
    let mut out = Vec::with_capacity(haystack.len());
    let mut rest = haystack;
    while let Some(index) = find_subsequence(rest, needle) {
        out.extend_from_slice(&rest[..index]);
        out.extend_from_slice(replacement);
        rest = &rest[index + needle.len()..];
    }
    out.extend_from_slice(rest);
    out
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
