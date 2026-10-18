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
    NameDraft(String),
    SaveCopy(String),
    ReloadSaved,
    NewDraft(Starter, String),
    Update {
        revision: u64,
        mutation: LibraryMutation,
    },
}

pub struct View<'a> {
    pub dirty: bool,
    pub busy: bool,
    pub connected: bool,
    pub draft_name: Option<&'a str>,
    pub setup_revision: u64,
}

#[derive(Default)]
pub struct Panel {
    /// Visibility of the host's setup-browser window, independent of destination.
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

    pub fn show(&mut self, ui: &mut egui::Ui, view: View<'_>) -> Option<Intent> {
        let mut intent = None;
        ui.scope(|ui| {
            ui.strong("Current draft");
            ui.label("Name for draft / copy");
            ui.add(egui::TextEdit::singleline(&mut self.name)
                .desired_width(ui.available_width()));
            ui.horizontal_wrapped(|ui| {
                let named = !self.name.trim().is_empty();
                if ui.add_enabled(view.draft_name.is_some() && named && !view.busy, egui::Button::new("Name draft")).clicked() {
                    intent = Some(Intent::NameDraft(self.name.trim().into()));
                }
                if ui.add_enabled(view.draft_name.is_some() && named && !view.busy && view.connected && self.report.is_some(), egui::Button::new("Save a copy")).clicked() {
                    intent = Some(Intent::SaveCopy(self.name.trim().into()));
                }
                if ui.add_enabled(!view.busy && view.connected, egui::Button::new("Reload saved version")).clicked() {
                    intent = Some(Intent::ReloadSaved);
                }
            });
            if let Some(name) = view.draft_name { ui.small(format!("Current draft: {name}")); }
            ui.small("Name draft edits locally. Save in the header persists it. Save a copy stores this draft without opening it.");
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.label("New draft:");
                for (starter, label) in [(Starter::Blank, "Blank"), (Starter::Aurora, "Aurora"), (Starter::MatrixPi, "Matrix + Pi")] {
                    if ui.add_enabled(!view.busy, egui::Button::new(label)).clicked() {
                        intent = Some(Intent::NewDraft(starter, label.into()));
                    }
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Saved setups");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!view.busy && view.connected, egui::Button::new("Refresh library")).clicked() {
                        intent = Some(Intent::Refresh);
                    }
                });
            });
            if let Some(report) = &self.report {
                let selected_name = report.setups.iter().find(|entry| Some(&entry.id) == self.selected.as_ref()).map(|entry| entry.name.as_str()).unwrap_or("Select saved setup");
                egui::ComboBox::from_id_salt("saved-setup-picker").width(ui.available_width().min(400.0)).truncate().selected_text(selected_name).show_ui(ui, |ui| {
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
                        if ui.add_enabled(!view.busy && view.connected && (!current || view.dirty), egui::Button::new("Open saved setup")).clicked() {
                            mutation = Some(LibraryMutation::Switch { id: id.clone(), expected_setup_revision: view.setup_revision });
                        }
                    });
                    ui.collapsing("Manage saved setup", |ui| {
                        ui.small("Uses the name entered above. The current setup is renamed through its draft and Save.");
                        if ui.add_enabled(!view.busy && view.connected && !self.name.trim().is_empty(), egui::Button::new("Duplicate saved setup")).clicked() {
                            mutation = Some(LibraryMutation::Duplicate { id: id.clone(), new_id: fresh_id(), name: self.name.trim().into() });
                        }
                        // Naming the current setup uses the draft + primary Save.
                        if ui.add_enabled(!view.busy && view.connected && !current && !self.name.trim().is_empty(), egui::Button::new("Rename saved setup")).clicked() {
                            mutation = Some(LibraryMutation::Rename { id: id.clone(), name: self.name.trim().into() });
                        }
                        if ui.add_enabled(!view.busy && view.connected && !current, egui::Button::new(egui::RichText::new("Delete saved setup").color(super::brand::ERROR))).clicked() { self.confirm_delete = true; }
                    });
                    if self.confirm_delete && !current {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Delete only this saved copy?");
                            if ui.add_enabled(!view.busy && view.connected, egui::Button::new(egui::RichText::new("Confirm delete setup").color(super::brand::ERROR))).clicked() { mutation = Some(LibraryMutation::Delete { id }); self.confirm_delete = false; }
                            if ui.button("Cancel delete").clicked() { self.confirm_delete = false; }
                        });
                    }
                    if let Some(mutation) = mutation { intent = Some(Intent::Update { revision: report.revision, mutation }); }
                }
            }
            if view.dirty { ui.small("Opening or creating another setup offers Save / Discard / Cancel."); }
            if !view.connected { ui.small("Engine offline: edit, name and test drafts locally. Reconnect before saving or opening a saved setup."); }
        });
        intent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_rejects_unknown_schema_without_replacing_known_library() {
        let mut panel = Panel::default();
        panel
            .receive(Report {
                schema_version: 1,
                revision: 4,
                current_id: None,
                setups: vec![],
            })
            .unwrap();
        assert!(
            panel
                .receive(Report {
                    schema_version: 2,
                    revision: 5,
                    current_id: None,
                    setups: vec![]
                })
                .is_err()
        );
        assert_eq!(panel.report.unwrap().revision, 4);
    }
}
