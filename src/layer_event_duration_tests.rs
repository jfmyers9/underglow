//! Configured canonical event windows, using only synthetic registry facts.
use super::*;
use crate::external::{Message, Payload, Registry};
use crate::setup_library::{Starter, starter};

fn fixture(duration_ms: u64, base: bool) -> (SetupDocument, CompiledSetup, Registry) {
    let mut doc = starter(Starter::MatrixPi, "duration-test".into(), "Duration".into()).unwrap();
    let layer = &mut doc.layers[1];
    if let Activation::OnEvent {
        duration_ms: value, ..
    } = &mut layer.variants[1].activation
    {
        *value = duration_ms;
    }
    if base {
        let finished = layer.variants.pop().unwrap();
        layer.activation = finished.activation;
        layer.renderer = finished.renderer;
        layer.variants.clear();
    }
    let registry = Registry::default();
    let mut runtime = CompiledSetup::compile(&doc).unwrap();
    runtime.baseline(&registry.snapshot(), Duration::ZERO);
    (doc, runtime, registry)
}

fn publish(registry: &mut Registry, id: &str, sequence: u64, payload: Payload, now: u64) {
    registry
        .apply(
            Message {
                schema_version: 1,
                source: "pi".into(),
                instance: id.into(),
                sequence,
                payload,
            },
            Duration::from_millis(now),
        )
        .unwrap();
}
fn state(registry: &mut Registry, id: &str, sequence: u64, ttl_ms: u64, now: u64) {
    publish(
        registry,
        id,
        sequence,
        Payload::State {
            state: "idle".into(),
            ttl_ms,
        },
        now,
    );
}
fn finish(registry: &mut Registry, sequence: u64, now: u64) {
    publish(
        registry,
        "a",
        sequence,
        Payload::Event {
            event: "finished".into(),
            ttl_ms: 2000,
        },
        now,
    );
}
fn observe(runtime: &mut CompiledSetup, registry: &mut Registry, now: u64) {
    registry.expire(Duration::from_millis(now)).unwrap();
    runtime.observe(
        &registry.snapshot(),
        &registry.events_since(runtime.event_cursor()),
        Duration::from_millis(now),
    );
}
fn deadline(runtime: &CompiledSetup) -> Option<Duration> {
    runtime
        .selected_appearance("pi-status")
        .and_then(|a| a.expires)
}

#[test]
fn configured_event_windows_honor_short_long_and_maximum_durations_without_extending_on_renewal() {
    for base in [false, true] {
        for duration in [750, 2000, 5000, 30_000, 60_000] {
            let (doc, mut runtime, mut registry) = fixture(duration, base);
            let original = doc.to_toml().unwrap();
            state(&mut registry, "a", 1, 15_000, 0);
            finish(&mut registry, 2, 10);
            observe(&mut runtime, &mut registry, 10);
            let end = Duration::from_millis(10 + duration);
            assert_eq!(
                deadline(&runtime),
                Some(end),
                "base={base}, duration={duration}"
            );
            for tick in (5000..10 + duration).step_by(5000) {
                state(&mut registry, "a", tick, 15_000, tick);
                observe(&mut runtime, &mut registry, tick);
                assert_eq!(
                    deadline(&runtime),
                    Some(end),
                    "renewal must not restart the animation"
                );
            }
            observe(&mut runtime, &mut registry, 9 + duration);
            assert_eq!(deadline(&runtime), Some(end));
            observe(&mut runtime, &mut registry, 10 + duration);
            assert_eq!(deadline(&runtime), None, "configured deadline is exclusive");
            assert_eq!(doc.to_toml().unwrap(), original);
            assert_eq!(SetupDocument::from_toml(&original).unwrap(), doc);
        }
    }
}

#[test]
fn delayed_observation_and_duplicate_batches_never_restart_an_accepted_event() {
    let (_, mut runtime, mut registry) = fixture(5000, false);
    state(&mut registry, "a", 1, 15_000, 0);
    finish(&mut registry, 2, 1000);
    observe(&mut runtime, &mut registry, 1500);
    assert_eq!(deadline(&runtime), Some(Duration::from_millis(6000)));
    runtime.observe(
        &registry.snapshot(),
        &registry.events_since(0),
        Duration::from_millis(1800),
    );
    assert_eq!(deadline(&runtime), Some(Duration::from_millis(6000)));
    runtime.baseline(&registry.snapshot(), Duration::from_millis(1900));
    runtime.observe(
        &registry.snapshot(),
        &registry.events_since(0),
        Duration::from_millis(1950),
    );
    assert_eq!(
        deadline(&runtime),
        None,
        "resume must not replay even a still-fresh occurrence"
    );
}

#[test]
fn expired_unseen_events_cannot_start_a_long_configured_window() {
    let (_, mut runtime, mut registry) = fixture(60_000, false);
    state(&mut registry, "a", 1, 15_000, 0);
    finish(&mut registry, 2, 10);
    observe(&mut runtime, &mut registry, 2010);
    assert_eq!(deadline(&runtime), None);
}

#[test]
fn removal_expiry_and_renewal_after_a_gap_stop_the_emitting_sessions_window() {
    for base in [false, true] {
        for reason in ["remove", "expire", "gap"] {
            let (_, mut runtime, mut registry) = fixture(5000, base);
            state(&mut registry, "a", 1, 100, 0);
            state(&mut registry, "other", 1, 15_000, 0);
            finish(&mut registry, 2, 10);
            observe(&mut runtime, &mut registry, 10);
            assert_eq!(deadline(&runtime), Some(Duration::from_millis(5010)));
            let now = match reason {
                "remove" => {
                    publish(&mut registry, "a", 3, Payload::Remove, 50);
                    50
                }
                "gap" => {
                    state(&mut registry, "a", 3, 15_000, 150);
                    150
                }
                _ => 100,
            };
            observe(&mut runtime, &mut registry, now);
            assert_eq!(
                deadline(&runtime),
                None,
                "{reason}: another session cannot keep a stale completion alive"
            );
        }
    }
}

#[test]
fn event_only_windows_are_finite_and_never_create_presence() {
    let (_, mut runtime, mut registry) = fixture(5000, false);
    finish(&mut registry, 1, 10);
    observe(&mut runtime, &mut registry, 10);
    assert!(registry.snapshot().states.is_empty());
    observe(&mut runtime, &mut registry, 2010);
    assert_eq!(deadline(&runtime), Some(Duration::from_millis(5010)));
    observe(&mut runtime, &mut registry, 5010);
    assert_eq!(deadline(&runtime), None);
    assert!(registry.snapshot().states.is_empty());
}
