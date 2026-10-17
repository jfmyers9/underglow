use std::time::Duration;
use underglow::binding_repair::{BindingRepair, bindings, repair_binding};
use underglow::capability_manifest::{ManifestFormat, import_descriptor, parse, read_file};
use underglow::connection_registry::{ConnectionRegistry, ConnectorConfig};
use underglow::external::{Message, Registry};
use underglow::layer_runtime::CompiledSetup;
use underglow::setup::{Activation, BehaviorVariant, Renderer, SetupDocument, Target};
use underglow::source_capabilities::{
    CapabilityWarning, FactKind, MAX_DESCRIPTOR_BYTES, pi_descriptor,
};

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ug31-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn custom() -> underglow::source_capabilities::SourceDescriptor {
    parse(
        include_str!("../connectors/templates/capabilities.toml")
            .replace("__SOURCE__", "custom")
            .as_bytes(),
        ManifestFormat::Toml,
    )
    .unwrap()
}

fn setup() -> SetupDocument {
    let mut doc = SetupDocument::from_toml(include_str!("../examples/layer-setup.toml")).unwrap();
    doc.revision = 7;
    doc.layers[0].enabled = false;
    doc.layers[0].activation = Activation::WhileState {
        source: "missing".into(),
        instance: Some("old-session".into()),
        state: Some("working".into()),
    };
    doc.layers[0].variants = vec![
        BehaviorVariant {
            id: "event".into(),
            name: "Event".into(),
            enabled: false,
            activation: Activation::OnEvent {
                source: "missing-event".into(),
                instance: None,
                event: "finished".into(),
                duration_ms: 432,
            },
            renderer: Renderer::Solid {
                color: [11, 22, 33],
            },
            brightness: Some(67),
        },
        BehaviorVariant {
            id: "transition".into(),
            name: "Transition".into(),
            enabled: true,
            activation: Activation::OnTransition {
                source: "missing-transition".into(),
                instance: Some("old".into()),
                from_state: Some("idle".into()),
                state: "working".into(),
                duration_ms: 876,
            },
            renderer: Renderer::Pulse {
                color: [44, 55, 66],
                period_ms: 1234,
            },
            brightness: None,
        },
    ];
    doc.validate().unwrap();
    doc
}

#[test]
fn manifests_are_strict_bounded_declarative_data_not_executable_imports() {
    let descriptor = custom();
    assert_eq!(
        parse(
            &serde_json::to_vec(&descriptor).unwrap(),
            ManifestFormat::Json
        )
        .unwrap(),
        descriptor
    );
    for format in [ManifestFormat::Toml, ManifestFormat::Json] {
        assert!(parse(&vec![b' '; MAX_DESCRIPTOR_BYTES + 1], format).is_err());
        assert!(parse(b"\xff", format).is_err());
    }
    for field in [
        "producer",
        "command",
        "argv",
        "env",
        "working_dir",
        "trusted",
        "enabled",
        "script",
        "include",
    ] {
        let mut input = serde_json::to_value(&descriptor).unwrap();
        input[field] = serde_json::json!(true);
        assert!(
            parse(&serde_json::to_vec(&input).unwrap(), ManifestFormat::Json).is_err(),
            "{field}"
        );
        let input = format!("{field} = true\n{}", descriptor.to_toml().unwrap());
        assert!(
            parse(input.as_bytes(), ManifestFormat::Toml).is_err(),
            "{field}"
        );
    }
    let mut input = serde_json::to_value(&descriptor).unwrap();
    input["lifecycle"]["script"] = "not executable".into();
    assert!(parse(&serde_json::to_vec(&input).unwrap(), ManifestFormat::Json).is_err());
    for mutation in ["schema", "ttl", "fact"] {
        let mut invalid = descriptor.clone();
        match mutation {
            "schema" => invalid.schema_version = 2,
            "ttl" => invalid.lifecycle.state_ttl.as_mut().unwrap().min_ms = 0,
            _ => invalid.states.push(invalid.states[0].clone()),
        }
        assert!(parse(&serde_json::to_vec(&invalid).unwrap(), ManifestFormat::Json).is_err());
    }
    assert!(
        parse(
            &serde_json::to_vec(&ConnectionRegistry::default()).unwrap(),
            ManifestFormat::Json
        )
        .is_err()
    );
}

#[test]
fn manifest_file_reads_are_bounded_and_reject_nonregular_inputs() {
    let dir = TempDir::new();
    let path = dir.0.join("manifest.toml");
    std::fs::write(&path, custom().to_toml().unwrap()).unwrap();
    assert_eq!(read_file(&path, ManifestFormat::Toml).unwrap(), custom());
    assert!(read_file(&dir.0, ManifestFormat::Toml).is_err());
    assert!(read_file(&dir.0.join("missing"), ManifestFormat::Toml).is_err());
    #[cfg(unix)]
    {
        let link = dir.0.join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(read_file(&link, ManifestFormat::Toml).is_err());
    }
    std::fs::write(&path, vec![b' '; MAX_DESCRIPTOR_BYTES + 1]).unwrap();
    assert!(
        read_file(&path, ManifestFormat::Toml)
            .unwrap_err()
            .contains("64 KiB")
    );
}

#[test]
fn metadata_collision_requires_confirmation_and_preserves_private_producer_and_cas() {
    let mut registry = ConnectionRegistry {
        revision: 19,
        ..Default::default()
    };
    registry.connections[0].producer = Some(ConnectorConfig {
        id: "reviewed".into(),
        source: "pi".into(),
        command: vec!["/never-executed".into()],
        env: [("TOKEN".into(), "private-fixture".into())].into(),
        working_dir: Some("/local-only".into()),
        trusted: true,
        enabled: true,
        ..Default::default()
    });
    let before = registry.clone();
    let mut descriptor = pi_descriptor();
    descriptor.display_name = "New metadata".into();
    assert!(import_descriptor(&registry, 19, descriptor.clone(), false).is_err());
    assert!(import_descriptor(&registry, 18, descriptor.clone(), true).is_err());
    let candidate = import_descriptor(&registry, 19, descriptor.clone(), true).unwrap();
    assert_eq!(candidate.revision, 19);
    assert_eq!(candidate.managed_configs(), registry.managed_configs());
    assert_eq!(candidate.connections[0].descriptor, descriptor);
    let saved = registry.replace(19, &candidate).unwrap();
    assert_eq!(saved.revision, 20);
    assert!(saved.replace(19, &candidate).is_err());
    assert_eq!(registry, before);
    let candidate = import_descriptor(&registry, 19, custom(), false).unwrap();
    assert!(candidate.connection("custom").unwrap().producer.is_none());
    assert_eq!(candidate.connections[0], registry.connections[0]);
    let exported = custom().to_toml().unwrap();
    for forbidden in [
        "private-fixture",
        "local-only",
        "never-executed",
        "trusted",
        "enabled",
    ] {
        assert!(!exported.contains(forbidden));
    }
    let mut full = ConnectionRegistry::default();
    for i in 1..32 {
        let mut descriptor = custom();
        descriptor.source = format!("source-{i}");
        full = import_descriptor(&full, 0, descriptor, false).unwrap();
    }
    let before = full.clone();
    assert!(import_descriptor(&full, 0, custom(), false).is_err());
    assert_eq!(full, before);
}

#[test]
fn repair_includes_all_disabled_variants_and_preserves_every_nonbinding_field() {
    let doc = setup();
    let registry = import_descriptor(&ConnectionRegistry::default(), 0, custom(), false).unwrap();
    let inputs = bindings(&doc);
    assert_eq!(
        inputs
            .iter()
            .filter(|b| b.source.starts_with("missing"))
            .count(),
        3
    );
    assert!(
        inputs
            .iter()
            .filter(|b| b.source.starts_with("missing"))
            .all(|b| !b.enabled)
    );
    assert_eq!(
        registry.missing_sources(&doc),
        ["missing", "missing-event", "missing-transition"]
    );
    for binding in inputs
        .into_iter()
        .filter(|b| b.source.starts_with("missing"))
    {
        for instance in [None, Some("new-session".into())] {
            let repair = BindingRepair {
                setup_id: doc.id.clone(),
                key: binding.key.clone(),
                expected_activation: binding.activation.clone(),
                source: "custom".into(),
                instance: instance.clone(),
            };
            let (mut candidate, warnings) = repair_binding(&doc, &registry, &repair).unwrap();
            assert_eq!(candidate.revision, doc.revision);
            assert!(!warnings.is_empty(), "no cross-source vocabulary fiction");
            match &mut candidate.layers[0].activation {
                Activation::WhileState {
                    source, instance, ..
                } if repair.key.variant_id.is_none() => {
                    assert_eq!(source, "custom");
                    assert_eq!(instance, &repair.instance);
                    *source = "missing".into();
                    *instance = Some("old-session".into());
                }
                _ => {}
            }
            if let Some(id) = &repair.key.variant_id {
                let variant = candidate.layers[0]
                    .variants
                    .iter_mut()
                    .find(|v| &v.id == id)
                    .unwrap();
                match &variant.activation {
                    Activation::OnEvent {
                        event, duration_ms, ..
                    } => {
                        assert_eq!(event, "finished");
                        assert_eq!(*duration_ms, 432);
                        assert!(warnings.contains(&CapabilityWarning::UndeclaredEvent {
                            event: "finished".into()
                        }));
                    }
                    Activation::OnTransition {
                        state,
                        from_state,
                        duration_ms,
                        ..
                    } => {
                        assert_eq!(state, "working");
                        assert_eq!(from_state.as_deref(), Some("idle"));
                        assert_eq!(*duration_ms, 876);
                    }
                    _ => unreachable!(),
                }
                variant.activation = repair.expected_activation.clone();
            }
            assert_eq!(
                candidate, doc,
                "appearance, target, groups, enabled flags, ordering and unrelated bindings survive"
            );
        }
    }
}

#[test]
fn stale_deleted_invalid_and_missing_repairs_leave_the_original_draft_unchanged() {
    let doc = setup();
    let binding = &bindings(&doc)[0];
    let mut repair = BindingRepair {
        setup_id: doc.id.clone(),
        key: binding.key.clone(),
        expected_activation: binding.activation.clone(),
        source: "pi".into(),
        instance: None,
    };
    let registry = ConnectionRegistry::default();
    let original = doc.clone();
    repair.expected_activation = Activation::Always {};
    assert!(repair_binding(&doc, &registry, &repair).is_err());
    repair.expected_activation = binding.activation.clone();
    repair.source = "still-missing".into();
    assert!(repair_binding(&doc, &registry, &repair).is_err());
    repair.source = "pi".into();
    repair.instance = Some("invalid space".into());
    assert!(repair_binding(&doc, &registry, &repair).is_err());
    repair.instance = None;
    repair.key.variant_id = Some("deleted".into());
    assert!(repair_binding(&doc, &registry, &repair).is_err());
    repair.key.variant_id = None;
    repair.setup_id = "other".into();
    assert!(repair_binding(&doc, &registry, &repair).is_err());
    assert_eq!(doc, original);
}

#[test]
fn scaffolded_python_manifest_and_protocol_drive_generic_hardware_free_preview() {
    use std::process::Command;
    let temp = TempDir::new();
    let dir = temp.0.join("scaffold");
    let result = Command::new("python3")
        .args([
            "scripts/scaffold-connector.py",
            "--language",
            "python",
            "--name",
            "custom-timer",
            "--capabilities",
            "--output",
        ])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let descriptor = read_file(&dir.join("capabilities.toml"), ManifestFormat::Toml).unwrap();
    assert_eq!(descriptor.source, "custom-timer");
    let registry = import_descriptor(&ConnectionRegistry::default(), 0, descriptor, false).unwrap();
    let capabilities = registry.capability_registry().unwrap();
    assert_eq!(
        capabilities
            .choices("custom-timer", FactKind::State, &[])
            .unwrap()[0]
            .id,
        "focusing"
    );
    assert_eq!(
        capabilities
            .choices("custom-timer", FactKind::Event, &[])
            .unwrap()[0]
            .id,
        "done"
    );
    let result = Command::new("python3")
        .arg(dir.join("connector.py"))
        .args(["--stdout", "--duration-seconds", "0"])
        .output()
        .unwrap();
    assert!(result.status.success());
    // The only subprocess is the checked-in scaffold, explicitly run by this
    // isolated test. Manifest parsing itself has no executable code path.
    let mut doc = setup();
    doc.layers.truncate(1);
    doc.layers[0].enabled = true;
    doc.layers[0].target = Target::Whole {};
    doc.layers[0].renderer = Renderer::Solid { color: [1, 2, 3] };
    doc.layers[0].activation = Activation::WhileState {
        source: "custom-timer".into(),
        instance: None,
        state: Some("focusing".into()),
    };
    doc.layers[0].variants.truncate(1);
    doc.layers[0].variants[0].enabled = true;
    doc.layers[0].variants[0].activation = Activation::OnEvent {
        source: "custom-timer".into(),
        instance: None,
        event: "done".into(),
        duration_ms: 500,
    };
    let mut preview = CompiledSetup::compile(&doc).unwrap();
    let mut facts = Registry::default(); // private in-memory ingress, no installed engine
    preview.baseline(&facts.snapshot(), Duration::ZERO);
    let messages: Vec<Message> = result
        .stdout
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    assert_eq!(messages.len(), 3);
    for (i, message) in messages.into_iter().enumerate() {
        let now = Duration::from_millis((i + 1) as u64);
        facts.apply(message, now).unwrap();
        preview.observe(
            &facts.snapshot(),
            &facts.events_since(preview.event_cursor()),
            now,
        );
        if i == 2 {
            assert!(
                preview.selected_appearance(&doc.layers[0].id).is_none(),
                "removal ends state-backed visuals, not the accepted event fact"
            );
            assert_eq!(facts.events_since(0).events.len(), 1);
        } else {
            let selected = preview.selected_appearance(&doc.layers[0].id).unwrap();
            assert_eq!(
                selected.variant_id,
                if i == 0 { None } else { Some("event") }
            );
            let info = underglow::device::DeviceInfo::synthetic_80he();
            let layout = underglow::layout::KeyboardLayout::for_device(&info);
            let context = underglow::render::RenderContext {
                info: &info,
                layout: &layout,
                brightness: 255,
                palette: underglow::render::PaletteName::Heat,
                tick: 0,
                animation_seconds: 0.0,
            };
            let frame = preview.render(&context, now);
            assert_ne!(frame, underglow::render::Frame::black());
        }
    }
    assert!(
        registry
            .connection("custom-timer")
            .unwrap()
            .producer
            .is_none()
    );
}
