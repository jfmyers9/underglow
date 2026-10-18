//! Real styled secondary/legacy journeys; channels and synthetic fixtures only.
use super::tests::controller_fixture;
use super::workspace_test_ui::{DEFAULT_SIZE, Harness, MINIMUM_SIZE};
use super::*;
use underglow::connection_registry::ConnectionRegistry;
use underglow::external::{EventBatch, Identity, Snapshot, StateSnapshot};
use underglow::setup::Activation;
use underglow::setup_library::{LibraryMutation, SetupSummary, Starter, starter};

fn fixture(enabled: bool) -> (Controller, Receiver<Action>, Sender<Completion>) {
    let (mut app, incoming) = controller_fixture();
    let (outgoing, replies) = mpsc::channel();
    app.replies = replies;
    app.status = Some(serde_json::from_value(status(enabled)).unwrap());
    let mut document = starter(Starter::MatrixPi, "daily".into(), "Daily".into()).unwrap();
    document.revision = 4;
    app.setup_editor.load(Some(document.clone()));
    app.setup_editor.selected = Some("pi-status".into());
    app.setup_library
        .receive(setup_library_panel::Report {
            schema_version: 1,
            revision: 8,
            current_id: Some(document.id.clone()),
            setups: vec![SetupSummary {
                id: document.id,
                name: document.name,
                layers: 2,
            }],
        })
        .unwrap();
    let registry = ConnectionRegistry::default();
    app.connections_panel
        .receive_report(Ok(serde_json::json!({"ok":true,"connections":{
        "registry":registry,"runtime_index":{}}})
        .to_string()));
    (app, incoming, outgoing)
}

fn status(enabled: bool) -> serde_json::Value {
    serde_json::json!({"schema_version":1,"state":if enabled {"active"} else {"paused"},
        "enabled":enabled,"mode":"matrix","brightness":128,"palette":"terminal","fps":30,"retry_attempt":0})
}

fn live(stale: bool) -> setup_preview::LiveSample {
    setup_preview::LiveSample {
        generation: (1, 1),
        now: Duration::from_millis(100),
        received_at: Instant::now(),
        connected: true,
        stale,
        snapshot: Snapshot {
            revision: 1,
            event_cursor: 0,
            states: vec![StateSnapshot {
                identity: Identity {
                    source: "pi".into(),
                    instance: "live-session".into(),
                },
                sequence: 1,
                state: "working".into(),
                updated_at: Duration::ZERO,
                expires_at: Duration::from_secs(100),
                revision: 1,
            }],
        },
        events: EventBatch {
            cursor: 0,
            gap: false,
            events: vec![],
        },
    }
}

fn text(harness: &mut Harness, app: &mut Controller) -> String {
    harness
        .settle(|ctx| app.show_lighting(ctx))
        .texts()
        .into_iter()
        .map(|t| t.text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn click(harness: &mut Harness, app: &mut Controller, label: &str) {
    let switcher = app
        .setup_editor
        .draft
        .as_ref()
        .map(|doc| doc.name.as_str())
        .unwrap_or_else(|| effect_description(&app.preset).0)
        .to_owned();
    let label = if label == "Change setup..." {
        switcher.as_str()
    } else {
        label
    };
    for _ in 0..35 {
        // Resolve the hit after the real pane has settled. The quick-test rows
        // leave less compact inspector height, so an earlier visible hit can
        // move out of the clip while smooth scrolling finishes.
        for _ in 0..20 {
            harness.run(vec![], |ctx| app.show_lighting(ctx));
        }
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let texts = frame.texts();
        if let Some(hit) = texts.iter().find(|t| t.text == label && t.fully_visible()) {
            harness.click_at(hit.rect.center(), |ctx| app.show_lighting(ctx));
            return;
        }
        let pane = texts
            .iter()
            .find(|t| t.text == "Close sharing")
            .or_else(|| texts.iter().find(|t| t.text == "Close setup browser"))
            .map(|t| t.clip)
            .or_else(|| {
                texts
                    .iter()
                    .find(|t| t.text == "Layer actions")
                    .map(|t| t.clip)
            });
        let pos = pane.map_or(
            egui::pos2(
                harness.screen().center().x,
                harness.screen().bottom() - 90.0,
            ),
            |r| r.center(),
        );
        let distance = pane.map_or(180.0, |r| (r.height() * 0.6).min(180.0));
        harness.wheel(pos, egui::vec2(0.0, -distance), |ctx| {
            app.show_lighting(ctx)
        });
    }
    panic!("unreachable {label}; {}", text(harness, app));
}

fn set_source(activation: &mut Activation, value: &str) {
    match activation {
        Activation::WhileState { source, .. }
        | Activation::OnEvent { source, .. }
        | Activation::OnTransition { source, .. } => *source = value.into(),
        Activation::Always {} => {}
    }
}

#[test]
fn secondary_missing_layer_connection_opens_correct_context_and_preserves_draft_on_return() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, _) = fixture(false);
        let layer = app
            .setup_editor
            .draft
            .as_mut()
            .unwrap()
            .layers
            .last_mut()
            .unwrap();
        set_source(&mut layer.activation, "missing-monitor");
        for variant in &mut layer.variants {
            set_source(&mut variant.activation, "missing-monitor");
        }
        app.setup_editor.dirty = true;
        let draft = app.setup_editor.draft.clone();
        let mut harness = Harness::new(size);
        click(&mut harness, &mut app, "Connections: missing-monitor");
        assert!(app.page == Page::Connections);
        let rendered = text(&mut harness, &mut app);
        assert!(rendered.contains("Selected connection: missing-monitor"));
        assert!(rendered.contains("Affected layer: Pi status"));
        assert!(rendered.contains("Missing connection binding"));
        assert!(rendered.contains("Process health: unknown"));
        assert!(!rendered.contains("running=true"));
        if let Ok(action) = incoming.try_recv() {
            assert!(matches!(action, Action::ConnectionsShow));
            app.pending = false;
        }
        click(&mut harness, &mut app, "Repair layer binding");
        click(&mut harness, &mut app, "Editor");
        assert_eq!(app.setup_editor.draft, draft);
        assert_eq!(app.setup_editor.selected.as_deref(), Some("pi-status"));
        assert!(!app.status.as_ref().unwrap().enabled);
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn secondary_layer_notices_use_real_facts_never_private_scenario_and_do_not_invent_process_death() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, _) = fixture(false);
        app.setup_editor
            .preview
            .set_mode(setup_preview::Mode::Scenario);
        app.setup_editor
            .preview
            .scenario
            .publish_state("pi", "private-session", "working", 10000)
            .unwrap();
        let mut harness = Harness::new(size);
        harness.settle(|ctx| app.show_lighting(ctx));
        assert!(
            app.setup_editor
                .connection_notices
                .iter()
                .any(|n| n.message.contains("No recent data"))
        );
        app.setup_editor.preview.set_sample(Some(live(true)));
        harness.settle(|ctx| app.show_lighting(ctx));
        assert!(
            app.setup_editor.connection_notices[0]
                .message
                .contains("Live data delayed")
        );
        app.setup_editor.preview.set_sample(Some(live(false)));
        harness.settle(|ctx| app.show_lighting(ctx));
        assert!(
            app.setup_editor.connection_notices[0]
                .message
                .contains("Recent data available")
        );
        app.connected = false; // Fact and control transports are independent.
        harness.settle(|ctx| app.show_lighting(ctx));
        assert!(
            app.setup_editor.connection_notices[0]
                .message
                .contains("Recent data available")
        );
        assert!(app.setup_editor.preview.scenario.snapshot().states.len() == 1);
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn secondary_connection_summary_hides_details_and_administration_remains_explicit() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, _) = fixture(false);
        app.page = Page::Connections;
        app.last_connections = Instant::now();
        app.setup_editor.preview.set_sample(Some(live(false)));
        let mut harness = Harness::new(size);
        let rendered = text(&mut harness, &mut app);
        assert!(rendered.contains("Recent data available"));
        assert!(rendered.contains("Process health: unknown"));
        assert!(!rendered.contains("Instance live-session"));
        assert!(!rendered.contains("Managed producer controls"));
        click(&mut harness, &mut app, "Connection details");
        assert!(text(&mut harness, &mut app).contains("Source ID: pi"));
        click(&mut harness, &mut app, "Connection details");
        click(&mut harness, &mut app, "Connection administration");
        click(&mut harness, &mut app, "Configure producer");
        assert!(app.setup_editor.draft.is_some());
        assert!(!app.status.as_ref().unwrap().enabled);
        assert!(
            incoming.try_recv().is_err(),
            "presentation must not install/enable anything"
        );
    }
}

#[test]
fn secondary_sharing_is_explicit_and_import_store_does_not_replace_or_activate() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, _) = fixture(false);
        app.setup_editor.name_draft("Private draft".into()).unwrap();
        let draft = app.setup_editor.draft.clone();
        let registry = app.connections_panel.registry().cloned();
        let mut harness = Harness::new(size);
        assert!(!text(&mut harness, &mut app).contains("Validate pasted setup"));
        click(&mut harness, &mut app, "Change setup...");
        click(&mut harness, &mut app, "Share setups");
        assert!(app.sharing_open);
        assert!(!app.setup_library.open);
        click(
            &mut harness,
            &mut app,
            "Paste a strict SetupDocument, not application/connection configuration",
        );
        let imported = starter(Starter::Blank, "external".into(), "Imported".into()).unwrap();
        harness.run(
            vec![egui::Event::Text(imported.to_toml().unwrap())],
            |ctx| app.show_lighting(ctx),
        );
        click(&mut harness, &mut app, "Validate pasted setup");
        click(
            &mut harness,
            &mut app,
            "Store imported new copy (do not switch)",
        );
        let action = incoming.try_recv().unwrap();
        assert!(
            matches!(action,Action::LibraryUpdate { revision:8,mutation:LibraryMutation::Store { document } } if document.id.starts_with("imported-") && document.layers.is_empty())
        );
        assert_eq!(app.setup_editor.draft, draft);
        assert_eq!(app.connections_panel.registry(), registry.as_ref());
        assert!(!app.status.as_ref().unwrap().enabled);
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn secondary_offline_browser_can_create_a_local_blank_but_cannot_save_it() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, _) = fixture(false);
        app.connected = false;
        let original = app.setup_editor.applied.clone();
        let mut harness = Harness::new(size);
        click(&mut harness, &mut app, "Change setup...");
        click(&mut harness, &mut app, "Blank");
        assert!(app.setup_editor.draft.as_ref().unwrap().layers.is_empty());
        assert!(app.setup_editor.dirty);
        assert_eq!(app.setup_editor.applied, original);
        click(&mut harness, &mut app, "Save");
        assert!(incoming.try_recv().is_err());
        assert!(!app.status.as_ref().unwrap().enabled);

        let (mut unknown, incoming, _) = fixture(false);
        unknown.connected = false;
        unknown.setup_checked = false;
        unknown.setup_editor = setup_editor::Editor::default();
        let mut startup = Harness::new(size);
        assert!(!text(&mut startup, &mut unknown).contains("Your current profile uses legacy"));
        click(&mut startup, &mut unknown, "Begin new setup");
        click(&mut startup, &mut unknown, "Blank");
        assert!(
            unknown
                .setup_editor
                .draft
                .as_ref()
                .unwrap()
                .layers
                .is_empty()
        );
        assert!(unknown.setup_editor.dirty);
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn secondary_legacy_keep_new_cancel_and_explicit_adoption_preserve_original_until_confirmed() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        for enabled in [false, true] {
            let (mut app, incoming, outgoing) = fixture(enabled);
            app.setup_editor = setup_editor::Editor::default();
            let registry = app.connections_panel.registry().cloned();
            let mut harness = Harness::new(size);
            let entry = text(&mut harness, &mut app);
            assert!(entry.contains("Keep existing lighting or begin a setup"));
            assert!(!entry.contains("Choose an effect"));
            click(&mut harness, &mut app, "Keep current behavior");
            assert!(incoming.try_recv().is_err());
            assert!(app.setup_editor.draft.is_none());
            click(&mut harness, &mut app, "Begin new setup");
            click(&mut harness, &mut app, "Blank");
            let draft = app.setup_editor.draft.clone();
            assert!(draft.as_ref().unwrap().layers.is_empty());
            assert!(app.setup_editor.applied.is_none());
            assert!(incoming.try_recv().is_err());
            click(&mut harness, &mut app, "Save");
            if enabled {
                click(&mut harness, &mut app, "Pause and save");
            }
            assert!(app.pending_adoption.is_some());
            let rendered = text(&mut harness, &mut app);
            assert!(rendered.contains("Unsupported legacy behavior is not carried"));
            assert!(rendered.contains("original source file"));
            assert!(
                incoming.try_recv().is_err(),
                "hold private snapshot changes before confirmation"
            );
            click(&mut harness, &mut app, "Cancel adoption");
            assert_eq!(app.setup_editor.draft, draft);
            assert_eq!(app.status.as_ref().unwrap().enabled, enabled);
            click(&mut harness, &mut app, "Save");
            if enabled {
                click(&mut harness, &mut app, "Pause and save");
            }
            click(&mut harness, &mut app, "Confirm setup adoption");
            let mut action = incoming.try_recv().unwrap();
            if enabled {
                assert!(
                    matches!(&action,Action::Control(args) if args==&[OsString::from("pause")])
                );
                outgoing
                    .send(Completion {
                        action,
                        result: Ok(
                            serde_json::json!({"ok":true,"status":status(false)}).to_string()
                        ),
                    })
                    .unwrap();
                app.receive();
                action = incoming.try_recv().unwrap();
            }
            assert!(matches!(action, Action::SetupReplace { revision: 0, .. }));
            assert_eq!(app.connections_panel.registry(), registry.as_ref());
            assert!(incoming.try_recv().is_err());
        }
    }
}

#[test]
fn secondary_opening_a_saved_setup_from_legacy_requires_adoption_before_switch() {
    for size in [DEFAULT_SIZE, MINIMUM_SIZE] {
        let (mut app, incoming, _) = fixture(false);
        app.setup_editor = setup_editor::Editor::default();
        app.setup_library.report.as_mut().unwrap().current_id = None;
        let mut harness = Harness::new(size);
        click(&mut harness, &mut app, "Begin new setup");
        click(&mut harness, &mut app, "Open saved setup");
        assert!(matches!(
            app.pending_adoption.as_ref(),
            Some(Action::LibraryUpdate {
                mutation: LibraryMutation::Switch { .. },
                ..
            })
        ));
        assert!(incoming.try_recv().is_err());
        click(&mut harness, &mut app, "Cancel adoption");
        assert!(app.setup_editor.draft.is_none());
        assert!(!app.status.as_ref().unwrap().enabled);
        click(&mut harness, &mut app, "Open saved setup");
        click(&mut harness, &mut app, "Confirm setup adoption");
        assert!(matches!(
            incoming.try_recv(),
            Ok(Action::LibraryUpdate {
                revision: 8,
                mutation: LibraryMutation::Switch {
                    expected_setup_revision: 0,
                    ..
                }
            })
        ));
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn secondary_long_connection_identity_keeps_summary_bounded_and_truthful() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE, egui::vec2(1280.0, 800.0)] {
        let (mut app, incoming, _) = fixture(false);
        let mut descriptor = underglow::source_capabilities::pi_descriptor();
        descriptor.display_name = "A source with a deliberately long descriptive connection name that must not push its summary outside the application window".into();
        app.connections_panel
            .receive_report(Ok(serde_json::json!({"ok":true,"connections":{
            "registry":{"schema_version":1,"revision":3,"connections":[{"descriptor":descriptor}]},
            "runtime_index":{}}})
            .to_string()));
        app.page = Page::Connections;
        let original = app.setup_editor.draft.clone();
        let mut harness = Harness::new(size);
        let frame = harness.settle(|ctx| app.show_lighting(ctx));
        let title = frame
            .texts()
            .into_iter()
            .find(|t| t.text.starts_with("A source with"))
            .expect("connection identity");
        assert!(
            title.fully_visible(),
            "long name must truncate, not overflow: {title:?}"
        );
        assert!(harness.screen().contains_rect(title.rect));
        let summary = text(&mut harness, &mut app);
        assert!(summary.contains("No recent data"));
        assert!(summary.contains("Process health: unknown"));
        assert!(!summary.contains("Application state: Fresh"));
        assert_eq!(app.setup_editor.draft, original);
        assert!(incoming.try_recv().is_err());
    }
}

#[test]
fn secondary_saved_copy_management_remains_explicit_and_reachable() {
    for size in [MINIMUM_SIZE, DEFAULT_SIZE] {
        for action in [
            "Duplicate saved setup",
            "Rename saved setup",
            "Delete saved setup",
        ] {
            let (mut app, incoming, _) = fixture(false);
            app.setup_library
                .report
                .as_mut()
                .unwrap()
                .setups
                .push(SetupSummary {
                    id: "other".into(),
                    name: "Other saved copy".into(),
                    layers: 1,
                });
            let original = app.setup_editor.draft.clone();
            let mut harness = Harness::new(size);
            click(&mut harness, &mut app, "Change setup...");
            let frame = harness.settle(|ctx| app.show_lighting(ctx));
            let picker = frame
                .texts()
                .into_iter()
                .rev()
                .find(|t| t.text == "Daily" && t.fully_visible())
                .expect("saved setup picker");
            harness.click_at(picker.rect.center(), |ctx| app.show_lighting(ctx));
            click(&mut harness, &mut app, "Other saved copy · 1 layers");
            assert!(incoming.try_recv().is_err(), "selection only browses");
            click(&mut harness, &mut app, "Manage saved setup");
            click(&mut harness, &mut app, action);
            if action == "Delete saved setup" {
                assert!(incoming.try_recv().is_err(), "delete requires confirmation");
                click(&mut harness, &mut app, "Confirm delete setup");
            }
            let Ok(Action::LibraryUpdate { revision, mutation }) = incoming.try_recv() else {
                panic!("expected explicit library update for {action}");
            };
            assert_eq!(revision, 8);
            match (action, mutation) {
                ("Duplicate saved setup", LibraryMutation::Duplicate { id, .. })
                | ("Rename saved setup", LibraryMutation::Rename { id, .. })
                | ("Delete saved setup", LibraryMutation::Delete { id }) => assert_eq!(id, "other"),
                (_, mutation) => panic!("wrong mutation for {action}: {mutation:?}"),
            }
            assert_eq!(app.setup_editor.draft, original);
            assert!(!app.status.as_ref().unwrap().enabled);
            assert!(incoming.try_recv().is_err());
        }
    }
}
