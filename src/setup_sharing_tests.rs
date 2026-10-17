use super::*;
use crate::connection_registry::ConnectorConfig;
use crate::setup::BehaviorVariant;

fn document() -> SetupDocument {
    let mut doc = SetupDocument::from_toml(include_str!("../examples/layer-setup.toml")).unwrap();
    doc.revision = 65;
    doc.layers[1].enabled = false;
    doc.layers[0].variants = vec![
        BehaviorVariant {
            id: "completion".into(),
            name: "Completion".into(),
            enabled: false,
            activation: Activation::OnEvent {
                source: "pi".into(),
                instance: Some("old-session".into()),
                event: "unknown-done".into(),
                duration_ms: 900,
            },
            renderer: doc.layers[0].renderer.clone(),
            brightness: Some(81),
        },
        BehaviorVariant {
            id: "transition".into(),
            name: "Transition".into(),
            enabled: true,
            activation: Activation::OnTransition {
                source: "other".into(),
                instance: None,
                from_state: Some("unknown-before".into()),
                state: "unknown-after".into(),
                duration_ms: 700,
            },
            renderer: doc.layers[0].renderer.clone(),
            brightness: None,
        },
    ];
    doc
}

fn mappings(preview: &ImportPreview) -> Vec<BindingMapping> {
    preview
        .bindings()
        .into_iter()
        .map(|b| BindingMapping {
            key: b.key,
            source: "pi".into(),
            instance: None,
        })
        .collect()
}

#[test]
fn both_formats_roundtrip_appearance_and_all_conditions_without_private_configuration() {
    let mut registry = ConnectionRegistry::default();
    registry.connections[0].producer = Some(ConnectorConfig {
        id: "private".into(),
        source: "pi".into(),
        command: vec!["/PRIVATE_COMMAND".into()],
        env: [("TOKEN".into(), "PRIVATE_CREDENTIAL".into())].into(),
        working_dir: Some("/PRIVATE_CWD".into()),
        enabled: true,
        trusted: true,
        ..Default::default()
    });
    registry.validate().unwrap();
    let registry_before = registry.clone();
    let original = document();
    for format in [Format::Toml, Format::Json] {
        let encoded = export_document(&original, format).unwrap();
        for private in [
            "PRIVATE_COMMAND",
            "PRIVATE_CREDENTIAL",
            "PRIVATE_CWD",
            "trusted",
            "sdk_path",
            "command",
            "working_dir",
        ] {
            assert!(!encoded.contains(private));
        }
        let preview = ImportPreview::parse(encoded.as_bytes(), format).unwrap();
        assert_eq!(preview.document().revision, 0);
        assert_eq!(preview.bindings().len(), 3);
        assert!(
            preview
                .bindings()
                .iter()
                .all(|b| b.source.starts_with("role-"))
        );
        let mut mapped = mappings(&preview);
        mapped[0].instance = Some("my-session".into());
        let candidate = preview
            .prepare(
                "new-copy",
                &mapped,
                &registry,
                std::slice::from_ref(&original.id),
            )
            .unwrap();
        assert!(
            !candidate.warnings.is_empty(),
            "undeclared facts remain warned"
        );
        assert_eq!(candidate.document.id, "new-copy");
        assert_eq!(candidate.document.revision, 0);
        assert_eq!(candidate.document.groups, original.groups);
        let mut expected = original.clone();
        expected.id = candidate.document.id.clone();
        expected.revision = 0;
        for layer in &mut expected.layers {
            for activation in std::iter::once(&mut layer.activation)
                .chain(layer.variants.iter_mut().map(|v| &mut v.activation))
            {
                if let Some(source) = source_mut(activation) {
                    *source = "pi".into();
                }
                match activation {
                    Activation::Always {} => {}
                    Activation::WhileState { instance, .. }
                    | Activation::OnTransition { instance, .. } => *instance = None,
                    Activation::OnEvent { instance, .. } => *instance = Some("my-session".into()),
                }
            }
        }
        assert_eq!(candidate.document, expected);
    }
    assert_eq!(registry, registry_before);
    assert_eq!(original.revision, 65);
}

#[test]
fn same_names_disabled_bindings_unknown_destinations_and_id_collisions_require_explicit_choices() {
    let original = document();
    let preview =
        ImportPreview::parse(original.to_toml().unwrap().as_bytes(), Format::Toml).unwrap();
    let registry = ConnectionRegistry::default();
    assert!(preview.prepare("new", &[], &registry, &[]).is_err());
    let mapped = mappings(&preview);
    assert!(
        preview
            .prepare("new", &mapped[..2], &registry, &[])
            .is_err()
    );
    let mut duplicate = mapped.clone();
    duplicate[2] = duplicate[0].clone();
    assert!(preview.prepare("new", &duplicate, &registry, &[]).is_err());
    let mut unknown = mapped.clone();
    unknown[0].source = "unconfigured-raw-source".into();
    assert!(preview.prepare("new", &unknown, &registry, &[]).is_err());
    assert!(
        preview
            .prepare(&original.id, &mapped, &registry, &[])
            .is_err()
    );
    assert!(
        preview
            .prepare("new", &mapped, &registry, &["new".into()])
            .is_err()
    );
    assert!(preview.prepare("", &mapped, &registry, &[]).is_err());
    assert!(preview.prepare("new", &mapped, &registry, &[]).is_ok());
    let mut normalized = original;
    normalized.revision = 0;
    assert_eq!(preview.document(), &normalized);
}

#[test]
fn strict_version_bounded_parser_rejects_legacy_secrets_trust_unknown_renderer_and_nested_commands()
{
    let doc = document();
    let json = serde_json::to_value(&doc).unwrap();
    for field in [
        "producers",
        "connections",
        "command",
        "env",
        "trusted",
        "enabled",
        "sdk_path",
        "url",
    ] {
        let mut bad = json.clone();
        bad[field] = serde_json::json!("forbidden");
        assert!(
            ImportPreview::parse(
                serde_json::to_string(&bad).unwrap().as_bytes(),
                Format::Json
            )
            .is_err(),
            "{field}"
        );
    }
    for path in [
        vec!["layers", "0", "renderer"],
        vec!["layers", "0", "activation"],
        vec!["layers", "0", "target"],
        vec!["layers", "0", "variants", "0"],
    ] {
        let mut bad = json.clone();
        let mut value = &mut bad;
        for key in path {
            value = if key == "0" {
                &mut value[0]
            } else {
                &mut value[key]
            };
        }
        value["command"] = serde_json::json!("sh");
        assert!(
            ImportPreview::parse(
                serde_json::to_string(&bad).unwrap().as_bytes(),
                Format::Json
            )
            .is_err()
        );
    }
    let toml = doc.to_toml().unwrap();
    for bad in [
        toml.replace("schema_version = 1", "schema_version = 99"),
        toml.replace("kind = \"effect\"", "kind = \"download-renderer\""),
        format!("command='sh'\n{toml}"),
        " ".repeat(MAX_DOCUMENT_BYTES + 1),
    ] {
        assert!(ImportPreview::parse(bad.as_bytes(), Format::Toml).is_err());
    }
    assert!(ImportPreview::parse(&[0xff], Format::Json).is_err());
    assert!(ImportPreview::parse(&vec![b' '; MAX_DOCUMENT_BYTES + 1], Format::Json).is_err());
}

#[test]
fn export_is_private_atomic_create_new_and_failure_or_cancel_never_overwrites() {
    let dir = std::env::temp_dir().join(format!(
        "underglow-sharing-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&dir).unwrap();
    let path = dir.join("shared.toml");
    let doc = document();
    export_file(&path, &doc, Format::Toml).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(ImportPreview::read(&path, Format::Toml).is_ok());
    assert!(export_file(&path, &doc, Format::Json).is_err());
    let mut invalid = doc.clone();
    invalid.schema_version = 98;
    assert!(export_file(&path, &invalid, Format::Toml).is_err());
    assert!(export_file(&dir.join("invalid.toml"), &invalid, Format::Toml).is_err());
    assert!(export_file(&dir.join("missing/file.toml"), &doc, Format::Toml).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        fs::read_dir(&dir).unwrap().count(),
        1,
        "no partial or temporary files"
    );
    assert!(ImportPreview::read(&dir, Format::Toml).is_err());
    fs::write(dir.join("huge.json"), vec![b' '; MAX_DOCUMENT_BYTES + 1]).unwrap();
    assert!(ImportPreview::read(&dir.join("huge.json"), Format::Json).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = dir.join("link.toml");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(export_file(&link, &doc, Format::Toml).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compile_conflicts_are_rejected_before_import_or_export_can_change_anything() {
    let mut doc = document();
    for layer in &mut doc.layers {
        layer.enabled = true;
        layer.renderer = crate::setup::Renderer::Reactive {
            effect: crate::reactive::ReactiveKind::Heatmap,
            palette: None,
        };
        layer.variants.clear();
    }
    doc.validate().unwrap();
    assert!(export_document(&doc, Format::Json).is_err());
    let json = serde_json::to_string(&doc).unwrap();
    assert!(ImportPreview::parse(json.as_bytes(), Format::Json).is_err());
}
