//! Bounded responsive panes; no draft or renderer state is duplicated here.
use super::*;

const PANE_GAP: f32 = 16.0;
const LAYER_RAIL: f32 = 184.0;
const WIDE_INSPECTOR: f32 = 320.0;
const MIN_INSPECTOR: f32 = 177.0;

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
        let compact = ui.available_width() < 760.0;
        let wide = ui.available_width() >= 1200.0;
        if compact {
            self.show_layer_picker(ui);
        }
        let bounds = ui.available_rect_before_wrap();
        let main = if compact {
            bounds
        } else {
            let layers =
                egui::Rect::from_min_size(bounds.min, egui::vec2(LAYER_RAIL, bounds.height()));
            pane(ui, "workspace-layers", layers, |ui| {
                self.show_layer_list(ui)
            });
            egui::Rect::from_min_max(
                egui::pos2(layers.right() + PANE_GAP, bounds.top()),
                bounds.max,
            )
        };
        let (canvas, inspector) = if wide {
            let inspector = egui::Rect::from_min_max(
                egui::pos2(main.right() - WIDE_INSPECTOR, main.top()),
                main.max,
            );
            let canvas = egui::Rect::from_min_max(
                main.min,
                egui::pos2(inspector.left() - PANE_GAP, main.bottom()),
            );
            (canvas, inspector)
        } else {
            // Give the keyboard its natural aspect ratio first, but never take
            // away the compact inspector's independently scrolling viewport.
            let chrome = 78.0;
            let preferred = (main.width() - 2.0).min(700.0) / 18.5 * 6.4 + chrome;
            let height = preferred.min((main.height() - MIN_INSPECTOR - 12.0).max(0.0));
            let canvas = egui::Rect::from_min_size(main.min, egui::vec2(main.width(), height));
            let inspector =
                egui::Rect::from_min_max(egui::pos2(main.left(), canvas.bottom() + 12.0), main.max);
            (canvas, inspector)
        };
        pane(ui, "workspace-canvas", canvas, |ui| {
            self.show_workspace_canvas(ui)
        });
        pane(ui, "workspace-inspector", inspector, |ui| {
            self.show_inspector_pane(ui, compact);
        });
        ui.advance_cursor_after_rect(bounds);
    }

    fn show_inspector_pane(&mut self, ui: &mut egui::Ui, compact: bool) {
        let bounds = ui.max_rect();
        ui.painter().rect_filled(bounds, 6, crate::brand::SURFACE);
        let mut content = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("inspector-surface")
                .max_rect(bounds.shrink(12.0))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let ui = &mut content;
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
            let width = (ui.available_width() - 114.0).max(40.0);
            ui.add_sized(
                [width, 28.0],
                egui::Label::new(egui::RichText::new(&name).size(18.0).strong()).truncate(),
            )
            .on_hover_text(format!("Selected layer: {name}"));
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
