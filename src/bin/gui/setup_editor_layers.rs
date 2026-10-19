//! Layer navigation and stable-ID actions; the editor owns the only draft.
use super::*;

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
        let Some(doc) = self.draft.clone() else {
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
                .width((ui.available_width() - 116.0).max(100.0))
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
            self.show_add_layer_menu(ui);
        });
    }

    pub(super) fn layer_menu(&mut self, ui: &mut egui::Ui, doc: &SetupDocument, id: &str) {
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
            self.rename_editing = Some(id.into());
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
        let bounds = ui.max_rect();
        let background = ui.painter().add(egui::Shape::Noop);
        let mut content = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("layer-list-surface")
                .max_rect(bounds.shrink(12.0))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let ui = &mut content;
        ui.horizontal(|ui| {
            ui.strong("Layers");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.show_add_layer_menu(ui);
            });
        });
        ui.label(
            egui::RichText::new("Topmost first")
                .small()
                .color(crate::brand::MUTED),
        );
        crate::brand::divider(ui);
        egui::ScrollArea::vertical()
            .id_salt("layer-list")
            .max_height(ui.available_height())
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                for layer in doc.layers.iter().rev() {
                    ui.push_id(&layer.id, |ui| {
                        // Reserve the background before widgets so the full-row
                        // selection never paints over native hover/focus outlines.
                        let background = ui.painter().add(egui::Shape::Noop);
                        let row_width = ui.available_width();
                        let mut name_hovered = false;
                        let mut name_focused = false;
                        let row = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
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
                            let width = (ui.available_width() - 40.0).max(28.0);
                            // A vertical slot gives truncation a bounded wrap width even
                            // when a long label has expanded a horizontal row's min rect.
                            if ui
                                .allocate_ui_with_layout(
                                    egui::vec2(width, 32.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        let response = ui.add_sized(
                                            [width, 32.0],
                                            egui::Button::new(&layer.name).frame(false).truncate(),
                                        );
                                        name_hovered = response.hovered();
                                        name_focused = response.has_focus();
                                        response
                                    },
                                )
                                .inner
                                .on_hover_text(&layer.name)
                                .clicked()
                            {
                                self.selected = Some(layer.id.clone());
                            }
                            ui.menu_button("...", |ui| self.layer_menu(ui, &doc, &layer.id))
                                .response
                                .on_hover_text("Layer actions");
                        });
                        let row_rect = egui::Rect::from_min_size(
                            row.response.rect.min,
                            egui::vec2(row_width, row.response.rect.height()),
                        );
                        let selected = self.selected.as_ref() == Some(&layer.id);
                        if selected || name_hovered {
                            ui.painter().set(
                                background,
                                egui::Shape::rect_filled(
                                    row_rect,
                                    5,
                                    if selected {
                                        ui.visuals().selection.bg_fill
                                    } else {
                                        crate::brand::RAISED
                                    },
                                ),
                            );
                        }
                        if name_focused {
                            // The quiet name button still needs a focus cue,
                            // but it belongs to the row, not a box around text.
                            ui.painter().rect_stroke(
                                row_rect,
                                5,
                                egui::Stroke::new(2.0, crate::brand::ACCENT),
                                egui::StrokeKind::Inside,
                            );
                        }
                    });
                }
                if let Some(response) = ui
                    .memory(|memory| memory.focused())
                    .and_then(|id| ui.ctx().read_response(id))
                    && response.gained_focus()
                    && response.layer_id == ui.layer_id()
                    && ui.min_rect().contains_rect(response.rect)
                    && response.interact_rect != response.rect
                    && response.interact_rect == response.rect.intersect(ui.clip_rect())
                {
                    // Reveal real list focus without letting scrolled content
                    // geometrically capture pinned toolbar or canvas controls.
                    response.scroll_to_me(None);
                }
            });
        let panel = egui::Rect::from_min_max(
            bounds.min,
            egui::pos2(
                bounds.right(),
                (ui.min_rect().bottom() + 12.0).min(bounds.bottom()),
            ),
        );
        ui.painter().set(
            background,
            egui::Shape::rect_filled(panel, 6, crate::brand::SURFACE),
        );
    }
}
