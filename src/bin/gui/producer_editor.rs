//! Private machine configuration drafts; no I/O, shell parsing, or lighting controls.
use eframe::egui;
use underglow::connection_registry::{ConnectionRegistry, ConnectorConfig};

pub struct Replacement {
    pub revision: u64,
    pub registry: ConnectionRegistry,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(trusted: bool, enabled: bool) -> ConnectionRegistry {
        let mut registry = ConnectionRegistry::default();
        registry.connections[0].producer = Some(ConnectorConfig {
            id: "reviewed".into(),
            source: "pi".into(),
            command: vec![
                "/never-executed".into(),
                "a b".into(),
                "$HOME; touch nope".into(),
                "".into(),
            ],
            env: [("TOKEN".into(), "private-value".into())].into(),
            trusted,
            enabled,
            ..Default::default()
        });
        registry
    }

    #[test]
    fn opening_reading_and_saving_cannot_start_or_grant_trust() {
        let registry = configured(false, false);
        let mut editor = Editor::default();
        editor.open(&registry, "pi");
        let saved = editor.save(&registry).unwrap();
        let config = saved.registry.connections[0].producer.as_ref().unwrap();
        assert!(!config.trusted && !config.enabled);
        assert_eq!(
            config.command,
            registry.connections[0].producer.as_ref().unwrap().command
        );
        assert!(Editor::set_enabled(&registry, "pi", true).is_err());
        assert!(Editor::set_enabled(&ConnectionRegistry::default(), "pi", true).is_err());
        assert_eq!(registry.revision, 0);
    }

    #[test]
    fn command_environment_and_directory_edits_revoke_trust_and_enable() {
        for field in 0..3 {
            let registry = configured(true, true);
            let mut editor = Editor::default();
            editor.open(&registry, "pi");
            let config = &mut editor.draft.as_mut().unwrap().config;
            match field {
                0 => config.command.push("changed argument".into()),
                1 => {
                    config.env.insert("TOKEN".into(), "changed value".into());
                }
                _ => config.working_dir = Some("/different/directory".into()),
            }
            Editor::changed(config);
            assert!(!config.enabled && !config.trusted);
            let saved = editor.save(&registry).unwrap().registry;
            assert!(!saved.connections[0].producer.as_ref().unwrap().enabled);
            assert!(Editor::set_enabled(&saved, "pi", true).is_err());
        }
    }

    #[test]
    fn explicit_enable_disable_patch_only_selected_saved_definition() {
        let mut registry = configured(true, false);
        let mut unrelated = registry.connections[0].clone();
        unrelated.descriptor.source = "unrelated".into();
        unrelated.producer.as_mut().unwrap().source = "unrelated".into();
        unrelated.producer.as_mut().unwrap().id = "other".into();
        registry.connections.push(unrelated.clone());
        let enabled = Editor::set_enabled(&registry, "pi", true).unwrap().registry;
        assert!(enabled.connections[0].producer.as_ref().unwrap().enabled);
        assert_eq!(enabled.connections[1], unrelated);
        assert_eq!(enabled.revision, registry.revision);
        let disabled = Editor::set_enabled(&enabled, "pi", false).unwrap().registry;
        assert_eq!(disabled, registry);
        let mut editor = Editor::default();
        editor.open(&enabled, "pi");
        let saved = editor.save(&enabled).unwrap().registry;
        assert!(!saved.connections[0].producer.as_ref().unwrap().enabled);
        assert!(saved.connections[0].producer.as_ref().unwrap().trusted);
    }

    #[test]
    fn stale_poll_and_rejected_write_keep_private_draft_and_source_guard() {
        let registry = configured(true, false);
        let mut editor = Editor::default();
        editor.open(&registry, "pi");
        editor
            .draft
            .as_mut()
            .unwrap()
            .config
            .command
            .push("private edit".into());
        let mut polled = registry.clone();
        polled.revision += 1;
        assert!(editor.save(&polled).is_err());
        assert!(
            editor
                .draft
                .as_ref()
                .unwrap()
                .config
                .command
                .contains(&"private edit".into())
        );
        editor.draft.as_mut().unwrap().config.source = "different".into();
        assert!(editor.save(&registry).is_err());
        assert!(editor.draft.is_some());
    }

    fn draw(
        editor: &mut Editor,
        context: &egui::Context,
        registry: &ConnectionRegistry,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<Replacement>) {
        let mut replacement = None;
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1800.0, 5000.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    replacement = editor.show(ui, registry, true);
                });
            },
        );
        (output, replacement)
    }

    fn click(
        editor: &mut Editor,
        context: &egui::Context,
        registry: &ConnectionRegistry,
        label: &str,
    ) -> Option<Replacement> {
        let (output, _) = draw(editor, context, registry, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing control {label}"));
        let mut result = None;
        for pressed in [true, false] {
            let (_, request) = draw(
                editor,
                context,
                registry,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            if request.is_some() {
                result = request;
            }
        }
        result
    }

    #[test]
    fn headless_gui_review_save_enable_disable_is_explicit_and_masks_secrets() {
        let registry = configured(false, false);
        let context = egui::Context::default();
        let mut editor = Editor::default();
        assert!(click(&mut editor, &context, &registry, "Configure producer").is_none());
        let (output, request) = draw(&mut editor, &context, &registry, vec![]);
        assert!(request.is_none());
        let rendered = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!rendered.contains("private-value"));
        assert!(!rendered.contains("/never-executed"));
        assert!(click(&mut editor, &context, &registry, "I reviewed this exact executable, argv, environment and directory; trust this unsandboxed code").is_none());
        let saved = click(
            &mut editor,
            &context,
            &registry,
            "Save configuration (disabled)",
        )
        .unwrap();
        assert!(
            saved.registry.connections[0]
                .producer
                .as_ref()
                .unwrap()
                .trusted
        );
        assert!(
            !saved.registry.connections[0]
                .producer
                .as_ref()
                .unwrap()
                .enabled
        );
        editor.accept_saved(&saved.registry);
        assert!(editor.draft.is_none());
        let enabled = click(
            &mut editor,
            &context,
            &saved.registry,
            "Enable reviewed producer",
        )
        .unwrap();
        assert!(
            enabled.registry.connections[0]
                .producer
                .as_ref()
                .unwrap()
                .enabled
        );
        let disabled = click(&mut editor, &context, &enabled.registry, "Disable producer").unwrap();
        assert!(
            !disabled.registry.connections[0]
                .producer
                .as_ref()
                .unwrap()
                .enabled
        );
    }
}

struct Draft {
    source: String,
    revision: u64,
    config: ConnectorConfig,
}

#[derive(Default)]
pub struct Editor {
    draft: Option<Draft>,
    selected_source: Option<String>,
    error: Option<String>,
    reveal: bool,
}

impl Editor {
    /// Explicit selection only. Health refreshes must never call this.
    pub fn open(&mut self, registry: &ConnectionRegistry, source: &str) {
        if let Some(connection) = registry.connection(source) {
            self.selected_source = Some(source.into());
            self.draft = Some(Draft {
                source: source.into(),
                revision: registry.revision,
                config: connection
                    .producer
                    .clone()
                    .unwrap_or_else(|| ConnectorConfig {
                        id: source.into(),
                        source: source.into(),
                        command: vec![String::new()],
                        ..Default::default()
                    }),
            });
            self.error = None;
            self.reveal = false;
        }
    }

    fn changed(config: &mut ConnectorConfig) {
        config.trusted = false;
        config.enabled = false;
    }

    fn save(&self, registry: &ConnectionRegistry) -> Result<Replacement, String> {
        let draft = self.draft.as_ref().ok_or("Select a registered source")?;
        if draft.revision != registry.revision {
            return Err(
                "Registry changed. Draft retained; reload saved configuration and review again."
                    .into(),
            );
        }
        let mut candidate = registry.clone();
        let connection = candidate
            .connections
            .iter_mut()
            .find(|c| c.descriptor.source == draft.source)
            .ok_or("Source is no longer registered")?;
        let mut config = draft.config.clone();
        // Saving/reviewing never starts a producer. Enable is a separate action.
        config.enabled = false;
        connection.producer = Some(config);
        candidate.validate().map_err(|_| "Invalid producer configuration. Check argv, source, unique ID, environment, directory and restart bounds.".to_string())?;
        Ok(Replacement {
            revision: candidate.revision,
            registry: candidate,
        })
    }

    fn set_enabled(
        registry: &ConnectionRegistry,
        source: &str,
        enabled: bool,
    ) -> Result<Replacement, String> {
        let mut candidate = registry.clone();
        let config = candidate
            .connections
            .iter_mut()
            .find(|c| c.descriptor.source == source)
            .and_then(|c| c.producer.as_mut())
            .ok_or("Save a reviewed producer first")?;
        if enabled && !config.trusted {
            return Err(
                "Enable cannot grant trust. Review and save the configuration first.".into(),
            );
        }
        config.enabled = enabled;
        candidate
            .validate()
            .map_err(|_| "Invalid saved producer configuration".to_string())?;
        Ok(Replacement {
            revision: candidate.revision,
            registry: candidate,
        })
    }

    /// Acknowledged saves close only the submitted, unchanged draft. Failed writes
    /// never clear drafts; polling never acknowledges writes.
    pub fn accept_saved(&mut self, submitted: &ConnectionRegistry) {
        if let Some(draft) = &self.draft {
            let mut config = draft.config.clone();
            config.enabled = false;
            if draft.revision == submitted.revision
                && submitted
                    .connection(&draft.source)
                    .and_then(|c| c.producer.as_ref())
                    == Some(&config)
            {
                self.draft = None;
                self.error = None;
            }
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        registry: &ConnectionRegistry,
        can_write: bool,
    ) -> Option<Replacement> {
        let mut result = None;
        ui.separator();
        ui.heading("Managed producer controls");
        ui.label("Unsandboxed executable code runs with your user privileges. Review the executable, every argument, environment override and working directory before granting trust.");
        ui.small("Saving never starts a producer. Enable/disable only changes saved process intent; health updates asynchronously. Lighting is never resumed here.");
        for connection in &registry.connections {
            let source = &connection.descriptor.source;
            ui.push_id(("producer", source), |ui| {
                ui.horizontal(|ui| {
                    ui.label(source);
                    if ui.button("Configure producer").clicked() {
                        self.open(registry, source);
                    }
                    if let Some(config) = &connection.producer {
                        ui.label(format!(
                            "Saved intent: enabled={} · trusted={}",
                            config.enabled, config.trusted
                        ));
                        if self.selected_source.as_deref() == Some(source.as_str())
                            && ui
                                .add_enabled(
                                    can_write && config.trusted && !config.enabled,
                                    egui::Button::new("Enable reviewed producer"),
                                )
                                .clicked()
                        {
                            result = Some(Self::set_enabled(registry, source, true));
                        }
                        if self.selected_source.as_deref() == Some(source.as_str())
                            && ui
                                .add_enabled(
                                    can_write && config.enabled,
                                    egui::Button::new("Disable producer"),
                                )
                                .clicked()
                        {
                            result = Some(Self::set_enabled(registry, source, false));
                        }
                    }
                });
            });
        }
        let mut save = false;
        let mut reload = false;
        let mut close = false;
        if let Some(draft) = &mut self.draft {
            ui.separator();
            ui.strong(format!(
                "Private draft · source {} (read-only) · base revision {}",
                draft.source, draft.revision
            ));
            if draft.revision != registry.revision {
                ui.label("Registry changed; draft is retained but cannot be saved until explicitly reloaded.");
            }
            ui.checkbox(&mut self.reveal, "Reveal private argv / environment values");
            let config = &mut draft.config;
            let mut changed = false;
            ui.label("Producer ID (unique machine-local token)");
            changed |= ui.text_edit_singleline(&mut config.id).changed();
            ui.label("argv: first item is the executable; each row is exactly one argument. No shell quoting, splitting or interpolation.");
            let mut remove = None;
            for (index, argument) in config.command.iter_mut().enumerate() {
                ui.push_id(("argv", index), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(index.to_string());
                        changed |= ui
                            .add(egui::TextEdit::singleline(argument).password(!self.reveal))
                            .changed();
                        if ui.button("Remove argument").clicked() {
                            remove = Some(index);
                        }
                    });
                });
            }
            if let Some(index) = remove {
                config.command.remove(index);
                changed = true;
            }
            if ui.button("Add argument").clicked() {
                config.command.push(String::new());
                changed = true;
            }
            ui.label("Working directory (blank inherits engine directory)");
            let mut cwd = config.working_dir.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut cwd).changed() {
                config.working_dir = (!cwd.is_empty()).then_some(cwd);
                changed = true;
            }
            ui.label(
                "Environment overrides (values masked by default; other variables are inherited)",
            );
            let mut env: Vec<_> = config
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let mut env_changed = false;
            let mut remove = None;
            for (index, (key, value)) in env.iter_mut().enumerate() {
                ui.push_id(("env", index), |ui| {
                    ui.horizontal(|ui| {
                        env_changed |= ui.text_edit_singleline(key).changed();
                        env_changed |= ui
                            .add(egui::TextEdit::singleline(value).password(!self.reveal))
                            .changed();
                        if ui.button("Remove override").clicked() {
                            remove = Some(index);
                        }
                    });
                });
            }
            if let Some(index) = remove {
                env.remove(index);
                env_changed = true;
            }
            if ui.button("Add environment override").clicked() {
                // A unique placeholder avoids silently replacing an existing key.
                let mut key = "NEW_VARIABLE".to_string();
                while config.env.contains_key(&key) {
                    key.push('_');
                }
                env.push((key, String::new()));
                env_changed = true;
            }
            if env_changed {
                // Duplicate keys must not silently collapse into authorization.
                let map: std::collections::BTreeMap<_, _> = env.iter().cloned().collect();
                if map.len() == env.len() {
                    config.env = map;
                } else {
                    self.error = Some("Duplicate environment key; edit rejected".into());
                }
                changed = true;
            }
            ui.horizontal(|ui| {
                ui.label("Initial backoff ms");
                changed |= ui
                    .add(egui::DragValue::new(&mut config.restart_initial_ms))
                    .changed();
                ui.label("Maximum backoff ms");
                changed |= ui
                    .add(egui::DragValue::new(&mut config.restart_max_ms))
                    .changed();
                ui.label("Retry limit");
                changed |= ui
                    .add(egui::DragValue::new(&mut config.restart_limit))
                    .changed();
            });
            ui.small("Initial backoff 250–60000 ms; maximum initial–300000 ms; retry limit 0–32. Shared registry validation applies.");
            if changed {
                Self::changed(config);
            }
            if ui.checkbox(&mut config.trusted, "I reviewed this exact executable, argv, environment and directory; trust this unsandboxed code").changed() {
                config.enabled = false;
            }
            ui.horizontal(|ui| {
                save = ui
                    .add_enabled(
                        can_write && draft.revision == registry.revision,
                        egui::Button::new("Save configuration (disabled)"),
                    )
                    .clicked();
                reload = ui.button("Discard draft and reload saved").clicked();
                close = ui.button("Close private draft").clicked();
            });
        }
        if save {
            result = Some(self.save(registry));
        }
        if reload && let Some(source) = self.draft.as_ref().map(|d| d.source.clone()) {
            self.open(registry, &source);
        }
        if close {
            self.draft = None;
            self.reveal = false;
        }
        let replacement = match result {
            Some(Ok(replacement)) => {
                self.error = None;
                Some(replacement)
            }
            Some(Err(error)) => {
                self.error = Some(error);
                None
            }
            None => None,
        };
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        replacement
    }
}
