//! Metadata-only import UI, separate from the private executable editor.
use super::producer_editor::Replacement as Request;
use eframe::egui;
use underglow::capability_manifest::{self, ManifestFormat};
use underglow::connection_registry::ConnectionRegistry;
use underglow::source_capabilities::{MAX_DESCRIPTOR_BYTES, SourceDescriptor};

pub struct Editor {
    text: String,
    path: String,
    format: ManifestFormat,
    reviewed: Option<(u64, SourceDescriptor)>,
    replace: bool,
    error: Option<String>,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            text: String::new(),
            path: String::new(),
            format: ManifestFormat::Toml,
            reviewed: None,
            replace: false,
            error: None,
        }
    }
}

impl Editor {
    fn review(&mut self, registry: &ConnectionRegistry, result: Result<SourceDescriptor, String>) {
        self.reviewed = None;
        self.replace = false;
        match result {
            Ok(descriptor) => {
                self.reviewed = Some((registry.revision, descriptor));
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn candidate(&self, registry: &ConnectionRegistry) -> Result<Request, String> {
        let (revision, descriptor) = self.reviewed.as_ref().ok_or("Validate a manifest first")?;
        Ok(Request {
            revision: *revision,
            registry: capability_manifest::import_descriptor(
                registry,
                *revision,
                descriptor.clone(),
                self.replace,
            )?,
        })
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        registry: &ConnectionRegistry,
        can_save: bool,
    ) -> Option<Request> {
        let mut request = None;
        ui.collapsing("Import custom capability manifest (metadata only)", |ui| {
            ui.label("Strict v1 SourceDescriptor, TOML or JSON, at most 64 KiB. No code, argv, environment, working directory, trust or enable flags are accepted.");
            let old_format = self.format;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.format, ManifestFormat::Toml, "TOML");
                ui.selectable_value(&mut self.format, ManifestFormat::Json, "JSON");
            });
            let changed = ui.add(egui::TextEdit::multiline(&mut self.text)
                .char_limit(MAX_DESCRIPTOR_BYTES).desired_rows(5).hint_text("Paste descriptor text")) .changed();
            let path_changed = ui.add(egui::TextEdit::singleline(&mut self.path)
                .char_limit(4096).hint_text("Or enter a local manifest file path")) .changed();
            if changed || path_changed || old_format != self.format {
                self.reviewed = None;
                self.replace = false;
                self.error = None;
            }
            ui.horizontal(|ui| {
                if ui.button("Validate pasted manifest").clicked() {
                    self.review(registry, capability_manifest::parse(self.text.as_bytes(), self.format));
                }
                if ui.button("Read and validate file").clicked() {
                    self.review(registry, capability_manifest::read_file(std::path::Path::new(&self.path), self.format));
                }
            });
            if let Some((revision, descriptor)) = &self.reviewed {
                ui.strong(format!("{} · source {}", descriptor.display_name, descriptor.source));
                super::show_capabilities(ui, descriptor);
                let collision = registry.connection(&descriptor.source).is_some();
                if collision {
                    ui.checkbox(&mut self.replace, "Replace existing source metadata ONLY; keep its producer configuration and trust unchanged");
                    ui.small("This source ID already exists. Replacing vocabulary may leave undeclared layer facts; it does not remap them or restart the producer.");
                } else {
                    ui.label("New external-only connection. No producer is installed, trusted, enabled or started.");
                }
                if *revision != registry.revision {
                    ui.label("Registry changed since validation. Validate again and review the current collision before importing.");
                }
                if ui.add_enabled(can_save && *revision == registry.revision && (!collision || self.replace),
                    egui::Button::new("Import metadata into Connections")).clicked()
                {
                    match self.candidate(registry) {
                        Ok(candidate) => { request = Some(candidate); self.error = None; }
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            if let Some(error) = &self.error { ui.colored_label(egui::Color32::LIGHT_RED, format!("Manifest: {error}")); }
            ui.small("Import saves machine-local metadata only. It never saves a lighting draft or resumes lighting. Configure/review executable trust separately.");
        });
        request
    }
}

#[cfg(test)]
mod tests {
    use super::super::metadata_test_ui::{click, draw};
    use super::*;
    use underglow::connection_registry::ConnectorConfig;

    #[test]
    fn headless_collision_import_requires_validation_and_explicit_metadata_acknowledgment() {
        let ctx = egui::Context::default();
        let mut registry = ConnectionRegistry::default();
        registry.connections[0].producer = Some(ConnectorConfig {
            id: "reviewed".into(),
            source: "pi".into(),
            command: vec!["/never-executed".into()],
            env: [("TOKEN".into(), "PRIVATE_ENV".into())].into(),
            trusted: true,
            enabled: true,
            ..Default::default()
        });
        let original = registry.clone();
        let mut editor = Editor::default();
        let mut descriptor = registry.connections[0].descriptor.clone();
        descriptor.display_name = "Imported Pi metadata".into();
        editor.text = descriptor.to_toml().unwrap();
        assert!(editor.candidate(&registry).is_err());
        click(
            &ctx,
            "Import custom capability manifest (metadata only)",
            |ui| editor.show(ui, &registry, true),
        );
        assert!(
            click(&ctx, "Validate pasted manifest", |ui| editor
                .show(ui, &registry, true))
            .is_none()
        );
        assert!(editor.reviewed.is_some());
        assert!(
            click(&ctx, "Import metadata into Connections", |ui| editor
                .show(ui, &registry, true))
            .is_none()
        );
        click(
            &ctx,
            "Replace existing source metadata ONLY; keep its producer configuration and trust unchanged",
            |ui| editor.show(ui, &registry, true),
        );
        assert!(editor.replace);
        let request = click(&ctx, "Import metadata into Connections", |ui| {
            editor.show(ui, &registry, true)
        })
        .unwrap();
        assert_eq!(request.registry.connections[0].descriptor, descriptor);
        assert_eq!(
            request.registry.managed_configs(),
            registry.managed_configs()
        );
        assert_eq!(
            registry, original,
            "UI emits intent, never mutates the registry itself"
        );
        let mut saved = request.registry.clone();
        saved.revision += 1;
        assert!(
            editor.candidate(&saved).is_err(),
            "must validate/review after a revision change"
        );
        assert!(
            draw(&ctx, vec![], |ui| editor.show(ui, &registry, false))
                .1
                .is_none()
        );
        // Revalidation revokes any prior collision acknowledgment, including on errors.
        editor.review(&registry, Err("invalid schema".into()));
        assert!(!editor.replace && editor.reviewed.is_none());
    }
}
