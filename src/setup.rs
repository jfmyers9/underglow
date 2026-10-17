//! Canonical, declarative lighting setups. Parsing/validation never performs I/O,
//! loads an SDK, or grants trust to executable connectors.
//!
//! Layers are stored **bottom to top**. Higher enabled, active layers replace
//! lower layers on their target keys, including black pixels. Keys uncovered by
//! all layers are black while the engine owns lighting; pause releases ownership.
//! Activation is separate from rendering so future logical behaviors can select
//! between appearance variants without adding connector-specific renderers.
use crate::catalog::{self, PaletteControl, RendererKind};
use crate::effects::EffectKind;
use crate::key_targets::canonical_coord;
use crate::layout::Zone;
use crate::reactive::ReactiveKind;
use crate::render::PaletteName;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_DOCUMENT_BYTES: usize = 256 * 1024;
pub const MAX_LAYERS: usize = 64;
pub const MAX_BEHAVIOR_VARIANTS: usize = 16;
pub const MAX_GROUPS: usize = 64;
pub const MAX_KEYS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupDocument {
    pub schema_version: u32,
    /// Optimistic editing version; handwritten documents begin at zero.
    #[serde(default)]
    pub revision: u64,
    /// Stable identity, independent of the editable display name or file path.
    pub id: String,
    pub name: String,
    /// Bottom to top; order, not the ID or name, determines visual precedence.
    pub layers: Vec<Layer>,
    /// Flat reusable key groups. References and physical key names are distinct.
    #[serde(default)]
    pub groups: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub id: String,
    pub name: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    pub renderer: Renderer,
    #[serde(default = "brightness")]
    pub brightness: u8,
    pub target: Target,
    #[serde(default)]
    pub activation: Activation,
    /// Mutually exclusive appearances sharing this logical layer's target.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<BehaviorVariant>,
}

/// Transients outrank persistent variants; within either class the last active
/// declaration wins. The layer's original appearance is the final fallback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BehaviorVariant {
    pub id: String,
    pub name: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    pub activation: Activation,
    pub renderer: Renderer,
    /// None inherits the logical layer's brightness (not an extra multiplier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<u8>,
}

fn enabled() -> bool {
    true
}
fn brightness() -> u8 {
    crate::DEFAULT_BRIGHTNESS
}
fn speed() -> u32 {
    crate::animation::DEFAULT_SPEED
}
fn period() -> u64 {
    1000
}
fn focus_minutes() -> u64 {
    25
}
fn break_minutes() -> u64 {
    5
}
fn cycles() -> u32 {
    1
}

/// Typed settings prevent nonsensical combinations (for example speed on a
/// reactive effect). Missing palettes use that renderer's catalog default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Renderer {
    Effect {
        effect: EffectKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        palette: Option<PaletteName>,
        #[serde(default = "speed")]
        speed: u32,
    },
    Solid {
        color: [u8; 3],
    },
    Pulse {
        color: [u8; 3],
        #[serde(default = "period")]
        period_ms: u64,
    },
    Sweep {
        color: [u8; 3],
        #[serde(default = "period")]
        period_ms: u64,
    },
    Ripples {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        palette: Option<PaletteName>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_color: Option<[u8; 3]>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ripple_color: Option<[u8; 3]>,
    },
    Reactive {
        effect: ReactiveKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        palette: Option<PaletteName>,
    },
    Focus {
        #[serde(default = "focus_minutes")]
        focus_minutes: u64,
        #[serde(default = "break_minutes")]
        break_minutes: u64,
        #[serde(default = "cycles")]
        cycles: u32,
        #[serde(default)]
        start_paused: bool,
        #[serde(default)]
        meeting_safe: bool,
        #[serde(default)]
        dim: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Target {
    Whole {},
    /// Union of all selectors. Empty is invalid, never implicitly whole.
    Selection {
        #[serde(default)]
        zones: Vec<TargetZone>,
        #[serde(default)]
        keys: Vec<String>,
        #[serde(default)]
        groups: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetZone {
    Function,
    Alpha,
    Navigation,
    Arrows,
    System,
}

impl From<TargetZone> for Zone {
    fn from(zone: TargetZone) -> Self {
        match zone {
            TargetZone::Function => Self::Function,
            TargetZone::Alpha => Self::Alpha,
            TargetZone::Navigation => Self::Navigation,
            TargetZone::Arrows => Self::Arrows,
            TargetZone::System => Self::System,
        }
    }
}

/// External names are fact selectors, not executable/connection definitions.
/// Absent instance selects any instance; absent state selects source presence.
/// Unavailable sources hide conditional layers rather than invalidating setups.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Activation {
    Always {},
    WhileState {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<String>,
    },
    OnEvent {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance: Option<String>,
        event: String,
        duration_ms: u64,
    },
    OnTransition {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_state: Option<String>,
        state: String,
        duration_ms: u64,
    },
}

impl Default for Activation {
    fn default() -> Self {
        Self::Always {}
    }
}

fn label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn token(value: &str) -> bool {
    label(value)
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
}

fn validate_keys(keys: &[String]) -> Result<(), String> {
    if keys.len() > MAX_KEYS {
        return Err("at most 128 physical keys are supported".into());
    }
    for key in keys {
        if key.len() > 32 || canonical_coord(key).is_none() {
            return Err(format!(
                "unknown or unsupported physical key {key:?} (80HE ANSI)"
            ));
        }
    }
    Ok(())
}

impl SetupDocument {
    /// Empty stacks are valid: the result is black, not an implicit background.
    /// Disabled layers are validated too, so enabling one cannot reveal broken
    /// references or ignored settings later.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported setup schema_version {}; expected {SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if !token(&self.id) || !label(&self.name) {
            return Err("setup requires a bounded ID and display name".into());
        }
        if self.layers.len() > MAX_LAYERS || self.groups.len() > MAX_GROUPS {
            return Err("setup supports at most 64 layers and 64 groups".into());
        }
        for (id, keys) in &self.groups {
            if !token(id) || keys.is_empty() {
                return Err(format!(
                    "group {id:?} requires a valid ID and nonempty keys"
                ));
            }
            validate_keys(keys).map_err(|e| format!("group {id}: {e}"))?;
        }
        let mut ids = BTreeSet::new();
        for layer in &self.layers {
            if !token(&layer.id) || !label(&layer.name) || !ids.insert(&layer.id) {
                return Err("layers require unique bounded IDs and display names".into());
            }
            let check = || {
                layer.renderer.validate()?;
                layer.activation.validate()?;
                if layer.variants.len() > MAX_BEHAVIOR_VARIANTS {
                    return Err("at most 16 behavior variants per layer are supported".into());
                }
                let mut variant_ids = BTreeSet::new();
                for variant in &layer.variants {
                    if !token(&variant.id)
                        || !label(&variant.name)
                        || !variant_ids.insert(&variant.id)
                    {
                        return Err("variants require unique bounded IDs and display names".into());
                    }
                    variant
                        .activation
                        .validate()
                        .map_err(|e| format!("variant {}: {e}", variant.id))?;
                    variant
                        .renderer
                        .validate()
                        .map_err(|e| format!("variant {}: {e}", variant.id))?;
                }
                if let Target::Selection {
                    zones,
                    keys,
                    groups,
                } = &layer.target
                {
                    if zones.len() > 5
                        || groups.len() > MAX_GROUPS
                        || (zones.is_empty() && keys.is_empty() && groups.is_empty())
                    {
                        return Err(
                            "selection requires keys/zones/groups (at most 5 zones and 64 groups)"
                                .into(),
                        );
                    }
                    validate_keys(keys)?;
                    for id in groups {
                        if !self.groups.contains_key(id) {
                            return Err(format!("unknown group {id:?}"));
                        }
                    }
                }
                Ok(())
            };
            check().map_err(|e: String| format!("layer {}: {e}", layer.id))?;
        }
        // Also bound documents constructed through Rust or JSON, not just TOML
        // parsing. This check is nonrecursive and only serializes bounded types.
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_DOCUMENT_BYTES {
            return Err("setup exceeds 256 KiB".into());
        }
        Ok(())
    }

    pub fn from_toml(input: &str) -> Result<Self, String> {
        if input.len() > MAX_DOCUMENT_BYTES {
            return Err("setup exceeds 256 KiB".into());
        }
        let document: Self = toml::from_str(input).map_err(|e| e.to_string())?;
        document.validate()?;
        Ok(document)
    }

    pub fn to_toml(&self) -> Result<String, String> {
        self.validate()?;
        let output = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        if output.len() > MAX_DOCUMENT_BYTES {
            return Err("setup exceeds 256 KiB".into());
        }
        Ok(output)
    }

    /// Parse and validate before assignment: a rejected edit/import leaves the
    /// last valid document unchanged. No engine or hardware activation occurs.
    pub fn replace_from_toml(&mut self, input: &str) -> Result<(), String> {
        let candidate = Self::from_toml(input)?;
        *self = candidate;
        Ok(())
    }
}

impl Renderer {
    /// Stable existing catalog identity, or None for simple scene primitives.
    pub fn catalog_id(&self) -> Option<&'static str> {
        let kind = match self {
            Self::Effect { effect, .. } => RendererKind::Static(*effect),
            Self::Ripples { .. } => RendererKind::Ripples,
            Self::Reactive { effect, .. } => RendererKind::Reactive(*effect),
            Self::Focus { .. } => RendererKind::Focus,
            _ => return None,
        };
        catalog::all()
            .iter()
            .find(|v| v.renderer == kind)
            .map(|v| v.id)
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Effect {
                effect,
                palette,
                speed,
            } => {
                let entry = catalog::all()
                    .iter()
                    .find(|entry| entry.renderer == RendererKind::Static(*effect))
                    .ok_or("unregistered effect")?;
                if !(10..=400).contains(speed)
                    || (!entry.supports_speed && *speed != crate::animation::DEFAULT_SPEED)
                {
                    return Err("effect speed must be 10..400 and supported by the renderer".into());
                }
                if palette.is_some() && entry.palette == PaletteControl::Fixed {
                    return Err("this effect has fixed colors; palette is unsupported".into());
                }
            }
            Self::Ripples {
                palette,
                base_color,
                ripple_color,
            } => {
                if base_color.is_some() != ripple_color.is_some()
                    || (base_color.is_some() && palette.is_some())
                {
                    return Err(
                        "ripples require both two-tone colors or a palette, not both".into(),
                    );
                }
            }
            Self::Pulse { period_ms, .. } | Self::Sweep { period_ms, .. } => {
                if !(1..=60_000).contains(period_ms) {
                    return Err("period_ms must be 1..60000".into());
                }
            }
            Self::Focus {
                focus_minutes,
                break_minutes,
                cycles,
                ..
            } => {
                if !(1..=1440).contains(focus_minutes)
                    || *break_minutes > 1440
                    || !(1..=100).contains(cycles)
                {
                    return Err("focus requires 1..1440 focus minutes, 0..1440 break minutes, and 1..100 cycles".into());
                }
            }
            Self::Reactive { .. } | Self::Solid { .. } => {}
        }
        Ok(())
    }
}

impl Activation {
    pub fn validate(&self) -> Result<(), String> {
        let (source, instance, values, duration) = match self {
            Self::Always {} => return Ok(()),
            Self::WhileState {
                source,
                instance,
                state,
            } => (source, instance, vec![state.as_deref()], None),
            Self::OnEvent {
                source,
                instance,
                event,
                duration_ms,
            } => (
                source,
                instance,
                vec![Some(event.as_str())],
                Some(*duration_ms),
            ),
            Self::OnTransition {
                source,
                instance,
                from_state,
                state,
                duration_ms,
            } => (
                source,
                instance,
                vec![from_state.as_deref(), Some(state.as_str())],
                Some(*duration_ms),
            ),
        };
        if !token(source)
            || instance.as_deref().is_some_and(|v| !token(v))
            || values.into_iter().flatten().any(|v| !token(v))
        {
            return Err(
                "source, instance, state and event selectors must be bounded tokens".into(),
            );
        }
        if duration.is_some_and(|d| !(1..=60_000).contains(&d)) {
            return Err("event/transition duration_ms must be 1..60000".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../examples/layer-setup.toml");

    fn example() -> SetupDocument {
        SetupDocument::from_toml(EXAMPLE).unwrap()
    }

    fn variant() -> BehaviorVariant {
        BehaviorVariant {
            id: "working".into(),
            name: "Working".into(),
            enabled: true,
            activation: Activation::WhileState {
                source: "custom-source".into(),
                instance: None,
                state: Some("working".into()),
            },
            renderer: Renderer::Solid { color: [1, 2, 3] },
            brightness: None,
        }
    }

    #[test]
    fn behavior_variants_roundtrip_and_old_documents_remain_canonical() {
        let mut doc = example();
        assert!(doc.layers.iter().all(|layer| layer.variants.is_empty()));
        assert!(!doc.to_toml().unwrap().contains("variants"));
        doc.layers[0].variants = vec![
            variant(),
            BehaviorVariant {
                id: "finished".into(),
                brightness: Some(128),
                activation: Activation::OnEvent {
                    source: "custom-source".into(),
                    instance: Some("session".into()),
                    event: "finished".into(),
                    duration_ms: 500,
                },
                ..variant()
            },
        ];
        let encoded = doc.to_toml().unwrap();
        assert_eq!(SetupDocument::from_toml(&encoded).unwrap(), doc);
        assert_eq!(
            serde_json::from_str::<SetupDocument>(&serde_json::to_string(&doc).unwrap()).unwrap(),
            doc
        );
        let mut json = serde_json::to_value(&doc).unwrap();
        json["layers"][0]["variants"][0]
            .as_object_mut()
            .unwrap()
            .remove("enabled");
        assert!(
            serde_json::from_value::<SetupDocument>(json.clone())
                .unwrap()
                .layers[0]
                .variants[0]
                .enabled
        );
        for field in ["target", "command", "unknown"] {
            json["layers"][0]["variants"][0][field] = serde_json::json!("forbidden");
            assert!(serde_json::from_value::<SetupDocument>(json.clone()).is_err());
            json["layers"][0]["variants"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
        }
        json["layers"][0]["variants"][0]["brightness"] = serde_json::json!(256);
        assert!(serde_json::from_value::<SetupDocument>(json).is_err());
    }

    #[test]
    fn documented_behavior_setup_is_valid_with_one_editable_status_layer() {
        let docs = include_str!("../docs/setup-format.md");
        let example = docs
            .split("```toml\n")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap();
        let doc = SetupDocument::from_toml(example).unwrap();
        assert_eq!(doc.layers.len(), 2);
        assert_eq!(doc.layers[1].variants.len(), 3);
        assert_eq!(
            SetupDocument::from_toml(&doc.to_toml().unwrap()).unwrap(),
            doc
        );
    }

    #[test]
    fn behavior_variants_are_bounded_strict_and_validate_disabled_appearances() {
        let mut doc = example();
        doc.layers[0].variants = (0..MAX_BEHAVIOR_VARIANTS)
            .map(|i| BehaviorVariant {
                id: format!("v{i}"),
                ..variant()
            })
            .collect();
        doc.validate().unwrap();
        doc.layers[0].variants.push(variant());
        assert!(doc.validate().unwrap_err().contains("16"));
        doc.layers[0].variants = vec![variant(), variant()];
        assert!(doc.validate().unwrap_err().contains("unique"));
        for bad in ["", "not an id", &"x".repeat(129)] {
            doc.layers[0].variants = vec![BehaviorVariant {
                id: bad.into(),
                ..variant()
            }];
            assert!(doc.validate().is_err());
        }
        doc.layers[0].variants = vec![BehaviorVariant {
            enabled: false,
            activation: Activation::OnEvent {
                source: "custom-source".into(),
                instance: None,
                event: "done".into(),
                duration_ms: 60_001,
            },
            ..variant()
        }];
        assert!(doc.validate().unwrap_err().contains("duration"));
        doc.layers[0].variants[0].activation = Activation::Always {};
        doc.layers[0].variants[0].renderer = Renderer::Pulse {
            color: [1; 3],
            period_ms: 0,
        };
        assert!(doc.validate().is_err());
    }

    #[test]
    fn multi_layer_roundtrip_preserves_ids_order_settings_and_revision() {
        let mut doc = example();
        doc.revision = 42;
        assert_eq!(
            doc.layers.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
            ["ambient", "pi-working"]
        );
        assert_eq!(doc.layers[0].renderer.catalog_id(), Some("matrix"));
        assert_eq!(doc.layers[0].brightness, 160);
        let encoded = doc.to_toml().unwrap();
        assert_eq!(SetupDocument::from_toml(&encoded).unwrap(), doc);
        let json = serde_json::to_string(&doc).unwrap();
        let restored: SetupDocument = serde_json::from_str(&json).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored, doc);
    }

    #[test]
    fn renderer_and_activation_variants_roundtrip_without_losing_settings() {
        let renderers = [
            Renderer::Solid { color: [0, 0, 0] },
            Renderer::Pulse {
                color: [1, 2, 3],
                period_ms: 500,
            },
            Renderer::Ripples {
                palette: None,
                base_color: Some([0, 32, 64]),
                ripple_color: Some([100, 255, 255]),
            },
            Renderer::Ripples {
                palette: Some(PaletteName::Heat),
                base_color: None,
                ripple_color: None,
            },
            Renderer::Reactive {
                effect: ReactiveKind::Afterimage,
                palette: Some(PaletteName::Ocean),
            },
            Renderer::Focus {
                focus_minutes: 45,
                break_minutes: 0,
                cycles: 4,
                start_paused: true,
                meeting_safe: true,
                dim: true,
            },
            Renderer::Effect {
                effect: EffectKind::Aurora,
                palette: Some(PaletteName::Ocean),
                speed: 230,
            },
        ];
        let activations = [
            Activation::Always {},
            Activation::WhileState {
                source: "pi".into(),
                instance: None,
                state: None,
            },
            Activation::OnEvent {
                source: "ci".into(),
                instance: Some("build-1".into()),
                event: "finished".into(),
                duration_ms: 2000,
            },
            Activation::OnTransition {
                source: "pi".into(),
                instance: Some("session-1".into()),
                from_state: Some("working".into()),
                state: "idle".into(),
                duration_ms: 1000,
            },
        ];
        for renderer in renderers {
            for activation in &activations {
                let mut doc = example();
                doc.layers[0].renderer = renderer.clone();
                doc.layers[0].activation = activation.clone();
                assert_eq!(
                    SetupDocument::from_toml(&doc.to_toml().unwrap()).unwrap(),
                    doc
                );
            }
        }
    }

    #[test]
    fn invalid_references_never_replace_last_valid_setup() {
        let mut doc = example();
        let before = doc.clone();
        for invalid in [
            EXAMPLE.replace("groups = [\"agent-indicators\"]", "groups = [\"missing\"]"),
            EXAMPLE.replace("\"F1\"", "\"Fn\""),
            EXAMPLE.replace(
                "effect = \"matrix\"",
                "effect = \"matrix\"\npalette = \"ocean\"",
            ),
            EXAMPLE.replace("schema_version = 1", "schema_version = 2"),
            "not a document".into(),
        ] {
            assert!(doc.replace_from_toml(&invalid).is_err(), "{invalid}");
            assert_eq!(doc, before);
        }
        let changed = EXAMPLE.replace("speed = 100", "speed = 200");
        doc.replace_from_toml(&changed).unwrap();
        assert_ne!(doc, before);
    }

    #[test]
    fn defaults_are_explicit_after_serialization_not_injected_layers() {
        let doc = SetupDocument::from_toml("schema_version=1\nid='blank'\nname='Blank'\nlayers=[]")
            .unwrap();
        assert!(doc.layers.is_empty());
        assert_eq!(doc.revision, 0);
        assert!(
            SetupDocument::from_toml(&doc.to_toml().unwrap())
                .unwrap()
                .layers
                .is_empty()
        );
        let doc = SetupDocument::from_toml("schema_version=1\nid='test'\nname='Test'\n[[layers]]\nid='a'\nname='A'\nrenderer={kind='solid',color=[0,0,0]}\ntarget={kind='whole'}").unwrap();
        assert!(doc.layers[0].enabled);
        assert_eq!(doc.layers[0].activation, Activation::Always {});
        assert_eq!(doc.layers[0].brightness, 255);
    }

    #[test]
    fn rejects_unknown_settings_and_executable_data() {
        for invalid in [
            EXAMPLE.replace("speed = 100", "speed = 100\ncommand = 'sh'"),
            EXAMPLE.replace("kind = \"whole\"", "kind = \"whole\"\nkeys=['F1']"),
            EXAMPLE.replace("source = \"pi\"", "source = \"pi\"\nexecutable = 'pi'"),
            EXAMPLE.replace("kind = \"sweep\"", "kind = \"reactive\""),
            EXAMPLE.replace("color = [0, 100, 255]", "color = [0, 100, 256]"),
            EXAMPLE.replace("color = [0, 100, 255]", "color = [0, 100]"),
            EXAMPLE.replace(
                "name = \"Matrix with agent activity\"",
                "name = \"Matrix with agent activity\"\nsdk_path = 'anything'",
            ),
        ] {
            assert!(SetupDocument::from_toml(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn rejects_duplicate_ids_and_invalid_disabled_layers() {
        let mut doc = example();
        doc.layers[1].id = doc.layers[0].id.clone();
        assert!(doc.validate().unwrap_err().contains("unique"));
        doc.layers[1].id = "other".into();
        doc.layers[1].enabled = false;
        doc.groups.clear();
        assert!(doc.validate().unwrap_err().contains("unknown group"));
    }

    #[test]
    fn bounds_collections_names_and_text() {
        let mut doc = example();
        doc.layers = vec![doc.layers[0].clone(); MAX_LAYERS + 1];
        assert!(doc.validate().is_err());
        let mut doc = example();
        doc.groups
            .insert("too-many".into(), vec!["F1".into(); MAX_KEYS + 1]);
        assert!(doc.validate().is_err());
        let mut doc = example();
        doc.groups = (0..=MAX_GROUPS)
            .map(|i| (i.to_string(), vec!["F1".into()]))
            .collect();
        assert!(doc.validate().is_err());
        for bad_id in ["", "spaces not allowed", "bad\n", &"a".repeat(129)] {
            let mut doc = example();
            doc.id = bad_id.into();
            assert!(doc.validate().is_err());
        }
        assert!(SetupDocument::from_toml(&" ".repeat(MAX_DOCUMENT_BYTES + 1)).is_err());
    }

    #[test]
    fn empty_selection_never_means_whole_keyboard() {
        let mut doc = example();
        doc.layers[0].target = Target::Selection {
            zones: vec![],
            keys: vec![],
            groups: vec![],
        };
        assert!(doc.validate().is_err());
        doc.layers[0].target = Target::Selection {
            zones: vec![TargetZone::Arrows],
            keys: vec!["Escape".into()],
            groups: vec!["agent-indicators".into()],
        };
        doc.validate().unwrap();
    }

    #[test]
    fn renderer_settings_follow_catalog_capabilities() {
        for entry in catalog::all() {
            let renderer = match entry.renderer {
                RendererKind::Static(effect) => Renderer::Effect {
                    effect,
                    palette: None,
                    speed: 100,
                },
                RendererKind::Ripples => Renderer::Ripples {
                    palette: None,
                    base_color: None,
                    ripple_color: None,
                },
                RendererKind::Focus => Renderer::Focus {
                    focus_minutes: 25,
                    break_minutes: 5,
                    cycles: 1,
                    start_paused: false,
                    meeting_safe: false,
                    dim: false,
                },
                RendererKind::Reactive(effect) => Renderer::Reactive {
                    effect,
                    palette: None,
                },
            };
            renderer.validate().unwrap();
            assert_eq!(renderer.catalog_id(), Some(entry.id));
        }
        for renderer in [
            Renderer::Effect {
                effect: EffectKind::RowTest,
                palette: None,
                speed: 200,
            },
            Renderer::Effect {
                effect: EffectKind::Matrix,
                palette: Some(PaletteName::Ocean),
                speed: 100,
            },
            Renderer::Effect {
                effect: EffectKind::Aurora,
                palette: None,
                speed: 0,
            },
            Renderer::Ripples {
                palette: None,
                base_color: Some([0; 3]),
                ripple_color: None,
            },
            Renderer::Ripples {
                palette: Some(PaletteName::Ocean),
                base_color: Some([0; 3]),
                ripple_color: Some([0; 3]),
            },
            Renderer::Pulse {
                color: [0; 3],
                period_ms: 0,
            },
            Renderer::Sweep {
                color: [0; 3],
                period_ms: 60_001,
            },
            Renderer::Focus {
                focus_minutes: 0,
                break_minutes: 5,
                cycles: 1,
                start_paused: false,
                meeting_safe: false,
                dim: false,
            },
        ] {
            assert!(renderer.validate().is_err(), "{renderer:?}");
        }
    }

    #[test]
    fn activation_tokens_and_durations_are_bounded() {
        for activation in [
            Activation::WhileState {
                source: "pi".into(),
                instance: Some("bad instance".into()),
                state: None,
            },
            Activation::WhileState {
                source: "".into(),
                instance: None,
                state: None,
            },
            Activation::OnEvent {
                source: "pi".into(),
                instance: None,
                event: "finished".into(),
                duration_ms: 60_001,
            },
            Activation::OnTransition {
                source: "pi".into(),
                instance: None,
                from_state: Some("".into()),
                state: "idle".into(),
                duration_ms: 1000,
            },
        ] {
            assert!(activation.validate().is_err(), "{activation:?}");
        }
    }
}
