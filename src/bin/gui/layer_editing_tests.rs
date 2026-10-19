//! Production inspector and menus at realistic sizes; no workers or hardware.
use super::super::{Action, Completion, Controller, Status};
use super::super::{
    tests::{controller_fixture, setup_document},
    workspace_test_ui::{DEFAULT_SIZE, Harness, MINIMUM_SIZE, WIDE_SIZE},
};
use super::*;
use std::sync::mpsc::Receiver;

fn app() -> (Controller, Receiver<Action>) {
    let (mut app, incoming) = controller_fixture();
    let mut doc = setup_document();
    doc.layers.truncate(1);
    doc.layers[0].name = "Background".into();
    doc.layers[0].variants.clear();
    app.setup_editor.load(Some(doc));
    let status: Status = serde_json::from_value(serde_json::json!({
        "schema_version":1, "state":"paused", "enabled":false,
        "mode":"matrix", "brightness":128, "palette":"terminal", "fps":30, "retry_attempt":0
    }))
    .unwrap();
    app.status = Some(status);
    (app, incoming)
}

fn click(h: &mut Harness, app: &mut Controller, label: &str) {
    for step in 0..160 {
        // Settle the actual scroll animation before choosing a hit; never use stale geometry.
        for _ in 0..25 {
            h.run(vec![], |ctx| app.show_lighting(ctx));
        }
        let frame = h.settle(|ctx| app.show_lighting(ctx));
        let texts = frame.texts();
        let pane = texts
            .iter()
            .find(|t| t.text == "Layer actions")
            .map(|t| t.clip);
        // Use the inset scroll viewport, not the padded surface/header clip.
        let scroll = pane
            .and_then(|pane| {
                texts
                    .iter()
                    .filter(|t| {
                        t.clip != pane
                            && pane.contains_rect(t.clip)
                            && t.clip.width() > pane.width() * 0.75
                            && t.clip.height() < pane.height() - 4.0
                    })
                    .max_by(|a, b| a.clip.area().total_cmp(&b.clip.area()))
                    .map(|t| t.clip)
            })
            .or(pane);
        let keyboard = texts.iter().find(|t| t.text == "Keyboard").map(|t| t.clip);
        if let Some(t) = texts.iter().find(|t| {
            t.text == label
                && t.fully_visible()
                && (label != "F1" || keyboard.is_some_and(|r| r.contains(t.rect.center())))
                // Quick tests repeat capability labels such as Working. An
                // inspector choice must come from its pane/popup, not the
                // independently clipped canonical keyboard's test toolbar.
                && (matches!(label, "F1" | "Keyboard")
                    || keyboard.is_none_or(|r| r != t.clip))
        }) {
            h.click_at(t.rect.center(), |ctx| app.show_lighting(ctx));
            return;
        }
        if matches!(label, "Duplicate layer" | "Delete layer" | "Rename layer")
            && let Some(menu) = texts
                .iter()
                .find(|t| t.text == "Layer actions" && t.fully_visible())
        {
            h.click_at(menu.rect.center(), |ctx| app.show_lighting(ctx));
            continue;
        }
        if matches!(
            label,
            "Whole keyboard"
                | "Arrow keys (4 physical)"
                | "Saved zones"
                | "Reusable custom groups"
                | "Key selection help"
        ) && !app.setup_editor.target_options_open_for_test()
            && let Some(options) = texts
                .iter()
                .find(|t| t.text == "Selection options" && t.fully_visible())
        {
            h.click_at(options.rect.center(), |ctx| app.show_lighting(ctx));
            continue;
        }
        let pane = scroll.expect("selected-layer scroll viewport");
        let menu_item = texts.iter().rev().find(|t| {
            t.fully_visible()
                && (underglow::catalog::presets().any(|e| e.title == t.text)
                    || matches!(t.text.as_str(), "Solid" | "Pulse" | "Sweep"))
        });
        h.wheel(
            menu_item.map_or(egui::pos2(pane.center().x, pane.bottom() - 10.0), |t| {
                t.rect.center()
            }),
            egui::vec2(0.0, if step == 79 { 20000.0 } else { -60.0 }),
            |ctx| app.show_lighting(ctx),
        );
    }
    panic!("unreachable {label} at {:?}", h.screen());
}

fn labels(h: &mut Harness, app: &mut Controller) -> Vec<String> {
    h.settle(|ctx| app.show_lighting(ctx))
        .texts()
        .into_iter()
        .map(|t| t.text)
        .collect()
}

#[test]
fn appearance_is_default_and_matrix_change_preserves_the_other_layer_and_keys() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE] {
        let mut h = Harness::new(size);
        let (mut app, incoming) = app();
        click(&mut h, &mut app, "Add layer");
        click(&mut h, &mut app, "Solid color layer");
        let original = app.setup_editor.draft.clone().unwrap();
        app.setup_editor.selected = Some(original.layers[0].id.clone());
        let text = labels(&mut h, &mut app);
        for section in ["Where", "Looks", "When"] {
            click(&mut h, &mut app, section);
        }
        click(&mut h, &mut app, "Looks");
        assert!(!text.iter().any(|s| s.contains("Stable ID:")
            || s.contains("Transient appearances outrank")
            || s.contains("Source is present")));
        click(&mut h, &mut app, "Matrix");
        click(&mut h, &mut app, "Aurora");
        let changed = app.setup_editor.draft.as_ref().unwrap();
        assert_eq!(changed.layers[0].renderer.catalog_id(), Some("aurora"));
        let mut expected = original.clone();
        expected.layers[0].renderer = changed.layers[0].renderer.clone();
        assert_eq!(*changed, expected);
        click(&mut h, &mut app, "Edit keys");
        click(&mut h, &mut app, "F1");
        let changed = app.setup_editor.draft.as_ref().unwrap();
        assert_eq!(changed.layers[1], original.layers[1]);
        assert_eq!(
            changed.layers[0].target,
            Target::Selection {
                zones: vec![],
                keys: vec!["F1".into()],
                groups: vec![]
            }
        );
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn pi_is_one_layer_and_appearance_selection_is_shared_safely_with_behavior() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE] {
        let mut h = Harness::new(size);
        let (mut app, incoming) = app();
        let base = app.setup_editor.draft.clone().unwrap();
        click(&mut h, &mut app, "Add layer");
        assert_eq!(
            app.setup_editor.draft.as_ref(),
            Some(&base),
            "opening Add is not an edit"
        );
        click(&mut h, &mut app, "Pi status layer");
        let original = app.setup_editor.draft.clone().unwrap();
        assert_eq!(original.layers.len(), 2);
        assert_eq!(original.layers[1].variants.len(), 2);
        click(&mut h, &mut app, "Idle / fallback");
        click(&mut h, &mut app, "Working");
        click(&mut h, &mut app, "Sweep");
        click(&mut h, &mut app, "Solid");
        click(&mut h, &mut app, "Use layer brightness");
        let changed = app.setup_editor.draft.clone().unwrap();
        assert!(matches!(
            changed.layers[1].variants[0].renderer,
            underglow::setup::Renderer::Solid { .. }
        ));
        assert_eq!(
            changed.layers[1].variants[0].brightness,
            Some(original.layers[1].brightness)
        );
        let mut expected = original.clone();
        expected.layers[1].variants[0] = changed.layers[1].variants[0].clone();
        assert_eq!(changed, expected);
        click(&mut h, &mut app, "Edit condition");
        // Edit condition opens the actual condition controls directly, not another disclosure.

        click(&mut h, &mut app, "While a source matches");
        click(&mut h, &mut app, "Always");
        assert_eq!(
            app.setup_editor.draft.as_ref().unwrap().layers[1].variants[0].activation,
            Activation::Always {}
        );
        click(&mut h, &mut app, "Looks");
        click(&mut h, &mut app, "Working");
        click(&mut h, &mut app, "Finished (settled, not success)");
        assert_eq!(
            app.setup_editor.draft.as_ref().unwrap().layers[1].variants[1],
            original.layers[1].variants[1]
        );
        assert_eq!(
            app.setup_editor.draft.as_ref().unwrap().layers[0],
            base.layers[0]
        );
        assert!(
            incoming.try_recv().is_err(),
            "appearance and conditions are local edits"
        );
    }
}

#[test]
fn deleting_pi_stays_deleted_after_selection_save_and_document_restart() {
    let mut h = Harness::new(MINIMUM_SIZE);
    let (mut app, incoming) = app();
    let base = app.setup_editor.draft.clone().unwrap();
    click(&mut h, &mut app, "Add layer");
    click(&mut h, &mut app, "Pi status layer");
    click(&mut h, &mut app, "Delete layer");
    assert_eq!(app.setup_editor.draft.as_ref(), Some(&base));
    click(&mut h, &mut app, "Looks");
    click(&mut h, &mut app, "Save");
    let Action::SetupReplace { document, revision } = incoming.try_recv().unwrap() else {
        panic!("explicit setup save only");
    };
    assert_eq!(document, base);
    let mut saved = document.clone();
    saved.revision = revision + 1;
    let (outgoing, replies) = std::sync::mpsc::channel();
    app.replies = replies;
    outgoing
        .send(Completion {
            action: Action::SetupReplace { document, revision },
            result: Ok(serde_json::json!({
                "ok": true,
                "setup": {"revision": saved.revision, "document": saved},
                "status": {"schema_version":1,"state":"paused","enabled":false,
                    "mode":"matrix","brightness":128,"palette":"terminal",
                    "fps":30,"retry_attempt":0}
            })
            .to_string()),
        })
        .unwrap();
    app.receive();
    assert_eq!(app.setup_editor.applied.as_ref(), Some(&saved));
    assert!(!app.setup_editor.dirty);
    let serialized = saved.to_toml().unwrap();
    let mut restarted = Editor::default();
    restarted.load(Some(SetupDocument::from_toml(&serialized).unwrap()));
    assert_eq!(restarted.draft, Some(saved));
    assert_eq!(restarted.draft.as_ref().unwrap().layers.len(), 1);
    assert!(!restarted.dirty);
    assert!(!app.status.as_ref().unwrap().enabled);
    assert!(incoming.try_recv().is_err());
}

#[test]
fn advanced_blend_transition_and_real_only_observed_choices_remain_reachable() {
    let mut h = Harness::new(DEFAULT_SIZE);
    let (mut app, incoming) = app();
    let before = app.setup_editor.draft.clone().unwrap();
    click(&mut h, &mut app, "Layer mixing & transitions");
    click(&mut h, &mut app, "Normal");
    click(&mut h, &mut app, "Add (saturating)");
    click(&mut h, &mut app, "Linear");
    click(&mut h, &mut app, "Smooth step");
    let layer = &app.setup_editor.draft.as_ref().unwrap().layers[0];
    assert_eq!(layer.blend, underglow::setup::BlendMode::Add);
    assert_eq!(
        layer.transition.curve,
        underglow::setup::TransitionCurve::SmoothStep
    );
    assert_eq!(layer.renderer, before.layers[0].renderer);
    app.setup_editor
        .preview
        .set_mode(setup_preview::Mode::Scenario);
    app.setup_editor
        .preview
        .scenario
        .publish_state("pi", "private-only", "approval", 1000)
        .unwrap();
    click(&mut h, &mut app, "Edit condition");

    click(&mut h, &mut app, "Always");
    click(&mut h, &mut app, "While a source matches");
    click(&mut h, &mut app, "Source is present (any live state)");
    let text = labels(&mut h, &mut app);
    assert!(text.iter().any(|s| s == "Working"));
    assert!(
        !text
            .iter()
            .any(|s| s.contains("approval") || s.contains("private-only")),
        "synthetic facts never populate observed choices"
    );
    assert!(incoming.try_recv().is_err());
}

#[test]
fn compact_stack_overflow_actions_follow_top_first_display_and_stable_ids() {
    let mut h = Harness::new(DEFAULT_SIZE);
    let (mut app, incoming) = app();
    let bottom = app.setup_editor.draft.as_ref().unwrap().layers[0].clone();
    click(&mut h, &mut app, "Add layer");
    click(&mut h, &mut app, "Matrix layer");
    let top = app.setup_editor.selected.clone().unwrap();
    click(&mut h, &mut app, "...");
    click(&mut h, &mut app, "Enabled");
    assert!(!app.setup_editor.draft.as_ref().unwrap().layers[1].enabled);
    assert_eq!(app.setup_editor.draft.as_ref().unwrap().layers[0], bottom);
    click(&mut h, &mut app, "Keyboard");
    click(&mut h, &mut app, "...");
    click(&mut h, &mut app, "Toward bottom");
    let doc = app.setup_editor.draft.as_ref().unwrap();
    assert_eq!(doc.layers[0].id, top);
    assert_eq!(doc.layers[1], bottom);
    assert_eq!(app.setup_editor.selected.as_deref(), Some(top.as_str()));
    let frame = h.settle(|ctx| app.show_lighting(ctx));
    let texts = frame.texts();
    let rail = texts.iter().find(|t| t.text == "Layers").unwrap().clip;
    let upper = texts
        .iter()
        .find(|t| t.text == "Background" && rail.contains_rect(t.rect))
        .unwrap();
    let lower = texts
        .iter()
        .find(|t| t.text == "New layer" && rail.contains_rect(t.rect))
        .unwrap();
    assert!(upper.fully_visible() && lower.fully_visible());
    assert!(upper.rect.top() < lower.rect.top());
    click(&mut h, &mut app, "...");
    click(&mut h, &mut app, "Rename layer");
    h.run(
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    command: true,
                    ctrl: true,
                    ..Default::default()
                },
            },
            egui::Event::Text("Renamed".into()),
        ],
        |ctx| app.show_lighting(ctx),
    );
    assert_eq!(
        app.setup_editor.draft.as_ref().unwrap().layers[1].name,
        "Renamed"
    );
    assert_eq!(
        app.setup_editor.draft.as_ref().unwrap().layers[1].id,
        bottom.id
    );
    assert!(incoming.try_recv().is_err());
}

#[test]
fn inspector_sections_and_advanced_controls_remain_reachable_without_editing_at_all_sizes() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE, WIDE_SIZE] {
        let mut h = Harness::new(size);
        let (mut app, incoming) = app();
        click(&mut h, &mut app, "Add layer");
        click(&mut h, &mut app, "Pi status layer");
        let before = app.setup_editor.draft.clone();
        let id = app.setup_editor.selected.clone().unwrap();
        for label in [
            "Edit keys",
            "Selection options",
            "Reusable custom groups",
            "Effect details & input",
            "Edit condition",
            "While a source matches",
        ] {
            click(&mut h, &mut app, label);
        }
        // Escape dismisses the real popup without choosing a new activation.
        for pressed in [true, false] {
            h.run(
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                |ctx| app.show_lighting(ctx),
            );
        }
        assert_eq!(
            app.setup_editor.inspector_sections_for_test(&id),
            (true, true)
        );
        for label in [
            "Looks",
            "Idle / fallback",
            "Working",
            "Appearance actions",
            "Advanced appearance settings",
        ] {
            click(&mut h, &mut app, label);
        }
        click(&mut h, &mut app, "Looks");
        click(&mut h, &mut app, "Layer mixing & transitions");
        click(&mut h, &mut app, "Technical details");
        assert_eq!(app.setup_editor.draft, before);
        assert!(!app.setup_editor.preview.quick_active());
        assert!(app.setup_editor.preview.sample().is_none());
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn unavailable_saved_condition_is_visible_without_opening_its_editor_at_all_sizes() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE, WIDE_SIZE] {
        let mut h = Harness::new(size);
        let (mut app, incoming) = app();
        let mut doc = app.setup_editor.draft.clone().unwrap();
        doc.layers[0].activation = Activation::WhileState {
            source: "saved-custom-source".into(),
            instance: Some("saved-session".into()),
            state: Some("saved-state".into()),
        };
        app.setup_editor.load(Some(doc.clone()));
        let id = app.setup_editor.selected.clone().unwrap();
        let warnings = CapabilityRegistry::with_builtins()
            .validate_activation(&doc.layers[0].activation)
            .unwrap();
        let warning = format!("Repair selector: {:?}. Saved value retained.", warnings[0]);
        click(&mut h, &mut app, &warning);
        assert_eq!(
            app.setup_editor.inspector_sections_for_test(&id),
            (false, false)
        );
        assert_eq!(app.setup_editor.draft.as_ref(), Some(&doc));
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn default_lower_inspector_keeps_all_section_summaries_and_source_repair_visible() {
    let mut h = Harness::new(DEFAULT_SIZE);
    let (mut app, incoming) = app();
    let doc = underglow::setup_library::starter(
        underglow::setup_library::Starter::MatrixPi,
        "default-inspector".into(),
        "Matrix + Pi".into(),
    )
    .unwrap();
    app.setup_editor.load(Some(doc.clone()));
    app.setup_editor.selected = Some("pi-status".into());
    let frame = h.settle(|ctx| app.show_lighting(ctx));
    let texts = frame.texts();
    for label in ["Where", "Looks", "When", "Edit keys", "Edit condition"] {
        assert!(
            texts.iter().any(|t| t.text == label && t.fully_visible()),
            "default 940px inspector keeps {label} visible without scrolling"
        );
    }
    assert!(!app.setup_editor.connection_notices.is_empty());
    for notice in &app.setup_editor.connection_notices {
        let link = format!("Connections: {}", notice.label);
        let status = notice.message.split('.').next().unwrap_or(&notice.message);
        let link = texts
            .iter()
            .find(|t| t.text == link)
            .expect("source repair link");
        let status = texts
            .iter()
            .find(|t| t.text == status)
            .expect("truthful source status");
        assert!(
            link.fully_visible() && status.fully_visible(),
            "source footer is not half clipped"
        );
        assert!(
            link.clip.contains_rect(egui::Rect::from_center_size(
                link.rect.center(),
                egui::vec2(link.rect.width(), 28.0)
            )),
            "the repair control's full height remains inside the scroll viewport"
        );
        assert!(
            (link.rect.center().y - status.rect.center().y).abs() < 2.0,
            "the normal lower inspector uses one source footer row"
        );
    }
    assert_eq!(app.setup_editor.draft.as_ref(), Some(&doc));
    assert!(incoming.try_recv().is_err());
}

#[test]
fn finished_duration_is_edited_in_looks_in_seconds_and_saves_only_that_appearance() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE, WIDE_SIZE] {
        let (mut app, incoming) = app();
        let doc = underglow::setup_library::starter(
            underglow::setup_library::Starter::MatrixPi,
            "duration-ui".into(),
            "Duration UI".into(),
        )
        .unwrap();
        app.setup_editor.load(Some(doc.clone()));
        app.setup_editor.selected = Some("pi-status".into());
        let mut h = Harness::new(size);
        click(&mut h, &mut app, "Idle / fallback");
        click(&mut h, &mut app, "Finished (settled, not success)");
        // Reach the real value through the inset inspector's own scrolling.
        click(&mut h, &mut app, "Show for");
        let frame = h.settle(|ctx| app.show_lighting(ctx));
        let value = frame
            .texts()
            .into_iter()
            .find(|t| t.text.ends_with(" s") && t.fully_visible())
            .expect("seconds control");
        h.click_at(value.rect.center(), |ctx| app.show_lighting(ctx));
        h.run(
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        command: true,
                        ctrl: true,
                        ..Default::default()
                    },
                },
                egui::Event::Text("5.25".into()),
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                },
            ],
            |ctx| app.show_lighting(ctx),
        );
        h.settle(|ctx| app.show_lighting(ctx));
        let mut expected = doc.clone();
        if let Activation::OnEvent { duration_ms, .. } =
            &mut expected.layers[1].variants[1].activation
        {
            *duration_ms = 5250;
        }
        assert_eq!(
            app.setup_editor.draft.as_ref(),
            Some(&expected),
            "only duration changes at {size:?}"
        );
        assert_eq!(app.setup_editor.applied.as_ref(), Some(&doc));
        assert!(app.setup_editor.dirty);
        assert!(
            incoming.try_recv().is_err(),
            "editing must never activate or publish"
        );
        click(&mut h, &mut app, "Save");
        let Action::SetupReplace { document, .. } = incoming.try_recv().expect("explicit save")
        else {
            panic!("duration must use the ordinary canonical Save");
        };
        assert_eq!(document, expected);
        assert!(incoming.try_recv().is_err());
    }
}
#[test]
fn sweep_background_controls_edit_whole_layers_and_pi_appearances_without_other_changes() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE, WIDE_SIZE] {
        for pi in [false, true] {
            let (mut app, incoming) = app();
            let mut doc = underglow::setup_library::starter(
                underglow::setup_library::Starter::MatrixPi,
                "sweep-ui".into(),
                "Sweep UI".into(),
            )
            .unwrap();
            doc.layers[0].renderer = super::setup_inspector::default_renderer("sweep").unwrap();
            app.setup_editor.load(Some(doc.clone()));
            app.setup_editor.selected = Some(doc.layers[usize::from(pi)].id.clone());
            let mut h = Harness::new(size);
            if pi {
                click(&mut h, &mut app, "Idle / fallback");
                click(&mut h, &mut app, "Working");
            }
            click(&mut h, &mut app, "Solid color");
            click(&mut h, &mut app, "Transparent");
            let mut expected = doc.clone();
            let renderer = if pi {
                &mut expected.layers[1].variants[0].renderer
            } else {
                &mut expected.layers[0].renderer
            };
            if let underglow::setup::Renderer::Sweep { background, .. } = renderer {
                *background = underglow::setup::SweepBackground::Transparent {};
            } else {
                panic!("expected sweep");
            }
            assert_eq!(
                app.setup_editor.draft.as_ref(),
                Some(&expected),
                "only selected background changes at {size:?}"
            );
            assert_eq!(app.setup_editor.applied.as_ref(), Some(&doc));
            assert!(
                incoming.try_recv().is_err(),
                "editing must not enable lighting or publish"
            );
            click(&mut h, &mut app, "Save");
            let Action::SetupReplace { document, .. } = incoming.try_recv().unwrap() else {
                panic!("ordinary Save required");
            };
            assert_eq!(document, expected);
            assert!(incoming.try_recv().is_err());
        }
    }
}
