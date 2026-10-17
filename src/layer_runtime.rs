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
            let simulation = match &definition.renderer {
                Renderer::Reactive { effect, .. } => {
                    if definition.enabled {
                        analog_count += 1;
                    }
                    Simulation::Reactive(Box::new(ReactiveSimulation::new(*effect)))
                }
                Renderer::Ripples { .. } => {
                    if definition.enabled {
                        analog_count += 1;
                    }
                    Simulation::Ripples(Box::default())
                }
                _ => Simulation::None,
            };
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
            if let Some(rule) = condition_rule(definition) {
                conditions.rules.push(rule);
            }
            layers.push(CompiledLayer {
                definition: definition.clone(),
                coordinates,
                zones: target_zones,
                simulation,
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
        self.conditions.observe(snapshot, events, now);
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
                    && old.definition.renderer == layer.definition.renderer
            }) {
                std::mem::swap(&mut layer.simulation, &mut old.simulation);
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
    }

    pub fn observe(&mut self, snapshot: &Snapshot, events: &EventBatch, now: Duration) {
        self.conditions.observe(snapshot, events, now);
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
            match &mut layer.simulation {
                Simulation::Reactive(sim) => sim.clear(),
                Simulation::Ripples(sim) => **sim = RippleSimulation::default(),
                Simulation::None => {}
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
            match &mut layer.simulation {
                Simulation::Reactive(sim) => sim.advance(dt, keys.iter().copied()),
                Simulation::Ripples(sim) => sim.advance(dt, keys.iter().copied()),
                Simulation::None => {}
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
                if !definition.enabled
                    || (!matches!(definition.activation, Activation::Always {})
                        && !self.conditions.is_active(&definition.id))
                    || (!self.input_available && !matches!(layer.simulation, Simulation::None))
                {
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
                    frame: layer.render(ctx, elapsed, &mask),
                    mask,
                    priority: index as i32,
                    expires: None,
                })
            })
            .collect()
    }
}

impl CompiledLayer {
    fn render(&self, ctx: &RenderContext<'_>, elapsed: Duration, mask: &KeyMask) -> Frame {
        let renderer = &self.definition.renderer;
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
            brightness: ((u16::from(self.definition.brightness) * u16::from(ctx.brightness)) / 255)
                as u8,
            palette,
            ..ctx.clone()
        };
        match renderer {
            Renderer::Effect { effect, .. } => effect.render(&ctx),
            Renderer::Reactive { .. } => match &self.simulation {
                Simulation::Reactive(sim) => sim.render(&ctx),
                _ => unreachable!(),
            },
            Renderer::Ripples {
                base_color,
                ripple_color,
                ..
            } => {
                let Simulation::Ripples(sim) = &self.simulation else {
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
                    let phase = (elapsed.as_secs_f64() * 1000.0 / *period_ms as f64).fract();
                    color = color.scale(((1.0 - (phase * 2.0 - 1.0).abs()) * 255.0) as u8);
                }
                let keys: Vec<_> = ctx
                    .layout
                    .keys()
                    .iter()
                    .filter(|key| mask.contains(key))
                    .collect();
                let count = if let Renderer::Sweep { period_ms, .. } = renderer {
                    let phase = (elapsed.as_secs_f64() * 1000.0 / *period_ms as f64).fract();
                    (phase * keys.len() as f64).ceil() as usize
                } else {
                    keys.len()
                };
                let mut frame = Frame::black();
                for key in keys.into_iter().take(count) {
                    frame.set_coord(key.coord, color);
                }
                frame
            }
        }
    }
}

fn condition_rule(layer: &Layer) -> Option<Rule> {
    let (source, instance, mode, state, event, from_state, duration_ms) = match &layer.activation {
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
        id: layer.id.clone(),
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
    use crate::setup::{SCHEMA_VERSION, TargetZone};
    use std::collections::BTreeMap;

    fn layer(id: &str, renderer: Renderer, target: Target) -> Layer {
        Layer {
            id: id.into(),
            name: id.into(),
            enabled: true,
            brightness: 255,
            activation: Activation::Always {},
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
        assert_eq!(render(&pulse, 0.0).get_coord(a), Color::BLACK);
        assert_eq!(render(&pulse, 1.0).get_coord(a), Color::new(255, 80, 0));
        assert_eq!(render(&pulse, 2.0).get_coord(a), Color::BLACK);
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
                selection(&["a", "s"]),
            ),
        ]);
        let start = render(&sweep, 0.0);
        assert_eq!(start.get_coord(a), Color::BLACK); // covered even before sweep lights it
        assert_eq!(start.get_coord(canonical_coord("s").unwrap()), Color::BLACK);
        assert_eq!(
            start.get_coord(canonical_coord("d").unwrap()),
            Color::new(0, 0, 255)
        );
        let later = render(&sweep, 1.9);
        assert_eq!(later.get_coord(a), Color::new(255, 0, 0));
        assert_eq!(
            later.get_coord(canonical_coord("s").unwrap()),
            Color::new(255, 0, 0)
        );
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
