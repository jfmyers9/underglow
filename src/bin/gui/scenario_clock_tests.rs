//! Hardware-free tests against the actual preview evaluator and compositor.
use super::*;
use underglow::render::Color;

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn color(preview: &Preview, key: &str) -> Color {
    preview
        .frame()
        .get_coord(underglow::key_targets::canonical_coord(key).unwrap())
}

fn document() -> SetupDocument {
    serde_json::from_value(serde_json::json!({"schema_version":1,"id":"clock","name":"Clock","layers":[
        {"id":"background","name":"Background","renderer":{"kind":"solid","color":[8,8,16]},"target":{"kind":"whole"}},
        {"id":"pi-a","name":"Pi A","renderer":{"kind":"solid","color":[0,0,255]},"target":{"kind":"selection","keys":["A"]},"activation":{"kind":"while-state","source":"pi","instance":"a","state":"working"}},
        {"id":"pi-b","name":"Pi B","renderer":{"kind":"solid","color":[0,255,255]},"target":{"kind":"selection","keys":["C"]},"activation":{"kind":"while-state","source":"pi","instance":"b","state":"working"}},
        {"id":"ci-b","name":"CI B","renderer":{"kind":"solid","color":[255,0,255]},"target":{"kind":"selection","keys":["B"]},"activation":{"kind":"while-state","source":"ci","instance":"b","state":"busy"}},
        {"id":"alert","name":"Other alert","renderer":{"kind":"solid","color":[255,0,0]},"target":{"kind":"selection","keys":["A"]},"activation":{"kind":"on-event","source":"ci","instance":"a","event":"failed","duration_ms":1000}},
        {"id":"completion","name":"Pi completion","renderer":{"kind":"solid","color":[0,255,0]},"target":{"kind":"selection","keys":["A"]},"activation":{"kind":"on-event","source":"pi","instance":"a","event":"finished","duration_ms":200}}
    ]})).unwrap()
}

fn prepared(doc: &SetupDocument) -> Preview {
    let mut preview = Preview::default();
    preview.set_mode(Mode::Scenario);
    preview.update_at(Some(doc), 255, Duration::ZERO);
    preview
}

fn render(preview: &mut Preview, doc: &SetupDocument) {
    // Wall elapsed deliberately differs from scenario time.
    preview.update_at(Some(doc), 255, Duration::from_secs(400));
}

#[test]
fn status_transition_scenario_has_exact_composed_rgb_explanations_and_no_hardware_intent() {
    let mut doc = document();
    doc.layers.truncate(1);
    let status: underglow::setup::Layer = serde_json::from_value(serde_json::json!({
        "id":"status","name":"Status","brightness":255,"opacity":128,
        "transition":{"duration_ms":100,"curve":"linear"},
        "target":{"kind":"selection","keys":["A"]},
        "activation":{"kind":"while-state","source":"pi","instance":"a"},
        "renderer":{"kind":"solid","color":[0,0,0]},
        "variants":[
            {"id":"idle","name":"Idle","activation":{"kind":"while-state","source":"pi","instance":"a","state":"idle"},"renderer":{"kind":"solid","color":[0,0,200]}},
            {"id":"working","name":"Working","activation":{"kind":"while-state","source":"pi","instance":"a","state":"working"},"renderer":{"kind":"solid","color":[200,0,0]}},
            {"id":"finished","name":"Finished","activation":{"kind":"on-event","source":"pi","instance":"a","event":"finished","duration_ms":300},"renderer":{"kind":"solid","color":[0,200,0]}}
        ]
    })).unwrap();
    doc.layers.push(status);
    let mut p = prepared(&doc);
    state(&mut p, "pi", "a", "idle", 1000);
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(4, 4, 108));
    p.advance_scenario_by(ms(100)).unwrap();
    state(&mut p, "pi", "a", "working", 1000);
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(4, 4, 108));
    p.advance_scenario_by(ms(50)).unwrap();
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(54, 4, 58));
    let a = underglow::key_targets::canonical_coord("A").unwrap();
    let explanation = super::super::setup_explanation::report(&p, a).unwrap();
    assert_eq!(explanation.color, color(&p, "A"));
    assert_eq!(
        explanation.layers[1].selected_variant.as_deref(),
        Some("working")
    );
    let transition = explanation.layers[1].transition.as_ref().unwrap();
    assert_eq!(
        (&*transition.from, &*transition.to, transition.progress),
        ("idle", "working", 0.5)
    );
    assert!(
        !explanation.layers[1]
            .conditions
            .iter()
            .find(|c| c.variant_id.as_deref() == Some("idle"))
            .unwrap()
            .matched
    );
    p.scenario
        .publish_event("pi", "a", "finished", 1000)
        .unwrap();
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(54, 4, 58)); // no flash on interruption
    let deadline = p.coverage().unwrap().layers[1].deadline.unwrap().at;
    p.advance_scenario_by(ms(50)).unwrap();
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(29, 54, 33));
    let frozen = p.frame().clone();
    p.update_at(Some(&doc), 255, Duration::from_secs(9999));
    assert_eq!(p.frame(), &frozen); // paused evaluation clock, not wall elapsed
    assert!(!p.scenario_playing());
    assert!(p.sample().is_none()); // no real source transport was changed
    assert_eq!(
        p.coverage().unwrap().layers[1].deadline.unwrap().at,
        deadline
    );
    p.scenario.advance_to(deadline).unwrap();
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(104, 4, 8)); // exact release, no expiry fade
    assert!(p.coverage().unwrap().layers[1].transition.is_none());
    p.rebaseline();
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(104, 4, 8));
    p.reset_scenario();
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(8, 8, 16));
    assert!(p.coverage().unwrap().layers[1].transition.is_none());
}

fn state(preview: &mut Preview, source: &str, instance: &str, state: &str, ttl: u64) {
    preview
        .scenario
        .publish_state(source, instance, state, ttl)
        .unwrap();
}

#[test]
fn pi_completion_during_alert_restores_remaining_layer_and_disjoint_sessions() {
    let doc = document();
    let mut p = prepared(&doc);
    state(&mut p, "pi", "a", "working", 1500);
    state(&mut p, "pi", "b", "working", 700);
    state(&mut p, "ci", "a", "busy", 2000);
    state(&mut p, "ci", "b", "busy", 500);
    p.scenario.publish_event("ci", "a", "failed", 2000).unwrap();
    render(&mut p, &doc);
    assert_eq!(p.evaluation_facts().0.states.len(), 4);
    assert_eq!(color(&p, "A"), Color::new(255, 0, 0));
    assert_eq!(color(&p, "B"), Color::new(255, 0, 255));
    assert_eq!(color(&p, "C"), Color::new(0, 255, 255));
    p.advance_scenario_by(ms(100)).unwrap();
    p.scenario
        .publish_event("pi", "a", "finished", 1500)
        .unwrap();
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(0, 255, 0));
    let expiry = p
        .runtime()
        .unwrap()
        .selected_appearance("completion")
        .unwrap()
        .expires;
    for (time, a, b, c) in [
        (299, [0, 255, 0], [255, 0, 255], [0, 255, 255]),
        (300, [255, 0, 0], [255, 0, 255], [0, 255, 255]),
        (500, [255, 0, 0], [8, 8, 16], [0, 255, 255]),
        (700, [255, 0, 0], [8, 8, 16], [8, 8, 16]),
        (999, [255, 0, 0], [8, 8, 16], [8, 8, 16]),
        (1000, [0, 0, 255], [8, 8, 16], [8, 8, 16]),
        (1500, [8, 8, 16], [8, 8, 16], [8, 8, 16]),
    ] {
        // Same-state heartbeat must not renew a consumed event deadline.
        if time == 299 {
            state(&mut p, "pi", "a", "working", 1400);
        }
        p.scenario.advance_to(ms(time)).unwrap();
        render(&mut p, &doc);
        assert_eq!(color(&p, "A"), Color::new(a[0], a[1], a[2]), "A at {time}");
        assert_eq!(color(&p, "B"), Color::new(b[0], b[1], b[2]), "B at {time}");
        assert_eq!(color(&p, "C"), Color::new(c[0], c[1], c[2]), "C at {time}");
        if time == 299 {
            assert_eq!(
                p.runtime()
                    .unwrap()
                    .selected_appearance("completion")
                    .unwrap()
                    .expires,
                expiry
            );
        }
    }
}

#[test]
fn coverage_countdowns_and_obscuration_follow_evaluation_clock_not_wall_time() {
    use underglow::explanations::{CoverageStatus, DeadlineKind};
    let doc = document();
    let mut p = prepared(&doc);
    state(&mut p, "pi", "a", "working", 400);
    p.scenario
        .publish_event("pi", "a", "finished", 150)
        .unwrap();
    render(&mut p, &doc);
    let generation = p.scenario.generation();
    let baseline_frame = p.frame().clone();
    let baseline_deadline = p.coverage().unwrap().layers[5].deadline.unwrap();
    assert_eq!(baseline_deadline.kind, DeadlineKind::Transient);
    assert_eq!(
        baseline_deadline.remaining,
        ms(150),
        "trigger TTL caps duration"
    );
    assert_eq!(
        p.coverage().unwrap().layers[1].status,
        CoverageStatus::FullyObscured
    );
    p.advance_playback_at(Instant::now() + Duration::from_secs(900));
    p.update_at(Some(&doc), 255, Duration::from_secs(1000));
    assert_eq!(p.coverage().unwrap().evaluated_at, Duration::ZERO);
    assert_eq!(
        p.coverage().unwrap().layers[5].deadline,
        Some(baseline_deadline)
    );
    assert_eq!(
        p.coverage().unwrap().layers[1].deadline.unwrap().remaining,
        ms(400)
    );
    assert_eq!(p.frame(), &baseline_frame);
    assert_eq!(p.scenario.generation(), generation);
    for (time, remaining) in [(149, 1), (150, 0)] {
        p.scenario.advance_to(ms(time)).unwrap();
        render(&mut p, &doc);
        let coverage = p.coverage().unwrap();
        assert_eq!(coverage.evaluated_at, p.evaluation_now());
        assert_eq!(&coverage.frame, p.frame());
        if remaining > 0 {
            assert_eq!(
                coverage.layers[5].deadline.unwrap().remaining,
                ms(remaining)
            );
            assert_eq!(coverage.layers[1].status, CoverageStatus::FullyObscured);
        } else {
            assert!(coverage.layers[5].deadline.is_none());
            assert_eq!(coverage.layers[5].status, CoverageStatus::Inactive);
            assert_eq!(coverage.layers[1].status, CoverageStatus::Visible);
            assert_eq!(coverage.layers[1].deadline.unwrap().remaining, ms(250));
        }
    }
    p.scenario.advance_to(ms(400)).unwrap();
    render(&mut p, &doc);
    assert!(p.coverage().unwrap().layers[1].deadline.is_none());
    assert_eq!(
        p.coverage().unwrap().layers[1].status,
        CoverageStatus::Inactive
    );
    assert_eq!(color(&p, "A"), Color::new(8, 8, 16));
}

#[test]
fn paused_clock_freezes_all_animation_and_step_autopauses_without_catchup() {
    let mut doc = document();
    doc.layers[0].renderer = underglow::setup::Renderer::Pulse {
        color: [255, 255, 255],
        period_ms: 1000,
    };
    let mut p = prepared(&doc);
    state(&mut p, "pi", "a", "working", 100);
    render(&mut p, &doc);
    let original = p.frame().as_bytes().to_vec();
    let tick = Instant::now();
    p.advance_playback_at(tick + Duration::from_secs(900));
    render(&mut p, &doc);
    assert!(!p.scenario_playing());
    assert_eq!(p.scenario.now(), Duration::ZERO);
    assert_eq!(p.frame().as_bytes(), original);
    assert_eq!(p.evaluation_now(), Duration::ZERO);
    assert_eq!(p.evaluation_elapsed(), Duration::ZERO);
    assert_eq!(p.evaluation_brightness(), 255);
    p.set_scenario_playing(true);
    let anchor = p.scenario_tick.unwrap();
    p.advance_playback_at(anchor + ms(50));
    render(&mut p, &doc);
    assert_eq!(p.scenario.now(), ms(50));
    assert_eq!(color(&p, "A"), Color::new(0, 0, 255));
    p.advance_scenario_by(ms(50)).unwrap();
    assert!(!p.scenario_playing());
    p.advance_playback_at(anchor + Duration::from_secs(900));
    render(&mut p, &doc);
    assert_eq!(p.scenario.now(), ms(100));
    assert_ne!(p.frame().as_bytes(), original);
    assert!(p.scenario.snapshot().states.is_empty());
    let stepped = p.frame().as_bytes().to_vec();
    p.update_at(Some(&doc), 255, Duration::from_secs(1000));
    assert_eq!(p.frame().as_bytes(), stepped);
    p.set_scenario_playing(true);
    let resumed = p.scenario_tick.unwrap();
    p.advance_playback_at(resumed + ms(10));
    assert_eq!(p.scenario.now(), ms(110));
    p.reset_scenario();
    assert!(!p.scenario_playing());
    assert_eq!(p.scenario.now(), Duration::ZERO);
}

#[test]
fn replay_fixture_is_identical_and_retention_ttl_caps_duration_without_replay() {
    let doc = document();
    let replay = || {
        let mut p = prepared(&doc);
        let mut frames = Vec::new();
        state(&mut p, "pi", "a", "working", 500);
        state(&mut p, "ci", "b", "busy", 300);
        p.scenario
            .publish_event("pi", "a", "finished", 100)
            .unwrap();
        for time in [0, 99, 100, 199, 200, 300, 500, 1000] {
            p.scenario.advance_to(ms(time)).unwrap();
            render(&mut p, &doc);
            frames.push((
                p.frame().as_bytes().to_vec(),
                p.evaluation_facts().0.clone(),
                p.evaluation_facts().1.clone(),
            ));
            assert_eq!(
                color(&p, "A"),
                if time < 100 {
                    Color::new(0, 255, 0)
                } else if time < 500 {
                    Color::new(0, 0, 255)
                } else {
                    Color::new(8, 8, 16)
                }
            );
        }
        frames
    };
    assert_eq!(replay(), replay());
}

#[test]
fn invalid_time_is_atomic_and_does_not_pause_or_extend_deadlines() {
    let doc = document();
    let mut p = prepared(&doc);
    state(&mut p, "pi", "a", "working", 500);
    p.scenario
        .publish_event("pi", "a", "finished", 500)
        .unwrap();
    p.advance_scenario_by(ms(10)).unwrap();
    render(&mut p, &doc);
    p.set_scenario_playing(true);
    let before = (
        p.scenario.snapshot(),
        p.scenario.events_since(0),
        p.scenario_tick,
    );
    assert_eq!(
        p.advance_scenario_by(Duration::MAX),
        Err(SourceError::ClockOverflow)
    );
    assert_eq!(
        p.scenario.advance_to(ms(9)),
        Err(SourceError::ClockWentBackwards)
    );
    assert_eq!(p.scenario.now(), ms(10));
    assert!(p.scenario_playing());
    assert_eq!(
        (
            p.scenario.snapshot(),
            p.scenario.events_since(0),
            p.scenario_tick
        ),
        before
    );
}

#[test]
fn removing_one_channel_keeps_leased_event_without_replaying_on_reappearance() {
    let doc = document();
    let mut p = prepared(&doc);
    state(&mut p, "pi", "a", "working", 2000);
    state(&mut p, "pi", "b", "working", 2000);
    state(&mut p, "ci", "a", "busy", 2000);
    p.scenario.publish_event("ci", "a", "failed", 2000).unwrap();
    p.scenario
        .publish_event("pi", "a", "finished", 2000)
        .unwrap();
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(0, 255, 0));
    p.scenario.remove("pi", "a").unwrap();
    render(&mut p, &doc);
    // Top-level OnEvent keeps its leased occurrence after state removal; this
    // differs intentionally from the source-bound behavior-variant lifecycle.
    assert_eq!(color(&p, "A"), Color::new(0, 255, 0));
    assert_eq!(color(&p, "C"), Color::new(0, 255, 255));
    assert_eq!(
        p.evaluation_facts().1.events.len(),
        2,
        "retention is not activation"
    );
    p.advance_scenario_by(ms(100)).unwrap();
    state(&mut p, "pi", "a", "working", 2000);
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(0, 255, 0));
    p.advance_scenario_by(ms(100)).unwrap();
    render(&mut p, &doc);
    assert_eq!(
        color(&p, "A"),
        Color::new(255, 0, 0),
        "consumed completion cannot restart"
    );
    p.advance_scenario_by(ms(800)).unwrap();
    render(&mut p, &doc);
    assert_eq!(color(&p, "A"), Color::new(0, 0, 255));
}
