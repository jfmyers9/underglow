//! Declarative source metadata for generic editors, independent of renderers,
//! connection bindings, executable trust, and the live external registry.
//! Importing descriptors never starts a producer or activates hardware.
use crate::setup::Activation;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_DESCRIPTOR_BYTES: usize = 64 * 1024;
pub const MAX_FACTS: usize = 128;
pub const MAX_SOURCES: usize = 32;
pub const MAX_TTL_MS: u64 = 300_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDescriptor {
    pub schema_version: u32,
    /// Stable protocol source ID, never a connection or executable name.
    pub source: String,
    pub display_name: String,
    pub instances: InstanceCapabilities,
    pub lifecycle: LifecycleCapabilities,
    /// Separate namespaces: an event never implicitly declares a state.
    pub states: Vec<FactDescriptor>,
    pub events: Vec<FactDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactDescriptor {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceCapabilities {
    pub semantics: InstanceSemantics,
    pub selectors: Vec<InstanceSelector>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstanceSemantics {
    /// Unrelated, ephemeral identity per producer session, not a window/focus ID.
    Session,
    /// Identity meaning is not known to the editor.
    ProducerDefined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstanceSelector {
    Any,
    Exact,
}

/// External-v1 semantics are explicit and closed, not executable expressions.
/// An absent TTL declares no support for that fact kind. Declared TTL ranges
/// describe producer output, not a grant to change engine ingress limits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleCapabilities {
    pub presence: PresenceSemantics,
    pub absence: AbsenceSemantics,
    pub expiry: ExpirySemantics,
    pub renewal: RenewalSemantics,
    pub events: EventSemantics,
    pub removal: RemovalSemantics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_ttl: Option<TtlRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_ttl: Option<TtlRange>,
    /// Optional expected heartbeat, not a promise of delivery or availability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renewal_interval_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresenceSemantics {
    LiveStateLease,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AbsenceSemantics {
    /// Missing, expired, or removed state; not idle, failure, or success.
    NoLiveState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExpirySemantics {
    AtDeadlineNoSyntheticFact,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RenewalSemantics {
    HigherSequenceState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventSemantics {
    /// Events neither create/renew presence nor establish successful outcomes.
    OccurrenceOnly,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemovalSemantics {
    DropsStateKeepsEvents,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TtlRange {
    pub min_ms: u64,
    pub max_ms: u64,
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
}

fn label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn bound_input(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_DESCRIPTOR_BYTES {
        return Err("source descriptor exceeds 64 KiB".into());
    }
    Ok(())
}

impl SourceDescriptor {
    pub fn from_toml(input: &str) -> Result<Self, String> {
        bound_input(input.as_bytes())?;
        let descriptor: Self = toml::from_str(input).map_err(|e| e.to_string())?;
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn from_json(input: &[u8]) -> Result<Self, String> {
        bound_input(input)?;
        let descriptor: Self = serde_json::from_slice(input).map_err(|e| e.to_string())?;
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn to_toml(&self) -> Result<String, String> {
        self.validate()?;
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        bound_input(text.as_bytes())?;
        Ok(text)
    }

    /// Also required after raw serde or direct Rust construction.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err("unsupported source capability schema_version".into());
        }
        if !token(&self.source) || !label(&self.display_name) {
            return Err("source requires a bounded ID and display name".into());
        }
        let selectors = &self.instances.selectors;
        if selectors.is_empty()
            || selectors.len() > 2
            || selectors.iter().collect::<BTreeSet<_>>().len() != selectors.len()
        {
            return Err("instance selectors must be nonempty and unique".into());
        }
        for (facts, ttl) in [
            (&self.states, self.lifecycle.state_ttl),
            (&self.events, self.lifecycle.event_ttl),
        ] {
            if facts.len() > MAX_FACTS {
                return Err("at most 128 facts per kind are supported".into());
            }
            let mut ids = BTreeSet::new();
            for fact in facts {
                if !token(&fact.id) || !label(&fact.label) || !ids.insert(&fact.id) {
                    return Err("facts require unique bounded IDs and display labels".into());
                }
            }
            if !facts.is_empty() && ttl.is_none() {
                return Err("declared facts require TTL semantics".into());
            }
            if ttl.is_some_and(|t| t.min_ms == 0 || t.min_ms > t.max_ms || t.max_ms > MAX_TTL_MS) {
                return Err("TTL ranges must be within 1..300000 ms".into());
            }
        }
        if let Some(interval) = self.lifecycle.renewal_interval_ms
            && (interval == 0
                || self
                    .lifecycle
                    .state_ttl
                    .is_none_or(|ttl| interval >= ttl.min_ms))
        {
            return Err("renewal interval must be positive and below minimum state TTL".into());
        }
        bound_input(&serde_json::to_vec(self).map_err(|e| e.to_string())?)
    }
}

/// The verified connector vocabulary, not inferred from visuals or observations.
pub fn pi_descriptor() -> SourceDescriptor {
    SourceDescriptor::from_toml(include_str!("../examples/pi-source-capabilities.toml"))
        .expect("built-in Pi descriptor is valid")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactKind {
    State,
    Event,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceOrigin {
    Declared,
    /// Seen once, not guaranteed/supported; never upgrades descriptor claims.
    UnknownObserved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityChoice {
    pub id: String,
    pub label: String,
    pub origin: ChoiceOrigin,
}

/// Semantic diagnostics are repairable: retain the original activation. They
/// are separate from Activation's structural validation errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapabilityWarning {
    MissingDescriptor { source: String },
    UnsupportedInstanceSelector { selector: InstanceSelector },
    UnsupportedPresence,
    UndeclaredState { state: String },
    UndeclaredEvent { event: String },
}

/// Bounded metadata only. Connection bindings and live source availability are
/// deliberately not tracked here. Default/new is empty, not an invented catalog.
#[derive(Clone, Debug, Default)]
pub struct CapabilityRegistry {
    descriptors: BTreeMap<String, SourceDescriptor>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.insert(pi_descriptor()).expect("valid built-in");
        registry
    }

    /// Reject duplicates/capacity errors without modifying the existing entry.
    pub fn insert(&mut self, descriptor: SourceDescriptor) -> Result<(), String> {
        descriptor.validate()?;
        if self.descriptors.contains_key(&descriptor.source) {
            return Err("source descriptor already registered".into());
        }
        if self.descriptors.len() >= MAX_SOURCES {
            return Err("at most 32 source descriptors are supported".into());
        }
        self.descriptors
            .insert(descriptor.source.clone(), descriptor);
        Ok(())
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &SourceDescriptor> {
        self.descriptors.values()
    }

    pub fn descriptor(&self, source: &str) -> Option<&SourceDescriptor> {
        self.descriptors.get(source)
    }

    /// Supply only observations for this source AND fact kind. Observations are
    /// optional, bounded, de-duplicated, and never stored in the registry.
    pub fn choices(
        &self,
        source: &str,
        kind: FactKind,
        observed: &[String],
    ) -> Result<Vec<CapabilityChoice>, String> {
        if !token(source) || observed.len() > MAX_FACTS || observed.iter().any(|v| !token(v)) {
            return Err("choices require a bounded source and at most 128 observed tokens".into());
        }
        let facts = self.descriptor(source).map(|descriptor| match kind {
            FactKind::State => &descriptor.states,
            FactKind::Event => &descriptor.events,
        });
        let mut choices = Vec::new();
        let mut seen = BTreeSet::new();
        for fact in facts.into_iter().flatten() {
            seen.insert(fact.id.as_str());
            choices.push(CapabilityChoice {
                id: fact.id.clone(),
                label: fact.label.clone(),
                origin: ChoiceOrigin::Declared,
            });
        }
        for value in observed {
            if seen.insert(value.as_str()) {
                choices.push(CapabilityChoice {
                    id: value.clone(),
                    label: value.clone(),
                    origin: ChoiceOrigin::UnknownObserved,
                });
            }
        }
        Ok(choices)
    }

    /// Offline sources cannot be diagnosed from metadata. A missing descriptor
    /// or undeclared fact yields a warning, never drops or rewrites the selector.
    pub fn validate_activation(
        &self,
        activation: &Activation,
    ) -> Result<Vec<CapabilityWarning>, String> {
        activation.validate()?;
        let (source, instance) = match activation {
            Activation::Always {} => return Ok(Vec::new()),
            Activation::WhileState {
                source, instance, ..
            }
            | Activation::OnEvent {
                source, instance, ..
            }
            | Activation::OnTransition {
                source, instance, ..
            } => (source, instance),
        };
        let Some(descriptor) = self.descriptor(source) else {
            return Ok(vec![CapabilityWarning::MissingDescriptor {
                source: source.clone(),
            }]);
        };
        let mut warnings = Vec::new();
        let selector = if instance.is_some() {
            InstanceSelector::Exact
        } else {
            InstanceSelector::Any
        };
        if !descriptor.instances.selectors.contains(&selector) {
            warnings.push(CapabilityWarning::UnsupportedInstanceSelector { selector });
        }
        let mut states = Vec::new();
        match activation {
            Activation::WhileState { state, .. } => {
                if state.is_none() && descriptor.lifecycle.state_ttl.is_none() {
                    warnings.push(CapabilityWarning::UnsupportedPresence);
                }
                states.extend(state.iter());
            }
            Activation::OnEvent { event, .. } => {
                if !descriptor.events.iter().any(|fact| &fact.id == event) {
                    warnings.push(CapabilityWarning::UndeclaredEvent {
                        event: event.clone(),
                    });
                }
            }
            Activation::OnTransition {
                from_state, state, ..
            } => {
                states.extend(from_state.iter());
                states.push(state);
            }
            Activation::Always {} => unreachable!(),
        }
        let mut checked = BTreeSet::new();
        for state in states {
            if checked.insert(state) && !descriptor.states.iter().any(|fact| &fact.id == state) {
                warnings.push(CapabilityWarning::UndeclaredState {
                    state: state.clone(),
                });
            }
        }
        Ok(warnings)
    }
}
