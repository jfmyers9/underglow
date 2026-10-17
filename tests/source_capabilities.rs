use std::time::Duration;
use underglow::external::{Message, Payload, Registry};
use underglow::setup::Activation;
use underglow::source_capabilities::*;

fn while_state(state: Option<&str>) -> Activation {
    Activation::WhileState {
        source: "pi".into(),
        instance: None,
        state: state.map(str::to_owned),
    }
}

#[test]
fn fixture_roundtrips_and_declares_only_verified_pi_facts() {
    let pi = pi_descriptor();
    assert_eq!(
        SourceDescriptor::from_toml(&pi.to_toml().unwrap()).unwrap(),
        pi
    );
    assert_eq!(
        SourceDescriptor::from_json(&serde_json::to_vec(&pi).unwrap()).unwrap(),
        pi
    );
    assert_eq!(pi.instances.semantics, InstanceSemantics::Session);
    assert_eq!(
        pi.instances.selectors,
        [InstanceSelector::Any, InstanceSelector::Exact]
    );
    assert_eq!(
        pi.states.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
        ["idle", "working"]
    );
    assert_eq!(
        pi.events.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
        ["finished"]
    );
    assert_eq!(
        pi.lifecycle.state_ttl,
        Some(TtlRange {
            min_ms: 15000,
            max_ms: 15000
        })
    );
    assert_eq!(
        pi.lifecycle.event_ttl,
        Some(TtlRange {
            min_ms: 1,
            max_ms: 2000
        })
    );
    assert_eq!(pi.lifecycle.renewal_interval_ms, Some(5000));
    let registry = CapabilityRegistry::with_builtins();
    for forbidden in [
        "finished",
        "focus",
        "focused",
        "success",
        "approval",
        "awaiting-approval",
    ] {
        assert_eq!(
            registry
                .validate_activation(&while_state(Some(forbidden)))
                .unwrap(),
            [CapabilityWarning::UndeclaredState {
                state: forbidden.into()
            }]
        );
    }
    for forbidden in ["success", "approval", "focus"] {
        let activation = Activation::OnEvent {
            source: "pi".into(),
            instance: None,
            event: forbidden.into(),
            duration_ms: 1000,
        };
        assert_eq!(
            registry.validate_activation(&activation).unwrap(),
            [CapabilityWarning::UndeclaredEvent {
                event: forbidden.into()
            }]
        );
    }
}

#[test]
fn generic_discovery_keeps_observations_distinct_and_never_learns_capabilities() {
    let registry = CapabilityRegistry::with_builtins();
    let choices = registry
        .choices(
            "pi",
            FactKind::State,
            &[
                "working".into(),
                "custom".into(),
                "custom".into(),
                "finished".into(),
            ],
        )
        .unwrap();
    assert_eq!(
        choices
            .iter()
            .map(|c| (c.id.as_str(), c.origin))
            .collect::<Vec<_>>(),
        [
            ("idle", ChoiceOrigin::Declared),
            ("working", ChoiceOrigin::Declared),
            ("custom", ChoiceOrigin::UnknownObserved),
            ("finished", ChoiceOrigin::UnknownObserved),
        ]
    );
    assert_eq!(
        registry.choices("pi", FactKind::Event, &[]).unwrap()[0].id,
        "finished"
    );
    assert_eq!(
        registry.choices("pi", FactKind::State, &[]).unwrap().len(),
        2
    );
    assert!(
        registry
            .choices("custom", FactKind::State, &[])
            .unwrap()
            .is_empty()
    );
    let unknown = registry
        .choices("custom", FactKind::Event, &["ping".into()])
        .unwrap();
    assert_eq!(unknown[0].origin, ChoiceOrigin::UnknownObserved);
    assert!(registry.descriptor("custom").is_none());
    assert!(
        registry
            .choices("pi", FactKind::State, &vec!["idle".into(); MAX_FACTS + 1])
            .is_err()
    );
    assert!(
        registry
            .choices("pi", FactKind::State, &["bad token".into()])
            .is_err()
    );
}

#[test]
fn custom_descriptors_work_without_pi_specific_editor_logic() {
    let mut descriptor = pi_descriptor();
    descriptor.source = "custom:build".into();
    descriptor.display_name = "Build monitor".into();
    descriptor.instances.semantics = InstanceSemantics::ProducerDefined;
    descriptor.states = vec![FactDescriptor {
        id: "queued".into(),
        label: "Queued build".into(),
    }];
    descriptor.events.clear();
    descriptor.lifecycle.event_ttl = None;
    let mut registry = CapabilityRegistry::new();
    registry.insert(descriptor.clone()).unwrap();
    assert_eq!(registry.descriptors().collect::<Vec<_>>(), [&descriptor]);
    let activation = Activation::WhileState {
        source: descriptor.source.clone(),
        instance: Some("worker".into()),
        state: Some("queued".into()),
    };
    assert!(
        registry
            .validate_activation(&activation)
            .unwrap()
            .is_empty()
    );
    let choices = registry
        .choices(&descriptor.source, FactKind::State, &[])
        .unwrap();
    assert_eq!(choices[0].label, "Queued build");
    assert_eq!(choices[0].origin, ChoiceOrigin::Declared);
    assert!(
        registry
            .choices(&descriptor.source, FactKind::Event, &[])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn semantic_warnings_preserve_repairable_selectors_and_check_both_transition_sides() {
    let empty = CapabilityRegistry::new();
    let activation = while_state(Some("working"));
    let before = activation.clone();
    assert_eq!(
        empty.validate_activation(&activation).unwrap(),
        [CapabilityWarning::MissingDescriptor {
            source: "pi".into()
        }]
    );
    assert_eq!(activation, before);
    let registry = CapabilityRegistry::with_builtins();
    for valid in [
        Activation::Always {},
        while_state(None),
        activation,
        Activation::OnEvent {
            source: "pi".into(),
            instance: Some("absent-session".into()),
            event: "finished".into(),
            duration_ms: 60000,
        },
        Activation::OnTransition {
            source: "pi".into(),
            instance: None,
            from_state: Some("working".into()),
            state: "idle".into(),
            duration_ms: 1,
        },
    ] {
        assert!(registry.validate_activation(&valid).unwrap().is_empty());
    }
    let transition = Activation::OnTransition {
        source: "pi".into(),
        instance: None,
        from_state: Some("success".into()),
        state: "finished".into(),
        duration_ms: 1000,
    };
    assert_eq!(
        registry.validate_activation(&transition).unwrap(),
        [
            CapabilityWarning::UndeclaredState {
                state: "success".into()
            },
            CapabilityWarning::UndeclaredState {
                state: "finished".into()
            },
        ]
    );
    let invalid = Activation::OnEvent {
        source: "unknown".into(),
        instance: None,
        event: "finished".into(),
        duration_ms: 0,
    };
    assert!(empty.validate_activation(&invalid).is_err());
}

#[test]
fn selector_and_presence_support_come_from_metadata() {
    let mut descriptor = pi_descriptor();
    descriptor.instances.selectors = vec![InstanceSelector::Exact];
    descriptor.states.clear();
    descriptor.lifecycle.state_ttl = None;
    descriptor.lifecycle.renewal_interval_ms = None;
    let mut registry = CapabilityRegistry::new();
    registry.insert(descriptor).unwrap();
    assert_eq!(
        registry.validate_activation(&while_state(None)).unwrap(),
        [
            CapabilityWarning::UnsupportedInstanceSelector {
                selector: InstanceSelector::Any
            },
            CapabilityWarning::UnsupportedPresence,
        ]
    );
    let mut descriptor = pi_descriptor();
    descriptor.instances.selectors = vec![InstanceSelector::Any];
    let mut registry = CapabilityRegistry::new();
    registry.insert(descriptor).unwrap();
    let exact = Activation::WhileState {
        source: "pi".into(),
        instance: Some("session".into()),
        state: Some("idle".into()),
    };
    assert_eq!(
        registry.validate_activation(&exact).unwrap(),
        [CapabilityWarning::UnsupportedInstanceSelector {
            selector: InstanceSelector::Exact
        },]
    );
}

#[test]
fn nested_unknown_fields_versions_and_executable_trust_are_rejected() {
    let base = serde_json::to_value(pi_descriptor()).unwrap();
    for path in [
        "",
        "/instances",
        "/lifecycle",
        "/lifecycle/state_ttl",
        "/lifecycle/event_ttl",
        "/states/0",
        "/events/0",
    ] {
        for forbidden in ["extra", "executable", "credentials", "trusted"] {
            let mut value = base.clone();
            value
                .pointer_mut(path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(forbidden.into(), true.into());
            assert!(
                SourceDescriptor::from_json(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{path}/{forbidden}"
            );
        }
    }
    for path in [
        "/schema_version",
        "/instances/semantics",
        "/instances/selectors/0",
        "/lifecycle/presence",
        "/lifecycle/absence",
        "/lifecycle/expiry",
        "/lifecycle/renewal",
        "/lifecycle/events",
        "/lifecycle/removal",
    ] {
        let mut value = base.clone();
        *value.pointer_mut(path).unwrap() = if path == "/schema_version" {
            2.into()
        } else {
            "unsupported".into()
        };
        assert!(
            SourceDescriptor::from_json(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{path}"
        );
    }
    // Unit enums serialize as strings, not internally tagged unit variants.
    // Even if presented in Serde's alternate external-tag representation,
    // their unit payload cannot smuggle ignored executable/trust fields.
    for path in [
        "/instances/semantics",
        "/instances/selectors/0",
        "/lifecycle/presence",
        "/lifecycle/absence",
        "/lifecycle/expiry",
        "/lifecycle/renewal",
        "/lifecycle/events",
        "/lifecycle/removal",
    ] {
        let mut value = base.clone();
        let tag = value.pointer(path).unwrap().as_str().unwrap().to_owned();
        let mut payload = serde_json::Map::new();
        payload.insert("trusted".into(), true.into());
        let mut variant = serde_json::Map::new();
        variant.insert(tag, payload.into());
        *value.pointer_mut(path).unwrap() = variant.into();
        assert!(
            SourceDescriptor::from_json(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{path}"
        );
    }
    let fixture = include_str!("../examples/pi-source-capabilities.toml");
    assert!(SourceDescriptor::from_toml(&format!("trusted = true\n{fixture}")).is_err());
    assert!(
        SourceDescriptor::from_toml(
            &fixture.replace("semantics =", "executable = 'bad'\nsemantics =")
        )
        .is_err()
    );
}

#[test]
fn bounded_validation_covers_direct_construction_and_input_before_parsing() {
    let mut bad = Vec::new();
    let pi = pi_descriptor();
    for source in ["", "bad source", "\n", &"x".repeat(129)] {
        let mut d = pi.clone();
        d.source = source.into();
        bad.push(d);
    }
    for name in [" ", "bad\nname", &"x".repeat(129)] {
        let mut d = pi.clone();
        d.display_name = name.into();
        bad.push(d);
    }
    let mut d = pi.clone();
    d.states[0].id = "bad token".into();
    bad.push(d);
    let mut d = pi.clone();
    d.events[0].label = "\0".into();
    bad.push(d);
    let mut d = pi.clone();
    d.states.push(d.states[0].clone());
    bad.push(d);
    let mut d = pi.clone();
    d.events.push(d.events[0].clone());
    bad.push(d);
    let mut d = pi.clone();
    d.states = vec![d.states[0].clone(); MAX_FACTS + 1];
    bad.push(d);
    let mut d = pi.clone();
    d.events = vec![d.events[0].clone(); MAX_FACTS + 1];
    bad.push(d);
    let mut d = pi.clone();
    d.instances.selectors.clear();
    bad.push(d);
    let mut d = pi.clone();
    d.instances.selectors = vec![InstanceSelector::Any; 2];
    bad.push(d);
    for ttl in [
        TtlRange {
            min_ms: 0,
            max_ms: 1,
        },
        TtlRange {
            min_ms: 2,
            max_ms: 1,
        },
        TtlRange {
            min_ms: 1,
            max_ms: MAX_TTL_MS + 1,
        },
    ] {
        let mut d = pi.clone();
        d.lifecycle.event_ttl = Some(ttl);
        bad.push(d);
    }
    let mut d = pi.clone();
    d.lifecycle.event_ttl = None;
    bad.push(d);
    let mut d = pi.clone();
    d.lifecycle.state_ttl = None;
    bad.push(d);
    for interval in [0, 15000, u64::MAX] {
        let mut d = pi.clone();
        d.lifecycle.renewal_interval_ms = Some(interval);
        bad.push(d);
    }
    for descriptor in bad {
        assert!(descriptor.validate().is_err(), "{descriptor:?}");
    }
    assert!(
        SourceDescriptor::from_json(&vec![b' '; MAX_DESCRIPTOR_BYTES + 1])
            .unwrap_err()
            .contains("64 KiB")
    );
    assert!(
        SourceDescriptor::from_toml(&" ".repeat(MAX_DESCRIPTOR_BYTES + 1))
            .unwrap_err()
            .contains("64 KiB")
    );
}

#[test]
fn valid_boundaries_and_total_serialized_size_are_enforced() {
    let mut descriptor = pi_descriptor();
    descriptor.source = "x".repeat(128);
    descriptor.display_name = "x".repeat(128);
    descriptor.lifecycle.renewal_interval_ms = None;
    descriptor.lifecycle.state_ttl = Some(TtlRange {
        min_ms: 1,
        max_ms: MAX_TTL_MS,
    });
    descriptor.lifecycle.event_ttl = descriptor.lifecycle.state_ttl;
    descriptor.states = (0..MAX_FACTS)
        .map(|i| FactDescriptor {
            id: format!("state-{i}"),
            label: format!("State {i}"),
        })
        .collect();
    descriptor.events = (0..MAX_FACTS)
        .map(|i| FactDescriptor {
            id: format!("event-{i}"),
            label: format!("Event {i}"),
        })
        .collect();
    descriptor.validate().unwrap();
    assert_eq!(
        SourceDescriptor::from_toml(&descriptor.to_toml().unwrap()).unwrap(),
        descriptor
    );
    for (i, fact) in descriptor
        .states
        .iter_mut()
        .chain(descriptor.events.iter_mut())
        .enumerate()
    {
        fact.id = format!("{i:0128}");
        fact.label = "x".repeat(128);
    }
    assert!(descriptor.validate().unwrap_err().contains("64 KiB"));
    let mut registry = CapabilityRegistry::new();
    assert!(registry.insert(descriptor).is_err());
    assert_eq!(registry.descriptors().count(), 0);
}

#[test]
fn registry_capacity_and_failed_insert_are_atomic() {
    let mut registry = CapabilityRegistry::with_builtins();
    let mut duplicate = pi_descriptor();
    duplicate.display_name = "Different".into();
    assert!(registry.insert(duplicate).is_err());
    assert_eq!(registry.descriptor("pi").unwrap().display_name, "Pi");
    let mut invalid = pi_descriptor();
    invalid.source = "invalid".into();
    invalid.states[0].id.clear();
    assert!(registry.insert(invalid).is_err());
    assert!(registry.descriptor("invalid").is_none());
    for i in 1..MAX_SOURCES {
        let mut d = pi_descriptor();
        d.source = format!("source-{i}");
        registry.insert(d).unwrap();
    }
    let mut overflow = pi_descriptor();
    overflow.source = "overflow".into();
    assert!(registry.insert(overflow).is_err());
    assert_eq!(registry.descriptors().count(), MAX_SOURCES);
    assert!(registry.descriptor("overflow").is_none());
}

#[test]
fn declared_lifecycle_matches_existing_external_protocol_without_inventing_facts() {
    let descriptor = pi_descriptor();
    let mut live = Registry::default();
    let message = |sequence, payload| Message {
        schema_version: 1,
        source: descriptor.source.clone(),
        instance: "session-a".into(),
        sequence,
        payload,
    };
    let ms = Duration::from_millis;
    live.apply(
        message(
            1,
            Payload::State {
                state: "working".into(),
                ttl_ms: 15000,
            },
        ),
        ms(0),
    )
    .unwrap();
    live.apply(
        message(
            2,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 2000,
            },
        ),
        ms(1000),
    )
    .unwrap();
    assert_eq!(live.snapshot().states[0].expires_at, ms(15000)); // events don't renew
    live.apply(message(3, Payload::Remove), ms(1001)).unwrap();
    assert!(live.snapshot().states.is_empty());
    assert_eq!(live.events_since(0).events.len(), 1); // removal keeps events
    live.expire(ms(3000)).unwrap();
    assert!(live.events_since(0).events.is_empty()); // expires at deadline
    live.apply(
        message(
            4,
            Payload::State {
                state: "idle".into(),
                ttl_ms: 15000,
            },
        ),
        ms(4000),
    )
    .unwrap();
    live.expire(ms(19000)).unwrap();
    assert!(live.snapshot().states.is_empty());
    assert!(live.events_since(0).events.is_empty()); // expiry creates no finished/success
}
