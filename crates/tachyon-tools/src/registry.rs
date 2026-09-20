//! Capability registry: native capability descriptors (spec §28).
//!
//! Provider-native tool calls are only proposed invocations; they never
//! bypass compilation/policy/scheduling. The registry records what exists
//! and what each capability takes — enforcement lives in
//! [`crate::authorize`] plus the scheduler's conflict grants.

use std::collections::HashMap;
use tachyon_types::CapabilityId;

/// Describes one native capability.
#[derive(Clone, Debug)]
pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    /// Human-readable summary for routing and approval prompts.
    pub description: String,
    /// JSON Schema for the capability's arguments.
    pub arg_schema: serde_json::Value,
}

/// The native capability registry.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    capabilities: HashMap<String, CapabilityDescriptor>,
}

impl Registry {
    #[must_use]
    pub fn native() -> Self {
        let mut registry = Self::default();
        let object = serde_json::json!({"type": "object"});
        for (id, description, arg_schema) in [
            (
                "fs.read",
                "Read a workspace file (contained)",
                serde_json::json!({
                    "type": "object",
                    "required": ["path"],
                    "properties": {"path": {"type": "string"}},
                }),
            ),
            (
                "fs.list",
                "List a workspace directory (contained)",
                serde_json::json!({
                    "type": "object",
                    "required": ["path"],
                    "properties": {"path": {"type": "string"}},
                }),
            ),
            (
                "fs.metadata",
                "Stat a workspace path (contained)",
                serde_json::json!({
                    "type": "object",
                    "required": ["path"],
                    "properties": {"path": {"type": "string"}},
                }),
            ),
            (
                "fs.write",
                "Write a workspace file (contained; outside needs approval)",
                serde_json::json!({
                    "type": "object",
                    "required": ["path", "content"],
                    "properties": {
                        "path": {"type": "string"},
                        "content": {"type": "string"},
                    },
                }),
            ),
            (
                "process.spawn",
                "Run a process with captured, redacted output",
                serde_json::json!({
                    "type": "object",
                    "required": ["program"],
                    "properties": {
                        "program": {"type": "string"},
                        "args": {"type": "array", "items": {"type": "string"}},
                        "timeout_ms": {"type": "integer"},
                    },
                }),
            ),
            (
                "git.read",
                "Read-only git: status, diff, log, show (contained cwd)",
                serde_json::json!({
                    "type": "object",
                    "required": ["subcommand"],
                    "properties": {"subcommand": {"type": "string"}},
                }),
            ),
            (
                "artifact.store",
                "Spool bytes into the content-addressed artifact store",
                object.clone(),
            ),
            (
                "artifact.fetch",
                "Fetch bytes from the artifact store by id",
                serde_json::json!({
                    "type": "object",
                    "required": ["id"],
                    "properties": {"id": {"type": "string"}},
                }),
            ),
            (
                "search.lexical",
                "Lexical search over workspace files (contained)",
                serde_json::json!({
                    "type": "object",
                    "required": ["pattern"],
                    "properties": {"pattern": {"type": "string"}},
                }),
            ),
        ] {
            registry.register(CapabilityDescriptor {
                id: CapabilityId(id.to_owned()),
                description: description.to_owned(),
                arg_schema,
            });
        }
        registry
    }

    pub fn register(&mut self, descriptor: CapabilityDescriptor) {
        self.capabilities
            .insert(descriptor.id.0.clone(), descriptor);
    }

    #[must_use]
    pub fn get(&self, id: &CapabilityId) -> Option<&CapabilityDescriptor> {
        self.capabilities.get(&id.0)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }
}
