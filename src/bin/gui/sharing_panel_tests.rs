use super::*;
use underglow::connection_registry::ConnectorConfig;

fn document() -> SetupDocument {
    SetupDocument::from_toml(include_str!("../../../examples/layer-setup.toml")).unwrap()
}

fn view<'a>(
    document: &'a SetupDocument,
    registry: &'a ConnectionRegistry,
    can_store: bool,
) -> View<'a> {
    View {
        applied: Some(document),
        draft: Some(document),
        registry: Some(registry),
        existing_ids: &[],
        can_store,
    }
}

#[test]
fn headless_validation_explicit_binding_store_and_cancel_never_change_editor_or_connections() {
    let ctx = egui::Context::default();
    let mut registry = ConnectionRegistry::default();
    registry.connections[0].producer = Some(ConnectorConfig {
        id: "private".into(),
        source: "pi".into(),
        command: vec!["/NEVER_EXECUTED".into()],
        env: [("TOKEN".into(), "PRIVATE_FIXTURE".into())].into(),
        trusted: true,
        enabled: true,
        ..Default::default()
    });
    let registry_before = registry.clone();
    let mut editor = super::super::setup_editor::Editor::default();
    editor.load(Some(document()));
    editor.draft.as_mut().unwrap().name = "Unsaved draft".into();
    editor.dirty = true;
    let draft_before = editor.draft.clone();
    let applied_before = editor.applied.clone();
    let doc = document();
    // Even an un-anonymized same-name source must be mapped explicitly.
    let mut panel = Panel {
        text: doc.to_toml().unwrap(),
        ..Default::default()
    };
    assert!(panel.candidate(&view(&doc, &registry, true)).is_err());
    let (output, action) = test_ui::draw(&ctx, vec![], |ui| {
        panel.show(ui, view(&doc, &registry, true))
    });
    assert!(action.is_none());
    // The explicit sharing window needs no second disclosure click.
    test_ui::position(&output, "Share setups");
    test_ui::position(&output, "Export declarative setup");
    test_ui::position(&output, "Validate pasted setup");
    assert!(
        test_ui::click(&ctx, "Validate pasted setup", |ui| panel
            .show(ui, view(&doc, &registry, true)))
        .is_none()
    );
    assert!(panel.reviewed.is_some());
    assert!(panel.candidate(&view(&doc, &registry, true)).is_err());
    assert!(
        test_ui::click(&ctx, "Store imported new copy (do not switch)", |ui| panel
            .show(ui, view(&doc, &registry, true)))
        .is_none()
    );
    // Exercise actual widgets rather than treating same source names as approval.
    test_ui::click(&ctx, "Unresolved — choose local connection", |ui| {
        panel.show(ui, view(&doc, &registry, true))
    });
    test_ui::click(
        &ctx,
        &format!(
            "{} · {}",
            registry.connections[0].descriptor.display_name,
            registry.connections[0].descriptor.source
        ),
        |ui| panel.show(ui, view(&doc, &registry, true)),
    );
    assert!(
        panel.candidate(&view(&doc, &registry, true)).is_err(),
        "instance choice is independent"
    );
    test_ui::click(&ctx, "Any instance (explicit)", |ui| {
        panel.show(ui, view(&doc, &registry, true))
    });
    let candidate = panel.candidate(&view(&doc, &registry, true)).unwrap();
    assert_ne!(candidate.document.id, doc.id);
    assert!(
        test_ui::click(&ctx, "Store imported new copy (do not switch)", |ui| panel
            .show(ui, view(&doc, &registry, false)))
        .is_none()
    );
    let stored = test_ui::click(&ctx, "Store imported new copy (do not switch)", |ui| {
        panel.show(ui, view(&doc, &registry, true))
    })
    .unwrap();
    assert_eq!(stored, candidate.document);
    assert_eq!(registry, registry_before);
    assert_eq!(editor.draft, draft_before);
    assert_eq!(editor.applied, applied_before);
    assert!(editor.dirty);
    assert!(panel.preview.sample().is_none());
    assert!(
        test_ui::click(&ctx, "Cancel import", |ui| panel
            .show(ui, view(&doc, &registry, true)))
        .is_none()
    );
    assert!(panel.reviewed.is_none());
    assert!(panel.candidate(&view(&doc, &registry, true)).is_err());
}

#[test]
fn invalid_import_and_registry_revision_change_invalidate_previous_approval() {
    let doc = document();
    let mut registry = ConnectionRegistry::default();
    let mut panel = Panel::default();
    panel.review(ImportPreview::parse(
        doc.to_toml().unwrap().as_bytes(),
        Format::Toml,
    ));
    panel.choices[0].source = Some("pi".into());
    panel.choices[0].exact = Some(false);
    panel.registry_revision = Some(registry.revision);
    assert!(panel.candidate(&view(&doc, &registry, true)).is_ok());
    registry.revision += 1;
    let ctx = egui::Context::default();
    test_ui::draw(&ctx, vec![], |ui| {
        panel.show(ui, view(&doc, &registry, true))
    });
    assert!(panel.candidate(&view(&doc, &registry, true)).is_err());
    panel.review(ImportPreview::parse(b"schema_version=99", Format::Toml));
    assert!(panel.reviewed.is_none());
    assert!(panel.choices.is_empty());
    assert!(panel.message.is_some());
}

#[test]
fn disabled_layer_and_variant_still_require_independent_local_binding_approval() {
    let mut doc = document();
    let layer = &mut doc.layers[1];
    layer.enabled = false;
    layer.variants.push(underglow::setup::BehaviorVariant {
        id: "disabled-event".into(),
        name: "Disabled event".into(),
        enabled: false,
        activation: Activation::OnEvent {
            source: "pi".into(),
            instance: Some("original-private-instance".into()),
            event: "unknown-event".into(),
            duration_ms: 500,
        },
        renderer: layer.renderer.clone(),
        brightness: None,
    });
    let registry = ConnectionRegistry::default();
    let mut panel = Panel::default();
    panel.review(ImportPreview::parse(
        doc.to_toml().unwrap().as_bytes(),
        Format::Toml,
    ));
    panel.registry_revision = Some(registry.revision);
    assert_eq!(panel.choices.len(), 2);
    let ctx = egui::Context::default();
    let (output, action) = test_ui::draw(&ctx, vec![], |ui| {
        panel.show(ui, view(&doc, &registry, true))
    });
    assert!(action.is_none());
    test_ui::position(
        &output,
        "Pi activity / base · role pi (disabled, mapping still required)",
    );
    test_ui::position(
        &output,
        "Pi activity / Disabled event · role pi (disabled, mapping still required)",
    );

    panel.choices[0].source = Some("pi".into());
    panel.choices[0].exact = Some(false);
    assert!(panel.candidate(&view(&doc, &registry, true)).is_err());
    assert!(
        test_ui::click(&ctx, "Store imported new copy (do not switch)", |ui| {
            panel.show(ui, view(&doc, &registry, true))
        })
        .is_none()
    );
    panel.choices[1].source = Some("pi".into());
    assert!(panel.candidate(&view(&doc, &registry, true)).is_err());
    panel.choices[1].exact = Some(true);
    panel.choices[1].instance = "   ".into();
    assert!(panel.candidate(&view(&doc, &registry, true)).is_err());
    panel.choices[1].instance = "local-session".into();
    let candidate = panel.candidate(&view(&doc, &registry, true)).unwrap();
    assert!(!candidate.document.layers[1].enabled);
    assert!(!candidate.document.layers[1].variants[0].enabled);
    assert_eq!(
        candidate.document.layers[1].variants[0].activation,
        Activation::OnEvent {
            source: "pi".into(),
            instance: Some("local-session".into()),
            event: "unknown-event".into(),
            duration_ms: 500,
        }
    );
    assert!(panel.preview.sample().is_none());
}

#[test]
fn exported_roles_and_unknown_fact_tokens_can_be_simulated_without_local_connections() {
    let doc = document();
    let exported = setup_sharing::export_document(&doc, Format::Json).unwrap();
    let mut panel = Panel::default();
    panel.review(ImportPreview::parse(exported.as_bytes(), Format::Json));
    let reviewed = panel.reviewed.as_ref().unwrap();
    let binding = reviewed.bindings().remove(0);
    assert_eq!(binding.source, "role-1");
    panel.preview.update(Some(reviewed.document()), 255);
    Panel::simulate(&mut panel.preview, reviewed.document(), binding.activation).unwrap();
    assert_eq!(
        panel.preview.scenario.snapshot().states[0].identity.source,
        "role-1"
    );
    assert!(panel.preview.sample().is_none());
    assert!(
        panel.choices[0].source.is_none(),
        "synthetic preview never approves local binding"
    );
    for activation in [
        Activation::OnEvent {
            source: "role-1".into(),
            instance: Some("exact-preview".into()),
            event: "unknown-event".into(),
            duration_ms: 500,
        },
        Activation::OnTransition {
            source: "role-1".into(),
            instance: Some("exact-preview".into()),
            from_state: Some("unknown-before".into()),
            state: "unknown-after".into(),
            duration_ms: 500,
        },
    ] {
        Panel::simulate(&mut panel.preview, reviewed.document(), &activation).unwrap();
    }
    let events = panel.preview.scenario.events_since(0);
    assert_eq!(events.events[0].identity.source, "role-1");
    assert_eq!(events.events[0].event, "unknown-event");
    assert!(
        panel
            .preview
            .scenario
            .snapshot()
            .states
            .iter()
            .any(|state| state.state == "unknown-after")
    );
}
