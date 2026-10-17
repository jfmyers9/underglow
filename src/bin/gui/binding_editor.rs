use eframe::egui;
use underglow::binding_repair::{BindingKey, BindingRepair, repair_binding};
use underglow::connection_registry::ConnectionRegistry;
use underglow::setup::{Activation, SetupDocument};

pub struct Editor {
    repair: BindingRepair,
    instance_mode: Option<bool>,
    instance: String,
}

impl Editor {
    pub fn new(setup: &SetupDocument, key: BindingKey, activation: &Activation) -> Self {
        let instance = match activation {
            Activation::Always {} => None,
            Activation::WhileState { instance, .. }
            | Activation::OnEvent { instance, .. }
            | Activation::OnTransition { instance, .. } => instance.clone(),
        };
        Self {
            repair: BindingRepair {
                setup_id: setup.id.clone(),
                key,
                expected_activation: activation.clone(),
                source: String::new(),
                instance: None,
            },
            instance_mode: None,
            instance: instance.unwrap_or_default(),
        }
    }

    /// Returns only an explicit draft edit; no persistence or process intent.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        registry: &ConnectionRegistry,
        setup: &SetupDocument,
    ) -> (Option<BindingRepair>, bool) {
        let mut result = None;
        let mut close = false;
        ui.group(|ui| {
            ui.heading("Repair layer input binding");
            ui.label(format!("Layer {} · {}", self.repair.key.layer_id,
                self.repair.key.variant_id.as_deref().unwrap_or("default appearance")));
            ui.label(format!("Original selector: {:?}", self.repair.expected_activation));
            egui::ComboBox::from_id_salt("repair-source").selected_text(
                if self.repair.source.is_empty() { "Choose configured connection" } else { &self.repair.source }
            ).show_ui(ui, |ui| {
                for c in &registry.connections {
                    ui.selectable_value(&mut self.repair.source, c.descriptor.source.clone(),
                        format!("{} · {}", c.descriptor.display_name, c.descriptor.source));
                }
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.instance_mode, Some(false), "Any instance");
                ui.selectable_value(&mut self.instance_mode, Some(true), "Exact instance");
            });
            if self.instance_mode == Some(true) {
                ui.add(egui::TextEdit::singleline(&mut self.instance).char_limit(128).hint_text("Exact producer instance ID"));
            }
            self.repair.instance = (self.instance_mode == Some(true)).then(|| self.instance.clone());
            let checked = if self.instance_mode.is_none() {
                Err("Explicitly choose any or exact instance (identities are not portable between sources)".into())
            } else { repair_binding(setup, registry, &self.repair) };
            match &checked {
                Ok((_, warnings)) => {
                    for warning in warnings { ui.colored_label(egui::Color32::YELLOW, format!("Retained selector warning: {warning:?}")); }
                }
                Err(error) => { ui.label(error); }
            }
            ui.label("Only source and instance change. State/event tokens, appearance, target, variants and ordering stay intact. No fact names are assumed equivalent across sources.");
            if ui.add_enabled(checked.is_ok(), egui::Button::new("Rebind in draft (not yet saved)")).clicked() {
                result = Some(self.repair.clone());
            }
            if ui.button("Cancel binding repair").clicked() { close = true; }
        });
        (result, close)
    }
}

#[cfg(test)]
mod tests {
    use super::super::metadata_test_ui::{click, draw};
    use super::*;
    use underglow::binding_repair::bindings;

    #[test]
    fn headless_repair_requires_explicit_instance_choice_and_preserves_unknown_event() {
        let ctx = egui::Context::default();
        let mut doc =
            SetupDocument::from_toml(include_str!("../../../examples/layer-setup.toml")).unwrap();
        doc.layers[0].activation = Activation::OnEvent {
            source: "missing".into(),
            instance: Some("old-session".into()),
            event: "unknown-event".into(),
            duration_ms: 123,
        };
        let binding = &bindings(&doc)[0];
        let mut editor = Editor::new(&doc, binding.key.clone(), binding.activation);
        let registry = ConnectionRegistry::default();
        assert!(
            draw(&ctx, vec![], |ui| editor.show(ui, &registry, &doc))
                .1
                .0
                .is_none()
        );
        editor.repair.source = "pi".into();
        assert!(
            click(&ctx, "Rebind in draft (not yet saved)", |ui| editor
                .show(ui, &registry, &doc))
            .0
            .is_none()
        );
        click(&ctx, "Any instance", |ui| editor.show(ui, &registry, &doc));
        let request = click(&ctx, "Rebind in draft (not yet saved)", |ui| {
            editor.show(ui, &registry, &doc)
        })
        .0
        .unwrap();
        assert_eq!(request.instance, None);
        let (candidate, warnings) = repair_binding(&doc, &registry, &request).unwrap();
        assert!(!warnings.is_empty());
        assert_eq!(
            candidate.layers[0].activation,
            Activation::OnEvent {
                source: "pi".into(),
                instance: None,
                event: "unknown-event".into(),
                duration_ms: 123
            }
        );
        assert_eq!(
            doc.layers[0].activation, request.expected_activation,
            "dialog emits intent only"
        );
        click(&ctx, "Exact instance", |ui| {
            editor.show(ui, &registry, &doc)
        });
        let exact = click(&ctx, "Rebind in draft (not yet saved)", |ui| {
            editor.show(ui, &registry, &doc)
        })
        .0
        .unwrap();
        assert_eq!(exact.instance.as_deref(), Some("old-session"));
        assert!(
            click(&ctx, "Cancel binding repair", |ui| editor
                .show(ui, &registry, &doc))
            .1
        );
    }
}
