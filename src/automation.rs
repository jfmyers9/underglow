//! Declarative external facts → scenes. No I/O, SDKs, processes, or wall clock.
//!
//! Call `baseline` at activation/resume, then observe snapshots and broadcast event
//! batches on the registry's monotonic clock. One active slot per rule bounds
//! memory; a newer occurrence replaces that rule's earlier transient scene.
//! `on-transition` compares consecutive observed per-instance states, not a
//! journal: producers should emit events for occurrences that must survive rapid
//! intermediate state changes between observations. First appearance, expiry,
//! removal, and same-state lease renewals are not transitions.
use crate::compositor::{KeyMask, Layer, winning_layer};
use crate::device::DeviceInfo;
use crate::effects::EffectKind;
use crate::external::{EventBatch, Identity, Snapshot, StateSnapshot};
use crate::key_targets::canonical_coord;
use crate::layout::{KeyboardLayout, Zone};
use crate::render::{Color, Frame, PaletteName, RenderContext};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AutomationConfig {
    pub rules: Vec<Rule>,
    pub scenes: BTreeMap<String, Scene>,
    /// Flat, reusable groups of physical key names (no recursive groups).
    pub groups: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuleMode {
    WhileState,
    OnEvent,
    OnTransition,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub source: String,
    #[serde(default)]
    pub instance: Option<String>,
    pub mode: RuleMode,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub from_state: Option<String>,
    pub scene: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SceneKind {
    #[default]
    Solid,
    Pulse,
    Sweep,
    Effect,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SceneZone {
    Function,
    Alpha,
    Navigation,
    Arrows,
    System,
}

impl From<SceneZone> for Zone {
    fn from(zone: SceneZone) -> Self {
        match zone {
            SceneZone::Function => Self::Function,
            SceneZone::Alpha => Self::Alpha,
            SceneZone::Navigation => Self::Navigation,
            SceneZone::Arrows => Self::Arrows,
            SceneZone::System => Self::System,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scene {
    pub kind: SceneKind,
    pub color: [u8; 3],
    pub effect: Option<EffectKind>,
    pub palette: Option<PaletteName>,
    pub brightness: Option<u8>,
    /// The union of zones, keys, and groups. All three empty means whole keyboard.
    pub zones: Vec<SceneZone>,
    pub keys: Vec<String>,
    pub groups: Vec<String>,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            kind: SceneKind::Solid,
            color: [0, 180, 255],
            effect: None,
            palette: None,
            brightness: None,
            zones: Vec::new(),
            keys: Vec::new(),
            groups: Vec::new(),
        }
    }
}

fn label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn token(value: &str) -> bool {
    label(value)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
}

impl AutomationConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.rules.len() > 64 || self.scenes.len() > 64 || self.groups.len() > 64 {
            return Err("automation supports at most 64 rules, 64 scenes, and 64 groups".into());
        }
        for (name, keys) in &self.groups {
            if !token(name) || keys.is_empty() || keys.len() > 128 {
                return Err(format!(
                    "automation group {name}: requires a valid name and 1..128 keys"
                ));
            }
            for key in keys {
                if canonical_coord(key).is_none() {
                    return Err(format!(
                        "automation group {name}: unknown or unsupported key {key:?}"
                    ));
                }
            }
        }
        let mut ids = BTreeSet::new();
        for (name, scene) in &self.scenes {
            if !label(name) || scene.zones.len() > 5 {
                return Err("automation scene name or zones invalid".into());
            }
            if scene.keys.len() > 128 || scene.groups.len() > 64 {
                return Err(format!(
                    "automation scene {name}: at most 128 keys and 64 group references"
                ));
            }
            self.scene_mask(scene)
                .map_err(|error| format!("automation scene {name}: {error}"))?;
            if (scene.kind == SceneKind::Effect) != scene.effect.is_some()
                || (scene.kind != SceneKind::Effect && scene.palette.is_some())
            {
                return Err(format!(
                    "automation scene {name}: effect/palette require kind='effect' and effect is required"
                ));
            }
        }
        for rule in &self.rules {
            if !label(&rule.id)
                || !ids.insert(&rule.id)
                || !token(&rule.source)
                || rule.instance.as_deref().is_some_and(|s| !token(s))
                || [&rule.state, &rule.event, &rule.from_state]
                    .into_iter()
                    .any(|s| s.as_deref().is_some_and(|s| !token(s)))
            {
                return Err("automation rule ids must be unique; selectors must be nonempty, bounded labels".into());
            }
            if !self.scenes.contains_key(&rule.scene) {
                return Err(format!(
                    "automation rule {}: missing scene {}",
                    rule.id, rule.scene
                ));
            }
            let valid = match rule.mode {
                RuleMode::WhileState => {
                    rule.event.is_none() && rule.from_state.is_none() && rule.duration_ms.is_none()
                }
                RuleMode::OnEvent => {
                    rule.event.is_some() && rule.state.is_none() && rule.from_state.is_none()
                }
                RuleMode::OnTransition => rule.state.is_some() && rule.event.is_none(),
            };
            if !valid
                || (rule.mode != RuleMode::WhileState
                    && !rule.duration_ms.is_some_and(|d| (1..=60_000).contains(&d)))
            {
                return Err(format!(
                    "automation rule {}: invalid selectors/duration_ms (transients require 1..60000)",
                    rule.id
                ));
            }
        }
        Ok(())
    }

    /// Resolve custom groups as well as direct keys. Call after `validate`.
    pub fn scene_mask(&self, scene: &Scene) -> Result<KeyMask, String> {
        scene.resolve_mask(&self.groups)
    }

    /// Zones/whole-keyboard retain legacy support; physical names require the
    /// canonical ANSI map, even when their rule is currently inactive.
    pub fn validate_layout(&self, info: &DeviceInfo) -> Result<(), String> {
        if self.has_named_targets() {
            crate::key_targets::validate_device(info)?;
        }
        Ok(())
    }

    pub fn has_named_targets(&self) -> bool {
        self.scenes.values().any(Scene::has_named_targets)
    }
}

impl Rule {
    fn identity_matches(&self, identity: &Identity) -> bool {
        self.source == identity.source
            && self
                .instance
                .as_ref()
                .is_none_or(|instance| *instance == identity.instance)
    }

    fn state_matches(&self, state: &StateSnapshot, now: Duration) -> bool {
        state.expires_at > now
            && self.identity_matches(&state.identity)
            && self
                .state
                .as_ref()
                .is_none_or(|value| *value == state.state)
    }
}

#[derive(Clone, Copy, Debug)]
struct Activation {
    started: Duration,
    expires: Option<Duration>,
}

/// Stable declaration-order diagnostics; `visible_keys` uses compositor arbitration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RuleDiagnostic {
    pub rule_id: String,
    pub scene: String,
    pub matched: bool,
    pub active: bool,
    pub priority: i32,
    pub expires: Option<Duration>,
    pub targeted_keys: usize,
    pub visible_keys: usize,
}

pub struct AutomationRuntime {
    config: AutomationConfig,
    /// Resolve selectors once, not on every frame or diagnostic request.
    masks: BTreeMap<String, KeyMask>,
    previous: BTreeMap<Identity, StateSnapshot>,
    activations: Vec<Option<Activation>>,
    matched: Vec<bool>,
    cursor: u64,
    now: Duration,
    initialized: bool,
    follow_state_leases: bool,
    leased_identities: Vec<Option<Identity>>,
}

impl AutomationRuntime {
    pub fn new(config: AutomationConfig) -> Result<Self, String> {
        config.validate()?;
        let masks = config
            .scenes
            .iter()
            .map(|(name, scene)| Ok((name.clone(), config.scene_mask(scene)?)))
            .collect::<Result<_, String>>()?;
        Ok(Self {
            masks,
            activations: vec![None; config.rules.len()],
            matched: vec![false; config.rules.len()],
            leased_identities: vec![None; config.rules.len()],
            config,
            previous: BTreeMap::new(),
            cursor: 0,
            now: Duration::ZERO,
            initialized: false,
            follow_state_leases: false,
        })
    }

    /// Logical behaviors additionally hide state-backed transients on removal.
    /// Event-only producers continue to use the event's own bounded lease.
    pub(crate) fn follow_state_leases(mut self) -> Self {
        self.follow_state_leases = true;
        self
    }

    /// Cached condition match, without rendering diagnostic placeholder scenes.
    pub fn is_matched(&self, rule_id: &str) -> bool {
        self.config
            .rules
            .iter()
            .zip(&self.matched)
            .any(|(rule, matched)| rule.id == rule_id && *matched)
    }

    /// Active occurrence timing without rendering a diagnostic placeholder scene.
    pub fn activation_timing(&self, rule_id: &str) -> Option<(Duration, Option<Duration>)> {
        self.config
            .rules
            .iter()
            .zip(&self.activations)
            .find_map(|(rule, active)| {
                (rule.id == rule_id)
                    .then_some(active)
                    .and_then(|a| *a)
                    .filter(|a| a.expires.is_none_or(|e| self.now < e))
                    .map(|a| (a.started, a.expires))
            })
    }

    pub fn event_cursor(&self) -> u64 {
        self.cursor
    }

    /// Carry selected, unchanged rule slots into an already-baselined candidate.
    /// Used by canonical setup edits; caller compares declarations before reuse.
    pub(crate) fn preserve_rules_from(&mut self, previous: &Self, ids: &[&str]) {
        for (index, rule) in self.config.rules.iter().enumerate() {
            if ids.contains(&rule.id.as_str())
                && let Some(old) = previous
                    .config
                    .rules
                    .iter()
                    .position(|old| old.id == rule.id)
            {
                self.activations[index] = previous.activations[old];
                self.matched[index] = previous.matched[old];
                self.leased_identities[index] = previous.leased_identities[old].clone();
            }
        }
    }

    pub(crate) fn observed_at(&self) -> Duration {
        self.now
    }

    /// Evaluate a condition without rendering a dummy scene or resolving overlap.
    pub fn is_active(&self, rule_id: &str) -> bool {
        self.config
            .rules
            .iter()
            .zip(&self.activations)
            .any(|(rule, activation)| {
                rule.id == rule_id
                    && activation.is_some_and(|a| a.expires.is_none_or(|e| self.now < e))
            })
    }

    pub fn validate_layout(&self, info: &DeviceInfo) -> Result<(), String> {
        self.config.validate_layout(info)
    }

    /// Discard transient history while keeping currently leased persistent state.
    pub fn baseline(&mut self, snapshot: &Snapshot, now: Duration) {
        self.cursor = snapshot.event_cursor;
        self.now = now;
        self.initialized = true;
        self.activations.fill(None);
        self.leased_identities.fill(None);
        self.matched.fill(false);
        self.previous = Self::states(snapshot, now);
        self.update(
            snapshot,
            &EventBatch {
                cursor: self.cursor,
                gap: false,
                events: vec![],
            },
            now,
            false,
        );
    }

    pub fn observe(&mut self, snapshot: &Snapshot, events: &EventBatch, now: Duration) {
        if !self.initialized {
            self.baseline(snapshot, now);
        } else if now >= self.now {
            self.update(snapshot, events, now, true);
        }
    }

    fn states(snapshot: &Snapshot, now: Duration) -> BTreeMap<Identity, StateSnapshot> {
        snapshot
            .states
            .iter()
            .filter(|s| s.expires_at > now)
            .map(|s| (s.identity.clone(), s.clone()))
            .collect()
    }

    fn update(
        &mut self,
        snapshot: &Snapshot,
        events: &EventBatch,
        now: Duration,
        transitions: bool,
    ) {
        self.now = now;
        for (index, rule) in self.config.rules.iter().enumerate() {
            if self.leased_identities[index]
                .as_ref()
                .is_some_and(|identity| {
                    !snapshot
                        .states
                        .iter()
                        .any(|state| state.identity == *identity && state.expires_at > now)
                })
            {
                self.activations[index] = None;
            }
            if self.activations[index].is_some_and(|a| a.expires.is_some_and(|e| now >= e)) {
                self.activations[index] = None;
            }
            let mut occurrence = None;
            let mut occurrence_identity = None;
            self.matched[index] = match rule.mode {
                RuleMode::WhileState => snapshot.states.iter().any(|s| rule.state_matches(s, now)),
                RuleMode::OnEvent => {
                    for event in &events.events {
                        if event.cursor > self.cursor
                            && event.expires_at > now
                            && event.received_at <= now
                            && rule.identity_matches(&event.identity)
                            && rule.event.as_ref() == Some(&event.event)
                        {
                            occurrence = Some((event.received_at, event.expires_at));
                            occurrence_identity = Some(&event.identity);
                        }
                    }
                    occurrence.is_some()
                }
                RuleMode::OnTransition => {
                    if transitions {
                        for state in &snapshot.states {
                            if rule.state_matches(state, now)
                                && state.updated_at <= now
                                && self.previous.get(&state.identity).is_some_and(|old| {
                                    old.expires_at > state.updated_at
                                        && old.state != state.state
                                        && rule
                                            .from_state
                                            .as_ref()
                                            .is_none_or(|from| *from == old.state)
                                })
                                && occurrence.is_none_or(|(started, _)| state.updated_at > started)
                            {
                                occurrence = Some((state.updated_at, state.expires_at));
                                occurrence_identity = Some(&state.identity);
                            }
                        }
                    }
                    occurrence.is_some()
                }
            };
            if rule.mode == RuleMode::WhileState {
                if !self.matched[index] {
                    self.activations[index] = None;
                } else if self.activations[index].is_none() {
                    self.activations[index] = Some(Activation {
                        started: now,
                        expires: None,
                    });
                }
            } else if let Some((started, lease)) = occurrence {
                let state_lease = self
                    .follow_state_leases
                    .then(|| {
                        snapshot
                            .states
                            .iter()
                            .find(|state| Some(&state.identity) == occurrence_identity)
                    })
                    .flatten();
                self.leased_identities[index] = state_lease.map(|state| state.identity.clone());
                let expires = started
                    .saturating_add(Duration::from_millis(rule.duration_ms.unwrap()))
                    .min(lease)
                    .min(state_lease.map_or(Duration::MAX, |state| state.expires_at));
                if expires > now {
                    self.activations[index] = Some(Activation {
                        started,
                        expires: Some(expires),
                    });
                }
            }
        }
        self.cursor = self.cursor.max(events.cursor).max(snapshot.event_cursor);
        self.previous = Self::states(snapshot, now);
    }

    pub fn layers(&self, ctx: &RenderContext<'_>) -> Vec<Layer> {
        self.config
            .rules
            .iter()
            .zip(&self.activations)
            .filter_map(|(rule, activation)| {
                activation
                    .filter(|a| a.expires.is_none_or(|e| self.now < e))
                    .map(|activation| {
                        let scene = &self.config.scenes[&rule.scene];
                        let mask = &self.masks[&rule.scene];
                        Layer {
                            opacity: 255,
                            blend: Default::default(),
                            frame: scene.render_masked(
                                ctx,
                                self.now.saturating_sub(activation.started),
                                mask,
                            ),
                            mask: mask.clone(),
                            priority: rule.priority,
                            expires: activation.expires,
                        }
                    })
            })
            .collect()
    }

    pub fn diagnostics(&self, ctx: &RenderContext<'_>) -> Vec<RuleDiagnostic> {
        let layers = self.layers(ctx);
        let mut layer_index = 0;
        self.config
            .rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                let activation =
                    self.activations[index].filter(|a| a.expires.is_none_or(|e| self.now < e));
                let active = activation.is_some();
                let mask = &self.masks[&rule.scene];
                let visible_keys = if active {
                    let current = layer_index;
                    layer_index += 1;
                    ctx.layout
                        .keys()
                        .iter()
                        .filter(|key| winning_layer(&layers, key, self.now) == Some(current))
                        .count()
                } else {
                    0
                };
                RuleDiagnostic {
                    rule_id: rule.id.clone(),
                    scene: rule.scene.clone(),
                    matched: self.matched[index],
                    active,
                    priority: rule.priority,
                    expires: activation.and_then(|a| a.expires),
                    targeted_keys: ctx
                        .layout
                        .keys()
                        .iter()
                        .filter(|key| mask.contains(key))
                        .count(),
                    visible_keys,
                }
            })
            .collect()
    }
}

impl Scene {
    fn has_named_targets(&self) -> bool {
        !self.keys.is_empty() || !self.groups.is_empty()
    }

    /// Standalone direct-key/zone mask. Group references require
    /// `AutomationConfig::scene_mask`; unresolved selectors fail closed.
    pub fn mask(&self) -> KeyMask {
        self.resolve_mask(&BTreeMap::new())
            .unwrap_or_else(|_| KeyMask::Coordinates(Vec::new()))
    }

    fn resolve_mask(&self, groups: &BTreeMap<String, Vec<String>>) -> Result<KeyMask, String> {
        let zones: Vec<Zone> = self.zones.iter().copied().map(Into::into).collect();
        if !self.has_named_targets() {
            return Ok(if zones.is_empty() {
                KeyMask::Whole
            } else {
                KeyMask::Zones(zones)
            });
        }
        let mut names: Vec<&String> = self.keys.iter().collect();
        for name in &self.groups {
            let keys = groups
                .get(name)
                .ok_or_else(|| format!("unknown group {name:?}"))?;
            if keys.is_empty() {
                return Err(format!("group {name:?} is empty"));
            }
            names.extend(keys);
        }
        let mut coords = Vec::new();
        for name in names {
            let coord = canonical_coord(name)
                .ok_or_else(|| format!("unknown or unsupported key {name:?}"))?;
            if !coords.contains(&coord) {
                coords.push(coord);
            }
        }
        if !zones.is_empty() {
            let layout = KeyboardLayout::for_device(&DeviceInfo::synthetic_80he());
            for key in layout.keys().iter().filter(|key| zones.contains(&key.zone)) {
                if !coords.contains(&key.coord) {
                    coords.push(key.coord);
                }
            }
        }
        Ok(KeyMask::Coordinates(coords))
    }

    #[cfg(test)]
    fn render(&self, ctx: &RenderContext<'_>, elapsed: Duration) -> Frame {
        self.render_masked(ctx, elapsed, &self.mask())
    }

    fn render_masked(&self, ctx: &RenderContext<'_>, elapsed: Duration, mask: &KeyMask) -> Frame {
        let mut ctx = ctx.clone();
        ctx.brightness = ctx.brightness.min(self.brightness.unwrap_or(255));
        ctx.palette = self.palette.unwrap_or(ctx.palette);
        ctx.animation_seconds = elapsed.as_secs_f64();
        if self.kind == SceneKind::Effect {
            return self.effect.unwrap_or_default().render(&ctx);
        }
        let mut frame = Frame::black();
        let [r, g, b] = self.color;
        let color = Color::new(r, g, b).scale(ctx.brightness);
        let mut keys: Vec<_> = ctx
            .layout
            .keys()
            .iter()
            .filter(|key| mask.contains(key))
            .collect();
        keys.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        let phase = elapsed.as_secs_f64();
        let count = keys.len();
        for (index, key) in keys.into_iter().enumerate() {
            let amount = match self.kind {
                SceneKind::Pulse => {
                    (64.0 + 191.0 * (0.5 - 0.5 * (phase * std::f64::consts::TAU / 2.0).cos())) as u8
                }
                SceneKind::Sweep => {
                    let head = (phase * 10.0) as usize % count.max(1);
                    let distance = (head + count - index) % count.max(1);
                    if distance < 4 {
                        255u8.saturating_sub((distance * 60) as u8)
                    } else {
                        0
                    }
                }
                _ => 255,
            };
            frame.set_coord(key.coord, color.scale(amount));
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceInfo, DeviceType, Layout};
    use crate::external::{Message, Payload, Registry};
    use crate::layout::KeyboardLayout;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn config() -> AutomationConfig {
        toml::from_str(
            r#"
[[rules]]
id = 'present'
source = 'pi'
mode = 'while-state'
scene = 'idle'
priority = -10
[[rules]]
id = 'working'
source = 'pi'
mode = 'while-state'
state = 'working'
scene = 'working'
priority = 10
[[rules]]
id = 'finished'
source = 'pi'
mode = 'on-event'
event = 'finished'
scene = 'done'
priority = 20
duration_ms = 2000
[[rules]]
id = 'transition'
source = 'pi'
mode = 'on-transition'
from_state = 'working'
state = 'idle'
scene = 'done'
priority = 15
duration_ms = 1000
[scenes.idle]
color = [0, 255, 255]
brightness = 40
zones = ['function']
[scenes.working]
kind = 'sweep'
color = [0, 0, 255]
zones = ['function']
[scenes.done]
kind = 'pulse'
color = [0, 255, 0]
zones = ['function']
"#,
        )
        .unwrap()
    }

    fn publish(registry: &mut Registry, instance: &str, sequence: u64, payload: Payload, now: u64) {
        registry
            .apply(
                Message {
                    schema_version: 1,
                    source: "pi".into(),
                    instance: instance.into(),
                    sequence,
                    payload,
                },
                ms(now),
            )
            .unwrap();
    }

    fn state(
        registry: &mut Registry,
        instance: &str,
        sequence: u64,
        state: &str,
        now: u64,
        ttl: u64,
    ) {
        publish(
            registry,
            instance,
            sequence,
            Payload::State {
                state: state.into(),
                ttl_ms: ttl,
            },
            now,
        );
    }

    fn observe(runtime: &mut AutomationRuntime, registry: &mut Registry, now: u64) {
        registry.expire(ms(now)).unwrap();
        runtime.observe(
            &registry.snapshot(),
            &registry.events_since(runtime.event_cursor()),
            ms(now),
        );
    }

    fn info() -> DeviceInfo {
        DeviceInfo {
            connected: true,
            model: "test".into(),
            max_rows: 6,
            max_columns: 17,
            led_index_max: 0,
            device_type: DeviceType::Keyboard80,
            layout: Layout::Ansi,
            v2_interface: true,
            uses_small_packets: false,
            uses_multi_report: false,
        }
    }

    #[test]
    fn while_rules_aggregate_instances_and_expire_without_heartbeat_restarts() {
        let mut registry = Registry::default();
        let mut runtime = AutomationRuntime::new(config()).unwrap();
        runtime.baseline(&registry.snapshot(), ms(0));
        state(&mut registry, "a", 1, "working", 10, 1000);
        state(&mut registry, "b", 1, "idle", 10, 3000);
        observe(&mut runtime, &mut registry, 10);
        assert!(runtime.activations[0].is_some());
        assert!(runtime.activations[1].is_some());
        state(&mut registry, "a", 2, "working", 20, 1000);
        observe(&mut runtime, &mut registry, 20);
        assert_eq!(runtime.activations[1].unwrap().started, ms(10));
        assert!(runtime.activations[3].is_none());
        observe(&mut runtime, &mut registry, 1020);
        assert!(runtime.activations[0].is_some());
        assert!(runtime.activations[1].is_none());
        observe(&mut runtime, &mut registry, 3010);
        assert!(runtime.activations.iter().all(Option::is_none));
    }

    #[test]
    fn behavior_lease_following_is_opt_in_and_does_not_change_legacy_event_lifetimes() {
        let mut registry = Registry::default();
        let mut legacy = AutomationRuntime::new(config()).unwrap();
        let mut behavior = AutomationRuntime::new(config())
            .unwrap()
            .follow_state_leases();
        legacy.baseline(&registry.snapshot(), ms(0));
        behavior.baseline(&registry.snapshot(), ms(0));
        state(&mut registry, "a", 1, "working", 0, 100);
        publish(
            &mut registry,
            "a",
            2,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 500,
            },
            10,
        );
        observe(&mut legacy, &mut registry, 10);
        observe(&mut behavior, &mut registry, 10);
        assert_eq!(
            legacy.activation_timing("finished"),
            Some((ms(10), Some(ms(510))))
        );
        assert_eq!(
            behavior.activation_timing("finished"),
            Some((ms(10), Some(ms(100))))
        );
        publish(&mut registry, "a", 3, Payload::Remove, 20);
        observe(&mut legacy, &mut registry, 20);
        observe(&mut behavior, &mut registry, 20);
        assert!(legacy.is_active("finished"));
        assert!(!behavior.is_active("finished"));
        assert_eq!(behavior.activation_timing("finished"), None);
    }

    #[test]
    fn events_are_once_only_and_deadlines_use_receipt_not_observation() {
        let mut registry = Registry::default();
        let mut runtime = AutomationRuntime::new(config()).unwrap();
        runtime.baseline(&registry.snapshot(), ms(0));
        publish(
            &mut registry,
            "a",
            1,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 900,
            },
            10,
        );
        observe(&mut runtime, &mut registry, 500);
        assert_eq!(runtime.activations[2].unwrap().expires, Some(ms(910)));
        // Re-deliver the identical batch: no restart or lifetime extension.
        runtime.observe(&registry.snapshot(), &registry.events_since(0), ms(600));
        assert_eq!(runtime.activations[2].unwrap().started, ms(10));
        observe(&mut runtime, &mut registry, 910);
        assert!(runtime.activations[2].is_none());
        publish(
            &mut registry,
            "a",
            2,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 15000,
            },
            1000,
        );
        observe(&mut runtime, &mut registry, 4000);
        assert!(
            runtime.activations[2].is_none(),
            "an unexpired event cannot stretch its scene deadline"
        );
    }

    #[test]
    fn activation_and_resume_baseline_discard_even_unexpired_events_and_transitions() {
        let mut registry = Registry::default();
        state(&mut registry, "a", 1, "working", 0, 15000);
        publish(
            &mut registry,
            "a",
            2,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 15000,
            },
            1,
        );
        let mut runtime = AutomationRuntime::new(config()).unwrap();
        observe(&mut runtime, &mut registry, 2);
        assert!(runtime.activations[1].is_some());
        assert!(runtime.activations[2].is_none());
        state(&mut registry, "a", 3, "idle", 10, 15000);
        observe(&mut runtime, &mut registry, 10);
        assert!(runtime.activations[3].is_some());
        publish(
            &mut registry,
            "a",
            4,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 15000,
            },
            20,
        );
        runtime.baseline(&registry.snapshot(), ms(30));
        observe(&mut runtime, &mut registry, 40);
        assert!(runtime.activations[2].is_none());
        assert!(runtime.activations[3].is_none());
        assert!(runtime.activations[0].is_some());
    }

    #[test]
    fn transitions_require_changed_contiguous_instance_state_not_heartbeats() {
        let mut registry = Registry::default();
        let mut runtime = AutomationRuntime::new(config()).unwrap();
        runtime.baseline(&registry.snapshot(), ms(0));
        state(&mut registry, "a", 1, "working", 1, 1000);
        observe(&mut runtime, &mut registry, 1);
        state(&mut registry, "b", 1, "idle", 2, 1000);
        observe(&mut runtime, &mut registry, 2);
        assert!(
            runtime.activations[3].is_none(),
            "a different instance is not a transition"
        );
        state(&mut registry, "a", 2, "idle", 10, 1000);
        observe(&mut runtime, &mut registry, 10);
        assert_eq!(runtime.activations[3].unwrap().expires, Some(ms(1010)));
        state(&mut registry, "a", 3, "idle", 20, 1000);
        observe(&mut runtime, &mut registry, 20);
        assert_eq!(runtime.activations[3].unwrap().started, ms(10));
        observe(&mut runtime, &mut registry, 1010);
        assert!(runtime.activations[3].is_none());
        state(&mut registry, "a", 4, "working", 1020, 1);
        observe(&mut runtime, &mut registry, 1020);
        state(&mut registry, "a", 5, "idle", 1030, 1000);
        observe(&mut runtime, &mut registry, 1030);
        assert!(
            runtime.activations[3].is_none(),
            "an expired old lease is not a continuous transition"
        );
    }

    #[test]
    fn instance_filter_is_exact_and_other_sources_do_not_match() {
        let mut config = config();
        config.rules[1].instance = Some("wanted".into());
        let mut runtime = AutomationRuntime::new(config).unwrap();
        let mut registry = Registry::default();
        runtime.baseline(&registry.snapshot(), ms(0));
        state(&mut registry, "other", 1, "working", 1, 1000);
        observe(&mut runtime, &mut registry, 1);
        assert!(runtime.activations[1].is_none());
        state(&mut registry, "wanted", 1, "working", 2, 1000);
        observe(&mut runtime, &mut registry, 2);
        assert!(runtime.activations[1].is_some());
    }

    #[test]
    fn aggregate_transition_selects_newest_timestamp_not_last_identity() {
        let mut registry = Registry::default();
        state(&mut registry, "a", 1, "working", 0, 15000);
        state(&mut registry, "z", 1, "working", 0, 15000);
        let mut runtime = AutomationRuntime::new(config()).unwrap();
        runtime.baseline(&registry.snapshot(), ms(0));
        state(&mut registry, "z", 2, "idle", 10, 15000);
        state(&mut registry, "a", 2, "idle", 1000, 15000);
        observe(&mut runtime, &mut registry, 1100);
        assert_eq!(runtime.activations[3].unwrap().started, ms(1000));
        assert_eq!(runtime.activations[3].unwrap().expires, Some(ms(2000)));
    }

    #[test]
    fn layers_cap_brightness_preserve_base_and_report_arbitration() {
        let mut registry = Registry::default();
        state(&mut registry, "a", 1, "working", 0, 15000);
        let mut config = config();
        config.scenes.get_mut("working").unwrap().kind = SceneKind::Solid;
        let mut runtime = AutomationRuntime::new(config).unwrap();
        runtime.baseline(&registry.snapshot(), ms(0));
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 30,
            palette: PaletteName::Ocean,
            tick: 0,
            animation_seconds: 1.0,
        };
        let layers = runtime.layers(&ctx);
        let base = EffectKind::Matrix.render(&ctx);
        let frame = crate::compositor::compose(base.clone(), &layout, &layers, ms(0));
        for key in layout.keys() {
            if key.zone == Zone::Function {
                assert_eq!(frame.get_coord(key.coord), Color::new(0, 0, 30));
            } else {
                assert_eq!(frame.get_coord(key.coord), base.get_coord(key.coord));
            }
        }
        let diagnostic = runtime.diagnostics(&ctx);
        assert_eq!(diagnostic[0].visible_keys, 0);
        assert!(diagnostic[1].visible_keys > 0);
        assert_eq!(diagnostic[1].visible_keys, diagnostic[1].targeted_keys);
        for scene in [
            SceneKind::Solid,
            SceneKind::Pulse,
            SceneKind::Sweep,
            SceneKind::Effect,
        ] {
            let s = Scene {
                kind: scene,
                effect: (scene == SceneKind::Effect).then_some(EffectKind::Matrix),
                color: [255, 255, 255],
                ..Scene::default()
            };
            assert!(
                s.render(&ctx, ms(500))
                    .as_bytes()
                    .iter()
                    .all(|channel| *channel <= 30)
            );
        }
    }

    #[test]
    fn key_groups_preserve_old_defaults_and_resolve_union_without_duplicates() {
        assert_eq!(Scene::default().mask(), KeyMask::Whole);
        let old: AutomationConfig = toml::from_str("[scenes.old]\nzones=['function']").unwrap();
        old.validate().unwrap();
        assert!(old.groups.is_empty());
        assert_eq!(
            old.scenes["old"].mask(),
            KeyMask::Zones(vec![Zone::Function])
        );
        let config: AutomationConfig = toml::from_str(
            r#"
[groups]
WASD = ['W', 'A', 'S', 'D', 'w']
[scenes.selected]
keys = ['F1', 'f1', 'W']
groups = ['WASD', 'WASD']
[scenes.combined]
zones = ['arrows']
keys = ['Escape']
groups = ['WASD']
"#,
        )
        .unwrap();
        config.validate().unwrap();
        let mask = config.scene_mask(&config.scenes["selected"]).unwrap();
        let KeyMask::Coordinates(coords) = &mask else {
            panic!("expected coordinate mask")
        };
        assert_eq!(coords.len(), 5);
        for key in ["F1", "W", "A", "S", "D"] {
            assert!(coords.contains(&canonical_coord(key).unwrap()));
        }
        assert!(!coords.contains(&canonical_coord("F2").unwrap()));
        let layout = KeyboardLayout::for_device(&info());
        let mask = config.scene_mask(&config.scenes["combined"]).unwrap();
        for key in layout.keys() {
            let expected = key.zone == Zone::Arrows
                || ["Escape", "W", "A", "S", "D"]
                    .iter()
                    .any(|name| canonical_coord(name) == Some(key.coord));
            assert_eq!(mask.contains(key), expected);
        }
        // Standalone calls cannot resolve a config's groups: fail closed.
        assert_eq!(
            config.scenes["selected"].mask(),
            KeyMask::Coordinates(vec![])
        );
        let direct: Scene = toml::from_str("keys=['F1']").unwrap();
        assert_eq!(
            direct.mask(),
            KeyMask::Coordinates(vec![canonical_coord("F1").unwrap()])
        );
    }

    #[test]
    fn named_target_validation_reports_unknown_keys_groups_and_bounds() {
        for (input, expected) in [
            ("[scenes.x]\nkeys=['Fn']", "unsupported key"),
            ("[scenes.x]\nkeys=['F1-F4']", "unsupported key"),
            ("[scenes.x]\ngroups=['missing']", "unknown group"),
            ("[groups]\nempty=[]", "1..128 keys"),
            ("[groups]\nWASD=['W','bogus']", "unsupported key"),
            ("[groups]\n'bad name'=['F1']", "valid name"),
        ] {
            let config: AutomationConfig = toml::from_str(input).unwrap();
            let error = config.validate().unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
        let mut config = AutomationConfig::default();
        config
            .groups
            .insert("too-many".into(), vec!["F1".into(); 129]);
        assert!(config.validate().is_err());
        config.groups.clear();
        config.scenes.insert(
            "too-many".into(),
            Scene {
                keys: vec!["F1".into(); 129],
                ..Scene::default()
            },
        );
        assert!(config.validate().is_err());
        config.scenes.get_mut("too-many").unwrap().keys.clear();
        config.scenes.get_mut("too-many").unwrap().groups = vec!["g".into(); 65];
        assert!(config.validate().is_err());
        config.scenes.clear();
        config.groups = (0..65)
            .map(|i| (format!("group-{i}"), vec!["F1".into()]))
            .collect();
        assert!(config.validate().is_err());
    }

    #[test]
    fn named_layout_restriction_does_not_change_zone_only_profiles() {
        let named: AutomationConfig = toml::from_str("[scenes.x]\nkeys=['F1']").unwrap();
        let runtime = AutomationRuntime::new(named.clone()).unwrap();
        assert!(runtime.validate_layout(&info()).is_ok());
        let mut unsupported = info();
        unsupported.layout = Layout::Iso;
        assert!(named.validate_layout(&unsupported).is_err());
        assert!(runtime.validate_layout(&unsupported).is_err());
        assert!(config().validate_layout(&unsupported).is_ok());
        unsupported.layout = Layout::Ansi;
        unsupported.device_type = DeviceType::Keyboard60;
        assert!(named.validate_layout(&unsupported).is_err());
        let grouped: AutomationConfig =
            toml::from_str("[groups]\ng=['F1']\n[scenes.x]\ngroups=['g']").unwrap();
        assert!(grouped.validate_layout(&unsupported).is_err());
    }

    #[test]
    fn explicit_sessions_keep_individual_keys_and_group_events_compose_over_matrix() {
        #[derive(Deserialize)]
        struct Profile {
            automation: AutomationConfig,
        }
        let profile: Profile =
            toml::from_str(include_str!("../examples/automation-key-groups.toml")).unwrap();
        let mut config = profile.automation;
        for name in ["session-a", "session-b"] {
            config.scenes.get_mut(name).unwrap().kind = SceneKind::Solid;
        }
        let mut runtime = AutomationRuntime::new(config).unwrap();
        let mut registry = Registry::default();
        runtime.baseline(&registry.snapshot(), ms(0));
        state(&mut registry, "sessionA", 1, "working", 10, 1000);
        state(&mut registry, "sessionB", 1, "idle", 10, 3000);
        observe(&mut runtime, &mut registry, 10);
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 128,
            palette: PaletteName::Ocean,
            tick: 0,
            animation_seconds: 3.0,
        };
        assert_eq!(runtime.diagnostics(&ctx)[0].visible_keys, 1);
        assert_eq!(runtime.diagnostics(&ctx)[1].visible_keys, 0);
        state(&mut registry, "sessionB", 2, "working", 20, 3000);
        observe(&mut runtime, &mut registry, 20);
        let layers = runtime.layers(&ctx);
        assert_eq!(layers.len(), 2);
        let base = EffectKind::Matrix.render(&ctx);
        let frame = crate::compositor::compose(base.clone(), &layout, &layers, ms(20));
        for key in layout.keys() {
            let expected = if canonical_coord("F1") == Some(key.coord) {
                Color::new(0, 80, 255).scale(128)
            } else if canonical_coord("F2") == Some(key.coord) {
                Color::new(255, 0, 120).scale(128)
            } else {
                base.get_coord(key.coord)
            };
            assert_eq!(frame.get_coord(key.coord), expected);
        }
        publish(
            &mut registry,
            "sessionB",
            3,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 15000,
            },
            30,
        );
        observe(&mut runtime, &mut registry, 30);
        let diagnostic = runtime.diagnostics(&ctx);
        assert_eq!(diagnostic[0].visible_keys, 0);
        assert_eq!(diagnostic[1].visible_keys, 0);
        assert_eq!(diagnostic[3].targeted_keys, 4);
        assert_eq!(diagnostic[3].visible_keys, 4);
        observe(&mut runtime, &mut registry, 1530);
        let diagnostic = runtime.diagnostics(&ctx);
        assert_eq!(diagnostic[0].visible_keys, 0); // sessionA expired.
        assert_eq!(diagnostic[1].visible_keys, 1); // sessionB remains independent.
        assert_eq!(diagnostic[3].visible_keys, 0); // Completion overlay expired.
    }

    #[test]
    fn configuration_rejects_unknown_fields_missing_scenes_and_invalid_bounds() {
        assert!(toml::from_str::<AutomationConfig>("unknown = true").is_err());
        assert!(toml::from_str::<AutomationConfig>("[scenes.test]\nzones=['space']").is_err());
        assert!(toml::from_str::<AutomationConfig>("[scenes.test]\ncolor=[256,0,0]").is_err());
        assert!(toml::from_str::<AutomationConfig>("[scenes.test]\nunknown = true").is_err());
        for mutate in [
            |c: &mut AutomationConfig| c.rules[0].scene = "missing".into(),
            |c: &mut AutomationConfig| c.rules[0].duration_ms = Some(1),
            |c: &mut AutomationConfig| c.rules[2].duration_ms = Some(0),
            |c: &mut AutomationConfig| c.rules[2].duration_ms = Some(60001),
            |c: &mut AutomationConfig| c.rules[2].state = Some("working".into()),
            |c: &mut AutomationConfig| c.rules[1].id = "present".into(),
            |c: &mut AutomationConfig| c.scenes.get_mut("idle").unwrap().kind = SceneKind::Effect,
        ] {
            let mut config = config();
            mutate(&mut config);
            assert!(config.validate().is_err());
        }
        assert!(AutomationConfig::default().validate().is_ok());
        assert!(config().validate().is_ok());
    }
}
