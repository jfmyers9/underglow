//! A small diagnostics inspector. All subprocess work stays in the controller worker.
use eframe::egui;
use serde::Deserialize;
use serde_json::Value;
use std::ffi::OsString;
use std::time::Instant;

pub enum Request {
    Refresh,
    Preview(Vec<OsString>),
}

pub struct Panel {
    live: Option<Value>,
    captured: Option<Instant>,
    preview: Option<Value>,
    error: Option<String>,
    config: String,
    source: String,
    instance: String,
    state: String,
    event: String,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            live: None,
            captured: None,
            preview: None,
            error: None,
            config: String::new(),
            source: "pi".into(),
            instance: "preview".into(),
            state: "working".into(),
            event: String::new(),
        }
    }
}

impl Panel {
    pub fn receive_live(&mut self, value: Value) {
        self.live = Some(value);
        self.captured = Some(Instant::now());
        self.error = None;
    }

    pub fn receive_preview(&mut self, result: Result<String, String>) {
        match result.and_then(|body| {
            let value: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
            if value["ok"] != true || value["diagnostics"]["schema_version"] != 1 {
                return Err("Unsupported or unsuccessful synthetic preview response".into());
            }
            Ok(value)
        }) {
            Ok(value) => {
                self.preview = Some(value);
                self.error = None;
            }
            Err(error) => {
                self.preview = None;
                self.error = Some(error);
            }
        }
    }

    pub fn error(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn show(&mut self, ui: &mut egui::Ui, connected: bool, busy: bool) -> Option<Request> {
        let mut request = None;
        ui.collapsing("Connector diagnostics", |ui| {
            ui.label("Read-only snapshots. Refresh to inspect current sources and rules.");
            if ui.add_enabled(!busy && connected, egui::Button::new("Refresh connector diagnostics")).clicked() {
                request = Some(Request::Refresh);
            }
            if let Some(captured) = self.captured {
                ui.small(format!("{} snapshot · captured {:.1}s ago (lease times are at capture)",
                    if connected { "Engine" } else { "Last seen" }, captured.elapsed().as_secs_f32()));
            }
            if let Some(value) = &self.live { show_report(ui, value); }
            if let Some(error) = &self.error { ui.colored_label(egui::Color32::LIGHT_RED, error); }
            ui.separator();
            ui.label("Isolated synthetic overlay preview (80HE)");
            ui.small("No events are published, no providers/commands run, and real lighting is untouched. Uses the file’s base effect, not live input.");
            for (label, value) in [("Profile TOML", &mut self.config), ("Source", &mut self.source),
                ("Instance", &mut self.instance), ("State", &mut self.state), ("Event (optional)", &mut self.event)] {
                ui.horizontal(|ui| { ui.label(label); ui.text_edit_singleline(value); });
            }
            if ui.add_enabled(!busy && !self.config.trim().is_empty(), egui::Button::new("Preview synthetic source")).clicked() {
                let mut args: Vec<OsString> = vec!["preview".into(), "--config".into(), self.config.clone().into(),
                    "--source".into(), self.source.clone().into(), "--instance".into(), self.instance.clone().into(),
                    "--state".into(), self.state.clone().into()];
                if !self.event.is_empty() { args.extend(["--event".into(), self.event.clone().into()]); }
                request = Some(Request::Preview(args));
            }
            if let Some(value) = &self.preview {
                show_report(ui, &value["diagnostics"]);
                show_keyboard(ui, &value["preview"]);
            }
        });
        request
    }
}

fn show_report(ui: &mut egui::Ui, value: &Value) {
    let applied = value["lighting_applied"].as_bool().unwrap_or(false);
    ui.strong(if applied {
        "Live rule history — lighting active"
    } else {
        "Not applied to hardware — potential/synthetic rules only"
    });
    if let Some(instances) = value["instances"].as_array() {
        if instances.is_empty() {
            ui.label("No leased source instances reported.");
        }
        for instance in instances {
            ui.label(format!(
                "{} / {}: {} · update {}ms ago · lease {}ms remaining",
                plain(&instance["source"]),
                plain(&instance["instance"]),
                plain(&instance["state"]),
                instance["age_ms"],
                instance["lease_remaining_ms"]
            ));
        }
    }
    if let Some(rules) = value["rules"].as_array() {
        for rule in rules {
            ui.label(format!(
                "{} → {} · matched={} active={} · visible {}/{} keys{}",
                plain(&rule["rule_id"]),
                plain(&rule["scene"]),
                rule["matched"],
                rule["active"],
                rule["visible_keys"],
                rule["targeted_keys"],
                if rule["obscured"] == true {
                    " · obscured by another overlay"
                } else {
                    ""
                }
            ));
        }
    }
    ui.collapsing("Exact diagnostics JSON", |ui| {
        let mut json = serde_json::to_string_pretty(value).unwrap_or_default();
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut json)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
            });
    });
}

fn plain(value: &Value) -> &str {
    value.as_str().unwrap_or("?")
}

#[derive(Deserialize)]
struct Preview {
    keys: Vec<Key>,
}
#[derive(Deserialize)]
struct Key {
    x: f32,
    y: f32,
    rgb: [u8; 3],
}

fn show_keyboard(ui: &mut egui::Ui, value: &Value) {
    let Ok(preview) = serde_json::from_value::<Preview>(value.clone()) else {
        return;
    };
    let scale = (ui.available_width() / 19.0).min(32.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(scale * 19.0, scale * 7.0), egui::Sense::hover());
    for key in preview.keys {
        let position = rect.min + egui::vec2(key.x * scale, key.y * scale);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(position, egui::vec2(scale - 3.0, scale - 3.0)),
            3.0,
            egui::Color32::from_rgb(key.rgb[0], key.rgb[1], key.rgb[2]),
        );
    }
}
