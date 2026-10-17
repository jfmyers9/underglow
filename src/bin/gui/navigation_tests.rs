//! Exercise the production navigation shell without native windows or workers.
use super::tests::{controller_fixture, setup_document};
use super::*;

fn frame(
    ctx: &egui::Context,
    app: &mut Controller,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| app.show_lighting(ctx),
    )
}

fn text_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == label => {
            Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
        }
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, label)),
        _ => None,
    }
}

fn visible_rect(output: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
    output.shapes.iter().rev().find_map(|shape| {
        text_rect(&shape.shape, label).filter(|rect| shape.clip_rect.contains_rect(*rect))
    })
}

#[track_caller]
fn click(ctx: &egui::Context, app: &mut Controller, size: egui::Vec2, label: &str) {
    for _ in 0..20 {
        let _ = frame(ctx, app, size, vec![]);
        let output = frame(ctx, app, size, vec![]);
        let picker_name = app.setup_library.open
            && app
                .setup_editor
                .applied
                .as_ref()
                .is_some_and(|doc| doc.name == label);
        let rect = if picker_name {
            let below = visible_rect(&output, "New draft:").map(|rect| rect.bottom());
            let name_visible = visible_rect(&output, "Name for draft / copy").is_some();
            output.shapes.iter().rev().find_map(|shape| {
                text_rect(&shape.shape, label).filter(|rect| {
                    shape.clip_rect.contains_rect(*rect)
                        && below.map_or(!name_visible, |bottom| rect.top() > bottom)
                })
            })
        } else {
            visible_rect(&output, label)
        };
        if let Some(rect) = rect {
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
            for pressed in [true, false] {
                frame(
                    ctx,
                    app,
                    size,
                    vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            return;
        }
        assert!(
            app.setup_library.open,
            "missing visible {label} at {size:?}"
        );
        let pane = output
            .shapes
            .iter()
            .find(|shape| text_rect(&shape.shape, "Close setup browser").is_some())
            .expect("setup browser")
            .clip_rect;
        frame(
            ctx,
            app,
            size,
            vec![
                egui::Event::PointerMoved(pane.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -160.0),
                    modifiers: Default::default(),
                },
            ],
        );
        for _ in 0..30 {
            frame(ctx, app, size, vec![]);
        }
    }
    panic!("unreachable {label} in setup browser at {size:?}");
}

#[test]
fn navigation_browser_preserves_dirty_draft_selection_and_preview_at_real_window_sizes() {
    for size in [egui::vec2(620.0, 640.0), egui::vec2(940.0, 850.0)] {
        let ctx = egui::Context::default();
        configure_style(&ctx);
        let (mut app, incoming) = controller_fixture();
        app.status = Some(
            serde_json::from_value(serde_json::json!({
                "schema_version": 1, "state": "paused", "enabled": false,
                "mode": "matrix", "brightness": 128, "palette": "terminal",
                "fps": 30, "retry_attempt": 0
            }))
            .unwrap(),
        );
        let mut document = setup_document();
        document.name = "Navigation setup".into();
        let id = document.layers[0].id.clone();
        app.setup_editor.load(Some(document.clone()));
        app.setup_editor.selected = Some(id.clone());
        app.setup_editor
            .name_draft("Navigation draft".into())
            .unwrap();
        app.setup_editor.preview.mode = setup_preview::Mode::InputDemo;
        app.setup_library
            .receive(setup_library_panel::Report {
                schema_version: 1,
                revision: 7,
                current_id: Some(document.id.clone()),
                setups: vec![
                    underglow::setup_library::SetupSummary {
                        id: document.id.clone(),
                        name: document.name.clone(),
                        layers: document.layers.len(),
                    },
                    underglow::setup_library::SetupSummary {
                        id: "other".into(),
                        name: "Other setup".into(),
                        layers: 1,
                    },
                ],
            })
            .unwrap();
        let draft = app.setup_editor.draft.clone();
        let applied = app.setup_editor.applied.clone();
        let revision = app.setup_editor.base_revision;
        let _ = frame(&ctx, &mut app, size, vec![]);
        let output = frame(&ctx, &mut app, size, vec![]);
        for label in [
            "Editor",
            "Connections",
            "Settings",
            "Diagnostics",
            "Browse setups: Navigation draft",
            "Unsaved changes",
            "Keyboard lighting",
            "Resume lighting",
        ] {
            let rect = visible_rect(&output, label)
                .unwrap_or_else(|| panic!("missing {label} at {size:?}"));
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
        }
        assert!(visible_rect(&output, "Saved setups").is_none());
        assert!(visible_rect(&output, "Connector diagnostics").is_none());

        click(&ctx, &mut app, size, "Browse setups: Navigation draft");
        assert!(app.setup_library.open);
        assert!(app.page == Page::Editor);
        let output = frame(&ctx, &mut app, size, vec![]);
        assert!(visible_rect(&output, "Setup browser").is_some());
        click(&ctx, &mut app, size, &document.name);
        click(&ctx, &mut app, size, "Other setup · 1 layers");
        // A saved-item selection is browsing, not an implicit Switch.
        assert!(incoming.try_recv().is_err());
        click(&ctx, &mut app, size, "Open saved setup");
        click(&ctx, &mut app, size, "Cancel");
        click(&ctx, &mut app, size, "Blank");
        click(&ctx, &mut app, size, "Cancel");
        assert!(
            incoming.try_recv().is_err(),
            "dirty replacement actions stay blocked"
        );
        click(&ctx, &mut app, size, "Close setup browser");
        assert!(!app.setup_library.open);

        click(&ctx, &mut app, size, "Connections");
        assert!(app.page == Page::Connections);
        assert!(matches!(incoming.try_recv(), Ok(Action::ConnectionsShow)));
        app.pending = false; // complete the read-only fixture request
        click(&ctx, &mut app, size, "Browse setups: Navigation draft");
        assert!(app.setup_library.open);
        assert!(
            app.page == Page::Connections,
            "browser does not navigate to Editor"
        );
        click(&ctx, &mut app, size, "Close setup browser");
        click(&ctx, &mut app, size, "Editor");
        click(&ctx, &mut app, size, "Editor");
        assert!(app.page == Page::Editor);
        assert_eq!(app.setup_editor.draft, draft);
        assert_eq!(app.setup_editor.applied, applied);
        assert_eq!(app.setup_editor.base_revision, revision);
        assert_eq!(app.setup_editor.selected.as_deref(), Some(id.as_str()));
        assert!(app.setup_editor.dirty);
        assert!(app.setup_editor.preview.mode == setup_preview::Mode::InputDemo);
        assert!(!app.status.as_ref().unwrap().enabled);
        assert!(
            incoming.try_recv().is_err(),
            "navigation never saves or activates lighting"
        );
    }
}

#[test]
fn navigation_header_separates_setup_save_and_output_states_with_long_names() {
    for size in [egui::vec2(620.0, 640.0), egui::vec2(940.0, 850.0)] {
        for (connected, enabled, state, label) in [
            (true, true, "active", "Pause lighting"),
            (true, false, "paused", "Resume lighting"),
            (false, false, "paused", "Start engine"),
        ] {
            let ctx = egui::Context::default();
            configure_style(&ctx);
            let (mut app, incoming) = controller_fixture();
            app.connected = connected;
            app.status = Some(
                serde_json::from_value(serde_json::json!({
                    "schema_version": 1, "state": state, "enabled": enabled,
                    "mode": "matrix", "brightness": 128, "palette": "terminal",
                    "fps": 30, "retry_attempt": 0
                }))
                .unwrap(),
            );
            let mut document = setup_document();
            document.name = "A deliberately long setup title with many words to exercise truncation rather than pushing navigation off screen".into();
            app.setup_editor.load(Some(document));
            for dirty in [false, true] {
                app.setup_editor.dirty = dirty;
                let _ = frame(&ctx, &mut app, size, vec![]);
                let output = frame(&ctx, &mut app, size, vec![]);
                for text in [
                    "Editor",
                    "Connections",
                    "Settings",
                    "Diagnostics",
                    "Keyboard lighting",
                    label,
                    if dirty { "Unsaved changes" } else { "Saved" },
                ] {
                    let rect =
                        visible_rect(&output, text).unwrap_or_else(|| panic!("missing {text}"));
                    assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
                }
                assert!(incoming.try_recv().is_err());
            }
        }
    }
}

#[test]
fn navigation_secondary_windows_are_explicit_and_do_not_dispatch() {
    for size in [egui::vec2(620.0, 640.0), egui::vec2(940.0, 850.0)] {
        let ctx = egui::Context::default();
        configure_style(&ctx);
        let (mut app, incoming) = controller_fixture();
        click(&ctx, &mut app, size, "Diagnostics");
        assert!(app.diagnostics_open);
        assert!(app.page == Page::Editor);
        let output = frame(&ctx, &mut app, size, vec![]);
        assert!(visible_rect(&output, "Connector diagnostics").is_some());
        click(&ctx, &mut app, size, "Close diagnostics");
        assert!(!app.diagnostics_open);
        click(&ctx, &mut app, size, "Settings");
        assert!(app.settings_open);
        assert!(app.page == Page::Editor);
        assert!(incoming.try_recv().is_err());
    }
}
