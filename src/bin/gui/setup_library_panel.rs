//! Saved-library controls; the host owns transport and the editor owns drafts.
use eframe::egui;
use serde::Deserialize;
use underglow::setup_library::{LibraryMutation, SetupSummary, Starter};

#[derive(Clone, Debug, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub revision: u64,
    pub current_id: Option<String>,
    pub setups: Vec<SetupSummary>,
}

pub enum Intent {
    Refresh,
    SaveCurrent(String),
    NewDraft(Starter, String),
    Update {
        revision: u64,
        mutation: LibraryMutation,
    },
}

#[derive(Default)]
pub struct Panel {
    pub open: bool,
    pub report: Option<Report>,
    selected: Option<String>,
    name: String,
    confirm_delete: bool,
}

pub fn fresh_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "setup-{now:x}-{:x}-{:x}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    )
}

impl Panel {
    pub fn receive(&mut self, report: Report) -> Result<(), String> {
        if report.schema_version != 1 {
            return Err("Unsupported setup library schema".into());
        }
        if !report
            .setups
            .iter()
            .any(|entry| Some(&entry.id) == self.selected.as_ref())
        {
            self.selected = report
                .current_id
                .clone()
                .or_else(|| report.setups.first().map(|entry| entry.id.clone()));
        }
        if self.name.is_empty() {
            self.name = report
                .setups
                .iter()
                .find(|entry| Some(&entry.id) == self.selected.as_ref())
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| "My setup".into());
        }
        self.report = Some(report);
        Ok(())
    }

    pub fn can_switch(dirty: bool, paused: bool, busy: bool) -> bool {
        !dirty && paused && !busy
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        dirty: bool,
        paused: bool,
        busy: bool,
        has_draft: bool,
        setup_revision: u64,
    ) -> Option<Intent> {
        let mut intent = None;
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong("Saved setups");
                if ui.add_enabled(!busy, egui::Button::new("Refresh library")).clicked() { intent = Some(Intent::Refresh); }
            });
            ui.horizontal(|ui| {
                ui.label("Setup name");
                ui.text_edit_singleline(&mut self.name);
                if ui.add_enabled(has_draft && paused && !busy && !self.name.trim().is_empty(), egui::Button::new("Save current setup")).clicked() {
                    intent = Some(Intent::SaveCurrent(self.name.trim().into()));
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("New editable copy:");
                for (starter, label) in [(Starter::Blank, "Blank"), (Starter::Aurora, "Aurora"), (Starter::MatrixPi, "Matrix + Pi")] {
                    if ui.add_enabled(!dirty && !busy, egui::Button::new(label)).clicked() {
                        intent = Some(Intent::NewDraft(starter, if self.name.trim().is_empty() { label.into() } else { self.name.trim().into() }));
                    }
                }
            });
            if let Some(report) = &self.report {
                let selected_name = report.setups.iter().find(|entry| Some(&entry.id) == self.selected.as_ref()).map(|entry| entry.name.as_str()).unwrap_or("Select saved setup");
                egui::ComboBox::from_id_salt("saved-setup-picker").selected_text(selected_name).show_ui(ui, |ui| {
                    for entry in &report.setups {
                        let label = format!("{} · {} layers{}", entry.name, entry.layers, if Some(&entry.id) == report.current_id.as_ref() { " · current" } else { "" });
                        if ui.selectable_value(&mut self.selected, Some(entry.id.clone()), label).changed() {
                            self.name = entry.name.clone();
                            self.confirm_delete = false;
                        }
                    }
                });
                if let Some(id) = self.selected.clone() {
                    let current = report.current_id.as_ref() == Some(&id);
                    let mut mutation = None;
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(Self::can_switch(dirty, paused, busy) && !current, egui::Button::new("Switch setup (paused)")).clicked() { mutation = Some(LibraryMutation::Switch { id: id.clone(), expected_setup_revision: setup_revision }); }
                        if ui.add_enabled(!busy && !self.name.trim().is_empty(), egui::Button::new("Duplicate saved setup")).clicked() { mutation = Some(LibraryMutation::Duplicate { id: id.clone(), new_id: fresh_id(), name: self.name.trim().into() }); }
                        if ui.add_enabled(!busy && !self.name.trim().is_empty() && (!current || (!dirty && paused)), egui::Button::new("Rename saved setup")).clicked() { mutation = Some(LibraryMutation::Rename { id: id.clone(), name: self.name.trim().into() }); }
                        if ui.add_enabled(!busy && !current, egui::Button::new("Delete saved setup")).clicked() { self.confirm_delete = true; }
                    });
                    if self.confirm_delete && !current {
                        ui.horizontal(|ui| {
                            ui.label("Delete only this saved copy?");
                            if ui.add_enabled(!busy, egui::Button::new("Confirm delete setup")).clicked() { mutation = Some(LibraryMutation::Delete { id }); self.confirm_delete = false; }
                            if ui.button("Cancel delete").clicked() { self.confirm_delete = false; }
                        });
                    }
                    if let Some(mutation) = mutation { intent = Some(Intent::Update { revision: report.revision, mutation }); }
                }
            }
            if dirty { ui.small("Save or Discard the draft before switching or creating another composition."); }
            ui.small("Switching requires Pause and never resumes lighting. Starter layers are editable copies, not recurring defaults.");
        });
        intent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn switching_and_starters_cannot_silently_discard_dirty_drafts() {
        assert!(!Panel::can_switch(true, true, false));
        assert!(!Panel::can_switch(false, false, false));
        assert!(!Panel::can_switch(false, true, true));
        assert!(Panel::can_switch(false, true, false));
        let mut panel = Panel::default();
        panel
            .receive(Report {
                schema_version: 1,
                revision: 4,
                current_id: Some("daily".into()),
                setups: vec![SetupSummary {
                    id: "daily".into(),
                    name: "Everyday".into(),
                    layers: 2,
                }],
            })
            .unwrap();
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(panel.show(ui, true, true, false, true, 9).is_none());
            });
        });
        assert_eq!(panel.report.as_ref().unwrap().revision, 4);
    }

    #[test]
    fn headless_switch_click_is_disabled_for_dirty_drafts_and_carries_both_cas_tokens() {
        let mut panel = Panel::default();
        panel
            .receive(Report {
                schema_version: 1,
                revision: 4,
                current_id: Some("daily".into()),
                setups: vec![SetupSummary {
                    id: "copy".into(),
                    name: "Customized".into(),
                    layers: 0,
                }],
            })
            .unwrap();
        panel.selected = Some("copy".into());
        let ctx = egui::Context::default();
        let frame = |panel: &mut Panel, dirty, events| {
            let mut intent = None;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        intent = panel.show(ui, dirty, true, false, true, 9);
                    });
                },
            );
            (output, intent)
        };
        for dirty in [true, false] {
            let (output, _) = frame(&mut panel, dirty, vec![]);
            let pos = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Switch setup (paused)" => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .expect("switch button");
            let mut clicked = None;
            for pressed in [true, false] {
                let (_, intent) = frame(
                    &mut panel,
                    dirty,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
                clicked = clicked.or(intent);
            }
            if dirty {
                assert!(clicked.is_none());
            } else {
                assert!(
                    matches!(clicked, Some(Intent::Update { revision: 4, mutation: LibraryMutation::Switch { id, expected_setup_revision: 9 } }) if id == "copy")
                );
            }
        }
    }
}
