//! Read-only, hardware-free explanations of the actual canonical compositor.
//! Facts must be the effective snapshot last observed by the supplied runtime,
//! not an unevaluated transport batch. No event history is replayed here.
use crate::compositor;
use crate::external::{Snapshot, StateSnapshot};
use crate::layer_runtime::{AppearanceCondition, CompiledSetup};
use crate::layout::MatrixCoord;
use crate::render::{Color, RenderContext};
use crate::setup::{Activation, Renderer};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplanationMode {
    Live,
    InputDemo,
    Scenario,
}
impl ExplanationMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Live => "Live facts · draft preview only",
            Self::InputDemo => "Isolated input demo · simulated input, no source facts",
            Self::Scenario => "Scenario · simulated source facts",
        }
    }
}

/// All clocks and facts must describe the SAME evaluation as the displayed frame.
pub struct ExplanationContext<'a> {
    pub render: &'a RenderContext<'a>,
    pub elapsed: Duration,
    pub snapshot: &'a Snapshot,
    pub now: Duration,
    pub mode: ExplanationMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceState {
    pub instance: String,
    pub state: String,
    pub age: Duration,
    pub expires_in: Duration,
    pub fresh: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Freshness {
    NotRequired,
    Missing,
    InstanceMismatch,
    Expired,
    Fresh,
}

#[derive(Clone, Debug)]
pub struct ConditionExplanation {
    pub variant_id: Option<String>,
    pub name: String,
    pub enabled: bool,
    /// Cached evaluator match, NOT inferred from a recent event batch.
    pub matched: bool,
    pub active: bool,
    pub selected: bool,
    pub activation: Activation,
    pub started: Option<Duration>,
    pub expires: Option<Duration>,
    pub freshness: Freshness,
    pub states: Vec<SourceState>,
    pub reason: String,
}
#[derive(Clone, Debug)]
pub struct LayerExplanation {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub covers: bool,
    pub active: bool,
    pub selected_variant: Option<String>,
    pub selected_renderer: Option<Renderer>,
    pub selected_brightness: Option<u8>,
    /// The selected appearance's RGB before cross-layer occlusion, even black.
    pub contribution: Option<Color>,
    pub obscured_by: Option<usize>,
    pub conditions: Vec<ConditionExplanation>,
    pub reason: String,
}
#[derive(Clone, Debug)]
pub struct KeyExplanation {
    pub coord: MatrixCoord,
    pub mode: ExplanationMode,
    pub evaluated_at: Duration,
    pub global_brightness: u8,
    pub color: Color,
    /// Index in the complete canonical bottom-to-top stack, NOT filtered overlays.
    pub winning_layer: Option<usize>,
    pub layers: Vec<LayerExplanation>,
}

/// Reuses actual composed layers, masks, RGB and arbitration. Never mutates the
/// runtime or registry. Unknown/non-key cells return None rather than inventing names.
pub fn explain_key(
    runtime: &CompiledSetup,
    context: &ExplanationContext<'_>,
    coord: MatrixCoord,
) -> Option<KeyExplanation> {
    let key = context
        .render
        .layout
        .keys()
        .iter()
        .find(|k| k.coord == coord)?;
    let contributions = runtime.composition(context.render, context.elapsed);
    let winner = compositor::winning_layer(&contributions, key, Duration::ZERO);
    let winning_layer = winner.map(|i| contributions[i].priority as usize);
    let color = winner.map_or(Color::BLACK, |i| contributions[i].frame.get_coord(coord));
    let layers = runtime
        .inspect_key(context.render, coord)
        .into_iter()
        .enumerate()
        .map(|(index, inspection)| {
            let definition = inspection.definition;
            let selected = runtime.selected_appearance(&definition.id);
            let contribution = contributions
                .iter()
                .find(|layer| layer.priority == index as i32 && layer.mask.contains(key))
                .map(|layer| layer.frame.get_coord(coord));
            let active = selected.is_some();
            let obscured_by = contribution
                .and(winning_layer)
                .filter(|winner| *winner != index);
            let reason = if !definition.enabled {
                "Disabled layer".into()
            } else if !inspection.covers {
                "Does not target this key".into()
            } else if !active {
                "Inactive: no eligible appearance, or input unavailable".into()
            } else if let Some(winner) = obscured_by {
                format!("Obscured by higher stack layer #{}", winner + 1)
            } else if contribution == Some(Color::BLACK) {
                "Wins with opaque black (not transparency)".into()
            } else {
                "Wins: highest active targeting stack layer".into()
            };
            LayerExplanation {
                index,
                id: definition.id.clone(),
                name: definition.name.clone(),
                enabled: definition.enabled,
                covers: inspection.covers,
                active,
                selected_variant: selected.and_then(|s| s.variant_id.map(str::to_owned)),
                selected_renderer: selected.map(|s| s.renderer.clone()),
                selected_brightness: selected.map(|s| s.brightness),
                contribution,
                obscured_by,
                conditions: inspection
                    .conditions
                    .into_iter()
                    .map(|condition| {
                        let is_selected =
                            selected.is_some_and(|s| s.variant_id == condition.variant_id);
                        explain_condition(condition, is_selected, context.snapshot, context.now)
                    })
                    .collect(),
                reason,
            }
        })
        .collect();
    Some(KeyExplanation {
        coord,
        mode: context.mode,
        evaluated_at: context.now,
        global_brightness: context.render.brightness,
        color,
        winning_layer,
        layers,
    })
}

fn state_report(state: &StateSnapshot, now: Duration) -> SourceState {
    SourceState {
        instance: state.identity.instance.clone(),
        state: state.state.clone(),
        age: now.saturating_sub(state.updated_at),
        expires_in: state.expires_at.saturating_sub(now),
        fresh: now < state.expires_at,
    }
}

fn explain_condition(
    condition: AppearanceCondition<'_>,
    selected: bool,
    snapshot: &Snapshot,
    now: Duration,
) -> ConditionExplanation {
    let (source, instance) = match condition.activation {
        Activation::Always {} => (None, None),
        Activation::WhileState {
            source, instance, ..
        }
        | Activation::OnEvent {
            source, instance, ..
        }
        | Activation::OnTransition {
            source, instance, ..
        } => (Some(source), instance.as_ref()),
    };
    let all: Vec<_> = snapshot
        .states
        .iter()
        .filter(|s| Some(&s.identity.source) == source)
        .collect();
    let exact: Vec<_> = all
        .iter()
        .copied()
        .filter(|s| instance.is_none_or(|id| *id == s.identity.instance))
        .collect();
    let freshness = if source.is_none() {
        Freshness::NotRequired
    } else if all.is_empty() {
        Freshness::Missing
    } else if exact.is_empty() {
        Freshness::InstanceMismatch
    } else if exact.iter().all(|s| now >= s.expires_at) {
        Freshness::Expired
    } else {
        Freshness::Fresh
    };
    let reason = if !condition.enabled {
        "Disabled appearance".into()
    } else if selected {
        "Selected by cached evaluator".into()
    } else if condition.active {
        "Eligible but not selected: transient variants outrank persistent; later variants win ties; base is fallback".into()
    } else {
        match condition.activation {
            Activation::WhileState { .. } => match freshness {
                Freshness::Missing => "No currently evaluated source state (missing, removed or expired)",
                Freshness::InstanceMismatch => "Source exists, but exact instance does not match",
                Freshness::Expired => "Evaluated source lease has expired",
                _ => "Current evaluated state does not match",
            },
            _ => "No active cached occurrence: trigger unobserved, baselined, expired or source removed",
        }.into()
    };
    ConditionExplanation {
        variant_id: condition.variant_id.map(str::to_owned),
        name: condition.name.into(),
        enabled: condition.enabled,
        matched: condition.matched,
        active: condition.active,
        selected,
        activation: condition.activation.clone(),
        started: condition.started,
        expires: condition.expires,
        freshness,
        states: exact.into_iter().map(|s| state_report(s, now)).collect(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceInfo;
    use crate::external::{Message, Payload, Registry};
    use crate::key_targets::canonical_coord;
    use crate::layout::KeyboardLayout;
    use crate::render::PaletteName;
    use crate::setup::{Renderer, SetupDocument};

    fn document() -> SetupDocument {
        serde_json::from_value(serde_json::json!({
            "schema_version":1,"id":"explain","name":"Explain","layers":[
                {"id":"matrix","name":"Matrix","renderer":{"kind":"effect","effect":"matrix"},"target":{"kind":"whole"}},
                {"id":"pi","name":"Pi","renderer":{"kind":"solid","color":[0,80,80]},"target":{"kind":"selection","keys":["F3"]},
                 "activation":{"kind":"while-state","source":"pi"},
                 "variants":[
                    {"id":"idle","name":"Idle","activation":{"kind":"while-state","source":"pi","state":"idle"},"renderer":{"kind":"solid","color":[0,255,255]}},
                    {"id":"working","name":"Working","activation":{"kind":"while-state","source":"pi","state":"working"},"renderer":{"kind":"solid","color":[0,0,255]}},
                    {"id":"finished","name":"Finished","activation":{"kind":"on-event","source":"pi","event":"finished","duration_ms":300},"renderer":{"kind":"solid","color":[0,255,0]}}
                 ]},
                {"id":"alert","name":"Alert","renderer":{"kind":"solid","color":[0,0,0]},"target":{"kind":"selection","keys":["F3"]},
                 "activation":{"kind":"while-state","source":"alert","state":"on"}},
                {"id":"disabled","name":"Disabled","enabled":false,"renderer":{"kind":"solid","color":[255,0,0]},"target":{"kind":"whole"}},
                {"id":"arrows","name":"Arrows","renderer":{"kind":"solid","color":[255,255,255]},"target":{"kind":"selection","keys":["ArrowUp"]}}
            ]
        })).unwrap()
    }
    fn fact(
        registry: &mut Registry,
        source: &str,
        instance: &str,
        sequence: u64,
        payload: Payload,
        now: u64,
    ) {
        registry
            .apply(
                Message {
                    schema_version: 1,
                    source: source.into(),
                    instance: instance.into(),
                    sequence,
                    payload,
                },
                Duration::from_millis(now),
            )
            .unwrap();
    }
    fn state(registry: &mut Registry, source: &str, instance: &str, value: &str, ttl_ms: u64) {
        fact(
            registry,
            source,
            instance,
            1,
            Payload::State {
                state: value.into(),
                ttl_ms,
            },
            0,
        );
    }
    fn report(runtime: &CompiledSetup, registry: &Registry, now: u64) -> KeyExplanation {
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = RenderContext {
            animation_seconds: 0.73,
            info: &info,
            layout: &layout,
            brightness: 173,
            palette: PaletteName::Wooting,
            tick: 21,
        };
        let snapshot = registry.snapshot();
        let elapsed = Duration::from_millis(730);
        let context = ExplanationContext {
            render: &ctx,
            elapsed,
            snapshot: &snapshot,
            now: Duration::from_millis(now),
            mode: ExplanationMode::Scenario,
        };
        let coord = canonical_coord("F3").unwrap();
        let cursor = runtime.event_cursor();
        let before = runtime.render(&ctx, elapsed);
        let report = explain_key(runtime, &context, coord).unwrap();
        assert_eq!(report.color, before.get_coord(coord));
        let composed = runtime.composition(&ctx, elapsed);
        let key = layout.keys().iter().find(|k| k.coord == coord).unwrap();
        assert_eq!(
            report.winning_layer,
            compositor::winning_layer(&composed, key, Duration::ZERO)
                .map(|i| composed[i].priority as usize)
        );
        assert_eq!(
            runtime.event_cursor(),
            cursor,
            "inspection cannot consume events"
        );
        assert_eq!(
            runtime.render(&ctx, elapsed),
            before,
            "inspection cannot advance state"
        );
        assert_eq!(
            registry.snapshot(),
            snapshot,
            "inspection cannot publish facts"
        );
        report
    }

    #[test]
    fn f3_blue_explains_all_matching_variants_underlying_matrix_and_opaque_black() {
        let mut registry = Registry::default();
        state(&mut registry, "pi", "idle-session", "idle", 1000);
        state(&mut registry, "pi", "work-session", "working", 1000);
        let mut runtime = CompiledSetup::compile(&document()).unwrap();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        let blue = report(&runtime, &registry, 0);
        assert_eq!(blue.color, Color::new(0, 0, 173));
        assert_eq!(blue.winning_layer, Some(1));
        assert_eq!(blue.layers[0].obscured_by, Some(1));
        assert!(blue.layers[0].contribution.is_some());
        assert_eq!(blue.layers[1].selected_variant.as_deref(), Some("working"));
        assert!(blue.layers[1].conditions[1].matched, "idle also matched");
        assert!(!blue.layers[1].conditions[1].selected);
        assert!(blue.layers[1].conditions[2].selected);
        assert_eq!(blue.layers[1].conditions[2].states.len(), 2);
        assert!(!blue.layers[3].active);
        assert!(blue.layers[3].reason.contains("Disabled"));
        assert!(!blue.layers[4].covers);
        assert!(blue.layers[4].reason.contains("Does not target"));
        state(&mut registry, "alert", "alert-session", "on", 500);
        runtime.observe(
            &registry.snapshot(),
            &registry.events_since(runtime.event_cursor()),
            Duration::ZERO,
        );
        let black = report(&runtime, &registry, 0);
        assert_eq!(black.winning_layer, Some(2));
        assert_eq!(black.color, Color::BLACK);
        assert!(black.layers[2].reason.contains("opaque black"));
        assert_eq!(black.layers[1].obscured_by, Some(2));
    }

    #[test]
    fn reorder_and_source_expiry_keep_report_equal_to_actual_output() {
        let mut registry = Registry::default();
        state(&mut registry, "pi", "session", "working", 100);
        let mut doc = document();
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        assert_eq!(report(&runtime, &registry, 0).winning_layer, Some(1));
        doc.layers.swap(0, 1);
        runtime.reconfigure(
            CompiledSetup::compile(&doc).unwrap(),
            &registry.snapshot(),
            &registry.events_since(runtime.event_cursor()),
            Duration::ZERO,
        );
        assert_eq!(report(&runtime, &registry, 0).winning_layer, Some(1));
        assert_eq!(report(&runtime, &registry, 0).layers[1].id, "matrix");
        registry.expire(Duration::from_millis(100)).unwrap();
        runtime.observe(
            &registry.snapshot(),
            &registry.events_since(runtime.event_cursor()),
            Duration::from_millis(100),
        );
        let expired = report(&runtime, &registry, 100);
        assert!(!expired.layers[0].active);
        assert_eq!(
            expired.layers[0].conditions[2].freshness,
            Freshness::Missing
        );
        assert!(expired.layers[0].conditions[2].reason.contains("expired"));
        assert_eq!(expired.layers[1].id, "matrix");
    }

    #[test]
    fn event_history_is_not_replayed_and_transient_timing_is_cached() {
        let mut registry = Registry::default();
        state(&mut registry, "pi", "session", "working", 1000);
        fact(
            &mut registry,
            "pi",
            "session",
            2,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 500,
            },
            10,
        );
        let mut runtime = CompiledSetup::compile(&document()).unwrap();
        runtime.baseline(&registry.snapshot(), Duration::from_millis(10));
        let baseline = report(&runtime, &registry, 10);
        assert!(!baseline.layers[1].conditions[3].active);
        assert!(
            baseline.layers[1].conditions[3]
                .reason
                .contains("unobserved")
        );
        fact(
            &mut registry,
            "pi",
            "session",
            3,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 500,
            },
            20,
        );
        runtime.observe(
            &registry.snapshot(),
            &registry.events_since(runtime.event_cursor()),
            Duration::from_millis(20),
        );
        let green = report(&runtime, &registry, 20);
        let finished = &green.layers[1].conditions[3];
        assert!(finished.selected);
        assert_eq!(finished.started, Some(Duration::from_millis(20)));
        assert_eq!(finished.expires, Some(Duration::from_millis(320)));
        assert_eq!(green.color, Color::new(0, 173, 0));
        runtime.observe(
            &registry.snapshot(),
            &registry.events_since(runtime.event_cursor()),
            Duration::from_millis(320),
        );
        assert_eq!(
            report(&runtime, &registry, 320).layers[1]
                .selected_variant
                .as_deref(),
            Some("working")
        );
    }

    #[test]
    fn exact_instance_mismatch_input_unavailable_and_invalid_cells_are_honest() {
        let mut doc = document();
        doc.layers[1].variants.clear();
        doc.layers[1].activation = Activation::WhileState {
            source: "pi".into(),
            instance: Some("wanted".into()),
            state: Some("working".into()),
        };
        let mut registry = Registry::default();
        state(&mut registry, "pi", "other", "working", 1000);
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        let mismatch = report(&runtime, &registry, 0);
        assert_eq!(
            mismatch.layers[1].conditions[0].freshness,
            Freshness::InstanceMismatch
        );
        assert!(
            mismatch.layers[1].conditions[0]
                .reason
                .contains("exact instance")
        );
        doc.layers[1].activation = Activation::Always {};
        doc.layers[1].renderer = Renderer::Ripples {
            palette: None,
            base_color: None,
            ripple_color: None,
        };
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        runtime.input_unavailable();
        let unavailable = report(&runtime, &registry, 0);
        assert!(!unavailable.layers[1].active);
        assert!(unavailable.layers[1].reason.contains("input unavailable"));
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let render = RenderContext {
            animation_seconds: 0.0,
            info: &info,
            layout: &layout,
            brightness: 255,
            palette: PaletteName::Wooting,
            tick: 0,
        };
        let snapshot = registry.snapshot();
        assert!(
            explain_key(
                &runtime,
                &ExplanationContext {
                    render: &render,
                    elapsed: Duration::ZERO,
                    snapshot: &snapshot,
                    now: Duration::ZERO,
                    mode: ExplanationMode::Live
                },
                MatrixCoord {
                    row: 250,
                    column: 250
                }
            )
            .is_none()
        );
    }
}
