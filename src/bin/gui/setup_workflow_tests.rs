//! Production host Save/browser interactions; channels only, no worker or device.
use super::tests::{controller_fixture, setup_document};
use super::workspace_test_ui::{DEFAULT_SIZE, Harness, MINIMUM_SIZE};
use super::*;
use underglow::setup::SetupDocument;
use underglow::setup_library::{LibraryMutation, SetupSummary};

fn status(enabled: bool) -> serde_json::Value {
    serde_json::json!({"schema_version":1,"state":if enabled {"active"} else {"paused"},
        "enabled":enabled,"mode":"matrix","brightness":128,"palette":"terminal","fps":30,"retry_attempt":0})
}

fn fixture(enabled: bool) -> (Controller, Receiver<Action>, Sender<Completion>) {
    let (mut app, incoming) = controller_fixture();
    let (outgoing, replies) = mpsc::channel();
    app.replies = replies;
    app.status = Some(serde_json::from_value(status(enabled)).unwrap());
    let mut document = setup_document();
    document.name = "Saved setup".into();
    document.revision = 4;
    app.setup_editor.load(Some(document.clone()));
    app.setup_library
        .receive(setup_library_panel::Report {
            schema_version: 1,
            revision: 8,
            current_id: Some(document.id.clone()),
            setups: vec![
                SetupSummary {
                    id: document.id.clone(),
                    name: document.name,
                    layers: document.layers.len(),
                },
                SetupSummary {
                    id: "other".into(),
                    name: "Other setup".into(),
                    layers: 2,
                },
            ],
        })
        .unwrap();
    app.setup_editor.name_draft("Draft setup".into()).unwrap();
    (app, incoming, outgoing)
}

fn response(saved: Option<&SetupDocument>, library_revision: Option<u64>) -> serde_json::Value {
    let mut reply = serde_json::json!({"ok":true,"status":status(false)});
    if let Some(document) = saved {
        reply["setup"] = serde_json::json!({"revision":document.revision,"document":document});
        if let Some(revision) = library_revision {
            reply["library"] = serde_json::json!({"schema_version":1,"revision":revision,"current_id":document.id,
                "setups":[{"id":document.id,"name":document.name,"layers":document.layers.len()},
                    {"id":"other","name":"Other setup","layers":2}]});
        }
    }
    reply
}

fn complete(
    app: &mut Controller,
    outgoing: &Sender<Completion>,
    action: Action,
    reply: serde_json::Value,
) {
    outgoing
        .send(Completion {
            action,
            result: Ok(reply.to_string()),
        })
        .unwrap();
    app.receive();
}

fn click(harness: &mut Harness, app: &mut Controller, label: &str) {
    for _ in 0..18 {
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        if frame
            .texts()
            .iter()
            .any(|t| t.text == label && t.fully_visible())
        {
            for _ in 0..30 {
                harness.run(vec![], |ctx| app.show_lighting(ctx));
            }
            harness.click(label, None, |ctx| app.show_lighting(ctx));
            return;
        }
        let pane = frame
            .texts()
            .into_iter()
            .find(|t| t.text == "Name for draft / copy")
            .or_else(|| {
                frame
                    .texts()
                    .into_iter()
                    .find(|t| t.text == "Close setup browser")
            })
            .map(|t| t.clip)
            .unwrap_or_else(|| panic!("browser scroll pane while locating {label}"));
        harness.wheel(pane.center(), egui::vec2(0.0, -160.0), |ctx| {
            app.show_lighting(ctx)
        });
    }
    panic!("unreachable {label}");
}

fn open_other(harness: &mut Harness, app: &mut Controller) {
    click(harness, app, "Browse setups: Draft setup");
    let mut picker = None;
    for _ in 0..12 {
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let texts = frame.texts();
        let below = texts
            .iter()
            .find(|t| t.text == "New draft:")
            .map(|t| t.rect.bottom());
        let name_visible = texts.iter().any(|t| t.text == "Name for draft / copy");
        picker = texts
            .iter()
            .find(|t| {
                t.text == "Saved setup"
                    && t.fully_visible()
                    && below.map_or(!name_visible, |bottom| t.rect.top() > bottom)
            })
            .map(|t| t.rect.center());
        if picker.is_some() {
            break;
        }
        let pane = texts
            .iter()
            .find(|t| t.text == "Close setup browser")
            .unwrap()
            .clip;
        harness.wheel(pane.center(), egui::vec2(0.0, -160.0), |ctx| {
            app.show_lighting(ctx)
        });
        for _ in 0..30 {
            harness.run(vec![], |ctx| app.show_lighting(ctx));
        }
    }
    harness.click_at(
        picker.expect("reachable saved-setup picker, not name text field"),
        |ctx| app.show_lighting(ctx),
    );
    click(harness, app, "Other setup · 2 layers");
    click(harness, app, "Open saved setup");
}

#[test]
fn workflow_host_active_save_cancel_pause_ack_and_failed_save_keep_draft_paused() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, outgoing) = fixture(true);
        let mut harness = Harness::new(size);
        let draft = app.setup_editor.draft.clone();
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        assert_eq!(frame.texts().iter().filter(|t| t.text == "Save").count(), 1);
        assert!(
            !frame
                .texts()
                .iter()
                .any(|t| t.text == "Save composition" || t.text == "Save current setup")
        );
        click(&mut harness, &mut app, "Save");
        assert!(incoming.try_recv().is_err());
        click(&mut harness, &mut app, "Cancel");
        assert_eq!(app.setup_editor.draft, draft);
        assert!(app.status.as_ref().unwrap().enabled);
        click(&mut harness, &mut app, "Save");
        click(&mut harness, &mut app, "Pause and save");
        let pause = incoming.try_recv().unwrap();
        assert!(matches!(&pause, Action::Control(args) if args == &[OsString::from("pause")]));
        complete(
            &mut app,
            &outgoing,
            Action::Control(vec!["status".into()]),
            response(None, None),
        );
        assert!(
            incoming.try_recv().is_err(),
            "status cannot acknowledge Pause"
        );
        complete(&mut app, &outgoing, pause, response(None, None));
        let save = incoming.try_recv().unwrap();
        assert!(matches!(save, Action::SetupReplace { revision: 4, .. }));
        complete(
            &mut app,
            &outgoing,
            save,
            serde_json::json!({"ok":false,"status":status(false),"error":"Setup revision conflict; refresh and retry"}),
        );
        assert_eq!(app.setup_editor.draft, draft);
        assert!(app.setup_editor.dirty);
        assert!(!app.status.as_ref().unwrap().enabled);
        assert!(app.message.contains("conflict"));
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn workflow_host_paused_save_only_cas_and_older_ack_advances_baseline_without_losing_edits() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, outgoing) = fixture(false);
        let mut harness = Harness::new(size);
        click(&mut harness, &mut app, "Save");
        let action = incoming.try_recv().unwrap();
        let Action::SetupReplace {
            revision: 4,
            document,
        } = &action
        else {
            panic!("only CAS Save")
        };
        let mut saved = document.clone();
        saved.revision = 5;
        app.setup_editor
            .apply(underglow::setup_edits::LayerEdit::Remove {
                id: document.layers[1].id.clone(),
            })
            .unwrap();
        let newer = app.setup_editor.draft.clone();
        complete(&mut app, &outgoing, action, response(Some(&saved), Some(9)));
        assert_eq!(app.setup_editor.applied, Some(saved));
        assert_eq!(app.setup_editor.base_revision, 5);
        assert_eq!(app.setup_editor.draft, newer);
        assert!(app.setup_editor.dirty);
        assert!(incoming.try_recv().is_err());
        click(&mut harness, &mut app, "Save");
        assert!(matches!(
            incoming.try_recv(),
            Ok(Action::SetupReplace { revision: 5, .. })
        ));
    }
}

#[test]
fn workflow_host_dirty_open_cancel_discard_failure_and_save_first_use_current_cas() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        for choice in ["Cancel", "Discard changes", "Save changes"] {
            let (mut app, incoming, outgoing) = fixture(false);
            let mut harness = Harness::new(size);
            let draft = app.setup_editor.draft.clone();
            open_other(&mut harness, &mut app);
            assert!(incoming.try_recv().is_err());
            click(&mut harness, &mut app, choice);
            if choice == "Cancel" {
                assert_eq!(app.setup_editor.draft, draft);
                assert!(incoming.try_recv().is_err());
                continue;
            }
            let action = incoming.try_recv().unwrap();
            if choice == "Discard changes" {
                assert!(matches!(
                    action,
                    Action::LibraryUpdate {
                        revision: 8,
                        mutation: LibraryMutation::Switch {
                            expected_setup_revision: 4,
                            ..
                        }
                    }
                ));
                complete(
                    &mut app,
                    &outgoing,
                    action,
                    serde_json::json!({"ok":false,"error":"Cannot open saved setup"}),
                );
                assert_eq!(app.setup_editor.draft, draft);
                assert!(app.setup_editor.dirty);
            } else {
                let Action::SetupReplace { document, .. } = &action else {
                    panic!("Save first")
                };
                let mut saved = document.clone();
                saved.revision = 5;
                complete(&mut app, &outgoing, action, response(Some(&saved), None));
                let refresh = incoming.try_recv().unwrap();
                assert!(matches!(refresh, Action::LibraryList));
                assert_eq!(app.setup_editor.applied.as_ref(), Some(&saved));
                complete(
                    &mut app,
                    &outgoing,
                    refresh,
                    response(Some(&saved), Some(9)),
                );
                let switch = incoming.try_recv().unwrap();
                assert!(matches!(
                    switch,
                    Action::LibraryUpdate {
                        revision: 9,
                        mutation: LibraryMutation::Switch {
                            expected_setup_revision: 5,
                            ..
                        }
                    }
                ));
                let mut opened = saved;
                opened.id = "other".into();
                opened.name = "Other setup".into();
                opened.revision = 6;
                complete(
                    &mut app,
                    &outgoing,
                    switch,
                    response(Some(&opened), Some(10)),
                );
                assert_eq!(app.setup_editor.draft, Some(opened));
                assert!(!app.setup_editor.dirty);
            }
            assert!(!app.status.as_ref().unwrap().enabled);
            assert!(incoming.try_recv().is_err());
        }
    }
}

#[test]
fn workflow_host_copy_stores_current_draft_without_loading_unexpected_setup_envelope() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, outgoing) = fixture(true);
        let mut harness = Harness::new(size);
        let draft = app.setup_editor.draft.clone().unwrap();
        let applied = app.setup_editor.applied.clone();
        click(&mut harness, &mut app, "Browse setups: Draft setup");
        click(&mut harness, &mut app, "Save a copy");
        let action = incoming.try_recv().unwrap();
        let Action::LibraryUpdate {
            revision: 8,
            mutation: LibraryMutation::Store { document },
        } = &action
        else {
            panic!("inactive Store only")
        };
        assert_ne!(document.id, draft.id);
        assert_eq!(document.revision, 0);
        assert_eq!(document.layers, draft.layers);
        let mut malicious = draft.clone();
        malicious.id = "unexpected".into();
        malicious.revision = 22;
        let mut reply = response(Some(&malicious), Some(9));
        reply["status"] = status(true);
        complete(&mut app, &outgoing, action, reply);
        assert_eq!(app.setup_editor.draft.as_ref(), Some(&draft));
        assert_eq!(app.setup_editor.applied, applied);
        assert!(app.setup_editor.dirty);
        assert!(app.status.as_ref().unwrap().enabled);
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn workflow_host_offline_draft_is_editable_and_unexpected_ack_never_clears_it() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, outgoing) = fixture(false);
        app.connected = false;
        let mut harness = Harness::new(size);
        click(&mut harness, &mut app, "Save");
        assert!(incoming.try_recv().is_err());
        click(&mut harness, &mut app, "Add layer");
        let draft = app.setup_editor.draft.clone();
        assert!(
            draft.as_ref().unwrap().layers.len()
                > app.setup_editor.applied.as_ref().unwrap().layers.len()
        );
        app.connected = true;
        click(&mut harness, &mut app, "Save");
        let action = incoming.try_recv().unwrap();
        complete(
            &mut app,
            &outgoing,
            action,
            serde_json::json!({"ok":true,"status":status(false)}),
        );
        assert_eq!(app.setup_editor.draft, draft);
        assert!(app.setup_editor.dirty);
        assert!(!app.setup_workflow.pending());
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn workflow_host_conflict_recovery_reload_is_read_only_and_requires_explicit_discard() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, outgoing) = fixture(false);
        let mut harness = Harness::new(size);
        let draft = app.setup_editor.draft.clone();
        click(&mut harness, &mut app, "Browse setups: Draft setup");
        click(&mut harness, &mut app, "Reload saved version");
        click(&mut harness, &mut app, "Cancel");
        assert_eq!(app.setup_editor.draft, draft);
        assert!(incoming.try_recv().is_err());
        click(&mut harness, &mut app, "Reload saved version");
        click(&mut harness, &mut app, "Discard changes");
        let action = incoming.try_recv().unwrap();
        assert!(matches!(action, Action::SetupShow));
        let mut external = app.setup_editor.applied.clone().unwrap();
        external.revision = 17;
        external.name = "Updated elsewhere".into();
        complete(&mut app, &outgoing, action, response(Some(&external), None));
        assert_eq!(app.setup_editor.base_revision, 17);
        assert_eq!(app.setup_editor.draft, Some(external));
        assert!(!app.setup_editor.dirty);
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn workflow_host_missing_or_malformed_ack_aborts_save_first_without_switch() {
    for malformed in [false, true] {
        let (mut app, incoming, outgoing) = fixture(false);
        let mut harness = Harness::new(MINIMUM_SIZE);
        let draft = app.setup_editor.draft.clone();
        open_other(&mut harness, &mut app);
        click(&mut harness, &mut app, "Save changes");
        let action = incoming.try_recv().unwrap();
        let result = if malformed {
            Ok("not JSON".into())
        } else {
            Ok(response(None, None).to_string())
        };
        outgoing.send(Completion { action, result }).unwrap();
        app.receive();
        assert_eq!(app.setup_editor.draft, draft);
        assert!(app.setup_editor.dirty);
        assert!(!app.setup_workflow.pending());
        assert!(incoming.try_recv().is_err());
    }
}
