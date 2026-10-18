//! Production inspector and menus at realistic sizes; no workers or hardware.
use super::super::{Action, Completion, Controller, Status};
use super::super::{
    tests::{controller_fixture, setup_document},
    workspace_test_ui::{DEFAULT_SIZE, Harness, MINIMUM_SIZE},
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
            .find(|t| t.text.starts_with("Selected layer:"))
            .map(|t| t.clip);
        let keyboard = texts
            .iter()
            .find(|t| t.text == "Composition preview")
            .map(|t| t.clip);
        if let Some(t) = texts.iter().find(|t| {
            t.text == label
                && t.fully_visible()
                && (label != "F1" || keyboard.is_some_and(|r| r.contains(t.rect.center())))
                // Quick tests repeat capability labels such as Working. An
                // inspector choice must come from its pane/popup, not the
                // independently clipped canonical keyboard's test toolbar.
                && (matches!(label, "F1" | "Composition preview")
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
        let pane = pane.expect("selected-layer pane");
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
    click(&mut h, &mut app, "Composition preview");
    click(&mut h, &mut app, "...");
    click(&mut h, &mut app, "Toward bottom");
    let doc = app.setup_editor.draft.as_ref().unwrap();
    assert_eq!(doc.layers[0].id, top);
    assert_eq!(doc.layers[1], bottom);
    assert_eq!(app.setup_editor.selected.as_deref(), Some(top.as_str()));
    let frame = h.settle(|ctx| app.show_lighting(ctx));
    let texts = frame.texts();
    let upper = texts.iter().find(|t| t.text == "2 · Background").unwrap();
    let lower = texts.iter().find(|t| t.text == "1 · New layer").unwrap();
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
