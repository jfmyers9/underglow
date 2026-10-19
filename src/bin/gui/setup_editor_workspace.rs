//! Bounded responsive panes; no draft or renderer state is duplicated here.
use super::*;

const PANE_GAP: f32 = 16.0;
const LAYER_RAIL: f32 = 184.0;
// One 700px keyboard/form column plus its pane inset, rail, and gap.
const MAX_WORKSPACE: f32 = 902.0;
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
        if compact {
            self.show_layer_picker(ui);
        }
        let available = ui.available_rect_before_wrap();
        // Keep one editing column at every width. Extra window width becomes
        // balanced margins, not a tall empty canvas beside a narrow inspector.
        let width = available.width().min(MAX_WORKSPACE);
        let bounds = egui::Rect::from_min_size(
            egui::pos2(available.center().x - width / 2.0, available.top()),
            egui::vec2(width, available.height()),
        );
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
        // Give the keyboard its natural aspect ratio first, but never take
        // away the inspector's independently scrolling viewport. Each surface
        // shrinks to its content; these rectangles are only overflow limits.
        let chrome = 78.0;
        let preferred = (main.width() - 2.0).min(700.0) / 18.5 * 6.4 + chrome;
        let height = preferred.min((main.height() - MIN_INSPECTOR - 12.0).max(0.0));
        let canvas = egui::Rect::from_min_size(main.min, egui::vec2(main.width(), height));
        let inspector =
            egui::Rect::from_min_max(egui::pos2(main.left(), canvas.bottom() + 12.0), main.max);
        pane(ui, "workspace-canvas", canvas, |ui| {
            self.show_workspace_canvas(ui)
        });
        pane(ui, "workspace-inspector", inspector, |ui| {
            self.show_inspector_pane(ui, compact);
        });
        ui.advance_cursor_after_rect(available);
    }

    fn show_inspector_pane(&mut self, ui: &mut egui::Ui, compact: bool) {
        let bounds = ui.max_rect();
        let background = ui.painter().add(egui::Shape::Noop);
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
            return;
        }
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 114.0).max(40.0);
            ui.allocate_ui_with_layout(
                egui::vec2(width, 28.0),
                egui::Layout::left_to_right(egui::Align::Center).with_main_align(egui::Align::Min),
                |ui| {
                    // add_sized centers its inner allocation even when the
                    // Label's galley is left-aligned. Reserve a real left slot.
                    ui.set_min_size(egui::vec2(width, 28.0));
                    ui.add(
                        egui::Label::new(egui::RichText::new(&name).size(18.0).strong())
                            .halign(egui::Align::Min)
                            .truncate(),
                    )
                },
            )
            .inner
            .on_hover_text(format!("Selected layer: {name}"));
            self.show_compact_layer_actions(ui);
        });
        egui::ScrollArea::vertical()
            .id_salt(("selected-layer-inspector", &self.selected))
            .max_height(ui.available_height())
            .auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .show(ui, |ui| {
                self.show_selected_inspector(ui, compact);
                if let Some(response) = ui
                    .memory(|memory| memory.focused())
                    .and_then(|id| ui.ctx().read_response(id))
                    && response.gained_focus()
                    && response.layer_id == ui.layer_id()
                    && ui.min_rect().contains_rect(response.rect)
                    && response.interact_rect != response.rect
                    && response.interact_rect == response.rect.intersect(ui.clip_rect())
                {
                    // Generic egui controls do not reveal themselves on Tab.
                    // Match this scroll clip, not just content bounds: scrolled
                    // content can geometrically overlap pinned toolbar controls.
                    response.scroll_to_me(None);
                }
            });
        // Resolve after layout, but paint behind the controls. Expanded forms
        // grow naturally up to the available height, then scroll in place.
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
