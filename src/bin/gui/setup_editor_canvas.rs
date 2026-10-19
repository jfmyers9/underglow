//! Pinned canonical RGB canvas and secondary preview/inspection tools.
use super::*;

#[cfg(test)]
#[path = "setup_editor_canvas_tests.rs"]
mod tests;

impl Editor {
    /// Stable Keys-inspector mount, separate from the forms' atomic patch handling.
    pub(super) fn show_target_controls(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        target: &mut Target,
        groups: &mut BTreeMap<String, Vec<String>>,
        other_targets: &[Target],
    ) {
        self.targets
            .entry(id.to_owned())
            .or_default()
            .show_controls(
                ui,
                target,
                groups,
                other_targets,
                &underglow::device::DeviceInfo::synthetic_80he(),
            );
    }

    /// Where and the keyboard are two entry points to the same target editing.
    pub(super) fn select_keys_mode(&mut self) {
        self.canvas_mode = setup_targets::CanvasMode::SelectKeys;
        if let Some(id) = &self.selected {
            self.inspector_sections
                .entry(id.clone())
                .or_default()
                .reveal(inspector::Section::Where);
        }
    }

    /// Replacing a document must not inherit group drafts or in-flight gestures.
    pub(super) fn reset_canvas_context(&mut self) {
        self.targets.clear();
        self.inspector_sections.clear();
        self.behaviors = setup_behavior::BehaviorEditor::default();
        self.rename_focus = None;
        self.rename_editing = None;
        self.empty_canvas = setup_targets::TargetSelection::default();
        self.canvas_selection = None;
        self.canvas_mode = setup_targets::CanvasMode::Preview;
        self.explained_key = None;
    }

    fn sync_canvas_selection(&mut self) {
        if self.canvas_selection != self.selected {
            if let Some(state) = self
                .canvas_selection
                .as_ref()
                .and_then(|id| self.targets.get_mut(id))
            {
                state.cancel_interaction();
            }
            self.empty_canvas.cancel_interaction();
            if let Some(id) = &self.selected {
                self.targets
                    .entry(id.clone())
                    .or_default()
                    .cancel_interaction();
            }
            self.canvas_selection = self.selected.clone();
        }
    }

    pub(super) fn show_workspace_canvas(&mut self, ui: &mut egui::Ui) {
        self.sync_canvas_selection();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Keyboard interaction").color(crate::brand::MUTED))
                .on_hover_text("Choose what clicking the on-screen keyboard does. This does not change the activity being previewed or enable physical lighting.");
            self.show_canvas_modes(ui);
            let (caption, ink) = if self.preview.error.is_some() {
                ("Preview issue · retained frame", crate::brand::WARNING)
            } else {
                ("Preview only", crate::brand::MUTED)
            };
            ui.add(egui::Label::new(egui::RichText::new(caption).small().color(ink)).truncate())
                .on_hover_text(self.preview.error.as_deref().unwrap_or(
                    "Canonical draft preview only. No hardware output; never device feedback.",
                ));
        });
        // One mode row and one private-test strip stay pinned. The board uses
        // the remaining height without sacrificing the inspector viewport.
        let width = ui
            .available_width()
            .min((ui.available_height() - 40.0).max(40.0) * 18.5 / 6.4)
            .min(700.0);
        let content = egui::Rect::from_min_size(
            ui.cursor().min + egui::vec2((ui.available_width() - width) * 0.5, 0.0),
            egui::vec2(width, width / 18.5 * 6.4),
        );
        let mut board_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("composed-keyboard")
                .max_rect(content),
        );
        // The header already exposes the component's mode/help. Clip only the
        // duplicate caption below the component, never RGB or key tooltips.
        board_ui.shrink_clip_rect(content);
        let controls = egui::Rect::from_min_size(
            egui::pos2(ui.max_rect().left(), content.bottom() + 8.0),
            egui::vec2(ui.available_width(), 28.0),
        );
        let mut controls_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("quick-preview-controls")
                .max_rect(controls),
        );
        if let Some(doc) = self.draft.as_ref() {
            let layer = doc
                .layers
                .iter()
                .find(|layer| Some(&layer.id) == self.selected.as_ref());
            self.preview_tools_open |= self.preview.show_quick_controls(
                &mut controls_ui,
                doc,
                layer,
                self.preview_brightness.unwrap_or(255),
            );
        }
        let highlighted = self.preview.coverage().and_then(|report| {
            report.layers.iter().find(|layer| {
                self.highlight_coverage && self.selected.as_deref() == Some(layer.id.as_str())
            })
        });
        let layer = self.draft.as_ref().and_then(|doc| {
            doc.layers
                .iter()
                .find(|layer| Some(&layer.id) == self.selected.as_ref())
        });
        let mut target = layer.map(|layer| layer.target.clone());
        let layer_id = layer.map(|layer| layer.id.clone());
        let device = underglow::device::DeviceInfo::synthetic_80he();
        let selection = target
            .as_mut()
            .map(|target| setup_targets::SelectionContext {
                target,
                groups: &self
                    .draft
                    .as_ref()
                    .expect("selected layer has a document")
                    .groups,
                device: &device,
            });
        let state = if let Some(id) = &layer_id {
            self.targets.entry(id.clone()).or_default()
        } else {
            &mut self.empty_canvas
        };
        let response = state.show_canvas(
            &mut board_ui,
            self.preview.frame(),
            self.canvas_mode,
            selection,
            self.explained_key,
            highlighted,
        );
        if let Some(coord) = response.inspected {
            self.explained_key = Some(coord);
            self.preview_tools_open = true;
        }
        if response.changed
            && let (Some(id), Some(target)) = (layer_id, target)
        {
            // One authoritative document, using the same scoped typed edit path
            // as the inspector. Invalid/empty selectors never enter the draft.
            self.error = self
                .apply(LayerEdit::Update {
                    id,
                    patch: LayerPatch {
                        target: Some(target),
                        ..Default::default()
                    },
                })
                .err();
        }
        ui.advance_cursor_after_rect(controls);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }

    fn show_canvas_modes(&mut self, ui: &mut egui::Ui) {
        for mode in [
            setup_targets::CanvasMode::Preview,
            setup_targets::CanvasMode::SelectKeys,
            setup_targets::CanvasMode::Inspect,
        ] {
            if ui
                    .selectable_label(self.canvas_mode == mode, mode.label())
                    .on_hover_text(format!("{} 80HE ANSI physical keys only; unsupported positions have no invented named selector.", mode.help()))
                    .clicked()
                {
                    if mode == setup_targets::CanvasMode::SelectKeys {
                        self.select_keys_mode();
                    } else {
                        self.canvas_mode = mode;
                    }
                }
        }
    }

    pub(super) fn show_preview_tools_window(&mut self, context: &egui::Context) {
        let mut open = self.preview_tools_open;
        let bounds = context.screen_rect().shrink(12.0);
        egui::Window::new("Advanced preview").open(&mut open).collapsible(false)
            .default_width(560.0).max_width(bounds.width())
            .max_height(bounds.height() - 40.0).constrain_to(bounds).show(context, |ui| {
                if ui.button("Close advanced preview").clicked() { self.preview_tools_open = false; }
                egui::ScrollArea::vertical().id_salt("workspace-preview-tools")
                    .max_height(ui.available_height()).show(ui, |ui| {
                        // A key click must reveal its actual captured explanation,
                        // not bury it below the advanced source/clock forms.
                        if self.explained_key.is_some() {
                            setup_explanation::show(ui, &self.preview, self.explained_key);
                        }
                        self.preview.show_controls(ui);
                        if self.preview.mode == setup_preview::Mode::Live && self.preview.runtime()
                            .is_some_and(|r| r.requirements().analog_input) {
                            ui.small("Input-driven layers are hidden here. Choose Isolated input demo to simulate keystrokes.");
                        }
                        if let Some(error) = &self.preview.error { ui.colored_label(egui::Color32::LIGHT_RED, error); }
                        ui.checkbox(&mut self.highlight_coverage, "Highlight selected layer coverage");
                        if self.highlight_coverage {
                            ui.small("Outlines: green = visible, red × = obscured, blue = targeted but inactive. Yellow = inspected key. RGB fills are unchanged.");
                        }
                        if self.explained_key.is_none() {
                            ui.small("Choose Inspect and click a composed key to explain it.");
                        }
                        setup_coverage::show(ui, self.preview.coverage(), self.selected.as_deref());
                    });
            });
        self.preview_tools_open &= open;
    }
}
