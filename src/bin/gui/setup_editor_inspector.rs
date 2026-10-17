//! Selected-layer tabs and stack actions; the parent Editor owns every draft.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Appearance,
    Keys,
    Behavior,
}

impl Editor {
    pub(super) fn show_add_layer_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Add layer", |ui| {
            for (id, label) in [
                ("aurora", "Aurora layer"),
                ("matrix", "Matrix layer"),
                ("solid", "Solid color layer"),
            ] {
                if ui.button(label).clicked() {
                    self.error = self.add_effect(id).err();
                    ui.close_menu();
                }
            }
            if ui.button("Pi status layer").clicked() {
                self.error = self.add_pi_template().err();
                ui.close_menu();
            }
            ui.menu_button("Other effects", |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("add-effect-choices")
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for entry in underglow::catalog::presets()
                            .filter(|e| !matches!(e.id, "aurora" | "matrix"))
                        {
                            if ui.button(entry.title).clicked() {
                                self.error = self.add_effect(entry.id).err();
                                ui.close_menu();
                            }
                        }
                        for (id, label) in [("pulse", "Pulse"), ("sweep", "Sweep")] {
                            if ui.button(label).clicked() {
                                self.error = self.add_effect(id).err();
                                ui.close_menu();
                            }
                        }
                    });
            });
            ui.small("Adds an editable layer above the stack. Never enables lighting.");
        });
    }

    pub(super) fn show_layer_picker(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.draft.as_ref() else {
            return;
        };
        let title = doc
            .layers
            .iter()
            .find(|l| Some(&l.id) == self.selected.as_ref())
            .map(|l| l.name.as_str())
            .unwrap_or("No layers");
        ui.horizontal(|ui| {
            ui.label("Layer:");
            egui::ComboBox::from_id_salt("workspace-layer-picker")
                .width((ui.available_width() - 24.0).max(100.0))
                .truncate()
                .selected_text(title)
                .show_ui(ui, |ui| {
                    for layer in doc.layers.iter().rev() {
                        ui.selectable_value(
                            &mut self.selected,
                            Some(layer.id.clone()),
                            &layer.name,
                        );
                    }
                });
        });
    }

    fn layer_menu(&mut self, ui: &mut egui::Ui, doc: &SetupDocument, id: &str) {
        let Some(index) = doc.layers.iter().position(|l| l.id == id) else {
            return;
        };
        let layer = &doc.layers[index];
        let mut enabled = layer.enabled;
        if ui.checkbox(&mut enabled, "Enabled").changed() {
            self.error = self
                .apply(LayerEdit::Enable {
                    id: id.into(),
                    enabled,
                })
                .err();
        }
        if ui.button("Rename layer").clicked() {
            self.selected = Some(id.into());
            self.rename_focus = Some(id.into());
            ui.close_menu();
        }
        if ui.button("Duplicate layer").clicked() {
            let new_id = fresh_id("layer");
            self.error = self
                .apply(LayerEdit::Duplicate {
                    id: id.into(),
                    new_id: new_id.clone(),
                    index: index + 1,
                })
                .err();
            if self.error.is_none() {
                self.selected = Some(new_id);
            }
            ui.close_menu();
        }
        for (label, offset) in [("Toward top", 1_isize), ("Toward bottom", -1)] {
            let next = index
                .checked_add_signed(offset)
                .filter(|n| *n < doc.layers.len());
            if ui
                .add_enabled(next.is_some(), egui::Button::new(label))
                .clicked()
            {
                let mut ids: Vec<_> = doc.layers.iter().map(|l| l.id.clone()).collect();
                ids.swap(index, next.unwrap());
                self.error = self.apply(LayerEdit::Reorder { ids }).err();
                ui.close_menu();
            }
        }
        if ui.button("Delete layer").clicked() {
            self.error = self.apply(LayerEdit::Remove { id: id.into() }).err();
            ui.close_menu();
        }
    }

    pub(super) fn show_layer_list(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.draft.clone() else {
            return;
        };
        ui.strong("Layers");
        ui.small("Topmost first");
        egui::ScrollArea::vertical()
            .id_salt("layer-list")
            .max_height(ui.available_height())
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, layer) in doc.layers.iter().enumerate().rev() {
                    ui.push_id(&layer.id, |ui| {
                        ui.horizontal(|ui| {
                            let mut enabled = layer.enabled;
                            if ui
                                .checkbox(&mut enabled, "")
                                .on_hover_text("Enable layer")
                                .changed()
                            {
                                self.error = self
                                    .apply(LayerEdit::Enable {
                                        id: layer.id.clone(),
                                        enabled,
                                    })
                                    .err();
                            }
                            let width = (ui.clip_rect().width() - 72.0).max(28.0);
                            // A vertical slot gives truncation a bounded wrap width even
                            // when a long label has expanded a horizontal row's min rect.
                            if ui
                                .allocate_ui_with_layout(
                                    egui::vec2(width, 26.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.add_sized(
                                            [width, 26.0],
                                            egui::Button::new(format!(
                                                "{} · {}",
                                                index + 1,
                                                layer.name
                                            ))
                                            .selected(self.selected.as_ref() == Some(&layer.id))
                                            .truncate(),
                                        )
                                    },
                                )
                                .inner
                                .on_hover_text(&layer.name)
                                .clicked()
                            {
                                self.selected = Some(layer.id.clone());
                            }
                            ui.menu_button("⋯", |ui| self.layer_menu(ui, &doc, &layer.id))
                                .response
                                .on_hover_text("Layer actions");
                        });
                    });
                }
            });
    }

    pub(super) fn show_selected_inspector(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let Some(doc) = self.draft.clone() else {
            return;
        };
        let Some(layer) = doc.layers.iter().find(|l| l.id == id) else {
            return;
        };
        let mut edited = layer.clone();
        ui.push_id(&id, |ui| {
            ui.horizontal(|ui| {
                ui.label("Layer name");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut edited.name)
                        .char_limit(256)
                        .desired_width((ui.available_width() - 6.0).max(50.0)),
                );
                if self.rename_focus.as_deref() == Some(id.as_str()) {
                    field.request_focus();
                    self.rename_focus = None;
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Duplicate layer").clicked() {
                    let new_id = fresh_id("layer");
                    let index = doc.layers.iter().position(|l| l.id == id).unwrap() + 1;
                    self.error = self
                        .apply(LayerEdit::Duplicate {
                            id: id.clone(),
                            new_id: new_id.clone(),
                            index,
                        })
                        .err();
                    if self.error.is_none() {
                        self.selected = Some(new_id);
                    }
                }
                if ui.button("Delete layer").clicked() {
                    self.error = self.apply(LayerEdit::Remove { id: id.clone() }).err();
                }
            });
            let tab = self.inspector_tabs.entry(id.clone()).or_default();
            let mut select_keys = false;
            ui.horizontal(|ui| {
                ui.selectable_value(tab, Tab::Appearance, "Appearance");
                select_keys = ui.selectable_value(tab, Tab::Keys, "Keys").clicked();
                ui.selectable_value(tab, Tab::Behavior, "Behavior");
            });
            let tab = *tab;
            if select_keys {
                self.select_keys_mode();
            }
            let mut groups = doc.groups.clone();
            match tab {
                Tab::Appearance => self.behaviors.show_appearance(ui, &mut edited),
                Tab::Behavior => {
                    // Only real observations are offered, even while synthetic preview is selected.
                    let sample = self.preview.sample().filter(|s| !s.is_stale());
                    self.behaviors.show_behavior(
                        ui,
                        &mut edited,
                        self.capabilities
                            .get_or_insert_with(CapabilityRegistry::with_builtins),
                        sample.map(|s| &s.snapshot),
                        sample.map(|s| &s.events),
                    );
                }
                Tab::Keys => {
                    let others: Vec<_> = doc
                        .layers
                        .iter()
                        .filter(|l| l.id != id)
                        .map(|l| l.target.clone())
                        .collect();
                    self.show_target_controls(ui, &id, &mut edited.target, &mut groups, &others);
                }
            }
            ui.collapsing("Advanced layer details", |ui| {
                ui.small(format!("Stable ID: {id}"))
            });
            if edited != *layer || groups != doc.groups {
                // Group edits are a separate declarative map; validate atomically with
                // the selected layer patch, retaining every other layer byte-for-byte.
                let mut current = self.draft.clone().unwrap();
                current.groups = groups;
                let patch = LayerPatch {
                    transition: (edited.transition != layer.transition)
                        .then_some(edited.transition),
                    opacity: (edited.opacity != layer.opacity).then_some(edited.opacity),
                    blend: (edited.blend != layer.blend).then_some(edited.blend),
                    name: (edited.name != layer.name).then_some(edited.name),
                    renderer: (edited.renderer != layer.renderer).then_some(edited.renderer),
                    brightness: (edited.brightness != layer.brightness)
                        .then_some(edited.brightness),
                    target: (edited.target != layer.target).then_some(edited.target),
                    activation: (edited.activation != layer.activation)
                        .then_some(edited.activation),
                    variants: (edited.variants != layer.variants).then_some(edited.variants),
                };
                match apply_edits(
                    &current,
                    current.revision,
                    &[LayerEdit::Update {
                        id: id.clone(),
                        patch,
                    }],
                ) {
                    Ok(mut candidate) => {
                        candidate.revision = self.base_revision;
                        self.draft = Some(candidate);
                        self.dirty = true;
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
        });
    }

    pub(super) fn show_compact_layer_actions(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.draft.clone() else {
            return;
        };
        let Some(id) = self.selected.clone() else {
            return;
        };
        ui.menu_button("Layer actions", |ui| self.layer_menu(ui, &doc, &id));
    }
}
