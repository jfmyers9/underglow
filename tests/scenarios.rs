use std::time::Duration;
use underglow::external::{ApplyOutcome, Error, Limits, Message, Payload, Registry};
use underglow::scenarios::Scenario;

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

#[test]
fn independent_sessions_sources_clocks_sequences_and_reset() {
    let mut scenario = Scenario::default();
    let other = Scenario::default();
    let live = Registry::default();
    let original = live.snapshot();
    scenario.publish_state("custom", "a", "idle", 100).unwrap();
    scenario.publish_state("custom", "b", "busy", 200).unwrap();
    scenario
        .publish_state("another", "a", "online", 300)
        .unwrap();
    scenario.publish_event("custom", "b", "done", 500).unwrap();
    assert_eq!(scenario.snapshot().states.len(), 3);
    assert_eq!(scenario.events_since(0).events[0].sequence, 4);
    assert_eq!(scenario.events_since(0), scenario.events_since(0));
    scenario.advance_to(ms(100)).unwrap();
    assert_eq!(scenario.snapshot().states.len(), 2);
    scenario.remove("custom", "b").unwrap();
    assert_eq!(scenario.snapshot().states.len(), 1);
    assert_eq!(
        scenario.events_since(0).events.len(),
        1,
        "removal keeps events"
    );
    scenario.advance_by(ms(400)).unwrap();
    assert!(scenario.snapshot().states.is_empty());
    assert!(scenario.events_since(0).events.is_empty());
    assert!(scenario.events_since(0).gap);
    assert_eq!(live.snapshot(), original);
    assert_eq!(other.now(), Duration::ZERO);
    assert_eq!(other.snapshot(), original);
    scenario.reset();
    assert_eq!(scenario.generation(), 1);
    assert_eq!(scenario.now(), Duration::ZERO);
    assert_eq!(scenario.snapshot(), original);
    assert!(!scenario.events_since(0).gap);
    scenario.publish_state("custom", "a", "idle", 1).unwrap();
    assert_eq!(scenario.snapshot().states[0].sequence, 1);
}

#[test]
fn invalid_messages_capacity_clocks_and_sequences_are_atomic_and_repairable() {
    let mut scenario = Scenario::new(Limits {
        max_active_instances: 1,
        ..Limits::default()
    })
    .unwrap();
    scenario.publish_state("custom", "a", "idle", 100).unwrap();
    let snapshot = scenario.snapshot();
    let events = scenario.events_since(0);
    assert_eq!(
        scenario.publish_state("custom", "b", "busy", 100),
        Err(Error::Capacity("active instance"))
    );
    for (source, instance, label, ttl) in [
        ("bad source", "a", "idle", 100),
        ("custom", "", "idle", 100),
        ("custom", "a", "bad label", 100),
        ("custom", "a", "idle", 0),
        ("custom", "a", "idle", 300_001),
    ] {
        assert!(
            scenario
                .publish_state(source, instance, label, ttl)
                .is_err()
        );
        assert_eq!(scenario.snapshot(), snapshot);
        assert_eq!(scenario.events_since(0), events);
        assert_eq!(scenario.now(), Duration::ZERO);
    }
    let message = Message {
        schema_version: 2,
        source: "custom".into(),
        instance: "a".into(),
        sequence: 200,
        payload: Payload::Event {
            event: "done".into(),
            ttl_ms: 100,
        },
    };
    assert_eq!(
        scenario.apply(message.clone()),
        Err(Error::UnsupportedVersion(2))
    );
    assert_eq!(
        scenario
            .apply(Message {
                schema_version: 1,
                sequence: 1,
                ..message.clone()
            })
            .unwrap(),
        ApplyOutcome::IgnoredStale
    );
    scenario.publish_event("custom", "a", "done", 100).unwrap();
    assert_eq!(
        scenario.events_since(0).events[0].sequence,
        2,
        "errors never consume sequence"
    );
    scenario
        .apply(Message {
            schema_version: 1,
            ..message
        })
        .unwrap();
    scenario.publish_state("custom", "a", "busy", 100).unwrap();
    assert_eq!(scenario.snapshot().states[0].sequence, 201);
    scenario.advance_to(ms(10)).unwrap();
    let before = scenario.snapshot();
    assert_eq!(scenario.advance_to(ms(9)), Err(Error::ClockWentBackwards));
    assert_eq!(
        scenario.advance_by(Duration::MAX),
        Err(Error::ClockOverflow)
    );
    assert_eq!(scenario.now(), ms(10));
    assert_eq!(scenario.snapshot(), before);
    scenario.advance_to(Duration::MAX).unwrap();
    let before = scenario.snapshot();
    assert_eq!(
        scenario.publish_event("custom", "a", "done", 1),
        Err(Error::ClockOverflow)
    );
    assert_eq!(scenario.snapshot(), before);
}

#[test]
fn all_channels_share_protocol_capacity_and_bounded_event_retention() {
    let mut scenario = Scenario::new(Limits {
        max_sources: 2,
        max_active_instances: 2,
        max_tracked_instances: 2,
        max_events: 2,
        ..Limits::default()
    })
    .unwrap();
    scenario.publish_state("pi", "a", "working", 1000).unwrap();
    scenario.publish_state("ci", "b", "busy", 1000).unwrap();
    let before = scenario.snapshot();
    assert!(scenario.publish_state("pi", "c", "working", 1000).is_err());
    assert!(
        scenario
            .publish_state("other", "a", "working", 1000)
            .is_err()
    );
    assert_eq!(scenario.snapshot(), before);
    for (source, instance, event) in [
        ("pi", "a", "finished"),
        ("ci", "b", "failed"),
        ("pi", "a", "finished"),
    ] {
        scenario
            .publish_event(source, instance, event, 1000)
            .unwrap();
    }
    let batch = scenario.events_since(0);
    assert!(batch.gap);
    assert_eq!(batch.events.len(), 2);
    assert_eq!(batch.events[0].identity.source, "ci");
    assert_eq!(batch.events[1].identity.source, "pi");
    assert!(scenario.events_since(batch.cursor).events.is_empty());
    scenario.advance_to(ms(1000)).unwrap();
    assert!(scenario.events_since(0).events.is_empty());
    assert!(scenario.snapshot().states.is_empty());
}
