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
    // A semantic test intent resolves to the actual dynamic setup switcher.
    // Do not retain obsolete compatibility labels in the production UI.
    let switcher = (label == "Change setup...").then(|| {
        let title = app
            .setup_editor
            .draft
            .as_ref()
            .map(|doc| doc.name.as_str())
            .unwrap_or_else(|| effect_description(&app.preset).0);
        title.to_owned()
    });
    let label = switcher.as_deref().unwrap_or(label);
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
            .find(|t| t.text == "Layer actions")
            .map(|t| t.clip);
        // Measure the actual scroll viewport inside the continuous inspector.
        // Its inset surface can end above the pane edge; do not assume equal bottoms.
        let scroll = inspector
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
            .or(inspector);
        let tools = texts
            .iter()
            .find(|t| t.text == "Close preview tools")
            .map(|t| t.clip);
        let keyboard = keyboard_region(&frame);
        let popup_action = matches!(
            label,
            "Duplicate layer"
                | "Delete layer"
                | "Rename layer"
                | "Add behavior appearance"
                | "Appearance enabled"
                | "Appearance name"
                | "Delete appearance"
                | "Advanced appearance settings"
                | "Lower priority"
                | "Higher priority"
        );
        let region = match label {
            "F1" => Some(keyboard),
            // These controls intentionally live outside scrollable inspectors.
            "Preview tools" | "Preview" | "Select keys" | "Inspect" | "Save composition"
            | "Save" | "Cancel" | "Add layer" | "Test behavior…" | "Live data" | "Private test"
            | "Play test" | "Pause test" => None,
            _ if popup_action => None, // Management popup is an explicit overlay.
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
        if matches!(label, "Duplicate layer" | "Delete layer" | "Rename layer")
            && let Some(menu) = texts
                .iter()
                .find(|t| t.text == "Layer actions" && t.fully_visible())
        {
            harness.click_at(menu.rect.center(), |ctx| app.show_lighting(ctx));
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
            harness.click_at(options.rect.center(), |ctx| app.show_lighting(ctx));
            continue;
        }
        if matches!(
            label,
            "Add behavior appearance"
                | "Appearance enabled"
                | "Appearance name"
                | "Delete appearance"
                | "Advanced appearance settings"
                | "Lower priority"
                | "Higher priority"
        ) && let Some(menu) = texts
            .iter()
            .find(|t| t.text == "Appearance actions" && t.fully_visible())
        {
            harness.click_at(menu.rect.center(), |ctx| app.show_lighting(ctx));
            continue;
        }
        let pane = tools.or(scroll).expect("a scrollable workspace pane");
        harness.wheel(
            if tools.is_some() {
                pane.center()
            } else {
                egui::pos2(pane.center().x, pane.bottom() - 10.0)
            },
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

/// Select a private test through the actual picker, whose title changes after
/// the first run; its identity is separate from the appearance edit picker.
pub(super) fn choose_private_test(harness: &mut Harness, app: &mut Controller, label: &str) {
    let frame = harness.settle(|ctx| app.show_lighting(ctx));
    let canvas = keyboard_region(&frame);
    let chooser = frame
        .texts()
        .into_iter()
        .find(|t| {
            t.fully_visible()
                && canvas.contains_rect(t.rect)
                && (t.text == "Test behavior…"
                    || t.text.starts_with("Testing:")
                    || t.text.starts_with("No test facts")
                    || t.text.starts_with("Advanced scenario ·"))
        })
        .expect("a real visible contextual private-test picker");
    harness.click_at(chooser.rect.center(), |ctx| app.show_lighting(ctx));
    let menu = harness.settle(|ctx| app.show_lighting(ctx));
    let option = menu
        .texts()
        .into_iter()
        .find(|t| {
            t.text == label
                && t.fully_visible()
                && harness
                    .context
                    .layer_id_at(t.rect.center())
                    .is_some_and(|layer| layer.order == egui::Order::Foreground)
        })
        .expect("a real visible foreground test option, not the editing appearance");
    harness.click_at(option.rect.center(), |ctx| app.show_lighting(ctx));
}

fn visible(frame: &workspace_test_ui::Frame, label: &str) -> egui::Rect {
    visible_in(frame, label, None)
}

fn keyboard_region(frame: &workspace_test_ui::Frame) -> egui::Rect {
    let texts = frame.texts();
    let key = texts
        .iter()
        .find(|t| t.text == "F1" && t.rect.height() <= 14.0)
        .expect("the actual named keyboard remains mounted");
    // The board may have a tighter clip than its pinned mode/test controls.
    // Resolve the containing canvas from an actual mode control, not its title.
    texts
        .iter()
        .find(|t| t.text == "Preview" && t.clip.contains_rect(key.rect))
        .map(|t| t.clip)
        .unwrap_or(key.clip)
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
            "Workspace draft",
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
        let heading = visible_in(
            &frame,
            &selected.name,
            Some(
                frame
                    .texts()
                    .iter()
                    .find(|t| t.text == "Layer actions")
                    .unwrap()
                    .clip,
            ),
        );
        let canvas_top = key(&frame, "Esc");
        let canvas_bottom = key(&frame, "→");
        assert!(canvas_bottom.bottom() > canvas_top.bottom());
        if size == MINIMUM_SIZE {
            visible(&frame, "Layer:");
        } else {
            let layers = visible(&frame, "Layers");
            assert!(layers.left() < canvas_top.left());
        }
        if size == WIDE_SIZE {
            assert!(
                heading.left() > canvas_bottom.right(),
                "wide inspector alongside keyboard"
            );
        } else {
            assert!(
                heading.top() > canvas_bottom.bottom(),
                "normal and compact inspector below keyboard"
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
        click_workspace_control(&mut harness, &mut app, "Edit keys");
        click_workspace_control(&mut harness, &mut app, "Edit condition");
        click_workspace_control(&mut harness, &mut app, "Layer mixing & transitions");
        let expanded = harness.settle(|ctx| app.show_lighting(ctx));
        let pane = expanded
            .texts()
            .into_iter()
            .find(|t| t.text == "Layer actions")
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
        let inspector_label = "Where";
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
            app.setup_editor
                .apply(underglow::setup_edits::LayerEdit::Update {
                    id: format!("copy-{i}"),
                    patch: underglow::setup_edits::LayerPatch {
                        name: Some(format!("Ambient copy {i:02}")),
                        ..Default::default()
                    },
                })
                .unwrap();
        }
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let esc = key(&frame, "Esc");
        let inspector = visible(&frame, "Looks");
        let doc = app.setup_editor.draft.as_ref().unwrap();
        visible(&frame, &doc.layers.last().unwrap().name);
        let list_pos = visible(&frame, "Topmost first").center() + egui::vec2(0.0, 55.0);
        for _ in 0..16 {
            harness.wheel(list_pos, egui::vec2(0.0, -500.0), |ctx| {
                app.show_lighting(ctx)
            });
        }
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        assert_eq!(key(&frame, "Esc"), esc);
        assert_eq!(visible(&frame, "Looks"), inspector);
        let bottom = original.name.clone();
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
        let inspector = visible(&frame, "Layer actions");
        let pane = frame
            .texts()
            .into_iter()
            .find(|t| t.text == "Layer actions")
            .unwrap()
            .clip;
        visible_in(&frame, "Where", Some(pane));
        // Cards scroll as a single form in compact windows; keep the actual
        // keyboard and layer context pinned while revealing the color control.
        click_workspace_control(&mut harness, &mut app, "Color");
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        visible_in(&frame, "Solid", Some(pane));
        visible_in(&frame, "Color", Some(pane));
        if size == MINIMUM_SIZE {
            assert!(!frame.texts().iter().any(|t| t.text == "Layer name"
                || t.text == "Duplicate layer"
                || t.text == "Delete layer"));
            visible(&frame, "Layer actions");
        }
        let color = visible_in(&frame, "Color", Some(pane));
        assert!(color.top() > inspector.top());
        // Find the actual RGB well, not a coordinate assumed from old label order.
        let saved_color = match &document.layers[1].renderer {
            Renderer::Solid { color } => *color,
            _ => unreachable!(),
        };
        let well = frame
            .output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if rect.fill
                        == egui::Color32::from_rgb(
                            saved_color[0],
                            saved_color[1],
                            saved_color[2],
                        )
                        && rect.rect.width() >= 12.0
                        && rect.rect.width() <= 80.0
                        && pane.contains_rect(rect.rect)
                        && (rect.rect.center().y - color.center().y).abs() < 16.0 =>
                {
                    Some(rect.rect.center())
                }
                _ => None,
            })
            .expect("a real visible color well in the Color row");
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
            click_workspace_control(&mut harness, &mut app, "Looks");
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
fn workspace_connection_notice_is_reachable_in_real_scroll_content() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE] {
        let (mut app, incoming) = app();
        app.setup_editor.selected = Some(
            app.setup_editor.draft.as_ref().unwrap().layers[1]
                .id
                .clone(),
        );
        let mut harness = Harness::new(size);
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let message = app.setup_editor.connection_notices[0]
            .message
            .split('.')
            .next()
            .unwrap()
            .to_owned();
        let pane = frame
            .texts()
            .into_iter()
            .find(|t| t.text == "Layer actions")
            .unwrap()
            .clip;
        // In a short viewport the note can lie above the very bottom offset.
        // Reveal its actual text through bounded scrolling, not a max-offset assumption.
        let draft = app.setup_editor.draft.clone();
        click_workspace_control(&mut harness, &mut app, &message);
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        assert!(
            frame
                .texts()
                .iter()
                .any(|t| t.text == message && t.fully_visible() && pane.contains(t.rect.center())),
            "a real connection note is readable through scrolling, not only a hover tooltip"
        );
        assert_eq!(app.setup_editor.draft, draft);
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn workspace_preview_badge_explains_source_and_playback_without_open_tools() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE, WIDE_SIZE] {
        let mut harness = Harness::new(size);
        let (mut app, incoming) = app();
        let draft = app.setup_editor.draft.clone();
        for (mode, playing, context, playback) in [
            (setup_preview::Mode::Live, false, "Live data", None),
            (setup_preview::Mode::InputDemo, false, "Input demo", None),
            (
                setup_preview::Mode::Scenario,
                false,
                "Advanced scenario",
                Some("paused"),
            ),
            (
                setup_preview::Mode::Scenario,
                true,
                "Advanced scenario",
                Some("playing"),
            ),
        ] {
            app.setup_editor.preview.set_mode(mode);
            app.setup_editor.preview.set_scenario_playing(playing);
            let frame = harness.settle(|ctx| app.show_lighting(ctx));
            visible(&frame, "Preview only");
            visible(&frame, context);
            if let Some(playback) = playback {
                visible(&frame, &format!("Advanced scenario · {playback}"));
            }
            key(&frame, "Esc");
            assert_eq!(app.setup_editor.draft, draft);
            assert!(incoming.try_recv().is_err());
        }
    }
}

#[test]
fn inspector_keeps_editing_choice_separate_from_preview_input_at_real_sizes() {
    use underglow::setup_library::{Starter, starter};
    for size in [MINIMUM_SIZE, DEFAULT_SIZE, WIDE_SIZE] {
        let (mut app, incoming) = app();
        let document = starter(Starter::MatrixPi, "concepts".into(), "Concepts".into()).unwrap();
        app.setup_editor = setup_editor::Editor::default();
        app.setup_editor.load(Some(document.clone()));
        let mut harness = Harness::new(size);
        let first = harness.settle(|ctx| app.show_lighting(ctx));
        visible(&first, "Concepts");
        visible(&first, "Function zone");
        let keyboard = key(&first, "Esc");
        for section in ["Where", "Looks", "When"] {
            click_workspace_control(&mut harness, &mut app, section);
        }
        click_workspace_control(&mut harness, &mut app, "Idle / fallback");
        click_workspace_control(&mut harness, &mut app, "Working");
        click_workspace_control(&mut harness, &mut app, "Shown while pi is working");
        assert_eq!(app.setup_editor.draft.as_ref(), Some(&document));
        assert!(app.setup_editor.preview.mode == setup_preview::Mode::Live);
        assert!(!app.setup_editor.preview.quick_active());
        assert!(app.setup_editor.preview.sample().is_none());

        let editing = harness.settle(|ctx| app.show_lighting(ctx));
        harness.click("Test behavior…", Some(keyboard_region(&editing)), |ctx| {
            app.show_lighting(ctx)
        });
        harness.click("Finished", None, |ctx| app.show_lighting(ctx));
        let testing = harness.settle(|ctx| app.show_lighting(ctx));
        assert!(
            testing
                .texts()
                .iter()
                .any(|t| t.text.contains("Testing: Finished") && t.fully_visible())
        );
        assert_eq!(
            app.setup_editor
                .preview
                .runtime()
                .unwrap()
                .selected_appearance("pi-status")
                .unwrap()
                .variant_id,
            Some("finished")
        );
        click_workspace_control(&mut harness, &mut app, "Editing");
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let pane = frame
            .texts()
            .iter()
            .find(|t| t.text == "Layer actions")
            .unwrap()
            .clip;
        visible_in(&frame, "Working", Some(pane));
        assert_eq!(app.setup_editor.draft.as_ref(), Some(&document));
        assert_eq!(key(&frame, "Esc"), keyboard);
        assert!(app.setup_editor.preview.sample().is_none());
        assert!(incoming.try_recv().is_err());

        let mut other = document.clone();
        other.id = "another-concepts-setup".into();
        app.setup_editor.load(Some(other.clone()));
        click_workspace_control(&mut harness, &mut app, "Editing");
        let reopened = harness.settle(|ctx| app.show_lighting(ctx));
        visible_in(&reopened, "Idle / fallback", Some(pane));
        assert_eq!(app.setup_editor.draft.as_ref(), Some(&other));
    }
}

#[test]
fn inspector_retains_independent_editors_without_an_extra_navigation_index() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE, WIDE_SIZE] {
        let (mut app, incoming) = app();
        let mut harness = Harness::new(size);
        let original = app.setup_editor.draft.clone();
        let id = app.setup_editor.selected.clone().unwrap();
        let first = harness.settle(|ctx| app.show_lighting(ctx));
        let save = visible(&first, "Save");
        let keyboard = key(&first, "Esc");
        click_workspace_control(&mut harness, &mut app, "Edit keys");
        click_workspace_control(&mut harness, &mut app, "Edit condition");
        assert_eq!(
            app.setup_editor.inspector_sections_for_test(&id),
            (true, true)
        );
        // The condition fields are direct, not another hidden Advanced condition.
        let activation = &original
            .as_ref()
            .unwrap()
            .layers
            .iter()
            .find(|layer| layer.id == id)
            .unwrap()
            .activation;
        let mode = match activation {
            underglow::setup::Activation::Always {} => "Always",
            underglow::setup::Activation::WhileState { .. } => "While a source matches",
            underglow::setup::Activation::OnEvent { .. } => "When an event occurs",
            underglow::setup::Activation::OnTransition { .. } => "When a state changes",
        };
        click_workspace_control(&mut harness, &mut app, mode);
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        assert!(!frame.texts().iter().any(|t| {
            [
                "Advanced condition",
                "Edit look",
                "Edit behavior",
                "Close behavior",
                "Done choosing keys",
            ]
            .contains(&t.text.as_str())
        }));
        // Close the mode picker without making a model change.
        harness.click("Keyboard", None, |ctx| app.show_lighting(ctx));
        for section in ["Where", "Looks", "When"] {
            click_workspace_control(&mut harness, &mut app, section);
        }
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        assert_eq!(visible(&frame, "Save"), save);
        assert_eq!(key(&frame, "Esc"), keyboard);
        assert_eq!(app.setup_editor.draft, original);
        assert!(incoming.try_recv().is_err());
    }
}
