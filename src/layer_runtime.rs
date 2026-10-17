//! Canonical layers compiled into the shared renderers and opaque compositor.
//! No I/O, clocks, SDKs, producers or executable connector definitions live here.
//! A preview owns its own instance and supplies synthetic input/facts explicitly.
use crate::automation::{AutomationConfig, AutomationRuntime, Rule, RuleMode, Scene};
use crate::catalog;
use crate::compositor::{self, KeyMask};
use crate::device::DeviceInfo;
use crate::external::{EventBatch, Snapshot};
use crate::focus::{self, FocusConfig};
use crate::key_targets::canonical_coord;
use crate::layout::{MatrixCoord, Zone};
use crate::reactive::ReactiveSimulation;
use crate::render::{Color, Frame, PaletteName, RenderContext};
use crate::ripple::{KeyGeometry, RippleSimulation};
use crate::setup::{Activation, Layer, Renderer, SetupDocument, Target};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Requirements {
    /// At most one enabled analog consumer is allowed, even across conditions.
    pub analog_input: bool,
    pub named_targets: bool,
}

enum Simulation {
    None,
    Reactive(Box<ReactiveSimulation>),
    Ripples(Box<RippleSimulation>),
}

struct CompiledLayer {
    definition: Layer,
    coordinates: Vec<MatrixCoord>,
    zones: Vec<Zone>,
    simulation: Simulation,
    variant_simulations: Vec<Simulation>,
    variant_conditions: AutomationRuntime,
}

/// The one selected appearance before cross-layer occlusion. No UI or I/O.
#[derive(Clone, Copy, Debug)]
pub struct SelectedAppearance<'a> {
    pub layer_id: &'a str,
    /// None means the compatible base/fallback appearance.
    pub variant_id: Option<&'a str>,
    pub activation: &'a Activation,
    pub renderer: &'a Renderer,
    pub brightness: u8,
    pub started: Option<Duration>,
    pub expires: Option<Duration>,
    /// Conditional animation time on the evaluator's unscaled clock.
    /// None for Always appearances, which use the caller's setup elapsed time.
    pub elapsed: Option<Duration>,
}

pub struct CompiledSetup {
    layers: Vec<CompiledLayer>,
    conditions: AutomationRuntime,
    requirements: Requirements,
    input_available: bool,
}

impl CompiledSetup {
    /// Validates all data and resource conflicts before any live acquisition.
    pub fn compile(document: &SetupDocument) -> Result<Self, String> {
        document.validate()?;
        let mut requirements = Requirements::default();
        let mut analog_count = 0;
        let mut conditions = AutomationConfig::default();
        // Conditions use the existing lease/event/transition evaluator; its scene
        // is only a validated placeholder and is never rendered.
        conditions
            .scenes
            .insert("condition".into(), Scene::default());
        let mut layers = Vec::new();
        for definition in &document.layers {
            let simulation = Simulation::for_renderer(&definition.renderer);
            let variant_simulations = definition
                .variants
                .iter()
                .map(|variant| Simulation::for_renderer(&variant.renderer))
                .collect();
            // Mutually exclusive appearances are a single logical SDK consumer.
            if definition.enabled
                && (needs_input(&definition.renderer)
                    || definition
                        .variants
                        .iter()
                        .any(|v| v.enabled && needs_input(&v.renderer)))
            {
                analog_count += 1;
            }
            let mut variant_config = AutomationConfig::default();
            variant_config
                .scenes
                .insert("condition".into(), Scene::default());
            for variant in &definition.variants {
                if let Some(rule) = activation_rule(&variant.id, &variant.activation) {
                    variant_config.rules.push(rule);
                }
            }
            let mut coordinates = Vec::new();
            let mut target_zones = Vec::new();
            if let Target::Selection {
                zones,
                keys,
                groups,
            } = &definition.target
            {
                target_zones.extend(zones.iter().copied().map(Zone::from));
                for name in keys
                    .iter()
                    .chain(groups.iter().flat_map(|id| document.groups[id].iter()))
                {
                    let coord = canonical_coord(name).expect("validated key");
                    if !coordinates.contains(&coord) {
                        coordinates.push(coord);
                    }
                }
                requirements.named_targets |= !coordinates.is_empty();
            }
            if let Some(rule) = activation_rule(&definition.id, &definition.activation) {
                conditions.rules.push(rule);
            }
            layers.push(CompiledLayer {
                definition: definition.clone(),
                coordinates,
                zones: target_zones,
                simulation,
                variant_simulations,
                variant_conditions: AutomationRuntime::new(variant_config)?.follow_state_leases(),
            });
        }
        if analog_count > 1 {
            return Err(
                "at most one enabled analog-input layer (ripples or reactive) is supported".into(),
            );
        }
        requirements.analog_input = analog_count == 1;
        Ok(Self {
            layers,
            conditions: AutomationRuntime::new(conditions)?,
            requirements,
            input_available: true,
        })
    }

    pub fn requirements(&self) -> Requirements {
        self.requirements
    }

    /// Adopt a fully compiled/device-validated candidate after persistence.
    /// No fallible work, hardware acquisition, or producer lifecycle occurs here.
    /// Unchanged renderers retain simulations; unchanged activation declarations
    /// retain their deadlines. New conditions baseline current state, not events.
    pub fn reconfigure(
        &mut self,
        mut candidate: Self,
        snapshot: &Snapshot,
        events: &EventBatch,
        now: Duration,
    ) {
        self.observe(snapshot, events, now);
        candidate.baseline(snapshot, now);
        let preserved: Vec<_> = candidate
            .layers
            .iter()
            .filter_map(|layer| {
                self.layers
                    .iter()
                    .find(|old| {
                        old.definition.id == layer.definition.id
                            && old.definition.enabled == layer.definition.enabled
                            && old.definition.activation == layer.definition.activation
                    })
                    .map(|_| layer.definition.id.as_str())
            })
            .collect();
        candidate
            .conditions
            .preserve_rules_from(&self.conditions, &preserved);
        for layer in &mut candidate.layers {
            if let Some(old) = self.layers.iter_mut().find(|old| {
                old.definition.id == layer.definition.id
                    && old.definition.enabled == layer.definition.enabled
            }) {
                if old.definition.renderer == layer.definition.renderer {
                    std::mem::swap(&mut layer.simulation, &mut old.simulation);
                }
                let mut preserved_variants = Vec::new();
                for (index, variant) in layer.definition.variants.iter().enumerate() {
                    if let Some((old_index, old_variant)) = old
                        .definition
                        .variants
                        .iter()
                        .enumerate()
                        .find(|(_, old)| old.id == variant.id && old.enabled == variant.enabled)
                    {
                        if old_variant.activation == variant.activation {
                            preserved_variants.push(variant.id.as_str());
                        }
                        if old_variant.renderer == variant.renderer {
                            std::mem::swap(
                                &mut layer.variant_simulations[index],
                                &mut old.variant_simulations[old_index],
                            );
                        }
                    }
                }
                layer
                    .variant_conditions
                    .preserve_rules_from(&old.variant_conditions, &preserved_variants);
            }
        }
        candidate.input_available = self.input_available;
        *self = candidate;
    }

    pub fn validate_device(&self, info: &DeviceInfo) -> Result<(), String> {
        if self.requirements.analog_input || self.requirements.named_targets {
            crate::key_targets::validate_device(info)?;
        }
        Ok(())
    }

    pub fn event_cursor(&self) -> u64 {
        self.conditions.event_cursor()
    }

    /// Resume baselines leased state but deliberately discards historical events.
    pub fn baseline(&mut self, snapshot: &Snapshot, now: Duration) {
        self.conditions.baseline(snapshot, now);
        for layer in &mut self.layers {
            layer.variant_conditions.baseline(snapshot, now);
        }
    }

    pub fn observe(&mut self, snapshot: &Snapshot, events: &EventBatch, now: Duration) {
        self.conditions.observe(snapshot, events, now);
        for layer in &mut self.layers {
            layer.variant_conditions.observe(snapshot, events, now);
        }
    }

    /// Registry failure hides only conditional layers.
    pub fn clear_conditions(&mut self, now: Duration) {
        self.baseline(
            &Snapshot {
                revision: 0,
                event_cursor: self.event_cursor(),
                states: Vec::new(),
            },
            now,
        );
    }

    /// Input failure hides only input-dependent layers; other layers still render.
    /// Call advance_input after recovery to make them visible again.
    pub fn input_unavailable(&mut self) {
        self.reset_input();
        self.input_available = false;
    }

    pub fn reset_input(&mut self) {
        for layer in &mut self.layers {
            for simulation in
                std::iter::once(&mut layer.simulation).chain(&mut layer.variant_simulations)
            {
                match simulation {
                    Simulation::Reactive(sim) => sim.clear(),
                    Simulation::Ripples(sim) => **sim = RippleSimulation::default(),
                    Simulation::None => {}
                }
            }
        }
        self.input_available = true;
    }

    pub fn advance_input(&mut self, dt: f32, keys: &[(u16, f32)]) {
        self.input_available = true;
        for layer in &mut self.layers {
            if !layer.definition.enabled {
                continue;
            }
            for simulation in std::iter::once(&mut layer.simulation).chain(
                layer
                    .variant_simulations
                    .iter_mut()
                    .zip(&layer.definition.variants)
                    .filter(|(_, variant)| variant.enabled)
                    .map(|(simulation, _)| simulation),
            ) {
                match simulation {
                    Simulation::Reactive(sim) => sim.advance(dt, keys.iter().copied()),
                    Simulation::Ripples(sim) => sim.advance(dt, keys.iter().copied()),
                    Simulation::None => {}
                }
            }
        }
    }

    /// Deterministic isolated demo input, identical for GUI/headless callers.
    /// This never publishes facts or reads a shared live registry.
    pub fn preview_tick(&mut self, tick: u32) {
        self.preview_tick_at(tick, 0.1);
    }

    /// Same demo at a caller-supplied simulation step (for FPS-faithful previews).
    pub fn preview_tick_at(&mut self, tick: u32, dt: f32) {
        if tick == 0 {
            self.reset_input();
        }
        const KEYS: [u16; 8] = [0x04, 0x09, 0x0d, 0x09, 0x04, 0x09, 0x0f, 0x09];
        let input = (tick % 4 < 2).then_some((KEYS[((tick / 4) % 8) as usize], 0.9));
        let keys: Vec<_> = input.into_iter().collect();
        self.advance_input(dt, &keys);
    }

    /// ctx supplies geometry and display tick; elapsed is UNSCALED setup time.
    /// Global brightness multiplies layer brightness once; palette/speed are
    /// canonical per-layer settings, independent of the legacy global clock.
    /// Uncovered keys and non-key cells are black. Higher stack index wins,
    /// including black pixels. No pixels from previous frames are retained.
    pub fn render(&self, ctx: &RenderContext<'_>, elapsed: Duration) -> Frame {
        let layers = self.composition(ctx, elapsed);
        compositor::compose(Frame::black(), ctx.layout, &layers, Duration::ZERO)
    }

    pub fn selected_appearance(&self, layer_id: &str) -> Option<SelectedAppearance<'_>> {
        let layer = self
            .layers
            .iter()
            .find(|layer| layer.definition.id == layer_id)?;
        let (appearance, simulation) = layer.appearance(&self.conditions)?;
        (self.input_available || matches!(simulation, Simulation::None)).then_some(appearance)
    }

    /// Read-only logical-layer diagnostics against the complete canonical stack.
    /// Inactive conditions retain target coverage; black pixels are still opaque.
    pub fn diagnostics(&self, ctx: &RenderContext<'_>) -> Vec<crate::automation::RuleDiagnostic> {
        let contributions = self.composition(ctx, Duration::ZERO);
        let base_conditions = self.conditions.diagnostics(ctx);
        self.layers
            .iter()
            .enumerate()
            .map(|(index, layer)| {
                let definition = &layer.definition;
                let appearance = self.selected_appearance(&definition.id);
                let variant_conditions = layer.variant_conditions.diagnostics(ctx);
                let selected_condition = appearance.and_then(|selected| {
                    if let Some(id) = selected.variant_id {
                        variant_conditions
                            .iter()
                            .find(|condition| condition.rule_id == id)
                    } else {
                        base_conditions
                            .iter()
                            .find(|condition| condition.rule_id == definition.id)
                    }
                });
                let matched = if let Some(selected) = appearance {
                    matches!(selected.activation, Activation::Always {})
                        || selected_condition.is_some_and(|condition| condition.matched)
                } else {
                    base_conditions
                        .iter()
                        .filter(|condition| condition.rule_id == definition.id)
                        .chain(variant_conditions.iter())
                        .any(|condition| condition.matched)
                };
                let targeted_keys = ctx
                    .layout
                    .keys()
                    .iter()
                    .filter(|key| {
                        matches!(definition.target, Target::Whole {})
                            || layer.coordinates.contains(&key.coord)
                            || layer.zones.contains(&key.zone)
                    })
                    .count();
                let visible_keys = ctx
                    .layout
                    .keys()
                    .iter()
                    .filter(|key| {
                        compositor::winning_layer(&contributions, key, Duration::ZERO)
                            .is_some_and(|winner| contributions[winner].priority == index as i32)
                    })
                    .count();
                crate::automation::RuleDiagnostic {
                    rule_id: definition.id.clone(),
                    scene: definition.name.clone(),
                    matched,
                    active: appearance.is_some(),
                    priority: index as i32,
                    expires: appearance.and_then(|selected| selected.expires),
                    targeted_keys,
                    visible_keys,
                }
            })
            .collect()
    }

    pub fn composition(
        &self,
        ctx: &RenderContext<'_>,
        elapsed: Duration,
    ) -> Vec<compositor::Layer> {
        self.layers
            .iter()
            .enumerate()
            .filter_map(|(index, layer)| {
                let definition = &layer.definition;
                let (appearance, simulation) = layer.appearance(&self.conditions)?;
                if !self.input_available && !matches!(simulation, Simulation::None) {
                    return None;
                }
                let mask = match definition.target {
                    Target::Whole {} => KeyMask::Whole,
                    Target::Selection { .. } => KeyMask::Coordinates(
                        ctx.layout
                            .keys()
                            .iter()
                            .filter(|key| {
                                layer.coordinates.contains(&key.coord)
                                    || layer.zones.contains(&key.zone)
                            })
                            .map(|key| key.coord)
                            .collect(),
                    ),
                };
                Some(compositor::Layer {
                    frame: render_appearance(
                        appearance.renderer,
                        simulation,
                        appearance.brightness,
                        ctx,
                        appearance.elapsed.unwrap_or(elapsed),
                        &mask,
                    ),
                    mask,
                    priority: index as i32,
                    expires: None,
                })
            })
            .collect()
    }
}

impl CompiledLayer {
    fn appearance<'a>(
        &'a self,
        conditions: &AutomationRuntime,
    ) -> Option<(SelectedAppearance<'a>, &'a Simulation)> {
        let layer = &self.definition;
        if !layer.enabled {
            return None;
        }
        let selected = layer
            .variants
            .iter()
            .enumerate()
            .filter(|(_, variant)| {
                variant.enabled
                    && activation_active(&variant.id, &variant.activation, &self.variant_conditions)
            })
            .max_by_key(|(index, variant)| (is_transient(&variant.activation), *index));
        let (variant_id, activation, renderer, brightness, simulation, timing, observed_at) =
            if let Some((index, variant)) = selected {
                (
                    Some(variant.id.as_str()),
                    &variant.activation,
                    &variant.renderer,
                    variant.brightness.unwrap_or(layer.brightness),
                    &self.variant_simulations[index],
                    self.variant_conditions.activation_timing(&variant.id),
                    self.variant_conditions.observed_at(),
                )
            } else {
                if !activation_active(&layer.id, &layer.activation, conditions) {
                    return None;
                }
                (
                    None,
                    &layer.activation,
                    &layer.renderer,
                    layer.brightness,
                    &self.simulation,
                    conditions.activation_timing(&layer.id),
                    conditions.observed_at(),
                )
            };
        Some((
            SelectedAppearance {
                layer_id: &layer.id,
                variant_id,
                activation,
                renderer,
                brightness,
                started: timing.map(|(started, _)| started),
                expires: timing.and_then(|(_, expires)| expires),
                elapsed: timing.map(|(started, _)| observed_at.saturating_sub(started)),
            },
            simulation,
        ))
    }
}

fn render_appearance(
    renderer: &Renderer,
    simulation: &Simulation,
    brightness: u8,
    ctx: &RenderContext<'_>,
    elapsed: Duration,
    mask: &KeyMask,
) -> Frame {
    let default_palette = renderer
        .catalog_id()
        .and_then(|id| catalog::all().iter().find(|entry| entry.id == id))
        .map_or(PaletteName::default(), |entry| entry.defaults.palette);
    let (palette, speed) = match renderer {
        Renderer::Effect { palette, speed, .. } => (palette.unwrap_or(default_palette), *speed),
        Renderer::Reactive { palette, .. } | Renderer::Ripples { palette, .. } => {
            (palette.unwrap_or(default_palette), 100)
        }
        _ => (default_palette, 100),
    };
    let ctx = RenderContext {
        animation_seconds: elapsed.as_secs_f64() * f64::from(speed) / 100.0,
        brightness: ((u16::from(brightness) * u16::from(ctx.brightness)) / 255) as u8,
        palette,
        ..ctx.clone()
    };
    match renderer {
        Renderer::Effect { effect, .. } => effect.render(&ctx),
        Renderer::Reactive { .. } => match simulation {
            Simulation::Reactive(sim) => sim.render(&ctx),
            _ => unreachable!(),
        },
        Renderer::Ripples {
            base_color,
            ripple_color,
            ..
        } => {
            let Simulation::Ripples(sim) = simulation else {
                unreachable!()
            };
            let geometry: Vec<_> = ctx
                .layout
                .keys()
                .iter()
                .map(|key| KeyGeometry {
                    row: key.coord.row,
                    column: key.coord.column,
                    x: key.x,
                    y: key.y,
                })
                .collect();
            let palette = ctx.palette.palette();
            let colors = sim.render(
                &geometry,
                |position| {
                    let color = palette.gradient(position);
                    [color.red, color.green, color.blue]
                },
                ctx.brightness,
                *base_color,
                *ripple_color,
            );
            let mut frame = Frame::black();
            for (key, [r, g, b]) in ctx.layout.keys().iter().zip(colors) {
                frame.set_coord(key.coord, Color::new(r, g, b));
            }
            frame
        }
        Renderer::Focus {
            focus_minutes,
            break_minutes,
            cycles,
            start_paused,
            meeting_safe,
            dim,
        } => {
            let config = FocusConfig {
                focus_minutes: *focus_minutes,
                break_minutes: *break_minutes,
                cycles: *cycles,
                start_paused: *start_paused,
                meeting_safe: *meeting_safe,
                dim: *dim,
            };
            focus::render_focus(&ctx, focus::state_after(&config, elapsed), *dim)
        }
        Renderer::Solid { color }
        | Renderer::Pulse { color, .. }
        | Renderer::Sweep { color, .. } => {
            let mut color = Color::new(color[0], color[1], color[2]).scale(ctx.brightness);
            if let Renderer::Pulse { period_ms, .. } = renderer {
                let phase = elapsed.as_secs_f64() * 1000.0 / *period_ms as f64;
                color = color.scale(
                    (64.0 + 191.0 * (0.5 - 0.5 * (phase * std::f64::consts::TAU).cos())) as u8,
                );
            }
            let mut keys: Vec<_> = ctx
                .layout
                .keys()
                .iter()
                .filter(|key| mask.contains(key))
                .collect();
            keys.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
            let count = keys.len();
            let mut frame = Frame::black();
            for (index, key) in keys.into_iter().enumerate() {
                let amount = if let Renderer::Sweep { period_ms, .. } = renderer {
                    let head = (elapsed.as_secs_f64() * (count as f64 * 1000.0 / *period_ms as f64))
                        as usize
                        % count.max(1);
                    let distance = (head + count - index) % count.max(1);
                    if distance < 4 {
                        255u8.saturating_sub((distance * 60) as u8)
                    } else {
                        0
                    }
                } else {
                    255
                };
                frame.set_coord(key.coord, color.scale(amount));
            }
            frame
        }
    }
}

fn needs_input(renderer: &Renderer) -> bool {
    matches!(
        renderer,
        Renderer::Reactive { .. } | Renderer::Ripples { .. }
    )
}

impl Simulation {
    fn for_renderer(renderer: &Renderer) -> Self {
        match renderer {
            Renderer::Reactive { effect, .. } => {
                Self::Reactive(Box::new(ReactiveSimulation::new(*effect)))
            }
            Renderer::Ripples { .. } => Self::Ripples(Box::default()),
            _ => Self::None,
        }
    }
}

fn is_transient(activation: &Activation) -> bool {
    matches!(
        activation,
        Activation::OnEvent { .. } | Activation::OnTransition { .. }
    )
}

fn activation_active(id: &str, activation: &Activation, conditions: &AutomationRuntime) -> bool {
    matches!(activation, Activation::Always {}) || conditions.is_active(id)
}

fn activation_rule(id: &str, activation: &Activation) -> Option<Rule> {
    let (source, instance, mode, state, event, from_state, duration_ms) = match activation {
        Activation::Always {} => return None,
        Activation::WhileState {
            source,
            instance,
            state,
        } => (
            source,
            instance,
            RuleMode::WhileState,
            state.clone(),
            None,
            None,
            None,
        ),
        Activation::OnEvent {
            source,
            instance,
            event,
            duration_ms,
        } => (
            source,
            instance,
            RuleMode::OnEvent,
            None,
            Some(event.clone()),
            None,
            Some(*duration_ms),
        ),
        Activation::OnTransition {
            source,
            instance,
            from_state,
            state,
            duration_ms,
        } => (
            source,
            instance,
            RuleMode::OnTransition,
            Some(state.clone()),
            None,
            from_state.clone(),
            Some(*duration_ms),
        ),
    };
    Some(Rule {
        id: id.into(),
        source: source.clone(),
        instance: instance.clone(),
        mode,
        state,
        event,
        from_state,
        duration_ms,
        scene: "condition".into(),
        priority: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::EffectKind;
    use crate::external::{Message, Payload, Registry};
    use crate::layout::KeyboardLayout;
    use crate::reactive::ReactiveKind;
    use crate::setup::{BehaviorVariant, SCHEMA_VERSION, TargetZone};
    use std::collections::BTreeMap;

    fn layer(id: &str, renderer: Renderer, target: Target) -> Layer {
        Layer {
            id: id.into(),
            name: id.into(),
            enabled: true,
            brightness: 255,
            activation: Activation::Always {},
            variants: Vec::new(),
            renderer,
            target,
        }
    }
    fn document(layers: Vec<Layer>) -> SetupDocument {
        SetupDocument {
            schema_version: SCHEMA_VERSION,
            revision: 0,
            id: "test".into(),
            name: "Test".into(),
            layers,
            groups: BTreeMap::new(),
        }
    }
    fn effect(effect: EffectKind) -> Renderer {
        Renderer::Effect {
            effect,
            palette: None,
            speed: 100,
        }
    }
    fn selection(keys: &[&str]) -> Target {
        Target::Selection {
            keys: keys.iter().map(|key| key.to_string()).collect(),
            zones: vec![],
            groups: vec![],
        }
    }
    fn context<'a>(info: &'a DeviceInfo, layout: &'a KeyboardLayout) -> RenderContext<'a> {
        RenderContext {
            info,
            layout,
            brightness: 255,
            palette: PaletteName::Heat,
            tick: 11,
            animation_seconds: 999.0,
        }
    }
    fn render(doc: &SetupDocument, seconds: f64) -> Frame {
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        CompiledSetup::compile(doc)
            .unwrap()
            .render(&context(&info, &layout), Duration::from_secs_f64(seconds))
    }
    fn publish(registry: &mut Registry, sequence: u64, payload: Payload, now: u64) {
        registry
            .apply(
                Message {
                    schema_version: 1,
                    source: "pi".into(),
                    instance: "a".into(),
                    sequence,
                    payload,
                },
                Duration::from_millis(now),
            )
            .unwrap();
    }
    fn observe(runtime: &mut CompiledSetup, registry: &mut Registry, now: u64) {
        let now = Duration::from_millis(now);
        registry.expire(now).unwrap();
        runtime.observe(
            &registry.snapshot(),
            &registry.events_since(runtime.event_cursor()),
            now,
        );
    }

    fn fact(registry: &mut Registry, instance: &str, sequence: u64, payload: Payload, now: u64) {
        registry
            .apply(
                Message {
                    schema_version: 1,
                    source: "custom-source".into(),
                    instance: instance.into(),
                    sequence,
                    payload,
                },
                Duration::from_millis(now),
            )
            .unwrap();
    }
    fn state(value: &str, ttl_ms: u64) -> Payload {
        Payload::State {
            state: value.into(),
            ttl_ms,
        }
    }
    fn event(ttl_ms: u64) -> Payload {
        Payload::Event {
            event: "finished".into(),
            ttl_ms,
        }
    }
    fn while_state(value: Option<&str>) -> Activation {
        Activation::WhileState {
            source: "custom-source".into(),
            instance: None,
            state: value.map(str::to_owned),
        }
    }
    fn variant(id: &str, activation: Activation, color: [u8; 3]) -> BehaviorVariant {
        BehaviorVariant {
            id: id.into(),
            name: id.into(),
            enabled: true,
            activation,
            renderer: Renderer::Solid { color },
            brightness: None,
        }
    }
    fn behavior() -> Layer {
        let mut layer = layer(
            "sessions",
            Renderer::Solid { color: [10; 3] },
            selection(&["F1"]),
        );
        layer.activation = while_state(None);
        layer.variants = vec![
            variant(
                "finished",
                Activation::OnEvent {
                    source: "custom-source".into(),
                    instance: None,
                    event: "finished".into(),
                    duration_ms: 50,
                },
                [0, 255, 0],
            ),
            variant("idle", while_state(Some("idle")), [0, 0, 255]),
            variant("working", while_state(Some("working")), [255, 0, 0]),
        ];
        layer
    }
    fn selected(runtime: &CompiledSetup) -> Option<&str> {
        runtime
            .selected_appearance("sessions")
            .and_then(|appearance| appearance.variant_id)
    }

    #[test]
    fn logical_behavior_aggregates_sessions_with_one_targeted_appearance_and_fixed_deadlines() {
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = context(&info, &layout);
        let mut runtime = CompiledSetup::compile(&document(vec![behavior()])).unwrap();
        let mut registry = Registry::default();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        fact(&mut registry, "a", 1, state("idle", 200), 1);
        observe(&mut runtime, &mut registry, 1);
        assert_eq!(selected(&runtime), Some("idle"));
        fact(&mut registry, "a", 2, state("working", 200), 10);
        fact(&mut registry, "b", 1, state("idle", 200), 15);
        observe(&mut runtime, &mut registry, 15);
        assert_eq!(selected(&runtime), Some("working"));
        fact(&mut registry, "a", 3, event(200), 20);
        let repeated_batch = registry.events_since(runtime.event_cursor());
        observe(&mut runtime, &mut registry, 20);
        assert_eq!(selected(&runtime), Some("finished")); // transient beats later working
        let appearance = runtime.selected_appearance("sessions").unwrap();
        assert_eq!(appearance.started, Some(Duration::from_millis(20)));
        assert_eq!(appearance.expires, Some(Duration::from_millis(70)));
        let composed = runtime.composition(&ctx, Duration::from_millis(20));
        assert_eq!(composed.len(), 1);
        assert_eq!(composed[0].priority, 0);
        let frame = runtime.render(&ctx, Duration::from_millis(20));
        assert_eq!(
            frame.get_coord(canonical_coord("F1").unwrap()),
            Color::new(0, 255, 0)
        );
        assert_eq!(
            frame.get_coord(canonical_coord("F2").unwrap()),
            Color::BLACK
        );
        fact(&mut registry, "a", 4, state("working", 500), 30);
        runtime.observe(
            &registry.snapshot(),
            &repeated_batch,
            Duration::from_millis(30),
        );
        assert_eq!(
            runtime.selected_appearance("sessions").unwrap().expires,
            Some(Duration::from_millis(70))
        );
        observe(&mut runtime, &mut registry, 69);
        assert_eq!(selected(&runtime), Some("finished"));
        observe(&mut runtime, &mut registry, 70);
        assert_eq!(selected(&runtime), Some("working"));
        fact(&mut registry, "a", 5, Payload::Remove, 80);
        observe(&mut runtime, &mut registry, 80);
        assert_eq!(selected(&runtime), Some("idle"));
        observe(&mut runtime, &mut registry, 215);
        assert!(runtime.selected_appearance("sessions").is_none());
        assert!(runtime.composition(&ctx, Duration::ZERO).is_empty());
    }

    #[test]
    fn state_backed_transients_cap_to_original_lease_and_hide_on_emitting_session_removal() {
        let mut definition = behavior();
        if let Activation::OnEvent { duration_ms, .. } = &mut definition.variants[0].activation {
            *duration_ms = 500;
        }
        let mut runtime = CompiledSetup::compile(&document(vec![definition])).unwrap();
        let mut registry = Registry::default();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        fact(&mut registry, "a", 1, state("working", 50), 0);
        fact(&mut registry, "b", 1, state("idle", 500), 0);
        fact(&mut registry, "a", 2, event(500), 10);
        observe(&mut runtime, &mut registry, 10);
        assert_eq!(
            runtime.selected_appearance("sessions").unwrap().expires,
            Some(Duration::from_millis(50))
        );
        fact(&mut registry, "a", 3, state("working", 500), 20);
        observe(&mut runtime, &mut registry, 20);
        observe(&mut runtime, &mut registry, 50);
        assert_eq!(selected(&runtime), Some("working")); // heartbeat cannot extend completion
        fact(&mut registry, "a", 4, event(500), 60);
        observe(&mut runtime, &mut registry, 60);
        assert_eq!(selected(&runtime), Some("finished"));
        fact(&mut registry, "a", 5, Payload::Remove, 70);
        observe(&mut runtime, &mut registry, 70);
        assert_eq!(selected(&runtime), Some("idle")); // another session cannot keep a's transient alive
        fact(&mut registry, "b", 2, event(500), 80);
        observe(&mut runtime, &mut registry, 80);
        observe(&mut runtime, &mut registry, 500);
        assert!(runtime.selected_appearance("sessions").is_none());
    }

    #[test]
    fn behavior_transitions_heartbeats_repeated_snapshots_and_resume_never_replay() {
        let mut definition = behavior();
        definition.variants.push(variant(
            "transition",
            Activation::OnTransition {
                source: "custom-source".into(),
                instance: None,
                from_state: Some("working".into()),
                state: "idle".into(),
                duration_ms: 50,
            },
            [255, 255, 0],
        ));
        let mut runtime = CompiledSetup::compile(&document(vec![definition])).unwrap();
        let mut registry = Registry::default();
        fact(&mut registry, "a", 1, state("working", 500), 0);
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        fact(&mut registry, "a", 2, state("idle", 500), 10);
        fact(&mut registry, "a", 3, event(500), 10);
        observe(&mut runtime, &mut registry, 10);
        assert_eq!(selected(&runtime), Some("transition")); // later transient wins tie
        fact(&mut registry, "a", 4, state("idle", 500), 20);
        observe(&mut runtime, &mut registry, 20);
        assert_eq!(
            runtime.selected_appearance("sessions").unwrap().expires,
            Some(Duration::from_millis(60))
        );
        observe(&mut runtime, &mut registry, 30);
        runtime.baseline(&registry.snapshot(), Duration::from_millis(40));
        assert_eq!(selected(&runtime), Some("idle"));
        runtime.observe(
            &registry.snapshot(),
            &registry.events_since(0),
            Duration::from_millis(45),
        );
        assert_eq!(selected(&runtime), Some("idle"));
        observe(&mut runtime, &mut registry, 60);
        assert_eq!(selected(&runtime), Some("idle"));
        runtime.clear_conditions(Duration::from_millis(70));
        assert!(runtime.selected_appearance("sessions").is_none());
        runtime.baseline(&registry.snapshot(), Duration::from_millis(80));
        assert_eq!(selected(&runtime), Some("idle"));
    }

    #[test]
    fn event_only_behavior_obeys_event_lease_disabled_variants_and_base_fallback() {
        let mut definition = behavior();
        definition.variants[0].brightness = Some(128);
        let mut runtime = CompiledSetup::compile(&document(vec![definition.clone()])).unwrap();
        let mut registry = Registry::default();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        fact(&mut registry, "a", 1, event(20), 1);
        observe(&mut runtime, &mut registry, 1);
        assert_eq!(selected(&runtime), Some("finished"));
        assert_eq!(
            runtime.selected_appearance("sessions").unwrap().brightness,
            128
        );
        observe(&mut runtime, &mut registry, 21);
        assert!(runtime.selected_appearance("sessions").is_none());
        fact(&mut registry, "a", 2, state("unknown-state", 50), 22);
        observe(&mut runtime, &mut registry, 22);
        assert_eq!(
            runtime.selected_appearance("sessions").unwrap().variant_id,
            None
        );
        definition.variants[0].enabled = false;
        let mut runtime = CompiledSetup::compile(&document(vec![definition.clone()])).unwrap();
        runtime.baseline(&registry.snapshot(), Duration::from_millis(22));
        fact(&mut registry, "a", 3, event(50), 23);
        observe(&mut runtime, &mut registry, 23);
        assert_eq!(
            runtime.selected_appearance("sessions").unwrap().variant_id,
            None
        );
        definition.enabled = false;
        let mut runtime = CompiledSetup::compile(&document(vec![definition])).unwrap();
        runtime.baseline(&registry.snapshot(), Duration::from_millis(23));
        assert!(runtime.selected_appearance("sessions").is_none());
    }

    #[test]
    fn variant_input_budget_is_per_logical_layer_and_input_failure_hides_only_selected_input() {
        let mut definition = behavior();
        definition.renderer = Renderer::Reactive {
            effect: ReactiveKind::Heatmap,
            palette: None,
        };
        definition.variants[1].renderer = Renderer::Ripples {
            palette: None,
            base_color: None,
            ripple_color: None,
        };
        definition.variants[2].renderer = Renderer::Reactive {
            effect: ReactiveKind::Afterimage,
            palette: None,
        };
        let mut runtime = CompiledSetup::compile(&document(vec![definition.clone()])).unwrap();
        assert!(runtime.requirements().analog_input);
        let other = Layer {
            id: "other".into(),
            ..definition.clone()
        };
        assert!(CompiledSetup::compile(&document(vec![definition.clone(), other])).is_err());
        let mut registry = Registry::default();
        fact(&mut registry, "a", 1, state("working", 100), 0);
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        assert_eq!(selected(&runtime), Some("working"));
        runtime.preview_tick(1);
        runtime.input_unavailable();
        assert!(runtime.selected_appearance("sessions").is_none());
        fact(&mut registry, "a", 2, event(50), 1);
        observe(&mut runtime, &mut registry, 1);
        assert_eq!(selected(&runtime), Some("finished")); // solid transient unaffected by input failure
        runtime.advance_input(0.1, &[]);
        observe(&mut runtime, &mut registry, 51);
        assert_eq!(selected(&runtime), Some("working"));
        definition.renderer = Renderer::Solid { color: [0; 3] };
        definition.variants[1].enabled = false;
        definition.variants[2].enabled = false;
        assert!(
            !CompiledSetup::compile(&document(vec![definition]))
                .unwrap()
                .requirements()
                .analog_input
        );
    }

    #[test]
    fn variant_rule_capacity_is_bounded_per_layer_not_legacy_global_64() {
        let mut definition = behavior();
        definition.variants = (0..16)
            .map(|index| {
                variant(
                    &format!("variant-{index}"),
                    while_state(Some("working")),
                    [1; 3],
                )
            })
            .collect();
        let doc = document(
            (0..5)
                .map(|index| Layer {
                    id: format!("layer-{index}"),
                    ..definition.clone()
                })
                .collect(),
        );
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        let mut registry = Registry::default();
        fact(&mut registry, "a", 1, state("working", 50), 0);
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        assert_eq!(
            runtime.selected_appearance("layer-4").unwrap().variant_id,
            Some("variant-15")
        );
    }

    #[test]
    fn variant_brightness_overrides_once_and_black_variant_is_opaque_over_background() {
        let mut status = behavior();
        status.brightness = 32;
        status.variants = vec![
            variant("first", Activation::Always {}, [255; 3]),
            variant("last", Activation::Always {}, [255, 0, 0]),
        ];
        status.variants[1].brightness = Some(128);
        let background = layer(
            "background",
            Renderer::Solid { color: [255; 3] },
            Target::Whole {},
        );
        let mut doc = document(vec![background, status]);
        let runtime = CompiledSetup::compile(&doc).unwrap();
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let mut ctx = context(&info, &layout);
        ctx.brightness = 128;
        assert_eq!(selected(&runtime), Some("last"));
        assert_eq!(
            runtime
                .render(&ctx, Duration::ZERO)
                .get_coord(canonical_coord("F1").unwrap()),
            Color::new(64, 0, 0)
        );
        assert_eq!(runtime.composition(&ctx, Duration::ZERO).len(), 2);
        doc.layers[1].variants[1].renderer = Renderer::Solid { color: [0; 3] };
        let frame = CompiledSetup::compile(&doc)
            .unwrap()
            .render(&ctx, Duration::ZERO);
        assert_eq!(
            frame.get_coord(canonical_coord("F1").unwrap()),
            Color::BLACK
        );
        assert_eq!(
            frame.get_coord(canonical_coord("F2").unwrap()),
            Color::new(128, 128, 128)
        );
    }

    #[test]
    fn variant_reconfigure_preserves_deadlines_and_session_identity_across_reorder() {
        let mut doc = document(vec![behavior()]);
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        let mut registry = Registry::default();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        fact(&mut registry, "a", 1, state("working", 200), 0);
        fact(&mut registry, "a", 2, event(200), 10);
        observe(&mut runtime, &mut registry, 10);
        doc.layers[0].variants.reverse();
        doc.layers[0].name = "Renamed".into();
        doc.layers[0].target = Target::Whole {};
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(runtime.event_cursor()),
            Duration::from_millis(20),
        );
        let appearance = runtime.selected_appearance("sessions").unwrap();
        assert_eq!(appearance.variant_id, Some("finished"));
        assert_eq!(appearance.started, Some(Duration::from_millis(10)));
        assert_eq!(appearance.elapsed, Some(Duration::from_millis(10)));
        assert_eq!(appearance.expires, Some(Duration::from_millis(60)));
        fact(&mut registry, "a", 3, Payload::Remove, 30);
        observe(&mut runtime, &mut registry, 30);
        assert!(runtime.selected_appearance("sessions").is_none()); // preserved identity still gates transient
    }

    #[test]
    fn changed_or_new_variants_baseline_without_replaying_events_during_reconfigure() {
        let mut doc = document(vec![behavior()]);
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        let mut registry = Registry::default();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        fact(&mut registry, "a", 1, state("working", 200), 0);
        fact(&mut registry, "a", 2, event(200), 10);
        // Unchanged variants see new events exactly once even if an edit arrives before observe.
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(runtime.event_cursor()),
            Duration::from_millis(10),
        );
        assert_eq!(selected(&runtime), Some("finished"));
        if let Activation::OnEvent { duration_ms, .. } = &mut doc.layers[0].variants[0].activation {
            *duration_ms = 100;
        }
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(0),
            Duration::from_millis(20),
        );
        assert_eq!(selected(&runtime), Some("working"));
        doc.layers[0].variants[0].id = "new-identity".into();
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(0),
            Duration::from_millis(30),
        );
        assert_eq!(selected(&runtime), Some("working"));
        fact(&mut registry, "a", 3, event(200), 40);
        observe(&mut runtime, &mut registry, 40);
        assert_eq!(selected(&runtime), Some("new-identity"));
        doc.layers[0].variants[0].enabled = false;
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(0),
            Duration::from_millis(45),
        );
        doc.layers[0].variants[0].enabled = true;
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(0),
            Duration::from_millis(46),
        );
        assert_eq!(selected(&runtime), Some("working"));
    }

    #[test]
    fn variant_reconfigure_reuses_simulation_by_id_not_index() {
        let mut definition = behavior();
        definition.target = Target::Whole {};
        definition.variants[2].renderer = Renderer::Reactive {
            effect: ReactiveKind::Afterimage,
            palette: None,
        };
        let mut doc = document(vec![definition]);
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        let mut registry = Registry::default();
        fact(&mut registry, "a", 1, state("working", 200), 0);
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        runtime.advance_input(0.1, &[(0x04, 0.9)]);
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = context(&info, &layout);
        let before = runtime.render(&ctx, Duration::ZERO);
        assert_ne!(before, Frame::black());
        doc.layers[0].variants.reverse();
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(0),
            Duration::from_millis(1),
        );
        assert_eq!(before, runtime.render(&ctx, Duration::ZERO));
        runtime.reset_input();
        assert_eq!(runtime.render(&ctx, Duration::ZERO), Frame::black());
    }

    #[test]
    fn latest_session_occurrence_replaces_slot_without_queued_replay() {
        let mut runtime = CompiledSetup::compile(&document(vec![behavior()])).unwrap();
        let mut registry = Registry::default();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        fact(&mut registry, "a", 1, state("working", 200), 0);
        fact(&mut registry, "b", 1, state("idle", 200), 0);
        fact(&mut registry, "a", 2, event(200), 1);
        observe(&mut runtime, &mut registry, 1);
        fact(&mut registry, "b", 2, event(200), 20);
        observe(&mut runtime, &mut registry, 20);
        assert_eq!(
            runtime.selected_appearance("sessions").unwrap().expires,
            Some(Duration::from_millis(70))
        );
        fact(&mut registry, "b", 2, event(200), 25); // duplicate protocol sequence, no renewal
        observe(&mut runtime, &mut registry, 25);
        assert_eq!(
            runtime.selected_appearance("sessions").unwrap().expires,
            Some(Duration::from_millis(70))
        );
        fact(&mut registry, "b", 3, Payload::Remove, 30);
        observe(&mut runtime, &mut registry, 30);
        assert_eq!(selected(&runtime), Some("working")); // a's earlier completion is not replayed
    }

    #[test]
    fn matrix_black_top_reorder_and_disabled_are_opaque_not_pixel_transparency() {
        let matrix = layer("matrix", effect(EffectKind::Matrix), Target::Whole {});
        let black = layer(
            "black",
            Renderer::Solid { color: [0; 3] },
            selection(&["a", "f1"]),
        );
        let mut doc = document(vec![matrix.clone(), black.clone()]);
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let mut ctx = context(&info, &layout);
        ctx.animation_seconds = 0.7;
        let expected = EffectKind::Matrix.render(&ctx);
        let result = render(&doc, 0.7);
        for key in layout.keys() {
            let expected = if ["a", "f1"]
                .iter()
                .any(|name| canonical_coord(name) == Some(key.coord))
            {
                Color::BLACK
            } else {
                expected.get_coord(key.coord)
            };
            assert_eq!(result.get_coord(key.coord), expected);
        }
        assert_ne!(result, Frame::black());
        doc.layers.swap(0, 1);
        assert_eq!(render(&doc, 0.7), expected);
        doc.layers = vec![matrix, black];
        doc.layers[1].enabled = false;
        assert_eq!(render(&doc, 0.7), expected);
    }

    #[test]
    fn reconfigure_preserves_unrelated_input_simulation_across_reorder_and_edit() {
        let mut doc = document(vec![
            layer("matrix", effect(EffectKind::Matrix), Target::Whole {}),
            layer(
                "input",
                Renderer::Reactive {
                    effect: ReactiveKind::Afterimage,
                    palette: None,
                },
                selection(&["a"]),
            ),
        ]);
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        for tick in 0..7 {
            runtime.preview_tick(tick);
        }
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = context(&info, &layout);
        let a = canonical_coord("a").unwrap();
        let before = runtime.render(&ctx, Duration::from_secs(1)).get_coord(a);
        let Simulation::Reactive(sim) = &runtime.layers[1].simulation else {
            panic!()
        };
        let address = &**sim as *const _;
        doc.layers[0].renderer = effect(EffectKind::Aurora);
        doc.layers[0].target = selection(&["s"]);
        doc.layers.swap(0, 1);
        let registry = Registry::default();
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(0),
            Duration::ZERO,
        );
        let Simulation::Reactive(sim) = &runtime.layers[0].simulation else {
            panic!()
        };
        assert_eq!(&**sim as *const _, address);
        assert_eq!(
            runtime.render(&ctx, Duration::from_secs(1)).get_coord(a),
            before
        );
        assert_eq!(
            runtime.layers[1].definition.renderer,
            effect(EffectKind::Aurora)
        );
        let mut invalid = doc.clone();
        invalid.layers[0].name.clear();
        assert!(CompiledSetup::compile(&invalid).is_err());
        assert_eq!(
            runtime.render(&ctx, Duration::from_secs(1)).get_coord(a),
            before
        );
    }

    #[test]
    fn reconfigure_keeps_deadlines_and_pending_events_but_never_replays_for_new_rules() {
        let mut transient = layer(
            "event",
            Renderer::Solid { color: [0, 255, 0] },
            Target::Whole {},
        );
        transient.activation = Activation::OnEvent {
            source: "pi".into(),
            instance: None,
            event: "done".into(),
            duration_ms: 200,
        };
        let mut doc = document(vec![
            layer("base", effect(EffectKind::Matrix), Target::Whole {}),
            transient,
        ]);
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        let mut registry = Registry::default();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        publish(
            &mut registry,
            1,
            Payload::Event {
                event: "done".into(),
                ttl_ms: 1000,
            },
            20,
        );
        // This event has not been ticked yet: adoption must not drop it for the
        // old rule or replay it for a freshly duplicated rule.
        let mut new_rule = doc.layers[1].clone();
        new_rule.id = "new".into();
        doc.layers.push(new_rule);
        doc.layers[0].renderer = effect(EffectKind::Aurora);
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(runtime.event_cursor()),
            Duration::from_millis(100),
        );
        assert!(runtime.conditions.is_active("event"));
        assert!(!runtime.conditions.is_active("new"));
        observe(&mut runtime, &mut registry, 219);
        assert!(runtime.conditions.is_active("event"));
        // Another unrelated edit must preserve, not extend, the original deadline.
        doc.layers[0].brightness = 90;
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(runtime.event_cursor()),
            Duration::from_millis(219),
        );
        observe(&mut runtime, &mut registry, 220);
        assert!(!runtime.conditions.is_active("event"));
        assert!(!runtime.conditions.is_active("new"));
        doc.layers[1].activation = Activation::OnEvent {
            source: "pi".into(),
            instance: None,
            event: "done".into(),
            duration_ms: 900,
        };
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(0),
            Duration::from_millis(230),
        );
        assert!(!runtime.conditions.is_active("event"));
    }

    #[test]
    fn disjoint_effects_and_union_masks_preserve_sparse_physical_gaps() {
        let mut doc = document(vec![
            layer(
                "aurora",
                effect(EffectKind::Aurora),
                selection(&["a", "f1"]),
            ),
            layer(
                "prism",
                effect(EffectKind::Prism),
                Target::Selection {
                    keys: vec!["space".into()],
                    zones: vec![TargetZone::Arrows],
                    groups: vec!["home".into()],
                },
            ),
        ]);
        doc.groups
            .insert("home".into(), vec!["f1".into(), "f3".into()]);
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = context(&info, &layout);
        let runtime = CompiledSetup::compile(&doc).unwrap();
        let contributions = runtime.composition(&ctx, Duration::from_secs(1));
        assert_eq!(contributions.len(), 2);
        let result = runtime.render(&ctx, Duration::from_secs(1));
        for key in layout.keys() {
            let winner = compositor::winning_layer(&contributions, key, Duration::ZERO);
            let expected = winner.map_or(Color::BLACK, |i| {
                contributions[i].frame.get_coord(key.coord)
            });
            assert_eq!(result.get_coord(key.coord), expected);
        }
        let f1 = layout
            .keys()
            .iter()
            .find(|key| key.coord == canonical_coord("f1").unwrap())
            .unwrap();
        assert_eq!(
            compositor::winning_layer(&contributions, f1, Duration::ZERO),
            Some(1)
        );
        assert_eq!(result.get(0, 1), Color::BLACK); // gap before F1, never compressed
        assert_eq!(
            result.get_coord(canonical_coord("s").unwrap()),
            Color::BLACK
        );
        assert_eq!(render(&document(vec![]), 1.0), Frame::black());
    }

    #[test]
    fn palette_speed_layer_brightness_and_global_safety_multiplier_are_applied_once() {
        let mut doc = document(vec![layer(
            "aurora",
            Renderer::Effect {
                effect: EffectKind::Aurora,
                palette: Some(PaletteName::Ocean),
                speed: 230,
            },
            Target::Whole {},
        )]);
        doc.layers[0].brightness = 180;
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let mut ctx = context(&info, &layout);
        ctx.brightness = 90;
        let runtime = CompiledSetup::compile(&doc).unwrap();
        let frame = runtime.render(&ctx, Duration::from_secs(2));
        let expected_ctx = RenderContext {
            brightness: (180u16 * 90 / 255) as u8,
            palette: PaletteName::Ocean,
            animation_seconds: 4.6,
            ..ctx.clone()
        };
        assert_eq!(frame, EffectKind::Aurora.render(&expected_ctx));
        ctx.animation_seconds = 0.0;
        assert_eq!(frame, runtime.render(&ctx, Duration::from_secs(2)));
        ctx.brightness = 0;
        assert_eq!(runtime.render(&ctx, Duration::from_secs(2)), Frame::black());
        ctx.brightness = 255;
        assert_ne!(frame, runtime.render(&ctx, Duration::from_secs(2)));
    }

    #[test]
    fn pulse_period_and_sweep_cover_the_entire_mask_including_black_pixels() {
        let pulse = document(vec![layer(
            "pulse",
            Renderer::Pulse {
                color: [255, 80, 0],
                period_ms: 2000,
            },
            selection(&["a"]),
        )]);
        let a = canonical_coord("a").unwrap();
        assert_eq!(render(&pulse, 0.0).get_coord(a), Color::new(64, 20, 0));
        assert_eq!(render(&pulse, 1.0).get_coord(a), Color::new(255, 80, 0));
        assert_eq!(render(&pulse, 2.0).get_coord(a), Color::new(64, 20, 0));
        let sweep = document(vec![
            layer(
                "base",
                Renderer::Solid { color: [0, 0, 255] },
                Target::Whole {},
            ),
            layer(
                "sweep",
                Renderer::Sweep {
                    color: [255, 0, 0],
                    period_ms: 2000,
                },
                selection(&["a", "s", "d", "f", "g", "h"]),
            ),
        ]);
        let start = render(&sweep, 0.0);
        assert_eq!(start.get_coord(a), Color::new(255, 0, 0));
        assert_eq!(start.get_coord(canonical_coord("s").unwrap()), Color::BLACK);
        assert_eq!(
            start.get_coord(canonical_coord("j").unwrap()),
            Color::new(0, 0, 255)
        );
        assert_eq!(
            start.get_coord(canonical_coord("h").unwrap()),
            Color::new(195, 0, 0)
        );
        let later = render(&sweep, 1.0);
        assert_eq!(later.get_coord(a), Color::new(75, 0, 0));
        assert_eq!(
            later.get_coord(canonical_coord("s").unwrap()),
            Color::new(135, 0, 0)
        );
        assert_eq!(
            later.get_coord(canonical_coord("f").unwrap()),
            Color::new(255, 0, 0)
        );
        assert_eq!(later.get_coord(canonical_coord("g").unwrap()), Color::BLACK);
    }

    #[test]
    fn conditional_base_and_variant_render_on_activation_clock_and_match_legacy_primitives() {
        use crate::automation::SceneKind;
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = context(&info, &layout);
        let keys: Vec<String> = ["F1", "F2", "F3", "F4", "F5", "F6"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        for (renderer, kind, legacy_effect) in [
            (
                Renderer::Pulse {
                    color: [0, 180, 255],
                    period_ms: 2000,
                },
                SceneKind::Pulse,
                None,
            ),
            (
                Renderer::Sweep {
                    color: [0, 180, 255],
                    period_ms: 600,
                },
                SceneKind::Sweep,
                None,
            ),
            (
                effect(EffectKind::Matrix),
                SceneKind::Effect,
                Some(EffectKind::Matrix),
            ),
        ] {
            for as_variant in [false, true] {
                let mut layer = layer(
                    "sessions",
                    renderer.clone(),
                    Target::Selection {
                        keys: keys.clone(),
                        zones: vec![],
                        groups: vec![],
                    },
                );
                layer.activation = while_state(Some("working"));
                if as_variant {
                    let mut appearance = variant("working", layer.activation.clone(), [0; 3]);
                    appearance.renderer = renderer.clone();
                    layer.variants.push(appearance);
                }
                let mut config = AutomationConfig::default();
                config
                    .rules
                    .push(activation_rule("legacy", &layer.activation).unwrap());
                config.scenes.insert(
                    "condition".into(),
                    Scene {
                        kind,
                        effect: legacy_effect,
                        keys: keys.clone(),
                        ..Scene::default()
                    },
                );
                let mut legacy = AutomationRuntime::new(config).unwrap();
                let mut runtime = CompiledSetup::compile(&document(vec![layer])).unwrap();
                let mut registry = Registry::default();
                legacy.baseline(&registry.snapshot(), Duration::ZERO);
                runtime.baseline(&registry.snapshot(), Duration::ZERO);
                fact(&mut registry, "a", 1, state("working", 5000), 10);
                for now in [10, 110, 310, 1010, 2010] {
                    observe(&mut runtime, &mut registry, now);
                    legacy.observe(
                        &registry.snapshot(),
                        &registry.events_since(legacy.event_cursor()),
                        Duration::from_millis(now),
                    );
                    assert_eq!(
                        runtime.selected_appearance("sessions").unwrap().elapsed,
                        Some(Duration::from_millis(now - 10))
                    );
                    let expected = compositor::compose(
                        Frame::black(),
                        &layout,
                        &legacy.layers(&ctx),
                        Duration::from_millis(now),
                    );
                    assert_eq!(
                        runtime.render(&ctx, Duration::from_secs(999)),
                        expected,
                        "{kind:?} variant={as_variant} now={now}"
                    );
                }
            }
        }
    }
    #[test]
    fn every_catalog_renderer_keeps_its_actual_renderer_and_default_palette() {
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        for entry in catalog::all() {
            if let catalog::RendererKind::Static(effect_kind) = entry.renderer {
                let doc = document(vec![layer("effect", effect(effect_kind), Target::Whole {})]);
                let ctx = RenderContext {
                    palette: entry.defaults.palette,
                    animation_seconds: 1.0,
                    ..context(&info, &layout)
                };
                assert_eq!(render(&doc, 1.0), effect_kind.render(&ctx), "{}", entry.id);
            }
        }
    }

    #[test]
    fn live_resource_conflicts_reject_even_conditionally_hidden_analog_layers() {
        let reactive = layer(
            "reactive",
            Renderer::Reactive {
                effect: ReactiveKind::Afterimage,
                palette: None,
            },
            Target::Whole {},
        );
        let mut ripple = layer(
            "ripples",
            Renderer::Ripples {
                palette: None,
                base_color: None,
                ripple_color: None,
            },
            selection(&["a"]),
        );
        ripple.activation = Activation::WhileState {
            source: "absent".into(),
            instance: None,
            state: None,
        };
        let mut doc = document(vec![reactive, ripple]);
        assert!(
            CompiledSetup::compile(&doc)
                .err()
                .unwrap()
                .contains("one enabled analog")
        );
        doc.layers[1].enabled = false;
        let runtime = CompiledSetup::compile(&doc).unwrap();
        assert!(runtime.requirements().analog_input);
        let mut invalid = DeviceInfo::synthetic_80he();
        invalid.max_columns = 21;
        assert!(runtime.validate_device(&invalid).is_err());
    }

    #[test]
    fn reactive_preview_is_isolated_deterministic_and_input_failure_reveals_background() {
        for kind in [
            ReactiveKind::Afterimage,
            ReactiveKind::Heatmap,
            ReactiveKind::Constellation,
        ] {
            let doc = document(vec![
                layer(
                    "base",
                    Renderer::Solid { color: [23, 0, 0] },
                    Target::Whole {},
                ),
                layer(
                    "reaction",
                    Renderer::Reactive {
                        effect: kind,
                        palette: Some(PaletteName::Ocean),
                    },
                    selection(&["a", "s", "d", "f"]),
                ),
            ]);
            let mut a = CompiledSetup::compile(&doc).unwrap();
            let mut b = CompiledSetup::compile(&doc).unwrap();
            let untouched = CompiledSetup::compile(&doc).unwrap();
            let info = DeviceInfo::synthetic_80he();
            let layout = KeyboardLayout::for_device(&info);
            let ctx = context(&info, &layout);
            for tick in 0..12 {
                a.preview_tick(tick);
                b.preview_tick(tick);
                assert_eq!(
                    a.render(&ctx, Duration::ZERO),
                    b.render(&ctx, Duration::ZERO)
                );
            }
            assert_ne!(
                a.render(&ctx, Duration::ZERO),
                untouched.render(&ctx, Duration::ZERO)
            );
            a.input_unavailable();
            let base = render(&document(vec![doc.layers[0].clone()]), 0.0);
            assert_eq!(a.render(&ctx, Duration::ZERO), base);
            a.advance_input(0.1, &[(0x04, 1.0)]);
            assert_ne!(a.render(&ctx, Duration::ZERO), base);
            a.preview_tick(0);
            b.preview_tick(0);
            assert_eq!(
                a.render(&ctx, Duration::ZERO),
                b.render(&ctx, Duration::ZERO)
            );
        }
    }

    #[test]
    fn ripple_two_tone_and_focus_reuse_shared_pure_implementations() {
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = context(&info, &layout);
        let doc = document(vec![layer(
            "ripple",
            Renderer::Ripples {
                palette: None,
                base_color: Some([0, 0, 20]),
                ripple_color: Some([255, 255, 0]),
            },
            selection(&["a", "s"]),
        )]);
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        assert_eq!(
            runtime
                .render(&ctx, Duration::ZERO)
                .get_coord(canonical_coord("a").unwrap()),
            Color::new(0, 0, 20)
        );
        runtime.advance_input(0.0, &[(0x04, 1.0)]);
        assert_eq!(
            runtime
                .render(&ctx, Duration::ZERO)
                .get_coord(canonical_coord("a").unwrap()),
            Color::new(255, 255, 0)
        );
        let doc = document(vec![layer(
            "focus",
            Renderer::Focus {
                focus_minutes: 1,
                break_minutes: 1,
                cycles: 2,
                start_paused: false,
                meeting_safe: false,
                dim: true,
            },
            Target::Whole {},
        )]);
        for seconds in [0, 30, 60, 90, 120, 240] {
            let config = FocusConfig {
                focus_minutes: 1,
                break_minutes: 1,
                cycles: 2,
                dim: true,
                ..FocusConfig::default()
            };
            assert_eq!(
                render(&doc, seconds as f64),
                focus::render_focus(
                    &ctx,
                    focus::state_after(&config, Duration::from_secs(seconds)),
                    true
                )
            );
        }
    }

    #[test]
    fn condition_leases_events_transitions_and_resume_share_automation_semantics() {
        let mut state = layer(
            "state",
            Renderer::Solid { color: [0, 0, 255] },
            Target::Whole {},
        );
        state.activation = Activation::WhileState {
            source: "pi".into(),
            instance: Some("a".into()),
            state: Some("working".into()),
        };
        let mut event = layer(
            "event",
            Renderer::Solid { color: [0, 255, 0] },
            Target::Whole {},
        );
        event.activation = Activation::OnEvent {
            source: "pi".into(),
            instance: None,
            event: "done".into(),
            duration_ms: 200,
        };
        let mut transition = layer(
            "transition",
            Renderer::Solid {
                color: [255, 255, 0],
            },
            Target::Whole {},
        );
        transition.activation = Activation::OnTransition {
            source: "pi".into(),
            instance: None,
            from_state: Some("working".into()),
            state: "idle".into(),
            duration_ms: 100,
        };
        let base = layer(
            "base",
            Renderer::Solid { color: [77, 0, 0] },
            Target::Whole {},
        );
        let doc = document(vec![base, state, event, transition]);
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = context(&info, &layout);
        let color = |runtime: &CompiledSetup| {
            runtime
                .render(&ctx, Duration::ZERO)
                .get_coord(layout.keys()[0].coord)
        };
        let mut registry = Registry::default();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        assert_eq!(color(&runtime), Color::new(77, 0, 0));
        publish(
            &mut registry,
            1,
            Payload::State {
                state: "working".into(),
                ttl_ms: 1000,
            },
            10,
        );
        observe(&mut runtime, &mut registry, 10);
        assert_eq!(color(&runtime), Color::new(0, 0, 255));
        publish(
            &mut registry,
            2,
            Payload::Event {
                event: "done".into(),
                ttl_ms: 1000,
            },
            20,
        );
        observe(&mut runtime, &mut registry, 20);
        assert_eq!(color(&runtime), Color::new(0, 255, 0));
        publish(
            &mut registry,
            3,
            Payload::State {
                state: "idle".into(),
                ttl_ms: 1000,
            },
            30,
        );
        observe(&mut runtime, &mut registry, 30);
        assert_eq!(color(&runtime), Color::new(255, 255, 0));
        observe(&mut runtime, &mut registry, 130); // transition deadline exact
        assert_eq!(color(&runtime), Color::new(0, 255, 0));
        runtime.baseline(&registry.snapshot(), Duration::from_millis(140));
        observe(&mut runtime, &mut registry, 140); // old event never replayed
        assert_eq!(color(&runtime), Color::new(77, 0, 0));
        publish(
            &mut registry,
            4,
            Payload::State {
                state: "working".into(),
                ttl_ms: 100,
            },
            150,
        );
        observe(&mut runtime, &mut registry, 150);
        assert_eq!(color(&runtime), Color::new(0, 0, 255));
        observe(&mut runtime, &mut registry, 250);
        assert_eq!(color(&runtime), Color::new(77, 0, 0));
        runtime.clear_conditions(Duration::from_millis(260));
        assert_eq!(color(&runtime), Color::new(77, 0, 0));
    }
}
