//! Continuous Where / Looks / When inspector; the parent owns every draft.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Section {
    Where,
    Looks,
    When,
}

/// Disclosures and scroll intent only, never copies of the layer's data.
#[derive(Default)]
pub(super) struct Sections {
    pub keys_open: bool,
    pub behavior_open: bool,
    pub scroll_to: Option<Section>,
}

impl Sections {
    pub fn reveal(&mut self, section: Section) {
        match section {
            Section::Where => self.keys_open = true,
            Section::When => self.behavior_open = true,
            Section::Looks => {}
        }
        self.scroll_to = Some(section);
    }
}

/// Describe every selector; never pretend zones/groups are replaced by direct keys.
fn target_summary(target: &Target) -> String {
    match target {
        Target::Whole {} => "Whole keyboard".into(),
        Target::Selection {
            zones,
            keys,
            groups,
        } => {
            let mut parts = Vec::new();
            if !keys.is_empty() {
                let mut direct = keys.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
                if keys.len() > 4 {
                    direct.push_str(&format!(" + {} more keys", keys.len() - 4));
                }
                parts.push(direct);
            }
            parts.extend(zones.iter().map(|zone| format!("{zone:?} zone")));
            parts.extend(groups.iter().map(|group| format!("Group: {group}")));
            if parts.is_empty() {
                "No keys selected".into()
            } else {
                parts.join(" + ")
            }
        }
    }
}

impl Editor {
    pub(super) fn show_selected_inspector(&mut self, ui: &mut egui::Ui, compact: bool) {
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
            if self.rename_editing.as_deref() == Some(id.as_str()) {
                ui.horizontal(|ui| {
                    ui.label("Layer name");
                    let field = ui.add(egui::TextEdit::singleline(&mut edited.name)
                        .char_limit(256).desired_width((ui.available_width() - 6.0).max(50.0)));
                    if self.rename_focus.as_deref() == Some(id.as_str()) {
                        field.request_focus();
                        field.scroll_to_me(Some(egui::Align::Min));
                        self.rename_focus = None;
                    }
                    if field.lost_focus() { self.rename_editing = None; }
                });
            }
            let sections = self.inspector_sections.entry(id.clone()).or_default();
            let focus = sections.scroll_to.take();
            let mut keys_open = sections.keys_open;
            let mut behavior_open = sections.behavior_open;
            let mut groups = doc.groups.clone();

            let card = |ui: &mut egui::Ui| egui::Frame::group(ui.style())
                .inner_margin(10).corner_radius(8).fill(ui.visuals().faint_bg_color);

            let where_card = card(ui).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Where").strong().size(16.0));
                    let selecting = self.canvas_mode == setup_targets::CanvasMode::SelectKeys;
                    if ui.add(egui::Button::new("Edit keys").selected(selecting && keys_open)).on_hover_text("Select keys on the shared keyboard. Click again to finish selection.").clicked() {
                        keys_open = !keys_open || !selecting;
                        self.canvas_mode = if keys_open { setup_targets::CanvasMode::SelectKeys } else { setup_targets::CanvasMode::Preview };
                    }
                });
                ui.label(target_summary(&edited.target));
                if keys_open {
                    let others: Vec<_> = doc.layers.iter().filter(|l| l.id != id)
                        .map(|l| l.target.clone()).collect();
                    self.show_target_controls(ui, &id, &mut edited.target, &mut groups, &others);
                }
            });
            if focus == Some(Section::Where) { where_card.response.scroll_to_me(Some(egui::Align::Min)); }
            ui.add_space(8.0);

            let looks_card = card(ui).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Looks").strong().size(16.0));
                    self.behaviors.show_appearance_menu(ui, &mut edited);
                });
                if compact { self.behaviors.show_compact_appearance(ui, &mut edited); }
                else { self.behaviors.show_appearance(ui, &mut edited); }
            });
            if focus == Some(Section::Looks) { looks_card.response.scroll_to_me(Some(egui::Align::Min)); }
            ui.add_space(8.0);

            let when_card = card(ui).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("When").strong().size(16.0));
                    if ui.add(egui::Button::new("Edit condition").selected(behavior_open)).clicked() {
                        behavior_open = !behavior_open;
                    }
                });
                if behavior_open {
                    // Only real observations may populate condition suggestions.
                    let sample = self.preview.sample().filter(|s| !s.is_stale());
                    self.behaviors.show_condition(ui, &mut edited,
                        self.capabilities.get_or_insert_with(CapabilityRegistry::with_builtins),
                        sample.map(|s| &s.snapshot), sample.map(|s| &s.events));
                } else {
                    let (_, summary) = self.behaviors.summary(&edited);
                    ui.label(summary);
                }
                for notice in &self.connection_notices {
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_sized([(ui.available_width() * 0.55).min(180.0), 22.0],
                            egui::Button::new(format!("Connections: {}", notice.label)).truncate())
                            .on_hover_text(format!("{}: {}", notice.label, notice.message)).clicked() {
                            self.connection_request = Some(notice.source.clone());
                        }
                        // Preserve the full diagnostic in the tooltip without
                        // turning routine source status into a warning paragraph.
                        ui.small(notice.message.split('.').next().unwrap_or(&notice.message));
                    });
                }
            });
            if focus == Some(Section::When) { when_card.response.scroll_to_me(Some(egui::Align::Min)); }
            ui.add_space(8.0);
            let sections = self.inspector_sections.entry(id.clone()).or_default();
            sections.keys_open = keys_open;
            sections.behavior_open = behavior_open;

            ui.collapsing("Layer mixing & transitions", |ui| {
                setup_inspector::show_advanced(ui, &mut edited)
            });

            if doc.layers.iter().filter(|l| l.id.starts_with("legacy-rule-")
                && matches!(&l.activation,
                    Activation::WhileState { source, .. } | Activation::OnEvent { source, .. }
                    | Activation::OnTransition { source, .. } if source == "pi")).count() > 1 {
                ui.small("Older Pi rules remain separate layers to preserve your setup. To use one Pi status layer, save a copy, remove the old rules, then Add layer → Pi status layer.");
            }
            ui.collapsing("Technical details", |ui| {
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
