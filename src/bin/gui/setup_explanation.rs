//! Key selection is inspection only: this component has no draft or engine handle.
use super::{
    setup_preview::{Mode, Preview},
    setup_targets,
};
use eframe::egui;
use underglow::device::DeviceInfo;
use underglow::explanations::{ExplanationContext, ExplanationMode, KeyExplanation};
use underglow::layout::{KeyboardLayout, MatrixCoord};
use underglow::render::{PaletteName, RenderContext};

pub(super) fn report(preview: &Preview, coord: MatrixCoord) -> Option<KeyExplanation> {
    // Invalid edits retain a last-good frame. Never explain it as the raw draft.
    if preview.error.is_some() {
        return None;
    }
    let runtime = preview.runtime()?;
    let elapsed = preview.evaluation_elapsed();
    let info = DeviceInfo::synthetic_80he();
    let layout = KeyboardLayout::for_device(&info);
    let render = RenderContext {
        animation_seconds: elapsed.as_secs_f64(),
        info: &info,
        layout: &layout,
        brightness: preview.evaluation_brightness(),
        palette: PaletteName::Wooting,
        tick: (elapsed.as_secs_f64() * 30.0) as u32,
    };
    let (snapshot, _) = preview.evaluation_facts();
    preview.coverage()?.explain_key(
        runtime,
        &ExplanationContext {
            render: &render,
            elapsed,
            snapshot,
            now: preview.evaluation_now(),
            mode: match preview.mode {
                Mode::Live => ExplanationMode::Live,
                Mode::InputDemo => ExplanationMode::InputDemo,
                Mode::Scenario => ExplanationMode::Scenario,
            },
        },
        coord,
    )
}

pub fn show(ui: &mut egui::Ui, preview: &Preview, coord: Option<MatrixCoord>) {
    let Some(coord) = coord else {
        ui.small("Click a composed preview key to explain it (does not edit targets).");
        return;
    };
    let Some(report) = report(preview, coord) else {
        ui.label("Key explanation unavailable: fix the invalid draft; any retained frame is the last valid preview.");
        return;
    };
    egui::CollapsingHeader::new(format!("Why {}? · RGB {}, {}, {}", setup_targets::key_label(coord), report.color.red, report.color.green, report.color.blue)).id_salt("selected-key-explanation").default_open(true).show(ui, |ui| {
        ui.small(report.mode.label());
        ui.small(format!("Evaluated at {:?} · global brightness {}/255", report.evaluated_at, report.global_brightness));
        if preview.mode == Mode::Live && preview.evaluation_facts().0.states.is_empty() {
            ui.small("No source states in the current evaluated snapshot; event-only occurrences may still be active.");
        }
        ui.small("Preview only — not actual physical output. Engine ownership/paused status is separate.");
        if let Some(winner) = report.winning_layer {
            ui.label(format!("Winning layer #{}: {}", winner + 1, report.layers[winner].name));
        } else {
            ui.label("No active targeting layer: uncovered black.");
        }
        ui.small("Bottom → top; higher active layers are opaque, including black.");
        egui::ScrollArea::vertical().id_salt("key-explanation").max_height(260.0).show(ui, |ui| {
            for layer in &report.layers {
                ui.push_id(&layer.id, |ui| {
                    ui.label(format!("#{} {} — {}", layer.index + 1, layer.name, layer.reason));
                    if let Some(renderer) = &layer.selected_renderer {
                        ui.small(format!("Selected renderer {:?} · appearance brightness {:?}/255", renderer, layer.selected_brightness));
                    }
                    if let Some(rgb) = layer.contribution {
                        ui.small(format!("Underlying contribution RGB {}, {}, {}", rgb.red, rgb.green, rgb.blue));
                    }
                    for condition in &layer.conditions {
                        ui.small(format!("{}: {} · matched={} active={} · {:?}", condition.name, condition.reason, condition.matched, condition.active, condition.activation));
                        ui.small(format!("Source state freshness: {:?}", condition.freshness));
                        if let Some(start) = condition.started {
                            ui.small(format!("Cached occurrence start {:?}; deadline {:?}", start, condition.expires));
                        }
                        for state in &condition.states {
                            ui.small(format!("{}: {} · age {:?} · lease remaining {:?} · {}", state.instance, state.state, state.age, state.expires_in, if state.fresh {"fresh"} else {"expired"}));
                        }
                    }
                });
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use underglow::external::{Message, Payload, Registry};
    use underglow::key_targets::canonical_coord;
    use underglow::render::Color;
    use underglow::setup::{Activation, Renderer, SetupDocument};

    #[test]
    fn actual_preview_report_uses_effective_scenario_facts_and_retained_frame_guard() {
        let mut doc =
            SetupDocument::from_toml(include_str!("../../../examples/layer-setup.toml")).unwrap();
        doc.layers.truncate(1);
        doc.layers[0].activation = Activation::WhileState {
            source: "pi".into(),
            instance: None,
            state: Some("working".into()),
        };
        doc.layers[0].renderer = Renderer::Solid { color: [0, 0, 255] };
        doc.layers[0].brightness = 255;
        let mut p = Preview::default();
        let mut real = Registry::default();
        real.apply(
            Message {
                schema_version: 1,
                source: "pi".into(),
                instance: "real-session".into(),
                sequence: 1,
                payload: Payload::State {
                    state: "working".into(),
                    ttl_ms: 10000,
                },
            },
            Duration::ZERO,
        )
        .unwrap();
        let real_snapshot = real.snapshot();
        p.set_sample(Some(super::super::setup_preview::LiveSample {
            generation: (1, 2),
            now: Duration::ZERO,
            snapshot: real_snapshot.clone(),
            events: real.events_since(0),
            received_at: std::time::Instant::now(),
            connected: true,
            stale: false,
        }));
        p.set_mode(Mode::Scenario);
        p.scenario
            .publish_state("pi", "synthetic", "working", 100)
            .unwrap();
        p.update(Some(&doc), 177);
        let coord = canonical_coord("F3").unwrap();
        let first = report(&p, coord).unwrap();
        assert_eq!(first.color, p.frame().get_coord(coord));
        assert_eq!(first.color, Color::new(0, 0, 177));
        assert_eq!(first.mode, ExplanationMode::Scenario);
        assert_eq!(
            first.layers[0].conditions[0].states[0].instance,
            "synthetic"
        );
        assert_eq!(
            p.sample().unwrap().snapshot,
            real_snapshot,
            "Scenario explanations must not replace raw live observations"
        );
        let facts = (
            p.evaluation_facts().0.clone(),
            p.evaluation_facts().1.clone(),
        );
        let frame = p.frame().clone();
        report(&p, coord).unwrap();
        assert_eq!(p.frame(), &frame);
        assert_eq!(p.evaluation_facts(), (&facts.0, &facts.1));
        assert_eq!(p.sample().unwrap().snapshot, real_snapshot);
        p.advance_scenario_by(Duration::from_millis(100)).unwrap();
        p.update(Some(&doc), 177);
        assert_eq!(report(&p, coord).unwrap().winning_layer, None);
        doc.layers[0].id.clear();
        p.update(Some(&doc), 177);
        assert!(p.error.is_some());
        assert!(
            report(&p, coord).is_none(),
            "invalid draft cannot be explained as last-good runtime"
        );
    }

    #[test]
    fn animated_report_matches_last_render_after_scenario_steps_and_reorder() {
        let mut doc =
            SetupDocument::from_toml(include_str!("../../../examples/layer-setup.toml")).unwrap();
        doc.layers.truncate(1);
        doc.layers[0].renderer = Renderer::Effect {
            effect: underglow::effects::EffectKind::Matrix,
            palette: None,
            speed: 137,
        };
        doc.layers.push(underglow::templates::pi_status_layer(
            "pi-status".into(),
            underglow::setup::Target::Selection {
                zones: vec![],
                keys: vec!["F3".into()],
                groups: vec![],
            },
            None,
        ));
        let mut p = Preview::default();
        p.set_mode(Mode::Scenario);
        p.scenario
            .publish_state("pi", "synthetic", "working", 1000)
            .unwrap();
        for now in [0, 137, 512, 1000] {
            p.scenario.advance_to(Duration::from_millis(now)).unwrap();
            p.update(Some(&doc), 179);
            for name in ["F3", "W"] {
                let coord = canonical_coord(name).unwrap();
                assert_eq!(
                    report(&p, coord).unwrap().color,
                    p.frame().get_coord(coord),
                    "{name} at {now}"
                );
            }
        }
        doc.layers.swap(0, 1);
        p.update(Some(&doc), 179);
        let coord = canonical_coord("F3").unwrap();
        let reordered = report(&p, coord).unwrap();
        assert_eq!(reordered.color, p.frame().get_coord(coord));
        assert_eq!(
            reordered.layers[reordered.winning_layer.unwrap()].name,
            doc.layers[1].name
        );
    }
}
