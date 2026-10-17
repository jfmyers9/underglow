//! Machine-local connections, never lighting setup data. Parsing and exports are inert.
use crate::setup::{Activation, SetupDocument};
use crate::source_capabilities::{self, CapabilityRegistry, SourceDescriptor};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_REGISTRY_BYTES: usize = 256 * 1024;
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectorConfig {
    pub id: String,
    pub source: String,
    pub command: Vec<String>,
    pub enabled: bool,
    pub trusted: bool,
    pub restart_initial_ms: u64,
    pub restart_max_ms: u64,
    /// Retries after the initial attempt; zero disables retries.
    pub restart_limit: u32,
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            source: String::new(),
            command: Vec::new(),
            enabled: false,
            trusted: false,
            restart_initial_ms: 1000,
            restart_max_ms: 30000,
            restart_limit: 8,
        }
    }
}

pub fn validate_producers(configs: &[ConnectorConfig]) -> Result<(), String> {
    if configs.len() > 16 {
        return Err("at most 16 managed connectors are allowed".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut sources = std::collections::BTreeSet::new();
    for c in configs {
        let token = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
        };
        if !token(&c.id) || !token(&c.source) {
            return Err("connector id/source must be 1–128 ASCII token characters".into());
        }
        if !ids.insert(&c.id) || !sources.insert(&c.source) {
            return Err("connector ids and managed sources must be unique".into());
        }
        if c.enabled && !c.trusted {
            return Err(format!(
                "connector {} must be explicitly trusted before enabling",
                c.id
            ));
        }
        if c.command.is_empty()
            || c.command.len() > 64
            || c.command[0].is_empty()
            || c.command.iter().any(|a| a.contains('\0'))
            || c.command.iter().map(String::len).sum::<usize>() > 16384
        {
            return Err(format!(
                "connector {} requires a bounded nonempty argv command (64 arguments / 16 KiB maximum)",
                c.id
            ));
        }
        if !(250..=60000).contains(&c.restart_initial_ms)
            || !(c.restart_initial_ms..=300000).contains(&c.restart_max_ms)
            || c.restart_limit > 32
        {
            return Err("connector restart_initial_ms must be 250–60000, restart_max_ms initial–300000, restart_limit 0–32".into());
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    /// Stable protocol source ID is the connection identity (one per source).
    pub descriptor: SourceDescriptor,
    /// Explicit local executable authorization, absent for external-only sources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<ConnectorConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionRegistry {
    pub schema_version: u32,
    pub revision: u64,
    pub connections: Vec<Connection>,
}

impl Default for ConnectionRegistry {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            revision: 0,
            connections: vec![Connection {
                descriptor: source_capabilities::pi_descriptor(),
                producer: None,
            }],
        }
    }
}

impl ConnectionRegistry {
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_REGISTRY_BYTES {
            return Err("connection registry exceeds 256 KiB".into());
        }
        let registry: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        registry.validate()?;
        Ok(registry)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err("unsupported connection registry schema_version".into());
        }
        if self.connections.len() > source_capabilities::MAX_SOURCES {
            return Err("at most 32 connections are allowed".into());
        }
        let mut sources = BTreeSet::new();
        for connection in &self.connections {
            connection.descriptor.validate()?;
            if !sources.insert(&connection.descriptor.source) {
                return Err("connection source identities must be unique".into());
            }
            if let Some(producer) = &connection.producer
                && producer.source != connection.descriptor.source
            {
                return Err("producer source must equal its connection identity".into());
            }
        }
        validate_producers(&self.managed_configs())?;
        // Reserve one byte for the durable file's trailing newline.
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() >= MAX_REGISTRY_BYTES {
            return Err("connection registry exceeds 256 KiB".into());
        }
        Ok(())
    }

    /// Submitted revision must match expected_revision; engine assigns the next.
    pub fn replace(&self, expected_revision: u64, candidate: &Self) -> Result<Self, String> {
        if self.revision != expected_revision || candidate.revision != expected_revision {
            return Err("connection registry revision conflict".into());
        }
        candidate.validate()?;
        let mut next = candidate.clone();
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or("connection revision overflow")?;
        next.validate()?;
        Ok(next)
    }

    pub fn connection(&self, source: &str) -> Option<&Connection> {
        self.connections
            .iter()
            .find(|c| c.descriptor.source == source)
    }

    pub fn managed_configs(&self) -> Vec<ConnectorConfig> {
        let mut configs: Vec<_> = self
            .connections
            .iter()
            .filter_map(|c| c.producer.clone())
            .collect();
        configs.sort_by(|a, b| a.id.cmp(&b.id));
        configs
    }

    /// Shared JIM-24 descriptors only, never credentials or executable details.
    pub fn capability_registry(&self) -> Result<CapabilityRegistry, String> {
        let mut registry = CapabilityRegistry::new();
        for connection in &self.connections {
            registry.insert(connection.descriptor.clone())?;
        }
        Ok(registry)
    }

    /// Missing bindings are repairable warnings; never drop or alter layers.
    pub fn missing_sources(&self, setup: &SetupDocument) -> Vec<String> {
        let mut missing = BTreeSet::new();
        for layer in &setup.layers {
            for activation in std::iter::once(&layer.activation)
                .chain(layer.variants.iter().map(|variant| &variant.activation))
            {
                let source = match activation {
                    Activation::Always {} => continue,
                    Activation::WhileState { source, .. }
                    | Activation::OnEvent { source, .. }
                    | Activation::OnTransition { source, .. } => source,
                };
                if self.connection(source).is_none() {
                    missing.insert(source.clone());
                }
            }
        }
        missing.into_iter().collect()
    }

    /// Initial startup only, for existing local user config, never setup imports.
    pub fn from_legacy(configs: &[ConnectorConfig]) -> Result<Self, String> {
        validate_producers(configs)?;
        let mut registry = Self::default();
        for config in configs {
            let mut descriptor = source_capabilities::pi_descriptor();
            if config.source != "pi" {
                descriptor.source = config.source.clone();
                descriptor.display_name = config.source.clone();
                descriptor.states.clear();
                descriptor.events.clear();
                // Unknown producers promise no Pi-specific cadence or vocabulary.
                let ttl = source_capabilities::TtlRange {
                    min_ms: 1,
                    max_ms: source_capabilities::MAX_TTL_MS,
                };
                descriptor.lifecycle.state_ttl = Some(ttl);
                descriptor.lifecycle.event_ttl = Some(ttl);
                descriptor.instances.semantics =
                    source_capabilities::InstanceSemantics::ProducerDefined;
                descriptor.lifecycle.renewal_interval_ms = None;
            }
            let connection = Connection {
                descriptor,
                producer: Some(config.clone()),
            };
            if let Some(existing) = registry
                .connections
                .iter_mut()
                .find(|c| c.descriptor.source == config.source)
            {
                *existing = connection;
            } else {
                registry.connections.push(connection);
            }
        }
        registry.validate()?;
        Ok(registry)
    }
}
