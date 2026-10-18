//! Mounted workspace interactions; no native windows, workers, or SDK.
use super::*;
use crate::tests::controller_fixture;
use crate::workspace_test_ui::{DEFAULT_SIZE, Harness, MINIMUM_SIZE, pointer};
use crate::workspace_tests::click_workspace_control;
use underglow::device::DeviceInfo;
use underglow::key_targets::canonical_coord;
use underglow::layout::KeyboardLayout;
use underglow::setup::Renderer;

fn direct(names: &[&str]) -> Target {
    Target::Selection {
        zones: vec![],
        keys: names.iter().map(|name| (*name).into()).collect(),
        groups: vec![],
    }
}

fn document() -> SetupDocument {
    let mut doc = underglow::setup_library::starter(
        underglow::setup_library::Starter::MatrixPi,
        "canvas-test".into(),
        "Canvas test".into(),
    )
    .unwrap();
    // Deterministic capture with a real black Pi contribution over an unrelated
    // background. Both editable Pi appearances remain in the one logical layer.
    doc.layers[0].renderer = Renderer::Solid {
        color: [16, 32, 48],
    };
    doc.layers[0].brightness = 255;
    doc.layers[1].activation = Activation::Always {};
    doc.layers[1].renderer = Renderer::Solid { color: [0; 3] };
    doc.layers[1].brightness = 255;
    doc.layers[1].target = direct(&["F1"]);
    doc
}

fn app(doc: SetupDocument) -> (crate::Controller, std::sync::mpsc::Receiver<crate::Action>) {
    let (mut app, incoming) = controller_fixture();
    app.status = Some(
        serde_json::from_value(serde_json::json!({
            "schema_version": 1, "state": "paused", "enabled": false,
            "mode": "matrix", "brightness": 255, "palette": "terminal",
            "fps": 30, "retry_attempt": 0
        }))
        .unwrap(),
    );
    app.setup_editor.load(Some(doc));
    app.setup_editor
        .preview
        .set_mode(setup_preview::Mode::Scenario);
    (app, incoming)
}

fn key(frame: &crate::workspace_test_ui::Frame, label: &str) -> egui::Pos2 {
    let texts = frame.texts();
    let region = texts
        .iter()
        .find(|text| text.text == "Composition preview")
        .unwrap()
        .clip;
    let keys: Vec<_> = texts
        .iter()
        .filter(|text| {
            text.text == label && region.contains_rect(text.clip) && region.contains_rect(text.rect)
        })
        .collect();
    assert_eq!(
        keys.len(),
        1,
        "one keyboard in the entire workspace, not one per pane"
    );
    assert!(keys[0].fully_visible());
    assert!(region.contains_rect(keys[0].rect));
    keys[0].rect.center()
}

fn rgb_keys(frame: &crate::workspace_test_ui::Frame) -> Vec<(egui::Rect, egui::Color32)> {
    let region = frame
        .texts()
        .into_iter()
        .find(|text| text.text == "Composition preview")
        .unwrap()
        .clip;
    let result: Vec<_> = frame
        .output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect)
                if rect.fill != egui::Color32::TRANSPARENT
                    && rect.stroke.width == 0.0
                    && rect.rect.width() > 5.0
                    && (rect.rect.width() - rect.rect.height()).abs() < 0.01
                    && region.contains_rect(rect.rect) =>
            {
                assert!(shape.clip_rect.contains_rect(rect.rect));
                assert!(frame.screen.contains_rect(rect.rect));
                Some((rect.rect, rect.fill))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        result.len(),
        KeyboardLayout::for_device(&DeviceInfo::synthetic_80he())
            .keys()
            .len()
    );
    result
}

#[test]
fn mounted_modes_keep_one_readable_rgb_capture_and_usable_inspector_at_real_sizes() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let mut harness = Harness::new(size);
        let doc = document();
        let (mut app, incoming) = app(doc.clone());
        app.setup_editor.highlight_coverage = true;
        let mut geometry = None;
        for mode in [
            setup_targets::CanvasMode::Preview,
            setup_targets::CanvasMode::SelectKeys,
            setup_targets::CanvasMode::Inspect,
        ] {
            harness.click(mode.label(), None, |ctx| app.show_lighting(ctx));
            let frame = harness.settle(|ctx| app.show_lighting(ctx));
            assert_eq!(app.setup_editor.canvas_mode, mode);
            for label in ["Preview", "Select keys", "Inspect"] {
                assert!(
                    frame
                        .texts()
                        .iter()
                        .any(|text| text.text == label && text.fully_visible())
                );
            }
            key(&frame, "Esc");
            key(&frame, "→");
            let inspector = frame
                .texts()
                .into_iter()
                .find(|text| text.text.starts_with("Selected layer:"))
                .unwrap();
            assert!(inspector.fully_visible());
            assert!(
                inspector.clip.height() >= 120.0,
                "inspector retains usable height"
            );
            let keys = rgb_keys(&frame);
            assert!(keys[0].0.width() >= 12.0, "key faces remain readable");
            let rects: Vec<_> = keys.iter().map(|(rect, _)| *rect).collect();
            if let Some(previous) = &geometry {
                assert_eq!(&rects, previous, "explicit modes retain board geometry");
            }
            geometry = Some(rects);
            let layout = KeyboardLayout::for_device(&DeviceInfo::synthetic_80he());
            for ((_, fill), position) in keys.iter().zip(layout.keys()) {
                let color = app.setup_editor.preview.frame().get_coord(position.coord);
                assert_eq!(
                    *fill,
                    egui::Color32::from_rgb(color.red, color.green, color.blue)
                );
            }
            assert!(keys.iter().any(|(_, fill)| *fill == egui::Color32::BLACK));
            assert!(keys.iter().any(|(_, fill)| *fill != egui::Color32::BLACK));
            assert_eq!(app.setup_editor.draft.as_ref(), Some(&doc));
            assert!(!app.setup_editor.dirty);
            assert!(incoming.try_recv().is_err());
        }
    }
}

#[test]
fn mounted_pi_retargeting_is_scoped_and_inspect_uses_the_actual_capture() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let mut harness = Harness::new(size);
        let original = document();
        let (mut app, incoming) = app(original.clone());
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        harness.click_at(key(&frame, "F4"), |ctx| app.show_lighting(ctx));
        assert_eq!(app.setup_editor.draft.as_ref(), Some(&original));
        assert!(
            app.setup_editor.explained_key.is_none(),
            "Preview has no implicit click action"
        );
        let facts = app.setup_editor.preview.evaluation_facts().0.clone();
        harness.click("Select keys", None, |ctx| app.show_lighting(ctx));
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        harness.click_at(key(&frame, "F4"), |ctx| app.show_lighting(ctx));
        let mut expected = original.clone();
        expected.layers[1].target = direct(&["F4"]);
        assert_eq!(app.setup_editor.draft.as_ref(), Some(&expected));
        assert_eq!(app.setup_editor.applied.as_ref(), Some(&original));
        assert_eq!(expected.layers.len(), 2);
        assert_eq!(expected.layers[1].variants, original.layers[1].variants);
        assert!(app.setup_editor.dirty);
        harness.click("Inspect", None, |ctx| app.show_lighting(ctx));
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        harness.click_at(key(&frame, "F1"), |ctx| app.show_lighting(ctx));
        let coord = canonical_coord("F1").unwrap();
        assert_eq!(app.setup_editor.explained_key, Some(coord));
        let capture = app.setup_editor.preview.coverage().unwrap();
        let explanation = setup_explanation::report(&app.setup_editor.preview, coord).unwrap();
        assert_eq!(explanation.color, capture.frame.get_coord(coord));
        assert_eq!(&capture.frame, app.setup_editor.preview.frame());
        assert_ne!(
            explanation.color,
            underglow::render::Color::BLACK,
            "retargeting uncovers the unchanged background at F1"
        );
        assert_eq!(app.setup_editor.draft.as_ref(), Some(&expected));
        assert_eq!(app.setup_editor.preview.evaluation_facts().0, &facts);
        assert!(app.setup_editor.preview.sample().is_none());
        assert!(
            incoming.try_recv().is_err(),
            "modes and edits never write hardware or save"
        );
    }
}

#[test]
fn mounted_group_draft_save_is_atomic_and_disabled_layer_references_are_safe() {
    let mut harness = Harness::new(MINIMUM_SIZE);
    let mut original = document();
    original.groups.insert("shared".into(), vec!["F1".into()]);
    original.layers[1].target = Target::Selection {
        zones: vec![],
        keys: vec![],
        groups: vec!["shared".into()],
    };
    let mut disabled = original.layers[0].clone();
    disabled.id = "disabled-reference".into();
    disabled.name = "Disabled reference".into();
    disabled.enabled = false;
    disabled.target = original.layers[1].target.clone();
    original.layers.push(disabled);
    let (mut app, incoming) = app(original.clone());
    app.setup_editor.selected = Some(original.layers[1].id.clone());
    harness.click("Select keys", None, |ctx| app.show_lighting(ctx));
    click_workspace_control(&mut harness, &mut app, "Reusable custom groups");
    click_workspace_control(&mut harness, &mut app, "Edit / rename");
    harness.click("Select keys", None, |ctx| app.show_lighting(ctx));
    let frame = harness.settle(|ctx| app.show_lighting(ctx));
    harness.click_at(key(&frame, "F4"), |ctx| app.show_lighting(ctx));
    assert_eq!(app.setup_editor.draft.as_ref(), Some(&original));
    assert!(
        !app.setup_editor.dirty,
        "group members are only a UI draft before Save"
    );
    click_workspace_control(&mut harness, &mut app, "Save group");
    let mut expected = original.clone();
    expected.groups.insert("shared".into(), vec!["F4".into()]);
    assert_eq!(app.setup_editor.draft.as_ref(), Some(&expected));
    assert!(app.setup_editor.dirty);
    click_workspace_control(&mut harness, &mut app, "Edit / rename");
    click_workspace_control(&mut harness, &mut app, "shared");
    harness.run(
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            },
            egui::Event::Text("renamed".into()),
        ],
        |ctx| app.show_lighting(ctx),
    );
    click_workspace_control(&mut harness, &mut app, "Save group");
    assert_eq!(
        app.setup_editor.draft.as_ref(),
        Some(&expected),
        "shared rename must be refused"
    );
    click_workspace_control(&mut harness, &mut app, "Cancel group edit");
    click_workspace_control(&mut harness, &mut app, "Delete");
    assert_eq!(
        app.setup_editor.draft.as_ref(),
        Some(&expected),
        "referenced group cannot be deleted"
    );
    assert!(incoming.try_recv().is_err());
}

#[test]
fn mounted_layer_and_mode_changes_cannot_commit_an_old_drag_and_load_resets_context() {
    let mut harness = Harness::new(MINIMUM_SIZE);
    let original = document();
    let (mut app, incoming) = app(original.clone());
    harness.click("Select keys", None, |ctx| app.show_lighting(ctx));
    for change_layer in [false, true] {
        app.setup_editor.selected = Some(original.layers[1].id.clone());
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let start = key(&frame, "F1");
        let end = key(&frame, "F4");
        harness.run(pointer(start, true), |ctx| app.show_lighting(ctx));
        harness.run(vec![egui::Event::PointerMoved(end)], |ctx| {
            app.show_lighting(ctx)
        });
        if change_layer {
            app.setup_editor.selected = Some(original.layers[0].id.clone());
        } else {
            app.setup_editor.canvas_mode = setup_targets::CanvasMode::Preview;
            harness.run(vec![], |ctx| app.show_lighting(ctx));
            app.setup_editor.canvas_mode = setup_targets::CanvasMode::SelectKeys;
        }
        harness.run(pointer(end, false), |ctx| app.show_lighting(ctx));
        assert_eq!(app.setup_editor.draft.as_ref(), Some(&original));
        assert!(!app.setup_editor.dirty);
    }
    app.setup_editor.canvas_mode = setup_targets::CanvasMode::Inspect;
    app.setup_editor.explained_key = canonical_coord("F1");
    app.setup_editor.load(Some(original.clone()));
    assert_eq!(
        app.setup_editor.canvas_mode,
        setup_targets::CanvasMode::Inspect,
        "an identical status refresh preserves mode and inspector context"
    );
    assert!(app.setup_editor.explained_key.is_some());
    let mut replacement = original;
    replacement.id = "another-setup-with-same-layer-ids".into();
    app.setup_editor.load(Some(replacement));
    assert!(app.setup_editor.targets.is_empty());
    assert!(app.setup_editor.explained_key.is_none());
    assert_eq!(
        app.setup_editor.canvas_mode,
        setup_targets::CanvasMode::Preview
    );
    assert!(incoming.try_recv().is_err());
}

#[test]
fn acknowledged_switch_resets_canvas_context_when_setup_ids_change() {
    let original = document();
    let mut editor = Editor::default();
    editor.load(Some(original.clone()));
    let id = original.layers[0].id.clone();
    editor.targets.entry(id.clone()).or_default();
    editor
        .inspector_sections
        .entry(id.clone())
        .or_default()
        .reveal(inspector::Section::Where);
    editor.canvas_selection = Some(id);
    editor.canvas_mode = setup_targets::CanvasMode::Inspect;
    editor.explained_key = canonical_coord("F1");
    let mut replacement = original.clone();
    replacement.id = "acknowledged-copy-with-the-same-layer-ids".into();
    replacement.revision += 1;

    editor.acknowledge_persisted(Some(&original), replacement.clone());

    assert_eq!(editor.draft.as_ref(), Some(&replacement));
    assert_eq!(editor.applied.as_ref(), Some(&replacement));
    assert_eq!(editor.base_revision, replacement.revision);
    assert!(!editor.dirty);
    assert!(editor.targets.is_empty());
    assert!(editor.inspector_sections.is_empty());
    assert!(editor.canvas_selection.is_none());
    assert!(editor.explained_key.is_none());
    assert_eq!(editor.canvas_mode, setup_targets::CanvasMode::Preview);
}

#[test]
fn keys_tab_and_canvas_selection_mode_share_the_same_editing_context() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let mut harness = Harness::new(size);
        let (mut app, incoming) = app(document());
        let id = app.setup_editor.selected.clone().unwrap();
        click_workspace_control(&mut harness, &mut app, "Edit keys");
        assert_eq!(
            app.setup_editor.canvas_mode,
            setup_targets::CanvasMode::SelectKeys
        );
        assert_eq!(
            app.setup_editor
                .inspector_sections
                .get(&id)
                .map(|state| state.keys_open),
            Some(true)
        );
        harness.click("Preview", None, |ctx| app.show_lighting(ctx));
        assert_eq!(
            app.setup_editor.canvas_mode,
            setup_targets::CanvasMode::Preview
        );
        // Clicking an already-selected Keys tab is still an explicit request
        // to resume selection; merely repainting it never forces the mode.
        click_workspace_control(&mut harness, &mut app, "Edit keys");
        assert_eq!(
            app.setup_editor.canvas_mode,
            setup_targets::CanvasMode::SelectKeys
        );
        click_workspace_control(&mut harness, &mut app, "Looks");
        harness.click("Select keys", None, |ctx| app.show_lighting(ctx));
        assert_eq!(
            app.setup_editor
                .inspector_sections
                .get(&id)
                .map(|state| state.keys_open),
            Some(true)
        );
        assert!(incoming.try_recv().is_err());
        assert!(!app.setup_editor.dirty);
    }
}

#[test]
fn mounted_small_key_hover_reveals_the_full_canonical_physical_name() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let mut harness = Harness::new(size);
        let (mut app, incoming) = app(document());
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let layout = KeyboardLayout::for_device(&DeviceInfo::synthetic_80he());
        let index = layout
            .keys()
            .iter()
            .position(|key| Some(key.coord) == canonical_coord("ControlLeft"))
            .unwrap();
        let pos = rgb_keys(&frame)[index].0.center();
        harness.run(vec![egui::Event::PointerMoved(pos)], |ctx| {
            app.show_lighting(ctx)
        });
        let mut tooltip = false;
        for _ in 0..90 {
            let frame = harness.run(vec![], |ctx| app.show_lighting(ctx));
            tooltip |= frame
                .texts()
                .iter()
                .any(|text| text.text.starts_with("ControlLeft · row"));
        }
        assert!(
            tooltip,
            "abbreviated legends retain full physical-name/address tooltips"
        );
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn replacing_setups_with_reused_pi_ids_clears_pending_rename_context() {
    use underglow::setup_library::{Starter, starter};
    let old = starter(Starter::MatrixPi, "old".into(), "Old".into()).unwrap();
    let mut new = starter(Starter::MatrixPi, "new".into(), "New".into()).unwrap();
    assert_eq!(old.layers[1].id, new.layers[1].id);
    let mut editor = Editor::default();
    editor.load(Some(old.clone()));
    editor.rename_focus = Some(old.layers[1].id.clone());
    editor.rename_editing = editor.rename_focus.clone();
    editor.load(Some(new.clone()));
    assert!(editor.rename_focus.is_none() && editor.rename_editing.is_none());

    editor.load(Some(old.clone()));
    editor.rename_focus = Some(old.layers[1].id.clone());
    editor.rename_editing = editor.rename_focus.clone();
    new.revision = 1;
    editor.acknowledge_persisted(Some(&old), new);
    assert!(editor.rename_focus.is_none() && editor.rename_editing.is_none());
    editor.rename_focus = Some("pi-status".into());
    editor.rename_editing = editor.rename_focus.clone();
    editor.discard();
    assert!(editor.rename_focus.is_none() && editor.rename_editing.is_none());
}
