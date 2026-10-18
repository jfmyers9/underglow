//! Bounded responsive panes; no draft or renderer state is duplicated here.
use super::*;

fn pane(ui: &mut egui::Ui, id: &str, rect: egui::Rect, show: impl FnOnce(&mut egui::Ui)) {
    // Leave a pixel for font-position snapping without growing the pane's clip.
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect.shrink(1.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.shrink_clip_rect(rect);
    show(&mut child);
}

impl Editor {
    pub(super) fn show_workspace(&mut self, ui: &mut egui::Ui) {
        let compact = ui.available_width() < 880.0;
        if compact {
            self.show_layer_picker(ui);
        }
        let bounds = ui.available_rect_before_wrap();
        if compact {
            // A compact single-row header/test strip leaves real room for the
            // keyboard and the inspector; extra height is not caption chrome.
            let canvas_height = 226.0_f32.min((bounds.height() - 146.0).max(0.0));
            let canvas =
                egui::Rect::from_min_size(bounds.min, egui::vec2(bounds.width(), canvas_height));
            let inspector = egui::Rect::from_min_max(
                egui::pos2(bounds.left(), canvas.bottom() + 6.0),
                bounds.max,
            );
            pane(ui, "workspace-canvas", canvas, |ui| {
                self.show_workspace_canvas(ui, true)
            });
            pane(ui, "workspace-inspector", inspector, |ui| {
                self.show_inspector_pane(ui, true);
            });
        } else {
            let left = (bounds.width() * 0.18).clamp(150.0, 200.0);
            let right = (bounds.width() * 0.33).clamp(280.0, 360.0);
            let layers = egui::Rect::from_min_size(bounds.min, egui::vec2(left, bounds.height()));
            let inspector = egui::Rect::from_min_max(
                egui::pos2(bounds.right() - right, bounds.top()),
                bounds.max,
            );
            let canvas = egui::Rect::from_min_max(
                egui::pos2(layers.right() + 8.0, bounds.top()),
                egui::pos2(inspector.left() - 8.0, bounds.bottom()),
            );
            pane(ui, "workspace-layers", layers, |ui| {
                self.show_layer_list(ui)
            });
            pane(ui, "workspace-canvas", canvas, |ui| {
                self.show_workspace_canvas(ui, false)
            });
            pane(ui, "workspace-inspector", inspector, |ui| {
                self.show_inspector_pane(ui, false)
            });
        }
        ui.advance_cursor_after_rect(bounds);
    }

    fn show_inspector_pane(&mut self, ui: &mut egui::Ui, compact: bool) {
        let name = self
            .draft
            .as_ref()
            .and_then(|doc| {
                doc.layers
                    .iter()
                    .find(|l| Some(&l.id) == self.selected.as_ref())
            })
            .map(|l| l.name.clone())
            .unwrap_or_else(|| "No layer selected".into());
        if self.draft.as_ref().is_some_and(|doc| doc.layers.is_empty()) {
            ui.label("Empty composition · no hidden layers or Pi behavior. Add a layer to begin.");
            return;
        }
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 110.0).max(60.0);
            ui.add_sized(
                [width, 26.0],
                egui::Label::new(egui::RichText::new(format!("Selected layer: {name}")).strong())
                    .truncate(),
            )
            .on_hover_text(&name);
            self.show_compact_layer_actions(ui);
        });
        egui::ScrollArea::vertical()
            .id_salt(("selected-layer-inspector", &self.selected))
            .max_height(ui.available_height())
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| self.show_selected_inspector(ui, compact));
    }
}
