//! Whole-shell reachability at real window sizes, using production brand styling.
use super::tests::{controller_fixture, setup_document};
use super::workspace_test_ui::{DEFAULT_SIZE, Harness, MINIMUM_SIZE, WIDE_SIZE};
use super::*;

fn app() -> (Controller, Receiver<Action>) {
    let (mut app, incoming) = controller_fixture();
    app.status = Some(
        serde_json::from_value(serde_json::json!({
            "schema_version": 1, "state": "paused", "enabled": false,
            "mode": "matrix", "brightness": 128, "palette": "terminal",
            "fps": 30, "retry_attempt": 0
        }))
        .unwrap(),
    );
    let mut doc = setup_document();
    doc.name = "Workspace setup".into();
    app.setup_editor.load(Some(doc));
    app.setup_editor
        .name_draft("Workspace draft".into())
        .unwrap();
    (app, incoming)
}

/// Navigate the actual bounded pane to a control; never click an offscreen galley.
pub(super) fn click_workspace_control(harness: &mut Harness, app: &mut Controller, label: &str) {
    for step in 0..70 {
        // Choose the hit only after scrolling/layout has actually settled.
        // A short compact inspector can move controls out of an earlier clip.
        for _ in 0..30 {
            harness.run(vec![], |ctx| app.show_lighting(ctx));
        }
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let texts = frame.texts();
        let inspector = texts
            .iter()
            .find(|t| t.text.starts_with("Selected layer:"))
            .map(|t| t.clip);
        let tools = texts
            .iter()
            .find(|t| t.text == "Close preview tools")
            .map(|t| t.clip);
        let keyboard = keyboard_region(&frame);
        let layer_action = matches!(label, "Duplicate layer" | "Delete layer" | "Rename layer");
        let region = match label {
            "F1" => Some(keyboard),
            // These controls intentionally live outside scrollable inspectors.
            "Preview tools" | "Preview" | "Select keys" | "Inspect" | "Save composition"
            | "Save" | "Cancel" | "Add layer" => None,
            _ if layer_action => None, // Layer-actions popup is an explicit overlay.
            _ => tools.or(inspector),
        };
        if let Some(hit) = texts.iter().find(|t| {
            t.text == label
                && t.fully_visible()
                && region.is_none_or(|r| {
                    if tools == Some(r) || label == "F1" {
                        r.contains_rect(t.rect) && r.contains_rect(t.clip)
                    } else {
                        // Inspector popups may extend beyond their originating
                        // pane; their own viewport-intersected clip must still
                        // show the complete label. Exclude the quick toolbar's
                        // actual clip rather than all overlapping geometry.
                        r.contains(t.rect.center()) && t.clip != keyboard
                    }
                })
        }) {
            // Do not look the label up again using point-only region matching:
            // a tool window can overlap an unrelated quick-test button.
            harness.click_at(hit.rect.center(), |ctx| app.show_lighting(ctx));
            return;
        }
        if layer_action
            && let Some(menu) = texts
                .iter()
                .find(|t| t.text == "Layer actions" && t.fully_visible())
        {
            harness.click_at(menu.rect.center(), |ctx| app.show_lighting(ctx));
            continue;
        }
        let pane = tools.or(inspector).expect("a scrollable workspace pane");
        harness.wheel(
            pane.center(),
            egui::vec2(
                0.0,
                if step == 34 {
                    20000.0
                } else {
                    -(pane.height() * 0.6).min(220.0)
                },
            ),
            |ctx| app.show_lighting(ctx),
        );
    }
    panic!("unreachable {label} in {:?}", harness.screen());
}

fn visible(frame: &workspace_test_ui::Frame, label: &str) -> egui::Rect {
    visible_in(frame, label, None)
}

fn keyboard_region(frame: &workspace_test_ui::Frame) -> egui::Rect {
    frame
        .texts()
        .into_iter()
        .find(|t| t.text == "Composition preview")
        .unwrap()
        .clip
}

fn key(frame: &workspace_test_ui::Frame, label: &str) -> egui::Rect {
    visible_in(frame, label, Some(keyboard_region(frame)))
}

fn visible_in(
    frame: &workspace_test_ui::Frame,
    label: &str,
    region: Option<egui::Rect>,
) -> egui::Rect {
    let all: Vec<_> = frame
        .texts()
        .into_iter()
        .filter(|t| t.text == label)
        .collect();
    let matches: Vec<_> = frame
        .texts()
        .into_iter()
        .filter(|text| {
            text.text == label
                && text.fully_visible()
                && region.is_none_or(|r| r.contains(text.rect.center()))
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "one fully visible {label} in {:?}: {matches:?}; all={all:?}",
        frame.screen
    );
    matches[0].rect
}

#[test]
fn workspace_primary_controls_and_complete_keyboard_are_reachable_at_real_sizes() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE, WIDE_SIZE] {
        let mut harness = Harness::new(size);
        let (mut app, incoming) = app();
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        for label in [
            "Editor",
            "Connections",
            "Settings",
            "Browse setups: Workspace draft",
            "Unsaved changes",
            "Resume lighting",
            "Save",
            "Add layer",
            "Preview tools",
            "Preview",
            "Select keys",
            "Inspect",
        ] {
            visible(&frame, label);
        }
        let selected = app
            .setup_editor
            .draft
            .as_ref()
            .unwrap()
            .layers
            .iter()
            .find(|l| Some(&l.id) == app.setup_editor.selected.as_ref())
            .unwrap();
        let heading = visible(&frame, &format!("Selected layer: {}", selected.name));
        let canvas_top = key(&frame, "Esc");
        let canvas_bottom = key(&frame, "→");
        assert!(canvas_bottom.bottom() > canvas_top.bottom());
        if size == MINIMUM_SIZE {
            visible(&frame, "Layer:");
            assert!(
                heading.top() > canvas_bottom.bottom(),
                "compact inspector below keyboard"
            );
        } else {
            let layers = visible(&frame, "Layers");
            assert!(layers.left() < canvas_top.left());
            assert!(
                heading.left() > canvas_bottom.right(),
                "desktop inspector alongside keyboard"
            );
        }
        let draft = app.setup_editor.draft.clone().unwrap();
        harness.click("Save", None, |ctx| app.show_lighting(ctx));
        assert!(
            matches!(incoming.try_recv(), Ok(Action::SetupReplace { document, revision })
            if document == draft && revision == app.setup_editor.base_revision)
        );
        assert!(
            !app.status.as_ref().unwrap().enabled,
            "saving never enables lighting"
        );
    }
}

#[test]
fn workspace_inspector_scroll_preserves_pinned_canvas_save_and_selected_context() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE, WIDE_SIZE] {
        let mut harness = Harness::new(size);
        let (mut app, incoming) = app();
        app.setup_editor.preview.mode = setup_preview::Mode::InputDemo;
        // The default Appearance form is intentionally short. Expand real controls
        // to exercise overflow, then scroll back to the top before measuring.
        click_workspace_control(&mut harness, &mut app, "Advanced layer settings");
        let expanded = harness.settle(|ctx| app.show_lighting(ctx));
        let pane = expanded
            .texts()
            .into_iter()
            .find(|t| t.text.starts_with("Selected layer:"))
            .unwrap()
            .clip;
        harness.wheel(pane.center(), egui::vec2(0.0, 20000.0), |ctx| {
            app.show_lighting(ctx)
        });
        for _ in 0..30 {
            harness.run(vec![], |ctx| app.show_lighting(ctx));
        }
        let draft = app.setup_editor.draft.clone();
        let selected = app.setup_editor.selected.clone();
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let save = visible(&frame, "Save");
        let esc = key(&frame, "Esc");
        let arrow = key(&frame, "→");
        let inspector_label = if size == MINIMUM_SIZE {
            "Appearance"
        } else {
            "Layer name"
        };
        let inspector = visible(&frame, inspector_label);
        for _ in 0..8 {
            harness.wheel(inspector.center(), egui::vec2(0.0, -400.0), |ctx| {
                app.show_lighting(ctx)
            });
        }
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        assert_eq!(visible(&frame, "Save"), save);
        assert_eq!(key(&frame, "Esc"), esc);
        assert_eq!(key(&frame, "→"), arrow);
        assert!(
            !frame
                .texts()
                .iter()
                .any(|t| t.text == inspector_label && t.fully_visible()),
            "inspector actually scrolled"
        );
        assert_eq!(app.setup_editor.draft, draft);
        assert_eq!(app.setup_editor.selected, selected);
        assert!(app.setup_editor.preview.mode == setup_preview::Mode::InputDemo);
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn workspace_layer_list_scroll_is_independent_and_topmost_first() {
    for size in [DEFAULT_SIZE, WIDE_SIZE] {
        let mut harness = Harness::new(size);
        let (mut app, incoming) = app();
        let original = app.setup_editor.draft.as_ref().unwrap().layers[0].clone();
        for i in 0..24 {
            let index = app.setup_editor.draft.as_ref().unwrap().layers.len();
            app.setup_editor
                .apply(underglow::setup_edits::LayerEdit::Duplicate {
                    id: original.id.clone(),
                    new_id: format!("copy-{i}"),
                    index,
                })
                .unwrap();
        }
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let esc = key(&frame, "Esc");
        let inspector = visible(&frame, "Layer name");
        let doc = app.setup_editor.draft.as_ref().unwrap();
        visible(
            &frame,
            &format!("{} · {}", doc.layers.len(), doc.layers.last().unwrap().name),
        );
        let list_pos = visible(&frame, "Topmost first").center() + egui::vec2(0.0, 55.0);
        for _ in 0..16 {
            harness.wheel(list_pos, egui::vec2(0.0, -500.0), |ctx| {
                app.show_lighting(ctx)
            });
        }
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        assert_eq!(key(&frame, "Esc"), esc);
        assert_eq!(visible(&frame, "Layer name"), inspector);
        let bottom = format!("1 · {}", original.name);
        visible(&frame, &bottom);
        harness.click(&bottom, None, |ctx| app.show_lighting(ctx));
        assert_eq!(
            app.setup_editor.selected.as_deref(),
            Some(original.id.as_str())
        );
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn workspace_compact_edits_and_resizing_keep_one_draft_and_selection() {
    let mut harness = Harness::new(MINIMUM_SIZE);
    let (mut app, incoming) = app();
    let original = app.setup_editor.draft.clone().unwrap();
    harness.click("Add layer", None, |ctx| app.show_lighting(ctx));
    harness.click("Aurora layer", None, |ctx| app.show_lighting(ctx));
    let added = app.setup_editor.selected.clone().unwrap();
    assert_eq!(
        app.setup_editor.draft.as_ref().unwrap().layers.len(),
        original.layers.len() + 1
    );
    click_workspace_control(&mut harness, &mut app, "Duplicate layer");
    let duplicate = app.setup_editor.selected.clone().unwrap();
    assert_ne!(duplicate, added);
    assert_eq!(
        app.setup_editor.draft.as_ref().unwrap().layers.len(),
        original.layers.len() + 2
    );
    app.setup_editor.preview.mode = setup_preview::Mode::InputDemo;
    let draft = app.setup_editor.draft.clone();
    for size in [DEFAULT_SIZE, WIDE_SIZE, MINIMUM_SIZE] {
        harness.resize(size);
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        key(&frame, "Esc");
        key(&frame, "→");
        visible(&frame, "Save");
        assert_eq!(app.setup_editor.draft, draft);
        assert_eq!(
            app.setup_editor.selected.as_deref(),
            Some(duplicate.as_str())
        );
        assert!(app.setup_editor.preview.mode == setup_preview::Mode::InputDemo);
    }
    click_workspace_control(&mut harness, &mut app, "Delete layer");
    let edited = app.setup_editor.draft.as_ref().unwrap();
    assert_eq!(&edited.layers[..original.layers.len()], &original.layers);
    assert_eq!(edited.layers.len(), original.layers.len() + 1);
    assert!(app.setup_editor.dirty);
    assert!(
        incoming.try_recv().is_err(),
        "compact edits are local, not auto-saved"
    );
}

#[test]
fn workspace_pi_appearance_and_color_are_visible_first_and_editable_at_real_sizes() {
    use underglow::setup::{Renderer, SetupDocument};
    use underglow::setup_library::{Starter, starter};
    for size in [MINIMUM_SIZE, DEFAULT_SIZE] {
        let (mut app, incoming) = app();
        let document =
            starter(Starter::MatrixPi, "compact-pi".into(), "Compact Pi".into()).unwrap();
        app.setup_editor = setup_editor::Editor::default();
        app.setup_editor.load(Some(document.clone()));
        app.setup_editor.selected = Some("pi-status".into());
        let mut harness = Harness::new(size);
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let inspector = visible(&frame, "Selected layer: Pi status");
        let pane = frame
            .texts()
            .into_iter()
            .find(|t| t.text.starts_with("Selected layer:"))
            .unwrap()
            .clip;
        for label in ["Appearance", "Idle / fallback", "Solid", "Color"] {
            visible_in(&frame, label, Some(pane));
        }
        if size == MINIMUM_SIZE {
            assert!(!frame.texts().iter().any(|t| t.text == "Layer name"
                || t.text == "Duplicate layer"
                || t.text == "Delete layer"));
            visible(&frame, "Layer actions");
        }
        let color = visible_in(&frame, "Color", Some(pane));
        assert!(color.top() > inspector.top());
        // The real color well immediately precedes its Color label.
        let well = egui::pos2(color.left() - 15.0, color.center().y);
        assert!(pane.contains(well));
        harness.click_at(well, |ctx| app.show_lighting(ctx));
        let popup = harness.settle(|ctx| app.show_lighting(ctx));
        let red = popup
            .texts()
            .into_iter()
            .find(|t| t.text.starts_with("R ") && t.fully_visible())
            .expect("RGB picker is reachable without scrolling/properties");
        let start = red.rect.center();
        harness.run(workspace_test_ui::pointer(start, true), |ctx| {
            app.show_lighting(ctx)
        });
        harness.run(
            vec![egui::Event::PointerMoved(start + egui::vec2(40.0, 0.0))],
            |ctx| app.show_lighting(ctx),
        );
        harness.run(
            workspace_test_ui::pointer(start + egui::vec2(40.0, 0.0), false),
            |ctx| app.show_lighting(ctx),
        );
        let changed: &SetupDocument = app.setup_editor.draft.as_ref().unwrap();
        let mut expected = document.clone();
        expected.layers[1].renderer = changed.layers[1].renderer.clone();
        assert_eq!(changed, &expected);
        assert!(matches!(&changed.layers[1].renderer, Renderer::Solid { color } if color[0] > 0));
        assert_eq!(app.setup_editor.selected.as_deref(), Some("pi-status"));
        assert!(app.setup_editor.dirty);
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn workspace_compact_actions_rename_duplicate_delete_through_the_real_menu() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE] {
        let (mut app, incoming) = app();
        let mut harness = Harness::new(size);
        let selected = app.setup_editor.selected.clone().unwrap();
        let before = app.setup_editor.draft.clone().unwrap();
        // Compact has a single menu instead of duplicate permanent action rows.
        if size == MINIMUM_SIZE {
            click_workspace_control(&mut harness, &mut app, "Rename layer");
            harness.run(
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
                    egui::Event::Text("Compact renamed".into()),
                ],
                |ctx| app.show_lighting(ctx),
            );
            assert_eq!(
                app.setup_editor
                    .draft
                    .as_ref()
                    .unwrap()
                    .layers
                    .iter()
                    .find(|l| l.id == selected)
                    .unwrap()
                    .name,
                "Compact renamed"
            );
            // Clicking the tab finishes inline renaming and restores editing-first chrome.
            click_workspace_control(&mut harness, &mut app, "Appearance");
        }
        click_workspace_control(&mut harness, &mut app, "Duplicate layer");
        let duplicate = app.setup_editor.selected.clone().unwrap();
        assert_ne!(duplicate, selected);
        assert_eq!(
            app.setup_editor.draft.as_ref().unwrap().layers.len(),
            before.layers.len() + 1
        );
        click_workspace_control(&mut harness, &mut app, "Delete layer");
        assert!(
            !app.setup_editor
                .draft
                .as_ref()
                .unwrap()
                .layers
                .iter()
                .any(|l| l.id == duplicate)
        );
        assert!(
            app.setup_editor
                .draft
                .as_ref()
                .unwrap()
                .layers
                .iter()
                .any(|l| l.id == selected)
        );
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn workspace_compact_connection_notice_remains_visible_in_real_scroll_content() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE] {
        let (mut app, incoming) = app();
        app.setup_editor.selected = Some(
            app.setup_editor.draft.as_ref().unwrap().layers[1]
                .id
                .clone(),
        );
        let mut harness = Harness::new(size);
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let message = app.setup_editor.connection_notices[0].message.clone();
        let pane = frame
            .texts()
            .into_iter()
            .find(|t| t.text.starts_with("Selected layer:"))
            .unwrap()
            .clip;
        if size == MINIMUM_SIZE {
            harness.wheel(pane.center(), egui::vec2(0.0, -2000.0), |ctx| {
                app.show_lighting(ctx)
            });
            for _ in 0..30 {
                harness.run(vec![], |ctx| app.show_lighting(ctx));
            }
        }
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        assert!(
            frame
                .texts()
                .iter()
                .any(|t| t.text == message && t.fully_visible() && pane.contains(t.rect.center())),
            "a real connection note remains readable, not only a hover tooltip"
        );
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn workspace_preview_badge_explains_source_and_playback_without_open_tools() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE, WIDE_SIZE] {
        let mut harness = Harness::new(size);
        let (mut app, incoming) = app();
        let draft = app.setup_editor.draft.clone();
        for (mode, playing, context) in [
            (setup_preview::Mode::Live, false, "Live facts"),
            (setup_preview::Mode::InputDemo, false, "Isolated input demo"),
            (setup_preview::Mode::Scenario, false, "Scenario paused"),
            (setup_preview::Mode::Scenario, true, "Scenario running"),
        ] {
            app.setup_editor.preview.set_mode(mode);
            app.setup_editor.preview.set_scenario_playing(playing);
            let frame = harness.settle(|ctx| app.show_lighting(ctx));
            visible(
                &frame,
                &format!("Preview only · {context} · no hardware output"),
            );
            key(&frame, "Esc");
            assert_eq!(app.setup_editor.draft, draft);
            assert!(incoming.try_recv().is_err());
        }
    }
}
