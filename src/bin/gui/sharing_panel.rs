//! Local sharing workflow. The only returned action is a declarative new copy;
//! no Resume, connection replacement, producer or engine transport exists here.
use super::{setup_preview, setup_targets};
use eframe::egui;
use std::path::Path;
use underglow::binding_repair::BindingKey;
use underglow::connection_registry::ConnectionRegistry;
use underglow::setup::{Activation, MAX_DOCUMENT_BYTES, SetupDocument};
use underglow::setup_sharing::{self, BindingMapping, Format, ImportCandidate, ImportPreview};

#[cfg(test)]
use super::connections_screen::metadata_test_ui as test_ui;

#[cfg(test)]
#[path = "sharing_panel_tests.rs"]
mod tests;

struct Choice {
    key: BindingKey,
    source: Option<String>,
    exact: Option<bool>,
    instance: String,
}

pub struct View<'a> {
    pub applied: Option<&'a SetupDocument>,
    pub draft: Option<&'a SetupDocument>,
    pub registry: Option<&'a ConnectionRegistry>,
    pub existing_ids: &'a [String],
    pub can_store: bool,
}

pub struct Panel {
    text: String,
    import_path: String,
    export_path: String,
    format: Format,
    export_draft: bool,
    reviewed: Option<ImportPreview>,
    choices: Vec<Choice>,
    new_id: String,
    registry_revision: Option<u64>,
    preview: setup_preview::Preview,
    message: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            text: String::new(),
            import_path: String::new(),
            export_path: String::new(),
            format: Format::Toml,
            export_draft: false,
            reviewed: None,
            choices: vec![],
            new_id: String::new(),
            registry_revision: None,
            preview: Default::default(),
            message: None,
        }
    }
}

impl Panel {
    fn simulate(
        preview: &mut setup_preview::Preview,
        document: &SetupDocument,
        activation: &Activation,
    ) -> Result<(), String> {
        preview.set_mode(setup_preview::Mode::Scenario);
        let ttl = 15_000;
        let result = match activation {
            Activation::Always {} => return Ok(()),
            Activation::WhileState {
                source,
                instance,
                state,
            } => preview.scenario.publish_state(
                source,
                instance.as_deref().unwrap_or("sharing-preview"),
                state.as_deref().unwrap_or("preview-presence"),
                ttl,
            ),
            Activation::OnEvent {
                source,
                instance,
                event,
                ..
            } => preview.scenario.publish_event(
                source,
                instance.as_deref().unwrap_or("sharing-preview"),
                event,
                ttl,
            ),
            Activation::OnTransition {
                source,
                instance,
                from_state,
                state,
                ..
            } => {
                let instance = instance.as_deref().unwrap_or("sharing-preview");
                if let Some(from_state) = from_state {
                    preview
                        .scenario
                        .publish_state(source, instance, from_state, ttl)
                        .map_err(|e| e.to_string())?;
                } else {
                    preview
                        .scenario
                        .remove(source, instance)
                        .map_err(|e| e.to_string())?;
                }
                preview.update(Some(document), 255);
                preview
                    .scenario
                    .advance_by(std::time::Duration::from_millis(1))
                    .map_err(|e| e.to_string())?;
                preview.scenario.publish_state(source, instance, state, ttl)
            }
        };
        result.map_err(|e| e.to_string())?;
        preview.update(Some(document), 255);
        Ok(())
    }

    fn cancel(&mut self) {
        self.reviewed = None;
        self.choices.clear();
        self.preview = Default::default();
    }

    fn review(&mut self, result: Result<ImportPreview, String>) {
        self.cancel();
        match result {
            Ok(reviewed) => {
                self.choices = reviewed
                    .bindings()
                    .iter()
                    .map(|b| Choice {
                        key: b.key.clone(),
                        source: None,
                        exact: None,
                        instance: String::new(),
                    })
                    .collect();
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                self.new_id = format!("imported-{nonce}");
                self.preview.set_mode(setup_preview::Mode::Scenario);
                self.reviewed = Some(reviewed);
                self.message = None;
            }
            Err(error) => self.message = Some(error),
        }
    }

    fn candidate(&self, view: &View<'_>) -> Result<ImportCandidate, String> {
        let preview = self.reviewed.as_ref().ok_or("Validate a setup first")?;
        let registry = view
            .registry
            .ok_or("Load configured local connections first")?;
        let mappings = self
            .choices
            .iter()
            .map(|choice| {
                let source = choice
                    .source
                    .clone()
                    .ok_or("Unresolved source role: choose a configured local connection")?;
                let instance = match choice.exact {
                    Some(false) => None,
                    Some(true) if !choice.instance.trim().is_empty() => {
                        Some(choice.instance.clone())
                    }
                    _ => {
                        return Err(
                            "Explicitly choose any instance or enter an exact instance ID"
                                .to_owned(),
                        );
                    }
                };
                Ok(BindingMapping {
                    key: choice.key.clone(),
                    source,
                    instance,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        preview.prepare(&self.new_id, &mappings, registry, view.existing_ids)
    }

    pub fn show(&mut self, ui: &mut egui::Ui, view: View<'_>) -> Option<SetupDocument> {
        let mut store = None;
        ui.collapsing("Share setups safely", |ui| {
            ui.label("Declarative v1 only · 256 KiB maximum · no credentials, executable trust or producer installation. No automatic activation.");
            ui.horizontal(|ui| {
                let before = self.format;
                ui.selectable_value(&mut self.format, Format::Toml, "TOML");
                ui.selectable_value(&mut self.format, Format::Json, "JSON");
                if before != self.format { self.cancel(); }
            });
            ui.strong("Export a new file");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.export_draft, false, "Applied setup");
                ui.selectable_value(&mut self.export_draft, true, "Validated current draft (includes unsaved edits)");
            });
            ui.add(egui::TextEdit::singleline(&mut self.export_path).char_limit(4096).hint_text("Explicit new destination path; existing files are never overwritten"));
            let document = if self.export_draft { view.draft } else { view.applied };
            if ui.add_enabled(document.is_some() && !self.export_path.trim().is_empty(), egui::Button::new("Export declarative setup")).clicked() {
                self.message = Some(match setup_sharing::export_file(Path::new(&self.export_path), document.unwrap(), self.format) {
                    Ok(()) => "Exported private new file. Review names, fact and instance labels before sharing.".into(),
                    Err(error) => error,
                });
            }
            ui.separator();
            ui.strong("Import and preview before storing a new copy");
            let changed = ui.add(egui::TextEdit::multiline(&mut self.text).char_limit(MAX_DOCUMENT_BYTES).desired_rows(4).hint_text("Paste a strict SetupDocument, not application/connection configuration")).changed();
            let path_changed = ui.add(egui::TextEdit::singleline(&mut self.import_path).char_limit(4096).hint_text("Or a regular local file path; URLs are never fetched")).changed();
            if changed || path_changed { self.cancel(); }
            ui.horizontal(|ui| {
                if ui.button("Validate pasted setup").clicked() {
                    self.review(ImportPreview::parse(self.text.as_bytes(), self.format));
                }
                if ui.button("Read setup file for preview").clicked() {
                    self.review(ImportPreview::read(Path::new(&self.import_path), self.format));
                }
                if ui.button("Cancel import").clicked() { self.cancel(); self.message = None; }
            });
            let revision = view.registry.map(|r| r.revision);
            if revision != self.registry_revision {
                for choice in &mut self.choices { choice.source = None; choice.exact = None; }
                self.registry_revision = revision;
            }
            if let Some(reviewed) = &self.reviewed {
                let document = reviewed.document();
                ui.strong(format!("Preview: {} · {} layers (bottom → top)", document.name, document.layers.len()));
                ui.small("Isolated synthetic preview only; no live-source subscription. Conditional appearances remain absent until synthetic facts match. Configured connections may be disconnected; import does not start them.");
                self.preview.show_controls(ui);
                self.preview.update(Some(document), 255);
                setup_targets::composition_preview(ui, self.preview.frame(), None, None);
                for (binding, choice) in reviewed.bindings().iter().zip(&mut self.choices) {
                    ui.push_id((&choice.key.layer_id, &choice.key.variant_id), |ui| {
                        ui.label(format!("{} / {} · role {}{}", binding.layer_name, binding.variant_name.unwrap_or("base"), binding.source, if binding.enabled { "" } else { " (disabled, mapping still required)" }));
                        ui.small(format!("Original condition: {:?}", binding.activation));
                        if ui.button("Simulate original condition (isolated)").clicked()
                            && let Err(error) = Self::simulate(&mut self.preview, document, binding.activation) { self.message = Some(error); }
                        egui::ComboBox::from_id_salt("destination").selected_text(choice.source.as_deref().unwrap_or("Unresolved — choose local connection")).show_ui(ui, |ui| {
                            if let Some(registry) = view.registry {
                                for connection in &registry.connections {
                                    ui.selectable_value(&mut choice.source, Some(connection.descriptor.source.clone()), format!("{} · {}", connection.descriptor.display_name, connection.descriptor.source));
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut choice.exact, Some(false), "Any instance (explicit)");
                            ui.selectable_value(&mut choice.exact, Some(true), "Exact local instance");
                            if choice.exact == Some(true) {
                                ui.add(egui::TextEdit::singleline(&mut choice.instance).char_limit(128).hint_text("Local instance ID"));
                            }
                        });
                    });
                }
                ui.add(egui::TextEdit::singleline(&mut self.new_id).char_limit(128).hint_text("New local setup ID (cannot replace existing entries)"));
                let candidate = self.candidate(&view);
                match &candidate {
                    Ok(candidate) => for warning in &candidate.warnings { ui.label(format!("Capability warning (facts preserved): {warning:?}")); },
                    Err(error) => { ui.label(format!("Not ready to store: {error}")); }
                }
                if ui.add_enabled(view.can_store && candidate.is_ok(), egui::Button::new("Store imported new copy (do not switch)")).clicked() {
                    store = candidate.ok().map(|c| c.document);
                }
                ui.small("Store changes only the library. Your current editor draft stays intact. Explicit Switch loads it paused; only explicit Resume can activate lighting. Executable trust is managed separately in Connections.");
            }
            if let Some(message) = &self.message { ui.label(message); }
        });
        store
    }
}
