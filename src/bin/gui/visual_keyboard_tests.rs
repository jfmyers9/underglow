//! Keyboard-only editor journeys. No native window, worker, SDK or source publisher.
use super::tests::controller_fixture;
use super::workspace_test_ui::{DEFAULT_SIZE, Frame, Harness, MINIMUM_SIZE, WIDE_SIZE};
use super::*;

fn app() -> (Controller, Receiver<Action>) {
    let (mut app, incoming) = controller_fixture();
    app.status = Some(
        serde_json::from_value(serde_json::json!({
            "schema_version":1, "state":"paused", "enabled":false,
            "mode":"matrix", "brightness":255, "palette":"terminal",
            "fps":30, "retry_attempt":0
        }))
        .unwrap(),
    );
    let mut document = underglow::setup_library::starter(
        underglow::setup_library::Starter::MatrixPi,
        "keyboard-test".into(),
        "Keyboard setup".into(),
    )
    .unwrap();
    document.revision = 17;
    app.setup_editor.load(Some(document));
    app.setup_editor
        .name_draft("Keyboard draft".into())
        .unwrap();
    // Freeze the private preview clock; keyboard focus is not test input.
    app.setup_editor
        .preview
        .set_mode(setup_preview::Mode::Scenario);
    (app, incoming)
}

fn key(harness: &mut Harness, app: &mut Controller, key: egui::Key, shift: bool) -> Frame {
    let modifiers = egui::Modifiers {
        shift,
        ..Default::default()
    };
    for pressed in [true, false] {
        harness.run(
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            }],
            |ctx| app.show_lighting(ctx),
        );
    }
    // Reverse focus is applied next frame in egui 0.31. Also allow genuine
    // focus-following scroll animations to settle without synthesizing a wheel.
    for _ in 0..20 {
        harness.run(vec![], |ctx| app.show_lighting(ctx));
    }
    harness.settle(|ctx| app.show_lighting(ctx))
}

#[track_caller]
fn focused(harness: &Harness, frame: &Frame) -> egui::Response {
    let id = harness
        .context
        .memory(|memory| memory.focused())
        .expect("Tab assigns actual widget focus");
    let response = harness
        .context
        .read_response(id)
        .expect("focused widget remains mounted");
    assert!(response.has_focus());
    assert!(response.enabled());
    assert!(response.sense.is_focusable());
    let labels: Vec<_> = frame
        .texts()
        .into_iter()
        .filter(|text| response.rect.intersects(text.rect))
        .map(|text| text.text)
        .collect();
    assert!(
        frame.screen.contains_rect(response.rect),
        "focused widget {id:?} leaves the viewport: {:?}",
        response.rect
    );
    assert!(
        response.interact_rect.contains_rect(response.rect),
        "focused widget {id:?} {labels:?} is clipped at {:?}: rect={:?}, real interaction clip={:?}",
        frame.screen.size(),
        response.rect,
        response.interact_rect
    );
    response
}

fn is_save(frame: &Frame, response: &egui::Response) -> bool {
    frame.texts().iter().any(|text| {
        text.text == "Save" && text.fully_visible() && response.rect.contains_rect(text.rect)
    })
}

fn has_focus_outline(shape: &egui::Shape, clip: egui::Rect, rect: egui::Rect) -> bool {
    match shape {
        egui::Shape::Rect(painted) => {
            painted.rect == rect
                && painted.stroke.width >= 2.0
                && painted.stroke.color == brand::ACCENT
                && clip.contains_rect(painted.rect)
        }
        egui::Shape::Vec(shapes) => shapes
            .iter()
            .any(|shape| has_focus_outline(shape, clip, rect)),
        _ => false,
    }
}

#[test]
fn tab_and_reverse_tab_keep_editor_focus_inside_real_visible_clips() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE, WIDE_SIZE] {
        let (mut app, incoming) = app();
        let mut harness = Harness::new(size);
        harness.settle(|ctx| app.show_lighting(ctx));
        let draft = app.setup_editor.draft.clone();
        let applied = app.setup_editor.applied.clone();
        let selected = app.setup_editor.selected.clone();
        let preview = app.setup_editor.preview.frame().clone();
        let mut order = Vec::new();
        let mut outlined = false;
        let mut complete = false;
        for _ in 0..128 {
            let frame = key(&mut harness, &mut app, egui::Key::Tab, false);
            let response = focused(&harness, &frame);
            outlined |= frame
                .output
                .shapes
                .iter()
                .any(|shape| has_focus_outline(&shape.shape, shape.clip_rect, response.rect));
            if order.first() == Some(&response.id) {
                complete = true;
                break;
            }
            assert!(
                !order.contains(&response.id),
                "Tab cannot become trapped in a partial cycle"
            );
            order.push(response.id);
            assert_eq!(app.setup_editor.draft, draft);
            assert_eq!(app.setup_editor.applied, applied);
            assert_eq!(app.setup_editor.selected, selected);
            assert_eq!(app.setup_editor.preview.frame(), &preview);
            assert!(app.setup_editor.dirty);
            assert!(
                incoming.try_recv().is_err(),
                "focus navigation emits no engine/source actions"
            );
        }
        assert!(
            complete && order.len() >= 12,
            "exercise toolbar, workspace and inspector at {size:?}"
        );
        assert!(
            outlined,
            "keyboard focus has an actual visible accent outline"
        );
        for expected in order.iter().rev() {
            let frame = key(&mut harness, &mut app, egui::Key::Tab, true);
            assert_eq!(
                focused(&harness, &frame).id,
                *expected,
                "Shift-Tab reverses real widget order"
            );
            assert_eq!(app.setup_editor.draft, draft);
            assert_eq!(app.setup_editor.preview.frame(), &preview);
            assert!(incoming.try_recv().is_err());
        }
    }
}

#[test]
fn keyboard_save_sends_only_the_exact_paused_draft_cas_request() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE, WIDE_SIZE] {
        for activation in [egui::Key::Enter, egui::Key::Space] {
            let (mut app, incoming) = app();
            let mut harness = Harness::new(size);
            harness.settle(|ctx| app.show_lighting(ctx));
            let draft = app.setup_editor.draft.clone().unwrap();
            let applied = app.setup_editor.applied.clone();
            let selected = app.setup_editor.selected.clone();
            let preview = app.setup_editor.preview.frame().clone();
            let revision = app.setup_editor.base_revision;
            let mut reached_save = false;
            for _ in 0..32 {
                let frame = key(&mut harness, &mut app, egui::Key::Tab, false);
                let response = focused(&harness, &frame);
                assert_eq!(app.setup_editor.draft.as_ref(), Some(&draft));
                assert_eq!(app.setup_editor.applied, applied);
                assert_eq!(app.setup_editor.selected, selected);
                assert_eq!(app.setup_editor.preview.frame(), &preview);
                assert!(incoming.try_recv().is_err());
                if is_save(&frame, &response) {
                    // Exercise backward navigation too, without directly
                    // requesting focus or pointing at the Save button.
                    let previous = key(&mut harness, &mut app, egui::Key::Tab, true);
                    assert_ne!(focused(&harness, &previous).id, response.id);
                    let returned = key(&mut harness, &mut app, egui::Key::Tab, false);
                    assert_eq!(focused(&harness, &returned).id, response.id);
                    assert!(is_save(&returned, &response));
                    reached_save = true;
                    break;
                }
            }
            assert!(reached_save, "Save is in real keyboard order at {size:?}");
            key(&mut harness, &mut app, activation, false);
            match incoming
                .try_recv()
                .expect("keyboard activation dispatches Save")
            {
                Action::SetupReplace {
                    document,
                    revision: sent_revision,
                } => {
                    assert_eq!(document, draft);
                    assert_eq!(sent_revision, revision);
                }
                action => panic!("keyboard Save emitted unexpected {action:?}"),
            }
            assert!(
                incoming.try_recv().is_err(),
                "no Resume, producer, polling or duplicate Save action"
            );
            assert!(!app.status.as_ref().unwrap().enabled);
            assert_eq!(app.setup_editor.draft.as_ref(), Some(&draft));
            assert_eq!(app.setup_editor.applied, applied);
            assert_eq!(app.setup_editor.preview.frame(), &preview);
            assert!(
                app.setup_editor.dirty,
                "only an acknowledged Save may clean the draft"
            );
        }
    }
}
