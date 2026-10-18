//! Whole editing journeys through the production shell, without workers/SDKs.
use super::*;
use crate::tests::controller_fixture;
use crate::workspace_test_ui::{DEFAULT_SIZE, Harness, MINIMUM_SIZE, WIDE_SIZE};
use crate::workspace_tests::{choose_private_test, click_workspace_control};
use underglow::setup::{SetupDocument, Target};

fn paused() -> serde_json::Value {
    serde_json::json!({"schema_version":1,"state":"paused","enabled":false,
        "mode":"matrix","brightness":255,"palette":"terminal","fps":30,"retry_attempt":0})
}

fn save_and_acknowledge(
    harness: &mut Harness,
    app: &mut Controller,
    incoming: &Receiver<Action>,
    outgoing: &Sender<Completion>,
) -> SetupDocument {
    let submitted = app.setup_editor.draft.clone().unwrap();
    let revision = app.setup_editor.base_revision;
    harness.click("Save", None, |ctx| app.show_lighting(ctx));
    let action = incoming.try_recv().expect("an explicit Save action");
    assert!(
        matches!(&action, Action::SetupReplace {revision: expected, document}
        if *expected == revision && document == &submitted)
    );
    let mut saved = submitted;
    saved.revision = revision + 1;
    outgoing
        .send(Completion {
            action,
            result: Ok(serde_json::json!({"ok":true,"status":paused(),
                "setup":{"revision":saved.revision,"document":saved},
                "library":{"schema_version":1,"revision":saved.revision,"current_id":saved.id,
                    "setups":[{"id":saved.id,"name":saved.name,"layers":saved.layers.len()}]}
            })
            .to_string()),
        })
        .unwrap();
    app.receive();
    assert_eq!(app.setup_editor.applied.as_ref(), Some(&saved));
    assert_eq!(app.setup_editor.draft.as_ref(), Some(&saved));
    assert!(!app.setup_editor.dirty);
    assert!(!app.status.as_ref().unwrap().enabled);
    assert!(
        incoming.try_recv().is_err(),
        "Save cannot resume or start anything"
    );
    saved
}

#[test]
fn complete_matrix_pi_edit_test_save_and_delete_journey_at_real_sizes() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE, WIDE_SIZE] {
        let mut harness = Harness::new(size);
        let (mut app, incoming) = controller_fixture();
        let (outgoing, replies) = mpsc::channel();
        app.replies = replies;
        app.status = Some(serde_json::from_value(paused()).unwrap());
        let blank = underglow::setup_library::starter(
            underglow::setup_library::Starter::Blank,
            "journey".into(),
            "Journey".into(),
        )
        .unwrap();
        app.setup_editor.load(Some(blank));

        click_workspace_control(&mut harness, &mut app, "Add layer");
        harness.click("Matrix layer", None, |ctx| app.show_lighting(ctx));
        let matrix = app.setup_editor.draft.as_ref().unwrap().layers[0].clone();
        click_workspace_control(&mut harness, &mut app, "Add layer");
        harness.click("Pi status layer", None, |ctx| app.show_lighting(ctx));
        let pi = app.setup_editor.draft.as_ref().unwrap().layers[1].clone();
        assert_eq!(
            pi.variants.len(),
            2,
            "Pi is one logical layer, not three overlays"
        );

        click_workspace_control(&mut harness, &mut app, "Edit keys");
        click_workspace_control(&mut harness, &mut app, "F1");
        click_workspace_control(&mut harness, &mut app, "Use selected keys only");
        assert_eq!(
            app.setup_editor.draft.as_ref().unwrap().layers[1].target,
            Target::Selection {
                keys: vec!["F1".into()],
                zones: vec![],
                groups: vec![]
            }
        );
        assert_eq!(app.setup_editor.draft.as_ref().unwrap().layers[0], matrix);

        click_workspace_control(&mut harness, &mut app, "Looks");
        click_workspace_control(&mut harness, &mut app, "Idle / fallback");
        click_workspace_control(&mut harness, &mut app, "Working");
        click_workspace_control(&mut harness, &mut app, "Use layer brightness");
        assert!(
            app.setup_editor.draft.as_ref().unwrap().layers[1].variants[0]
                .brightness
                .is_some()
        );
        let edited = app.setup_editor.draft.clone();
        for (label, expected) in [
            ("Finished", Some("finished")),
            ("Idle", None),
            ("Working", Some("working")),
        ] {
            choose_private_test(&mut harness, &mut app, label);
            harness.settle(|ctx| app.show_lighting(ctx));
            assert_eq!(
                app.setup_editor
                    .preview
                    .runtime()
                    .unwrap()
                    .selected_appearance(&pi.id)
                    .unwrap()
                    .variant_id,
                expected
            );
            assert_eq!(
                app.setup_editor.draft, edited,
                "test controls never edit the setup"
            );
            assert!(
                app.setup_editor.preview.sample().is_none(),
                "no live sample is fabricated"
            );
            assert!(
                incoming.try_recv().is_err(),
                "editing/testing has no engine side effect"
            );
        }

        let saved = save_and_acknowledge(&mut harness, &mut app, &incoming, &outgoing);
        assert_eq!(saved.layers[0], matrix);
        click_workspace_control(&mut harness, &mut app, "Delete layer");
        assert_eq!(
            app.setup_editor.draft.as_ref().unwrap().layers,
            vec![matrix.clone()]
        );
        let saved = save_and_acknowledge(&mut harness, &mut app, &incoming, &outgoing);
        let document = SetupDocument::from_toml(&saved.to_toml().unwrap()).unwrap();
        let mut restarted = setup_editor::Editor::default();
        restarted.load(Some(document));
        assert_eq!(restarted.draft.as_ref().unwrap().layers, vec![matrix]);
        assert!(!restarted.dirty, "a reopened document does not reinsert Pi");
    }
}
