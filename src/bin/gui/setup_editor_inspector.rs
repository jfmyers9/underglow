//! Stack and selected-layer forms. All mutations belong to the parent Editor.
use super::*;

impl Editor {
    pub(super) fn show_layer_picker(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.draft.as_ref() else {
            return;
        };
        let title = doc
            .layers
            .iter()
            .find(|layer| Some(&layer.id) == self.selected.as_ref())
            .map(|layer| layer.name.as_str())
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

    pub(super) fn show_layer_list(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.draft.clone() else {
            return;
        };
        ui.strong("Layers");
        ui.small("Topmost first");
        let mut operation = None;
        egui::ScrollArea::vertical()
            .id_salt("layer-list")
            .max_height(ui.available_height())
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, layer) in doc.layers.iter().enumerate().rev() {
                    ui.push_id(&layer.id, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let mut enabled = layer.enabled;
                            if ui.checkbox(&mut enabled, "Enabled").changed() {
                                operation = Some(LayerEdit::Enable {
                                    id: layer.id.clone(),
                                    enabled,
                                });
                            }
                            if ui
                                .selectable_label(
                                    self.selected.as_ref() == Some(&layer.id),
                                    format!("{} · {}", index + 1, layer.name),
                                )
                                .clicked()
                            {
                                self.selected = Some(layer.id.clone());
                            }
                            for (label, offset) in [("Toward bottom", -1_isize), ("Toward top", 1)]
                            {
                                let next = index
                                    .checked_add_signed(offset)
                                    .filter(|next| *next < doc.layers.len());
                                if ui
                                    .add_enabled(next.is_some(), egui::Button::new(label))
                                    .clicked()
                                {
                                    let mut ids: Vec<_> =
                                        doc.layers.iter().map(|layer| layer.id.clone()).collect();
                                    ids.swap(index, next.unwrap());
                                    operation = Some(LayerEdit::Reorder { ids });
                                }
                            }
                        })
                    });
                }
            });
        if let Some(edit) = operation {
            self.error = self.apply(edit).err();
        }
    }

    pub(super) fn show_selected_inspector(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        // Resolve again after reorder/enable, always by stable ID, never list index.
        let Some(doc) = self.draft.clone() else {
            return;
        };
        let Some(layer) = doc.layers.iter().find(|layer| layer.id == id) else {
            return;
        };
        let mut edited = layer.clone();
        ui.separator();
        ui.push_id(&id, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Layer name");
                ui.add(egui::TextEdit::singleline(&mut edited.name).char_limit(256));
                if ui.button("Duplicate layer").clicked() {
                    let new_id = fresh_id("layer");
                    let index = doc.layers.iter().position(|layer| layer.id == id).unwrap() + 1;
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
            ui.small(format!("Stable ID: {id}"));
            // sample() is raw real-source transport data even in synthetic preview modes.
            let sample = self.preview.sample().filter(|sample| !sample.is_stale());
            self.behaviors.show(
                ui,
                &mut edited,
                self.capabilities
                    .get_or_insert_with(CapabilityRegistry::with_builtins),
                sample.map(|sample| &sample.snapshot),
                sample.map(|sample| &sample.events),
            );
            ui.separator();
            let others: Vec<_> = doc
                .layers
                .iter()
                .filter(|layer| layer.id != id)
                .map(|layer| layer.target.clone())
                .collect();
            let mut groups = doc.groups.clone();
            self.show_target_controls(ui, &id, &mut edited.target, &mut groups, &others);
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
        let Some(index) = doc
            .layers
            .iter()
            .position(|l| Some(&l.id) == self.selected.as_ref())
        else {
            return;
        };
        let layer = &doc.layers[index];
        ui.menu_button("Layer actions", |ui| {
            let mut enabled = layer.enabled;
            if ui.checkbox(&mut enabled, "Enabled").changed() {
                self.error = self
                    .apply(LayerEdit::Enable {
                        id: layer.id.clone(),
                        enabled,
                    })
                    .err();
            }
            for (label, offset) in [("Toward bottom", -1_isize), ("Toward top", 1)] {
                let next = index
                    .checked_add_signed(offset)
                    .filter(|next| *next < doc.layers.len());
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
        });
    }
}
