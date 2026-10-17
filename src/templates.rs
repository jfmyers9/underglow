//! Editable declarative starters, not runtime modes or sources of defaults.
//! Call a constructor only for an explicit Add action (or one-time onboarding).
//! Once inserted, the returned data is ordinary user-owned layer data.
use crate::setup::{Activation, BehaviorVariant, Layer, Renderer, Target};

/// One logical Pi status layer, independent of its background and target.
///
/// `instance = None` selects any leased session; `Some` selects one exact
/// connector instance for presence, working and finished alike. The caller owns
/// ID/target validation through `SetupDocument::validate` as for any layer.
/// Colors, brightness, sweep periods and completion duration are editable data.
/// `finished` means settled, not success. No approval or focus facts are assumed.
/// This function performs no I/O, acquires no hardware, and starts no producer.
pub fn pi_status_layer(id: String, target: Target, instance: Option<String>) -> Layer {
    Layer {
        id,
        name: "Pi status".into(),
        enabled: true,
        renderer: Renderer::Solid { color: [0, 36, 48] },
        brightness: crate::DEFAULT_BRIGHTNESS,
        target,
        activation: Activation::WhileState {
            source: "pi".into(),
            instance: instance.clone(),
            state: None,
        },
        variants: vec![
            BehaviorVariant {
                id: "working".into(),
                name: "Working".into(),
                enabled: true,
                activation: Activation::WhileState {
                    source: "pi".into(),
                    instance: instance.clone(),
                    state: Some("working".into()),
                },
                renderer: Renderer::Sweep {
                    color: [0, 100, 255],
                    period_ms: 1300,
                },
                brightness: None,
            },
            BehaviorVariant {
                id: "finished".into(),
                name: "Finished (settled, not success)".into(),
                enabled: true,
                activation: Activation::OnEvent {
                    source: "pi".into(),
                    instance,
                    event: "finished".into(),
                    duration_ms: 2000,
                },
                renderer: Renderer::Sweep {
                    color: [0, 255, 80],
                    period_ms: 1300,
                },
                brightness: None,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceInfo;
    use crate::effects::EffectKind;
    use crate::external::{Message, Payload, Registry};
    use crate::key_targets::canonical_coord;
    use crate::layer_runtime::CompiledSetup;
    use crate::layout::KeyboardLayout;
    use crate::render::{PaletteName, RenderContext};
    use crate::setup::{SetupDocument, TargetZone};
    use std::collections::BTreeMap;
    use std::time::Duration;

    fn document(layers: Vec<Layer>) -> SetupDocument {
        SetupDocument {
            schema_version: 1,
            revision: 0,
            id: "test".into(),
            name: "Template test".into(),
            layers,
            groups: BTreeMap::new(),
        }
    }
    fn key_target() -> Target {
        Target::Selection {
            zones: vec![],
            keys: vec!["ArrowUp".into()],
            groups: vec![],
        }
    }
    fn fact(registry: &mut Registry, instance: &str, sequence: u64, payload: Payload, now: u64) {
        registry
            .apply(
                Message {
                    schema_version: 1,
                    source: "pi".into(),
                    instance: instance.into(),
                    sequence,
                    payload,
                },
                Duration::from_millis(now),
            )
            .unwrap();
    }
    fn state(value: &str) -> Payload {
        Payload::State {
            state: value.into(),
            ttl_ms: 10_000,
        }
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
    fn single_item_roundtrip_and_deletions_are_plain_data() {
        let mut doc = document(vec![pi_status_layer("status".into(), key_target(), None)]);
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].variants.len(), 2);
        let encoded = doc.to_toml().unwrap();
        assert_eq!(SetupDocument::from_toml(&encoded).unwrap(), doc);
        assert!(
            !CompiledSetup::compile(&doc)
                .unwrap()
                .requirements()
                .analog_input
        );
        // Editing/removing an appearance and reopening never consults a template.
        doc.layers[0].variants.remove(1);
        doc.layers[0].renderer = Renderer::Solid {
            color: [17, 23, 41],
        };
        let reopened = SetupDocument::from_toml(&doc.to_toml().unwrap()).unwrap();
        assert_eq!(reopened, doc);
        assert_eq!(reopened.layers[0].variants.len(), 1);
        doc.layers.clear();
        assert!(
            SetupDocument::from_toml(&doc.to_toml().unwrap())
                .unwrap()
                .layers
                .is_empty()
        );
    }

    #[test]
    fn lifecycle_any_session_completion_and_expiry_over_arbitrary_backgrounds_and_targets() {
        for effect in [EffectKind::Aurora, EffectKind::Matrix] {
            for target in [
                key_target(),
                Target::Whole {},
                Target::Selection {
                    zones: vec![TargetZone::Function],
                    keys: vec![],
                    groups: vec![],
                },
            ] {
                let background = Layer {
                    id: "background".into(),
                    name: "Background".into(),
                    enabled: true,
                    renderer: Renderer::Effect {
                        effect,
                        palette: None,
                        speed: 100,
                    },
                    brightness: 255,
                    target: Target::Whole {},
                    activation: Activation::Always {},
                    variants: vec![],
                };
                let doc = document(vec![
                    background.clone(),
                    pi_status_layer("status".into(), target.clone(), None),
                ]);
                let mut runtime = CompiledSetup::compile(&doc).unwrap();
                let base = CompiledSetup::compile(&document(vec![background])).unwrap();
                let info = DeviceInfo::synthetic_80he();
                let layout = KeyboardLayout::for_device(&info);
                let ctx = RenderContext {
                    info: &info,
                    layout: &layout,
                    brightness: 255,
                    palette: PaletteName::Heat,
                    tick: 0,
                    animation_seconds: 0.4,
                };
                let mut registry = Registry::default();
                runtime.baseline(&registry.snapshot(), Duration::ZERO);
                assert_eq!(
                    runtime.render(&ctx, Duration::ZERO),
                    base.render(&ctx, Duration::ZERO)
                );
                fact(&mut registry, "idle-session", 1, state("idle"), 0);
                observe(&mut runtime, &mut registry, 0);
                assert_eq!(
                    runtime.selected_appearance("status").unwrap().renderer,
                    &Renderer::Solid { color: [0, 36, 48] }
                );
                if target == key_target() {
                    let frame = runtime.render(&ctx, Duration::ZERO);
                    let expected = base.render(&ctx, Duration::ZERO);
                    for key in layout.keys() {
                        if key.coord != canonical_coord("ArrowUp").unwrap() {
                            assert_eq!(frame.get_coord(key.coord), expected.get_coord(key.coord));
                        }
                    }
                }
                fact(&mut registry, "working-session", 1, state("working"), 10);
                observe(&mut runtime, &mut registry, 10);
                let working = runtime.selected_appearance("status").unwrap();
                assert_eq!(working.variant_id, Some("working"));
                assert_eq!(
                    working.renderer,
                    &Renderer::Sweep {
                        color: [0, 100, 255],
                        period_ms: 1300
                    }
                );
                assert_ne!(
                    runtime.render(&ctx, Duration::from_millis(10)),
                    base.render(&ctx, Duration::from_millis(10))
                );
                // A different idle session's settlement wins over working, briefly.
                fact(
                    &mut registry,
                    "idle-session",
                    2,
                    Payload::Event {
                        event: "finished".into(),
                        ttl_ms: 2000,
                    },
                    20,
                );
                observe(&mut runtime, &mut registry, 20);
                let finished = runtime.selected_appearance("status").unwrap();
                assert_eq!(finished.variant_id, Some("finished"));
                assert_eq!(
                    finished.renderer,
                    &Renderer::Sweep {
                        color: [0, 255, 80],
                        period_ms: 1300
                    }
                );
                observe(&mut runtime, &mut registry, 2019);
                assert_eq!(
                    runtime.selected_appearance("status").unwrap().variant_id,
                    Some("finished")
                );
                observe(&mut runtime, &mut registry, 2020);
                assert_eq!(
                    runtime.selected_appearance("status").unwrap().variant_id,
                    Some("working")
                );
                observe(&mut runtime, &mut registry, 10_010);
                assert!(runtime.selected_appearance("status").is_none());
                assert_eq!(
                    runtime.render(&ctx, Duration::from_millis(10_010)),
                    base.render(&ctx, Duration::from_millis(10_010))
                );
            }
        }
    }

    #[test]
    fn exact_session_filters_all_appearances_and_removal_hides_completion() {
        let doc = document(vec![pi_status_layer(
            "status".into(),
            key_target(),
            Some("chosen".into()),
        )]);
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        let mut registry = Registry::default();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        fact(&mut registry, "other", 1, state("working"), 0);
        fact(
            &mut registry,
            "other",
            2,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 2000,
            },
            1,
        );
        observe(&mut runtime, &mut registry, 1);
        assert!(runtime.selected_appearance("status").is_none());
        fact(&mut registry, "chosen", 1, state("idle"), 2);
        observe(&mut runtime, &mut registry, 2);
        assert_eq!(
            runtime.selected_appearance("status").unwrap().variant_id,
            None
        );
        fact(&mut registry, "chosen", 2, state("working"), 3);
        observe(&mut runtime, &mut registry, 3);
        assert_eq!(
            runtime.selected_appearance("status").unwrap().variant_id,
            Some("working")
        );
        fact(
            &mut registry,
            "chosen",
            3,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 2000,
            },
            4,
        );
        observe(&mut runtime, &mut registry, 4);
        assert_eq!(
            runtime.selected_appearance("status").unwrap().variant_id,
            Some("finished")
        );
        fact(&mut registry, "chosen", 4, Payload::Remove, 5);
        observe(&mut runtime, &mut registry, 5);
        assert!(runtime.selected_appearance("status").is_none());
        // Startup baselines current state but never replays old events.
        let mut restarted = CompiledSetup::compile(&doc).unwrap();
        restarted.baseline(&registry.snapshot(), Duration::from_millis(5));
        assert!(restarted.selected_appearance("status").is_none());
    }
}
