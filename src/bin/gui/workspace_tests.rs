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
        let region = if label == "F1" { inspector } else { None };
        if texts.iter().any(|t| {
            t.text == label
                && t.fully_visible()
                && region.is_none_or(|r| r.contains(t.rect.center()))
        }) {
            // Finish smooth scrolling on the harness clock before taking the click position.
            for _ in 0..30 {
                harness.run(vec![], |ctx| app.show_lighting(ctx));
            }
            harness.click(label, region, |ctx| app.show_lighting(ctx));
            return;
        }
        let pane = tools.or(inspector).expect("a scrollable workspace pane");
        harness.wheel(
            pane.center(),
            egui::vec2(0.0, if step == 34 { 20000.0 } else { -220.0 }),
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
            "Setup: Workspace draft ▾",
            "Unsaved changes",
            "Resume lighting",
            "Save composition",
            "Add layer",
            "Preview tools",
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
        harness.click("Save composition", None, |ctx| app.show_lighting(ctx));
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
        let draft = app.setup_editor.draft.clone();
        let selected = app.setup_editor.selected.clone();
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let save = visible(&frame, "Save composition");
        let esc = key(&frame, "Esc");
        let arrow = key(&frame, "→");
        let inspector = visible(&frame, "Layer name");
        for _ in 0..8 {
            harness.wheel(inspector.center(), egui::vec2(0.0, -400.0), |ctx| {
                app.show_lighting(ctx)
            });
        }
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        assert_eq!(visible(&frame, "Save composition"), save);
        assert_eq!(key(&frame, "Esc"), esc);
        assert_eq!(key(&frame, "→"), arrow);
        assert!(
            !frame
                .texts()
                .iter()
                .any(|t| t.text == "Layer name" && t.fully_visible()),
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
        visible(&frame, "Save composition");
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
