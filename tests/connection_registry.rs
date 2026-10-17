use underglow::connection_registry::{ConnectionRegistry, ConnectorConfig, MAX_REGISTRY_BYTES};
use underglow::setup::{Activation, BehaviorVariant, Renderer, SetupDocument};

#[test]
fn declarative_default_and_capability_export_never_authorize_producers() {
    let registry = ConnectionRegistry::default();
    registry.validate().unwrap();
    assert!(registry.managed_configs().is_empty());
    let capabilities = registry.capability_registry().unwrap();
    assert_eq!(
        capabilities.descriptor("pi").unwrap(),
        &registry.connections[0].descriptor
    );
    let json = serde_json::to_vec(&registry).unwrap();
    assert_eq!(ConnectionRegistry::from_json(&json).unwrap(), registry);
    let mut value = serde_json::to_value(&registry).unwrap();
    value["connections"][0]["descriptor"]["command"] = serde_json::json!(["untrusted"]);
    assert!(ConnectionRegistry::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn registry_rejects_duplicate_mismatched_untrusted_and_stale_edits() {
    let original = ConnectionRegistry::default();
    let mut candidate = original.clone();
    candidate.connections.push(candidate.connections[0].clone());
    assert!(original.replace(0, &candidate).is_err());
    candidate = original.clone();
    candidate.connections[0].producer = Some(ConnectorConfig {
        id: "producer".into(),
        source: "different".into(),
        command: vec!["never-run".into()],
        ..Default::default()
    });
    assert!(candidate.validate().is_err());
    candidate.connections[0].producer.as_mut().unwrap().source = "pi".into();
    candidate.connections[0].producer.as_mut().unwrap().enabled = true;
    assert!(candidate.validate().is_err());
    candidate.connections[0].producer.as_mut().unwrap().trusted = true;
    let next = original.replace(0, &candidate).unwrap();
    assert_eq!(next.revision, 1);
    assert!(next.replace(0, &candidate).is_err());
    assert!(next.replace(1, &candidate).is_err());
    assert!(ConnectionRegistry::from_json(&vec![b' '; MAX_REGISTRY_BYTES + 1]).is_err());
    assert_eq!(original.revision, 0);
}

#[test]
fn missing_connections_are_repairable_without_rewriting_setup() {
    let mut setup: SetupDocument = toml::from_str(include_str!("../examples/layers-profile.toml"))
        .map(|value: toml::Value| value["setup"].clone().try_into().unwrap())
        .unwrap();
    setup.layers[0].activation = Activation::WhileState {
        source: "missing".into(),
        instance: None,
        state: Some("working".into()),
    };
    setup.layers[0].variants.push(BehaviorVariant {
        id: "disabled-alert".into(),
        name: "Disabled alert".into(),
        enabled: false,
        activation: Activation::OnEvent {
            source: "missing-variant".into(),
            instance: None,
            event: "alert".into(),
            duration_ms: 1000,
        },
        renderer: Renderer::Solid { color: [1, 2, 3] },
        brightness: None,
    });
    let before = setup.clone();
    let registry = ConnectionRegistry::default();
    assert_eq!(
        registry.missing_sources(&setup),
        ["missing", "missing-variant"]
    );
    assert_eq!(setup, before);
    let capabilities = registry.capability_registry().unwrap();
    assert!(
        !capabilities
            .validate_activation(&setup.layers[0].activation)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn legacy_migration_preserves_explicit_authorization_and_machine_secrets_only_in_registry() {
    let config = ConnectorConfig {
        id: "test".into(),
        source: "private-source".into(),
        command: vec!["never-run".into(), "credential-secret".into()],
        enabled: true,
        trusted: true,
        ..Default::default()
    };
    let registry = ConnectionRegistry::from_legacy(std::slice::from_ref(&config)).unwrap();
    assert_eq!(registry.managed_configs(), [config]);
    let descriptors: Vec<_> = registry
        .capability_registry()
        .unwrap()
        .descriptors()
        .cloned()
        .collect();
    assert!(
        !serde_json::to_string(&descriptors)
            .unwrap()
            .contains("credential-secret")
    );
    assert!(
        serde_json::to_string(&registry)
            .unwrap()
            .contains("credential-secret")
    );
}
