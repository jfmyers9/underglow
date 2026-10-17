//! Actual branded workspace, with no native windows or producer workers.
use super::*;
use crate::tests::controller_fixture;
use crate::workspace_test_ui::{DEFAULT_SIZE, Frame, Harness, MINIMUM_SIZE};

fn app() -> (
    crate::Controller,
    std::sync::mpsc::Receiver<crate::Action>,
    SetupDocument,
) {
    let (mut app, incoming) = controller_fixture();
    app.status = Some(
        serde_json::from_value(serde_json::json!({
            "schema_version":1,"state":"paused","enabled":false,"mode":"matrix",
            "brightness":255,"palette":"terminal","fps":30,"retry_attempt":0
        }))
        .unwrap(),
    );
    let doc = underglow::setup_library::starter(
        underglow::setup_library::Starter::MatrixPi,
        "quick-ui".into(),
        "Quick UI".into(),
    )
    .unwrap();
    app.setup_editor.load(Some(doc.clone()));
    (app, incoming, doc)
}

fn canvas(frame: &Frame) -> egui::Rect {
    frame
        .texts()
        .into_iter()
        .find(|text| text.text == "Composition preview")
        .unwrap()
        .clip
}

fn visible(frame: &Frame, label: &str) -> egui::Rect {
    let region = canvas(frame);
    let matches: Vec<_> = frame
        .texts()
        .into_iter()
        .filter(|text| {
            text.text == label && region.contains_rect(text.rect) && text.fully_visible()
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "one visible {label} in the real clipped canvas"
    );
    matches[0].rect
}

#[test]
fn mounted_pi_quick_tests_are_reachable_read_only_and_keep_a_readable_stable_canvas() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, doc) = app();
        let mut harness = Harness::new(size);
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let region = canvas(&frame);
        let key = visible(&frame, "F1");
        for label in [
            "Live",
            "Try behavior",
            "Idle",
            "Working",
            "Finished",
            "Preview tools",
        ] {
            visible(&frame, label);
        }
        for (label, variant, state) in [
            ("Finished", Some("finished"), None),
            ("Idle", None, Some("idle")),
            ("Working", Some("working"), Some("working")),
        ] {
            harness.click(label, Some(region), |ctx| app.show_lighting(ctx));
            let frame = harness.settle(|ctx| app.show_lighting(ctx));
            assert!(app.setup_editor.preview.quick_active());
            assert_eq!(
                visible(&frame, "F1"),
                key,
                "testing cannot move the keyboard"
            );
            visible(&frame, "→");
            visible(&frame, "Pause test");
            visible(
                &frame,
                "Preview only · Quick test playing · no hardware output",
            );
            let inspector = frame
                .texts()
                .into_iter()
                .find(|text| text.text.starts_with("Selected layer:"))
                .unwrap();
            assert!(inspector.fully_visible());
            assert!(
                inspector.clip.height() >= 120.0,
                "testing must leave a usable inspector"
            );
            let face = frame
                .output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect)
                        if rect.fill != egui::Color32::TRANSPARENT
                            && rect.stroke.width == 0.0
                            && rect.rect.contains(key.center())
                            && (rect.rect.width() - rect.rect.height()).abs() < 0.01 =>
                    {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .expect("RGB face under the key legend");
            assert!(face.width() >= 12.0 && region.contains_rect(face));
            let appearance = app
                .setup_editor
                .preview
                .runtime()
                .unwrap()
                .selected_appearance(&doc.layers[1].id)
                .unwrap();
            assert_eq!(appearance.variant_id, variant);
            let (facts, events) = app.setup_editor.preview.evaluation_facts();
            if let Some(state) = state {
                assert_eq!(facts.states[0].state, state);
                assert!(
                    events.events.is_empty(),
                    "state choices discard outgoing quick transients"
                );
            } else {
                assert!(facts.states.is_empty());
                assert_eq!(events.events[0].event, "finished");
            }
            assert_eq!(app.setup_editor.draft.as_ref(), Some(&doc));
            assert_eq!(app.setup_editor.applied.as_ref(), Some(&doc));
            assert!(!app.setup_editor.dirty);
            assert!(!app.status.as_ref().unwrap().enabled);
            assert!(app.setup_editor.preview.sample().is_none());
            assert!(
                incoming.try_recv().is_err(),
                "quick tests have no engine/source/hardware intents"
            );
        }
        harness.click("Pause test", Some(region), |ctx| app.show_lighting(ctx));
        let frozen = app.setup_editor.preview.frame().clone();
        for _ in 0..5 {
            harness.run(vec![], |ctx| app.show_lighting(ctx));
        }
        assert_eq!(app.setup_editor.preview.frame(), &frozen);
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        visible(&frame, "Play test");
        visible(
            &frame,
            "Preview only · Quick test paused · no hardware output",
        );
        harness.click("Live", Some(region), |ctx| app.show_lighting(ctx));
        assert!(!app.setup_editor.preview.quick_active());
        assert!(app.setup_editor.preview.mode == Mode::Live);
        assert!(
            app.setup_editor
                .preview
                .evaluation_facts()
                .1
                .events
                .is_empty()
        );
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn inspect_click_reveals_the_canonical_report_before_advanced_controls() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, doc) = app();
        let mut harness = Harness::new(size);
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let region = canvas(&frame);
        harness.click("Working", Some(region), |ctx| app.show_lighting(ctx));
        harness.click("Inspect", Some(region), |ctx| app.show_lighting(ctx));
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        harness.click_at(visible(&frame, "F1").center(), |ctx| app.show_lighting(ctx));
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let texts = frame.texts();
        let explanation = texts
            .iter()
            .find(|text| text.text.starts_with("Why F1?"))
            .expect("click opens the report without finding a secondary tool");
        assert!(explanation.fully_visible());
        if let Some(advanced) = texts
            .iter()
            .find(|text| text.text == "Composition preview:")
        {
            assert!(explanation.rect.top() < advanced.rect.top());
        }
        let coord = underglow::key_targets::canonical_coord("F1").unwrap();
        let report = crate::setup_explanation::report(&app.setup_editor.preview, coord).unwrap();
        assert_eq!(
            report.color,
            app.setup_editor
                .preview
                .coverage()
                .unwrap()
                .frame
                .get_coord(coord)
        );
        assert_eq!(app.setup_editor.draft.as_ref(), Some(&doc));
        assert!(!app.setup_editor.dirty);
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn many_custom_choices_use_a_contextual_picker_instead_of_a_toolbar_wall() {
    let (mut app, incoming, mut doc) = app();
    let mut descriptor = underglow::source_capabilities::pi_descriptor();
    descriptor.source = "custom".into();
    descriptor.display_name = "Custom".into();
    descriptor.states = (0..6)
        .map(|index| underglow::source_capabilities::FactDescriptor {
            id: format!("state-{index}"),
            label: format!("State {index}"),
        })
        .collect();
    descriptor.events.clear();
    let mut registry = CapabilityRegistry::new();
    registry.insert(descriptor).unwrap();
    doc.layers[1].activation = Activation::WhileState {
        source: "custom".into(),
        instance: Some("exact".into()),
        state: None,
    };
    doc.layers[1].variants.clear();
    app.setup_editor.load(Some(doc.clone()));
    app.setup_editor.set_capabilities(registry);
    let mut harness = Harness::new(MINIMUM_SIZE);
    let frame = harness.settle(|ctx| app.show_lighting(ctx));
    let region = canvas(&frame);
    visible(&frame, "State 0");
    visible(&frame, "Try");
    assert!(
        !frame
            .texts()
            .iter()
            .any(|text| text.text == "State 5" && region.contains_rect(text.rect))
    );
    harness.click("State 0", Some(region), |ctx| app.show_lighting(ctx));
    harness.click("Custom · State 3", None, |ctx| app.show_lighting(ctx));
    harness.click("Try", Some(region), |ctx| app.show_lighting(ctx));
    let facts = app.setup_editor.preview.evaluation_facts().0;
    assert_eq!(facts.states[0].identity.source, "custom");
    assert_eq!(facts.states[0].identity.instance, "exact");
    assert_eq!(facts.states[0].state, "state-3");
    assert_eq!(app.setup_editor.draft.as_ref(), Some(&doc));
    assert!(incoming.try_recv().is_err());
}
