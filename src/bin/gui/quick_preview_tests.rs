//! Quick tests exercise the real evaluator, finite leases and canonical capture.
use super::*;
use underglow::external::{EventBatch, Snapshot};
use underglow::render::Color;

fn document() -> SetupDocument {
    let mut doc = underglow::setup_library::starter(
        underglow::setup_library::Starter::MatrixPi,
        "quick".into(),
        "Quick".into(),
    )
    .unwrap();
    doc.layers[0].renderer = underglow::setup::Renderer::Solid { color: [8, 8, 16] };
    doc.layers[0].brightness = 255;
    doc.layers[1].brightness = 255;
    doc.layers[1].variants[0].renderer = underglow::setup::Renderer::Solid { color: [0, 0, 255] };
    doc.layers[1].variants[1].renderer = underglow::setup::Renderer::Solid { color: [0, 255, 0] };
    doc
}

fn action(doc: &SetupDocument, registry: &CapabilityRegistry, label: &str) -> Action {
    choices(&doc.layers[1], registry)
        .actions
        .into_iter()
        .find(|action| action.label == label)
        .unwrap()
}

fn key(preview: &Preview) -> Color {
    preview
        .frame()
        .get_coord(underglow::key_targets::canonical_coord("F1").unwrap())
}

#[test]
fn pi_choices_keep_states_and_occurrences_distinct_and_finished_can_be_first_action() {
    let doc = document();
    let original = doc.to_toml().unwrap();
    let registry = CapabilityRegistry::with_builtins();
    let offered = choices(&doc.layers[1], &registry);
    assert_eq!(
        offered
            .actions
            .iter()
            .map(|action| action.label.as_str())
            .collect::<Vec<_>>(),
        ["Idle", "Working", "Finished"]
    );
    assert!(offered.warnings.is_empty());
    let mut preview = Preview::default();
    let raw = super::super::LiveSample {
        generation: (1, 2),
        now: Duration::ZERO,
        snapshot: Snapshot {
            revision: 71,
            event_cursor: 41,
            states: vec![],
        },
        events: EventBatch {
            cursor: 41,
            gap: false,
            events: vec![],
        },
        received_at: std::time::Instant::now(),
        connected: true,
        stale: false,
    };
    preview.set_sample(Some(raw.clone()));
    let finished = action(&doc, &registry, "Finished");
    preview
        .run_quick(&doc, &doc.layers[1], &finished, 255)
        .unwrap();
    assert!(preview.quick_active());
    assert!(preview.scenario_playing());
    assert!(
        preview.evaluation_facts().0.states.is_empty(),
        "an event must not create presence/state"
    );
    assert_eq!(preview.evaluation_facts().1.events.len(), 1);
    assert_eq!(preview.evaluation_facts().1.events[0].event, "finished");
    assert_eq!(key(&preview), Color::new(0, 255, 0));
    let coord = underglow::key_targets::canonical_coord("F1").unwrap();
    let explanation = crate::setup_explanation::report(&preview, coord).unwrap();
    assert_eq!(
        explanation.color,
        preview.coverage().unwrap().frame.get_coord(coord)
    );
    assert_eq!(
        explanation.layers[1].selected_variant.as_deref(),
        Some("finished")
    );
    let idle = action(&doc, &registry, "Idle");
    preview.run_quick(&doc, &doc.layers[1], &idle, 255).unwrap();
    assert!(preview.evaluation_facts().1.events.is_empty());
    assert_eq!(preview.evaluation_facts().0.states[0].state, "idle");
    assert_eq!(
        key(&preview),
        Color::new(0, 36, 48),
        "Idle clears the outgoing finished transient immediately"
    );
    let working = action(&doc, &registry, "Working");
    preview
        .run_quick(&doc, &doc.layers[1], &working, 255)
        .unwrap();
    assert_eq!(key(&preview), Color::new(0, 0, 255));
    assert_eq!(preview.sample().unwrap().snapshot, raw.snapshot);
    assert_eq!(preview.sample().unwrap().events, raw.events);
    assert_eq!(doc.to_toml().unwrap(), original);
}

#[test]
fn quick_expiry_play_pause_reset_and_rebaseline_never_renew_or_replay_transients() {
    let doc = document();
    let registry = CapabilityRegistry::with_builtins();
    let finished = action(&doc, &registry, "Finished");
    let mut preview = Preview::default();
    preview
        .run_quick(&doc, &doc.layers[1], &finished, 255)
        .unwrap();
    let deadline = preview.coverage().unwrap().layers[1].deadline.unwrap().at;
    assert_eq!(deadline, Duration::from_millis(finished.ttl_ms));
    let anchor = preview.scenario_tick.unwrap();
    preview.advance_playback_at(anchor + Duration::from_millis(1999));
    preview.update_at(Some(&doc), 255, Duration::from_secs(900));
    assert_eq!(key(&preview), Color::new(0, 255, 0));
    preview.set_scenario_playing(false);
    let frozen = preview.frame().clone();
    preview.advance_playback_at(anchor + Duration::from_secs(9000));
    preview.update_at(Some(&doc), 255, Duration::from_secs(10000));
    assert_eq!(preview.frame(), &frozen);
    assert_eq!(
        preview.coverage().unwrap().layers[1].deadline.unwrap().at,
        deadline
    );
    preview.quick.scenario.advance_to(deadline).unwrap();
    preview.update_at(Some(&doc), 255, Duration::ZERO);
    assert_eq!(key(&preview), Color::new(8, 8, 16));
    preview
        .run_quick(&doc, &doc.layers[1], &finished, 255)
        .unwrap();
    preview.rebaseline(); // same hook used by Save/Discard/document replacement
    preview.update_at(Some(&doc), 255, Duration::ZERO);
    assert_eq!(key(&preview), Color::new(8, 8, 16));
    assert!(preview.quick.action.is_none());
    assert!(!preview.scenario_playing());
    preview
        .run_quick(&doc, &doc.layers[1], &finished, 255)
        .unwrap();
    preview.reset_quick();
    preview.update_at(Some(&doc), 255, Duration::ZERO);
    assert!(preview.evaluation_facts().1.events.is_empty());
    assert_eq!(key(&preview), Color::new(8, 8, 16));
}

#[test]
fn quick_and_live_contexts_retain_advanced_channels_but_never_replay_their_events() {
    let doc = document();
    let mut preview = Preview::default();
    preview.set_mode(Mode::Scenario);
    preview.update_at(Some(&doc), 255, Duration::ZERO);
    preview
        .scenario
        .publish_state("ci", "a", "busy", 10000)
        .unwrap();
    preview
        .scenario
        .publish_state("pi", "b", "working", 10000)
        .unwrap();
    preview
        .scenario
        .publish_event("pi", "b", "finished", 2000)
        .unwrap();
    preview
        .scenario
        .advance_by(Duration::from_millis(100))
        .unwrap();
    preview.update_at(Some(&doc), 255, Duration::ZERO);
    let retained = (
        preview.scenario.snapshot(),
        preview.scenario.events_since(0),
        preview.scenario.now(),
        preview.scenario.generation(),
    );
    let idle = action(&doc, &CapabilityRegistry::with_builtins(), "Idle");
    preview.run_quick(&doc, &doc.layers[1], &idle, 255).unwrap();
    assert_eq!(preview.evaluation_facts().0.states.len(), 1);
    assert_eq!(
        preview.evaluation_facts().0.states[0].identity.instance,
        "quick-preview"
    );
    preview.set_mode(Mode::Live);
    preview.update_at(Some(&doc), 255, Duration::ZERO);
    assert!(!preview.quick_active());
    preview.set_mode(Mode::Scenario);
    preview.update_at(Some(&doc), 255, Duration::ZERO);
    assert_eq!(
        (
            preview.scenario.snapshot(),
            preview.scenario.events_since(0),
            preview.scenario.now(),
            preview.scenario.generation()
        ),
        retained
    );
    assert_eq!(
        key(&preview),
        Color::new(0, 0, 255),
        "retained Working state resumes, old Finished occurrence does not replay"
    );
    preview
        .advance_scenario_by(Duration::from_millis(100))
        .unwrap();
    assert_eq!(preview.scenario.now(), Duration::from_millis(200));
    assert!(!preview.scenario_playing());
}

#[test]
fn quick_state_lease_and_background_animation_share_one_finite_private_clock() {
    let mut doc = document();
    doc.layers[0].renderer = underglow::setup::Renderer::Pulse {
        color: [255, 255, 255],
        period_ms: 1000,
    };
    let mut preview = Preview::default();
    let working = action(&doc, &CapabilityRegistry::with_builtins(), "Working");
    preview
        .run_quick(&doc, &doc.layers[1], &working, 255)
        .unwrap();
    let lease = preview.evaluation_facts().0.states[0].expires_at;
    let a = underglow::key_targets::canonical_coord("A").unwrap();
    let initial = preview.frame().get_coord(a);
    let anchor = preview.scenario_tick.unwrap();
    preview.advance_playback_at(anchor + Duration::from_millis(250));
    preview.update_at(Some(&doc), 255, Duration::from_secs(99));
    assert_ne!(
        preview.frame().get_coord(a),
        initial,
        "background animation keeps advancing under the quick overlay"
    );
    assert_eq!(preview.evaluation_facts().0.states[0].expires_at, lease);
    assert_eq!(preview.evaluation_now(), preview.evaluation_elapsed());
    preview.quick.scenario.advance_to(lease).unwrap();
    preview.update_at(Some(&doc), 255, Duration::ZERO);
    assert!(preview.evaluation_facts().0.states.is_empty());
    assert!(
        preview.evaluation_facts().1.events.is_empty(),
        "absence must not synthesize Finished"
    );
    assert!(
        preview.coverage().unwrap().layers[1]
            .contributing
            .is_empty()
    );
}

#[test]
fn configured_unknown_facts_are_not_promoted_and_custom_transitions_keep_exact_identity() {
    let mut doc = document();
    let mut descriptor = underglow::source_capabilities::pi_descriptor();
    descriptor.source = "build".into();
    descriptor.display_name = "Build".into();
    descriptor.states[0].id = "ready".into();
    descriptor.states[0].label = "Ready".into();
    descriptor.states[1].id = "busy".into();
    descriptor.states[1].label = "Busy".into();
    descriptor.events.clear();
    let mut registry = CapabilityRegistry::new();
    registry.insert(descriptor).unwrap();
    let status = &mut doc.layers[1];
    status.activation = Activation::Always {};
    status.variants.truncate(1);
    status.variants[0].activation = Activation::OnTransition {
        source: "build".into(),
        instance: Some("exact-session".into()),
        from_state: Some("ready".into()),
        state: "busy".into(),
        duration_ms: 1000,
    };
    let offered = choices(status, &registry);
    assert_eq!(offered.actions.len(), 1);
    assert_eq!(offered.actions[0].label, "Ready → Busy");
    let mut preview = Preview::default();
    preview.set_capabilities(registry.clone());
    preview
        .run_quick(&doc, &doc.layers[1], &offered.actions[0], 255)
        .unwrap();
    assert_eq!(
        preview.evaluation_facts().0.states[0].identity.instance,
        "exact-session"
    );
    assert_eq!(preview.evaluation_facts().0.states[0].state, "busy");
    assert!(preview.evaluation_facts().1.events.is_empty());
    assert_eq!(key(&preview), Color::new(0, 0, 255));
    doc.layers[1].variants[0].activation = Activation::WhileState {
        source: "build".into(),
        instance: None,
        state: Some("approved".into()),
    };
    let offered = choices(&doc.layers[1], &registry);
    assert!(offered.actions.is_empty());
    assert!(!offered.warnings.is_empty());
    let before = preview.evaluation_facts().0.clone();
    let stale = Action {
        label: "Approved".into(),
        detail: "Approved".into(),
        source_label: "Build".into(),
        activation: doc.layers[1].variants[0].activation.clone(),
        ttl_ms: 15000,
    };
    assert!(
        preview
            .run_quick(&doc, &doc.layers[1], &stale, 255)
            .is_err()
    );
    assert_eq!(preview.evaluation_facts().0, &before);
}
