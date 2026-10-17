use super::*;
use underglow::source_capabilities::{FactDescriptor, pi_descriptor};

fn report() -> String {
    let mut descriptor = pi_descriptor();
    descriptor.source = "custom".into();
    descriptor.display_name = "Build monitor".into();
    descriptor.states = vec![FactDescriptor {
        id: "building".into(),
        label: "Building project".into(),
    }];
    descriptor.events = vec![FactDescriptor {
        id: "finished".into(),
        label: "Build finished".into(),
    }];
    serde_json::json!({"ok":true,"connections":{
        "registry":{"schema_version":1,"revision":7,"connections":[
            {"descriptor":descriptor,"producer":{"id":"custom-producer","source":"custom","enabled":true,"trusted":true,"command":["SECRET_ARGV"],"env":{"TOKEN":"SECRET_ENV"}}},
            {"descriptor":pi_descriptor()}]},
        "runtime_index":{"custom":{"kind":"managed","producer":{"state":"running","running":true,"healthy":true,"restarts":3,
            "last_message_age_ms":200,"retry_in_ms":null,"last_error":"Malformed protocol message"}},
            "pi":{"kind":"external","producer":null}}
    }}).to_string()
}
fn state(instance: &str, value: &str) -> StateSnapshot {
    StateSnapshot {
        identity: Identity {
            source: "custom".into(),
            instance: instance.into(),
        },
        sequence: 1,
        state: value.into(),
        updated_at: Duration::from_millis(100),
        expires_at: Duration::from_millis(1100),
        revision: 1,
    }
}
fn snapshot(states: Vec<StateSnapshot>) -> Snapshot {
    Snapshot {
        revision: 1,
        event_cursor: 0,
        states,
    }
}
fn events() -> EventBatch {
    EventBatch {
        cursor: 0,
        gap: false,
        events: vec![],
    }
}
fn sample<'a>(
    snapshot: &'a Snapshot,
    events: &'a EventBatch,
    now: u64,
    generation: u64,
) -> SourceSample<'a> {
    SourceSample {
        snapshot,
        events,
        now: Duration::from_millis(now),
        stale: false,
        generation: (1, generation),
    }
}
fn setup() -> SetupDocument {
    serde_json::from_value(serde_json::json!({"schema_version":1,"id":"draft","name":"Draft","layers":[{
        "id":"layer-stable","name":"Compiler","target":{"kind":"whole"},"renderer":{"kind":"solid","color":[1,2,3]},
        "activation":{"kind":"while-state","source":"custom","instance":"first","state":"building"},
        "variants":[{"id":"missing-variant","name":"Success flash","activation":{"kind":"on-event","source":"missing","event":"finished","duration_ms":500},
            "renderer":{"kind":"solid","color":[3,2,1]}}]}]})).unwrap()
}
fn draw(panel: &mut Panel, view: View<'_>) -> String {
    // Detail-content checks open real disclosures. Primary reachability uses
    // realistic default/minimum sizes in secondary_flow_tests.
    let mut harness = super::super::workspace_test_ui::Harness::new(egui::vec2(1500.0, 18000.0));
    let mut show = |ctx: &egui::Context| {
        egui::CentralPanel::default().show(ctx, |ui| {
            assert!(panel.show(ui, view).is_none());
        });
    };
    harness.click("Connection report details", None, &mut show);
    for _ in 0..20 {
        harness.run(vec![], &mut show);
    }
    let count = harness
        .settle(&mut show)
        .texts()
        .iter()
        .filter(|t| t.text == "Connection details")
        .count();
    for index in 0..count {
        let frame = harness.settle(&mut show);
        let pos = frame
            .texts()
            .into_iter()
            .filter(|t| t.text == "Connection details")
            .nth(index)
            .unwrap()
            .rect
            .center();
        harness.click_at(pos, &mut show);
        for _ in 0..20 {
            harness.run(vec![], &mut show);
        }
    }
    harness.click("Connection administration", None, &mut show);
    for _ in 0..20 {
        harness.run(vec![], &mut show);
    }
    harness
        .settle(show)
        .texts()
        .into_iter()
        .map(|t| t.text)
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn running_but_stale_is_not_process_death_or_fresh_state() {
    let mut panel = Panel::default();
    panel.receive_report(Ok(report()));
    let snapshot = snapshot(vec![state("first", "building")]);
    let events = events();
    let text = draw(
        &mut panel,
        View {
            sample: Some(sample(&snapshot, &events, 1200, 1)),
            setup: None,
            runtime: None,
            connected: true,
            busy: false,
        },
    );
    for expected in [
        "Build monitor",
        "running=true",
        "protocol healthy=true",
        "restarts=3",
        "Not reporting",
        "last seen / not reporting",
        "Process health: unknown",
        "Connector error:",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(!text.contains("SECRET"));
    assert!(text.contains("no fresh state lease"));
}

#[test]
fn old_health_capture_never_becomes_current_process_health() {
    let mut panel = Panel::default();
    panel.receive_report(Ok(report()));
    assert!(
        panel
            .process_summary("custom", true)
            .contains("running at last check")
    );
    panel.captured = Some(Instant::now() - Duration::from_secs(11));
    assert_eq!(
        panel.process_summary("custom", true),
        "Process health: unknown"
    );
    assert_eq!(panel.process_summary("pi", true), "Process health: unknown");
}
#[test]
fn fresh_multiple_instances_explain_selector_mismatch_and_missing_variant_impact() {
    let mut panel = Panel::default();
    panel.receive_report(Ok(report()));
    let setup = setup();
    let mut runtime = CompiledSetup::compile(&setup).unwrap();
    let snapshot = snapshot(vec![state("first", "idle"), state("second", "building")]);
    let events = events();
    runtime.baseline(&snapshot, Duration::from_millis(200));
    let text = draw(
        &mut panel,
        View {
            sample: Some(sample(&snapshot, &events, 200, 1)),
            setup: Some(&setup),
            runtime: Some(&runtime),
            connected: true,
            busy: false,
        },
    );
    for expected in [
        "Application state: Fresh",
        "Instance first",
        "Instance second",
        "900ms remaining",
        "No matching fresh state",
        "requires instance first / state building",
        "Missing connection binding",
        "Compiler [layer-stable]",
        "Success flash [missing-variant]",
        "Layers are preserved",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert_eq!(setup, super::tests::setup());
}
#[test]
fn declared_custom_and_pi_capabilities_are_exact_not_invented() {
    let mut panel = Panel::default();
    panel.receive_report(Ok(report()));
    assert_eq!(panel.capabilities().unwrap().descriptors().count(), 2);
    let report = panel.report.unwrap();
    let custom = &report.registry.connections[0].descriptor;
    assert_eq!(custom.source, "custom");
    assert_eq!(custom.states[0].id, "building");
    assert_eq!(custom.events[0].id, "finished");
    assert_eq!(report.registry.connections[1].descriptor, pi_descriptor());
    let pi = &report.registry.connections[1].descriptor;
    assert!(!pi.states.iter().any(|s| s.id == "error"));
}

#[test]
fn matching_state_uses_canonical_preview_selection_but_not_hardware_claims() {
    let mut panel = Panel::default();
    panel.receive_report(Ok(report()));
    let setup = setup();
    let mut runtime = CompiledSetup::compile(&setup).unwrap();
    let snapshot = snapshot(vec![state("first", "building")]);
    let events = events();
    runtime.baseline(&snapshot, Duration::from_millis(200));
    let text = draw(
        &mut panel,
        View {
            sample: Some(sample(&snapshot, &events, 200, 1)),
            setup: Some(&setup),
            runtime: Some(&runtime),
            connected: true,
            busy: false,
        },
    );
    assert!(text.contains("Selected in the composition preview"));
    assert!(text.contains("hardware activation is separate"));
}

#[test]
fn removed_or_expired_states_are_only_history_and_delayed_samples_are_not_live() {
    let mut panel = Panel::default();
    panel.receive_report(Ok(report()));
    let observed = snapshot(vec![state("first", "building")]);
    let events = events();
    panel.observe(&sample(&observed, &events, 200, 1));
    let empty = snapshot(vec![]);
    let text = draw(
        &mut panel,
        View {
            sample: Some(sample(&empty, &events, 300, 1)),
            setup: None,
            runtime: None,
            connected: true,
            busy: false,
        },
    );
    assert!(text.contains("last seen / not reporting"));
    assert!(!text.contains("Application state: Fresh"));
    let mut delayed = sample(&observed, &events, 400, 1);
    delayed.stale = true;
    let text = draw(
        &mut panel,
        View {
            sample: Some(delayed),
            setup: None,
            runtime: None,
            connected: true,
            busy: false,
        },
    );
    assert!(!text.contains("Application state: Fresh"));
    assert!(text.contains("Live facts unavailable or delayed"));
}
#[test]
fn partial_health_failure_retains_registry_and_facts_without_generic_lighting_error() {
    let mut panel = Panel::default();
    panel.receive_report(Ok(report()));
    panel.receive_report(Err("health transport timed out".into()));
    let snapshot = snapshot(vec![state("first", "building")]);
    let events = events();
    let text = draw(
        &mut panel,
        View {
            sample: Some(sample(&snapshot, &events, 200, 1)),
            setup: None,
            runtime: None,
            connected: true,
            busy: false,
        },
    );
    assert!(text.contains("Connection health: Connections request failed"));
    assert!(text.contains("Build monitor"));
    assert!(text.contains("Application state: Fresh"));
    assert!(text.contains("values below are last known"));
    panel.receive_report(Ok("{invalid".into()));
    assert!(panel.report.is_some());
}

#[test]
fn polling_and_rejected_stale_writes_preserve_private_draft_and_last_saved_registry() {
    let mut panel = Panel::default();
    panel.receive_report(Ok(report()));
    let original = panel.registry().unwrap().clone();
    panel.producer_editor().open(&original, "custom");
    let mut body: serde_json::Value = serde_json::from_str(&report()).unwrap();
    body["connections"]["registry"]["revision"] = 8.into();
    body["connections"]["registry"]["connections"][0]["producer"]["command"] =
        serde_json::json!(["NEW_PRIVATE_ARGV"]);
    panel.receive_report(Ok(body.to_string()));
    let saved = panel.registry().unwrap().clone();
    assert_eq!(saved.revision, 8);
    panel.receive_replacement(&original, Ok(serde_json::json!({"ok":false,"error":"SECRET_ENV","connections":{"registry":original,"runtime_index":{}}}).to_string()));
    assert_eq!(panel.registry(), Some(&saved));
    let view = View {
        sample: None,
        setup: None,
        runtime: None,
        connected: true,
        busy: false,
    };
    let text = draw(&mut panel, view);
    assert!(text.contains("Private draft"));
    assert!(text.contains("base revision 7"));
    assert!(text.contains("Registry changed"));
    assert!(!text.contains("SECRET") && !text.contains("NEW_PRIVATE_ARGV"));
    // A mismatched success envelope must not replace the last confirmed registry.
    panel.receive_replacement(&saved, Ok(report()));
    assert_eq!(panel.registry(), Some(&saved));
    assert!(draw(&mut panel, view).contains("Private draft"));
}
#[test]
fn control_transport_failure_does_not_hide_independently_fresh_facts() {
    let mut panel = Panel::default();
    panel.receive_report(Ok(report()));
    let snapshot = snapshot(vec![state("first", "building")]);
    let events = events();
    let text = draw(
        &mut panel,
        View {
            sample: Some(sample(&snapshot, &events, 200, 1)),
            setup: None,
            runtime: None,
            connected: false,
            busy: false,
        },
    );
    assert!(text.contains("Application state: Fresh"));
    assert!(text.contains("health is last known, not current"));
}

#[test]
fn metadata_import_acknowledgment_never_closes_or_acknowledges_a_private_producer_draft() {
    let mut panel = Panel::default();
    panel.receive_report(Ok(report()));
    let mut registry = panel.registry().unwrap().clone();
    registry
        .connections
        .iter_mut()
        .find(|c| c.descriptor.source == "custom")
        .unwrap()
        .producer
        .as_mut()
        .unwrap()
        .enabled = false;
    panel.report.as_mut().unwrap().registry = registry.clone();
    panel.producer_editor().open(&registry, "custom");
    let view = View {
        sample: None,
        setup: None,
        runtime: None,
        connected: true,
        busy: false,
    };
    assert!(draw(&mut panel, view).contains("Private draft"));
    let mut descriptor = registry.connection("custom").unwrap().descriptor.clone();
    descriptor.display_name = "Changed display name".into();
    let candidate = underglow::capability_manifest::import_descriptor(
        &registry,
        registry.revision,
        descriptor,
        true,
    )
    .unwrap();
    let saved = registry.replace(registry.revision, &candidate).unwrap();
    panel.metadata_write = true; // Explicit importer intent, not producer-save intent.
    panel.receive_replacement(
        &candidate,
        Ok(
            serde_json::json!({"ok":true,"connections":{"registry":saved,"runtime_index":{}}})
                .to_string(),
        ),
    );
    assert_eq!(panel.registry(), Some(&saved));
    let text = draw(&mut panel, view);
    assert!(text.contains("Private draft"));
    assert!(text.contains("Registry changed"));
    assert!(!text.contains("SECRET"));
}
#[test]
fn history_is_bounded_deduplicated_and_cleared_on_engine_generation_change() {
    let mut panel = Panel::default();
    let snapshot = snapshot(
        (0..300)
            .map(|i| state(&i.to_string(), "building"))
            .collect(),
    );
    let mut events = events();
    events.events = (1..=300)
        .map(|cursor| EventSnapshot {
            cursor,
            identity: state("first", "building").identity,
            sequence: cursor,
            event: "finished".into(),
            received_at: Duration::from_millis(cursor),
            expires_at: Duration::from_millis(cursor + 500),
            revision: cursor,
        })
        .collect();
    panel.observe(&sample(&snapshot, &events, 400, 1));
    panel.observe(&sample(&snapshot, &events, 400, 1));
    assert_eq!(panel.states.len(), MAX_HISTORY);
    assert_eq!(panel.events.len(), MAX_HISTORY);
    let empty = super::tests::snapshot(vec![]);
    let empty_events = super::tests::events();
    panel.observe(&sample(&empty, &empty_events, 400, 2));
    assert!(panel.states.is_empty());
    assert!(panel.events.is_empty());
}
