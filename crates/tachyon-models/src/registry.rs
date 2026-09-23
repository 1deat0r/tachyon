//! Provider registry and role mapping (spec §25).
//!
//! Configuration maps roles (`fast`, `primary`, `specialist`, `vision`) to a
//! provider id plus model name. [`ModelRegistry::select`] resolves a role to
//! a serving provider, but only when that provider's capabilities satisfy the
//! call's [`CapabilityRequirements`]. An unmapped role falls back to the
//! first capable provider in registration order — registration order is the
//! operator's cheapest-sufficient preference.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tachyon_types::ProviderId;

use crate::{CapabilityRequirements, ContextBlock, ModelProvider, ModelRequest};

/// A reasoning role. Configuration binds these to providers; core behavior
/// never branches on provider names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Cheap, fast triage and classification-adjacent calls.
    Fast,
    /// Default reasoning workhorse.
    #[default]
    Primary,
    /// Deep reasoning, migration-scale or architecture calls.
    Specialist,
    /// Calls whose context carries image blocks.
    Vision,
}

/// Role-to-provider binding, loaded from configuration.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleMap {
    /// Role to (provider id, model name).
    bindings: HashMap<Role, (ProviderId, String)>,
}

impl RoleMap {
    /// Creates an empty map; unmapped roles use registry fallback.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds `role` to a provider id and model name.
    pub fn bind(&mut self, role: Role, provider: ProviderId, model: &str) {
        self.bindings.insert(role, (provider, model.to_owned()));
    }

    /// Looks up the binding for `role`, if any.
    #[must_use]
    pub fn get(&self, role: Role) -> Option<&(ProviderId, String)> {
        self.bindings.get(&role)
    }
}

/// One provider plus the models and roles it serves.
pub struct RegisteredProvider {
    /// The serving backend.
    pub provider: Arc<dyn ModelProvider>,
    /// Model names this registration serves; empty means any model the
    /// provider accepts.
    pub models: Vec<String>,
    /// Roles this registration is eligible for.
    pub roles: Vec<Role>,
}

/// Selectable provider: backend plus the concrete model to request. The role
/// travels with the selection so callers cannot hand-copy a mismatched
/// model/role pair into the request.
#[derive(Clone)]
pub struct SelectedProvider {
    /// The serving backend.
    pub provider: Arc<dyn ModelProvider>,
    /// The concrete model name to request.
    pub model: String,
    /// The role this selection serves.
    pub role: Role,
}

impl SelectedProvider {
    /// Builds the request this selection serves. Model and role travel
    /// together from [`ModelRegistry::select`]; there is no separate
    /// hand-copy step to get wrong.
    #[must_use]
    pub fn into_request(
        self,
        context: Vec<ContextBlock>,
        max_output_tokens: u32,
        require_structured_output: bool,
    ) -> ModelRequest {
        ModelRequest {
            role: self.role,
            model: self.model,
            context,
            max_output_tokens,
            require_structured_output,
        }
    }
}

/// Registry of model backends with role bindings.
#[derive(Default)]
pub struct ModelRegistry {
    providers: Vec<RegisteredProvider>,
    roles: RoleMap,
}

impl ModelRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a provider. Order is preference order for fallback.
    pub fn register(&mut self, provider: RegisteredProvider) {
        self.providers.push(provider);
    }

    /// Replaces the role map (typically from configuration).
    pub fn set_roles(&mut self, roles: RoleMap) {
        self.roles = roles;
    }

    /// Resolves `role` to a capable provider for `requirements`.
    ///
    /// Prefers the configured binding when its provider is registered and
    /// capable; otherwise falls back to the first capable registration that
    /// lists the role, then to any capable provider. Returns `None` when no
    /// registered provider satisfies the requirements — the caller routes
    /// around models rather than failing the harness.
    #[must_use]
    pub fn select(
        &self,
        role: Role,
        requirements: CapabilityRequirements,
    ) -> Option<SelectedProvider> {
        if let Some((id, model)) = self.roles.get(role)
            && let Some(hit) = self
                .providers
                .iter()
                .filter(|entry| entry.roles.contains(&role) || entry.roles.is_empty())
                .find(|entry| {
                    entry.provider.id() == *id
                        && requirements.satisfied_by(&entry.provider.capabilities())
                        && (entry.models.is_empty() || entry.models.contains(model))
                })
        {
            return Some(SelectedProvider {
                provider: Arc::clone(&hit.provider),
                model: model.clone(),
                role,
            });
        }
        self.providers
            .iter()
            .filter(|entry| {
                requirements.satisfied_by(&entry.provider.capabilities())
                    && (entry.roles.is_empty() || entry.roles.contains(&role))
            })
            .map(|entry| SelectedProvider {
                provider: Arc::clone(&entry.provider),
                model: fallback_model(entry),
                role,
            })
            .next()
            .or_else(|| {
                self.providers
                    .iter()
                    .filter(|entry| requirements.satisfied_by(&entry.provider.capabilities()))
                    .map(|entry| SelectedProvider {
                        provider: Arc::clone(&entry.provider),
                        model: fallback_model(entry),
                        role,
                    })
                    .next()
            })
    }
}

/// Default model name for a registration: first listed, else empty (the
/// provider decides its default).
fn fallback_model(entry: &RegisteredProvider) -> String {
    entry.models.first().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeModelProvider;

    fn registered(id: &str, roles: Vec<Role>, models: &[&str]) -> RegisteredProvider {
        RegisteredProvider {
            provider: Arc::new(FakeModelProvider::new(ProviderId(id.to_owned()))),
            models: models.iter().map(ToString::to_string).collect(),
            roles,
        }
    }

    #[test]
    fn binding_wins_when_capable() {
        let mut registry = ModelRegistry::new();
        registry.register(registered("local", vec![Role::Primary], &["tiny"]));
        let mut roles = RoleMap::new();
        roles.bind(Role::Primary, ProviderId("local".to_owned()), "tiny");
        registry.set_roles(roles);
        let hit = registry
            .select(Role::Primary, CapabilityRequirements::default())
            .expect("bound provider");
        assert_eq!(hit.provider.id().0, "local");
        assert_eq!(hit.model, "tiny");
    }

    #[test]
    fn incapable_binding_falls_back_or_misses() {
        let mut registry = ModelRegistry::new();
        registry.register(registered("local", vec![Role::Primary], &[]));
        let mut roles = RoleMap::new();
        roles.bind(Role::Primary, ProviderId("local".to_owned()), "tiny");
        registry.set_roles(roles);
        // The fake has no vision: binding cannot serve vision calls.
        let vision = CapabilityRequirements {
            need_vision: true,
            ..CapabilityRequirements::default()
        };
        assert!(registry.select(Role::Vision, vision).is_none());
        // …but the unmapped primary role still resolves by fallback.
        assert!(
            registry
                .select(Role::Primary, CapabilityRequirements::default())
                .is_some()
        );
    }

    #[test]
    fn empty_registry_selects_nothing() {
        let registry = ModelRegistry::new();
        assert!(
            registry
                .select(Role::Primary, CapabilityRequirements::default())
                .is_none()
        );
    }

    #[test]
    fn impossible_requirements_select_nothing() {
        let mut registry = ModelRegistry::new();
        registry.register(registered("local", vec![Role::Primary], &[]));
        let requirements = CapabilityRequirements {
            min_context_window_tokens: 1_000_000,
            ..CapabilityRequirements::default()
        };
        assert!(registry.select(Role::Primary, requirements).is_none());
    }

    #[test]
    fn selection_builds_its_own_request() {
        let mut registry = ModelRegistry::new();
        registry.register(registered("local", vec![Role::Primary], &["tiny"]));
        let mut roles = RoleMap::new();
        roles.bind(Role::Primary, ProviderId("local".to_owned()), "tiny");
        registry.set_roles(roles);
        let hit = registry
            .select(Role::Primary, CapabilityRequirements::default())
            .expect("bound provider");
        let request = hit.into_request(vec![], 64, false);
        assert_eq!(request.model, "tiny");
        assert_eq!(request.role, Role::Primary);
        assert_eq!(request.max_output_tokens, 64);
    }
}
