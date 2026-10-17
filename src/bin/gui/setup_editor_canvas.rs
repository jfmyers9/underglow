//! Pinned canonical RGB canvas and secondary preview/inspection tools.
use super::*;

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
        self.targets.entry(id.to_owned()).or_default().show(
            ui,
            target,
            groups,
            other_targets,
            &underglow::device::DeviceInfo::synthetic_80he(),
        );
    }

    pub(super) fn show_workspace_canvas(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Composition preview");
            if ui.button("Preview tools").clicked() {
                self.preview_tools_open = true;
            }
        });
        let highlighted = self.preview.coverage().and_then(|report| {
            report.layers.iter().find(|layer| {
                self.highlight_coverage && self.selected.as_deref() == Some(layer.id.as_str())
            })
        });
        let width = ui
            .available_width()
            .min((ui.available_height() - 28.0).max(40.0) * 18.5 / 6.4)
            .min(700.0);
        let board = egui::Rect::from_min_size(
            ui.cursor().min + egui::vec2((ui.available_width() - width) * 0.5, 0.0),
            egui::vec2(width, width / 18.5 * 6.4),
        );
        let mut board_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("composed-keyboard")
                .max_rect(board),
        );
        if let Some(coord) = setup_targets::composition_preview(
            &mut board_ui,
            self.preview.frame(),
            self.explained_key,
            highlighted,
        ) {
            self.explained_key = Some(coord);
        }
        ui.advance_cursor_after_rect(board);
        let context = match self.preview.mode {
            setup_preview::Mode::Live => "Live facts",
            setup_preview::Mode::InputDemo => "Isolated input demo",
            setup_preview::Mode::Scenario if self.preview.scenario_playing() => "Scenario running",
            setup_preview::Mode::Scenario => "Scenario paused",
        };
        let caption = if self.preview.error.is_some() {
            format!("Preview only · {context} · retained last-valid frame")
        } else {
            format!("Preview only · {context} · no hardware output")
        };
        ui.add(egui::Label::new(egui::RichText::new(caption).small()).truncate())
            .on_hover_text(
                self.preview
                    .error
                    .as_deref()
                    .unwrap_or("Canonical draft preview, never device feedback."),
            );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }

    pub(super) fn show_preview_tools_window(&mut self, context: &egui::Context) {
        let mut open = self.preview_tools_open;
        let bounds = context.screen_rect().shrink(12.0);
        egui::Window::new("Preview tools").open(&mut open).collapsible(false)
            .default_width(560.0).max_width(bounds.width())
            .max_height(bounds.height() - 40.0).constrain_to(bounds).show(context, |ui| {
                if ui.button("Close preview tools").clicked() { self.preview_tools_open = false; }
                egui::ScrollArea::vertical().id_salt("workspace-preview-tools")
                    .max_height(ui.available_height()).show(ui, |ui| {
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
                        setup_explanation::show(ui, &self.preview, self.explained_key);
                        setup_coverage::show(ui, self.preview.coverage(), self.selected.as_deref());
                    });
            });
        self.preview_tools_open &= open;
    }
}
