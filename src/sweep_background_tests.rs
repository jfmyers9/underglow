//! Shared renderer/compositor checks; synthetic facts, no hardware or producers.
use super::*;
use crate::device::DeviceInfo;
use crate::explanations::{ExplanationContext, ExplanationMode, analyze_coverage};
use crate::external::{Message, Payload, Registry};
use crate::setup::SweepBackground;
use crate::setup_library::{Starter, starter};

fn fixture(
    background: SweepBackground,
    whole: bool,
    pi: bool,
) -> (SetupDocument, CompiledSetup, Registry) {
    let mut doc = starter(Starter::MatrixPi, "sweep-test".into(), "Sweep test".into()).unwrap();
    doc.layers[0].renderer = Renderer::Solid {
        color: [20, 40, 80],
    };
    doc.layers[0].brightness = 255;
    let sweep = Renderer::Sweep {
        color: [200, 100, 50],
        period_ms: 1000,
        background,
    };
    let layer = &mut doc.layers[1];
    layer.brightness = 255;
    layer.target = if whole {
        Target::Whole {}
    } else {
        Target::Selection {
            zones: vec![],
            groups: vec![],
            keys: (1..=6).map(|i| format!("F{i}")).collect(),
        }
    };
    if pi {
        layer.variants[0].renderer = sweep;
    } else {
        layer.renderer = sweep;
        layer.activation = Activation::Always {};
        layer.variants.clear();
    }
    let mut registry = Registry::default();
    registry
        .apply(
            Message {
                schema_version: 1,
                source: "pi".into(),
                instance: "a".into(),
                sequence: 1,
                payload: Payload::State {
                    state: "working".into(),
                    ttl_ms: 15_000,
                },
            },
            Duration::ZERO,
        )
        .unwrap();
    let mut runtime = CompiledSetup::compile(&doc).unwrap();
    runtime.baseline(&registry.snapshot(), Duration::ZERO);
    (doc, runtime, registry)
}
fn context<'a>(
    info: &'a DeviceInfo,
    layout: &'a crate::layout::KeyboardLayout,
) -> RenderContext<'a> {
    RenderContext {
        info,
        layout,
        brightness: 255,
        palette: PaletteName::Heat,
        tick: 0,
        animation_seconds: 0.0,
    }
}
fn coord(name: &str) -> crate::layout::MatrixCoord {
    canonical_coord(name).unwrap()
}

#[test]
fn backgrounds_roundtrip_losslessly_and_old_sweeps_default_to_opaque_black() {
    let old: Renderer =
        serde_json::from_str(r#"{"kind":"sweep","color":[1,2,3],"period_ms":1300}"#).unwrap();
    assert!(matches!(
        old,
        Renderer::Sweep {
            background: SweepBackground::Solid { color: [0, 0, 0] },
            ..
        }
    ));
    for background in [
        SweepBackground::default(),
        SweepBackground::Transparent {},
        SweepBackground::Solid {
            color: [17, 42, 99],
        },
    ] {
        for pi in [false, true] {
            let (doc, _, _) = fixture(background.clone(), false, pi);
            let encoded = doc.to_toml().unwrap();
            assert_eq!(SetupDocument::from_toml(&encoded).unwrap(), doc);
            assert_eq!(
                serde_json::from_str::<SetupDocument>(&serde_json::to_string(&doc).unwrap())
                    .unwrap(),
                doc
            );
            if background.is_default() {
                assert!(!encoded.contains("background ="));
            }
            if matches!(background, SweepBackground::Transparent {}) {
                assert!(encoded.contains("transparent"));
            }
        }
    }
    for background in [
        serde_json::json!({"kind":"unknown"}),
        serde_json::json!({"kind":"solid","color":[1,2,999]}),
        serde_json::json!({"kind":"transparent","command":"not allowed"}),
    ] {
        assert!(
            serde_json::from_value::<Renderer>(serde_json::json!({
                "kind":"sweep","color":[1,2,3],"background":background
            }))
            .is_err()
        );
    }
}

#[test]
fn sweep_uses_independent_active_and_background_colors_and_preserves_black_as_opaque() {
    let info = DeviceInfo::synthetic_80he();
    let layout = crate::layout::KeyboardLayout::for_device(&info);
    let ctx = context(&info, &layout);
    for pi in [false, true] {
        for (background, off) in [
            (SweepBackground::default(), Color::BLACK),
            (
                SweepBackground::Solid {
                    color: [10, 90, 120],
                },
                Color::new(10, 90, 120),
            ),
            (SweepBackground::Transparent {}, Color::new(20, 40, 80)),
        ] {
            let (_, runtime, _) = fixture(background, false, pi);
            let frame = runtime.render(&ctx, Duration::ZERO);
            assert_eq!(frame.get_coord(coord("F1")), Color::new(200, 100, 50));
            assert_eq!(frame.get_coord(coord("F2")), off);
            assert_eq!(
                frame.get_coord(coord("A")),
                Color::new(20, 40, 80),
                "target is unchanged"
            );
        }
        let (mut doc, _, _) = fixture(SweepBackground::Transparent {}, false, pi);
        let renderer = if pi {
            &mut doc.layers[1].variants[0].renderer
        } else {
            &mut doc.layers[1].renderer
        };
        if let Renderer::Sweep { color, .. } = renderer {
            *color = [0, 0, 0];
        }
        let (_, _, registry) = fixture(SweepBackground::Transparent {}, false, pi);
        let mut runtime = CompiledSetup::compile(&doc).unwrap();
        runtime.baseline(&registry.snapshot(), Duration::ZERO);
        assert_eq!(
            runtime.render(&ctx, Duration::ZERO).get_coord(coord("F1")),
            Color::BLACK,
            "a black active sweep must not become transparent"
        );
    }
}

#[test]
fn transparent_trail_blends_with_lower_layers_and_coverage_matches_actual_pixels() {
    let info = DeviceInfo::synthetic_80he();
    let layout = crate::layout::KeyboardLayout::for_device(&info);
    let ctx = context(&info, &layout);
    let (_, runtime, registry) = fixture(SweepBackground::Transparent {}, false, true);
    let report = analyze_coverage(
        &runtime,
        &ExplanationContext {
            render: &ctx,
            elapsed: Duration::ZERO,
            snapshot: &registry.snapshot(),
            now: Duration::ZERO,
            mode: ExplanationMode::Scenario,
        },
    );
    assert_eq!(report.frame, runtime.render(&ctx, Duration::ZERO));
    for (name, alpha, contributors, winner) in [
        ("F1", 255, vec![1], 1),
        ("F2", 0, vec![0], 0),
        ("F6", 195, vec![0, 1], 1),
    ] {
        let position = coord(name);
        let key = report.keys.iter().find(|k| k.coord == position).unwrap();
        assert_eq!(key.contributing_layers, contributors);
        assert_eq!(key.winning_layer, Some(winner));
        assert_eq!(report.contributions[1].frame.alpha_coord(position), alpha);
        let expected = compositor::blend_color(
            Color::new(20, 40, 80),
            Color::new(200, 100, 50),
            alpha,
            Default::default(),
        );
        assert_eq!(report.frame.get_coord(position), expected);
        let explanation = report
            .explain_key(
                &runtime,
                &ExplanationContext {
                    render: &ctx,
                    elapsed: Duration::ZERO,
                    snapshot: &registry.snapshot(),
                    now: Duration::ZERO,
                    mode: ExplanationMode::Scenario,
                },
                position,
            )
            .unwrap();
        assert_eq!(explanation.color, expected);
        assert_eq!(explanation.winning_layer, Some(winner));
        if alpha == 0 {
            assert!(explanation.layers[1].contribution.is_none());
        }
    }
}

#[test]
fn whole_keyboard_and_pi_sweeps_animate_through_the_same_pipeline() {
    let info = DeviceInfo::synthetic_80he();
    let layout = crate::layout::KeyboardLayout::for_device(&info);
    let ctx = context(&info, &layout);
    for whole in [false, true] {
        for pi in [false, true] {
            let (doc, mut runtime, registry) = fixture(SweepBackground::Transparent {}, whole, pi);
            let before = runtime.render(&ctx, Duration::ZERO);
            runtime.observe(
                &registry.snapshot(),
                &registry.events_since(0),
                Duration::from_millis(500),
            );
            let after = runtime.render(&ctx, Duration::from_millis(500));
            assert_ne!(before, after);
            assert_eq!(doc.layers[1].id, "pi-status");
            if whole {
                assert_eq!(
                    runtime.composition(&ctx, Duration::ZERO)[1]
                        .frame
                        .alpha_coord(coord("F1")),
                    0
                );
            }
        }
    }
}

#[test]
fn renderer_coverage_combines_once_with_layer_opacity_blends_brightness_and_safety_cap() {
    let info = DeviceInfo::synthetic_80he();
    let layout = crate::layout::KeyboardLayout::for_device(&info);
    for blend in [
        crate::setup::BlendMode::Normal,
        crate::setup::BlendMode::Add,
        crate::setup::BlendMode::Multiply,
    ] {
        for opacity in [0, 128, 255] {
            for brightness in [73, 255] {
                let (mut doc, _, registry) = fixture(SweepBackground::Transparent {}, false, true);
                doc.layers[1].opacity = opacity;
                doc.layers[1].blend = blend;
                let mut runtime = CompiledSetup::compile(&doc).unwrap();
                runtime.baseline(&registry.snapshot(), Duration::ZERO);
                let mut ctx = context(&info, &layout);
                ctx.brightness = brightness;
                let effective = ((u16::from(opacity) * 195 + 127) / 255) as u8;
                let expected = compositor::cap_color(
                    compositor::blend_color(
                        Color::new(20, 40, 80).scale(brightness),
                        Color::new(200, 100, 50).scale(brightness),
                        effective,
                        blend,
                    ),
                    brightness,
                );
                assert_eq!(
                    runtime.render(&ctx, Duration::ZERO).get_coord(coord("F6")),
                    expected
                );
                if opacity == 128 {
                    assert_eq!(
                        runtime.composition(&ctx, Duration::ZERO)[1].opacity_at(
                            layout
                                .keys()
                                .iter()
                                .find(|k| k.coord == coord("F6"))
                                .unwrap()
                        ),
                        98
                    );
                }
            }
        }
    }
}

#[test]
fn transparent_crossfades_keep_color_without_halos_and_composite_to_opaque_rgb() {
    let mut from = Frame::transparent();
    from.set(0, 0, Color::new(255, 0, 0)); // hidden RGB must not tint a transition
    let mut to = Frame::black();
    to.set(0, 0, Color::new(0, 0, 200));
    let halfway = crate::transitions::interpolate(&from, &to, 0.5);
    assert_eq!(halfway.get(0, 0), Color::new(0, 0, 200));
    assert_eq!(halfway.alpha(0, 0), 128);
    assert_eq!(crate::transitions::interpolate(&from, &to, 0.0), from);
    assert_eq!(crate::transitions::interpolate(&from, &to, 1.0), to);
    let info = DeviceInfo::synthetic_80he();
    let layout = crate::layout::KeyboardLayout::for_device(&info);
    let mut base = Frame::black();
    crate::scenes::fill(&mut base, &layout, Color::new(100, 0, 0));
    let layer = compositor::Layer {
        opacity: 255,
        blend: Default::default(),
        mask: compositor::KeyMask::Whole,
        priority: 0,
        expires: None,
        frame: halfway,
    };
    let output = compositor::compose(base, &layout, &[layer], Duration::ZERO);
    assert_eq!(output.get(0, 0), Color::new(50, 0, 100));
    assert_eq!(output.alpha(0, 0), 255);
}
#[test]
fn transparent_bottom_layer_is_off_outside_the_trail_not_stale_or_colored() {
    let (mut doc, _, _) = fixture(SweepBackground::Transparent {}, true, false);
    doc.layers.remove(0);
    let runtime = CompiledSetup::compile(&doc).unwrap();
    let info = DeviceInfo::synthetic_80he();
    let layout = crate::layout::KeyboardLayout::for_device(&info);
    let ctx = context(&info, &layout);
    let contributions = runtime.composition(&ctx, Duration::ZERO);
    let off = layout
        .keys()
        .iter()
        .find(|k| contributions[0].frame.alpha_coord(k.coord) == 0)
        .unwrap();
    let frame = runtime.render(&ctx, Duration::ZERO);
    assert_eq!(frame.get_coord(off.coord), Color::BLACK);
    assert_eq!(frame.alpha_coord(off.coord), 255);
}
