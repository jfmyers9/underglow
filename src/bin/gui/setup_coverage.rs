//! Read-only coverage of the exact displayed preview evaluation.
use eframe::egui;
use underglow::explanations::{CoverageReport, CoverageStatus, DeadlineKind};

pub fn show(ui: &mut egui::Ui, report: Option<&CoverageReport>, selected: Option<&str>) {
    egui::CollapsingHeader::new("Layer coverage and expiry")
        .id_salt("layer-coverage").default_open(true).show(ui, |ui| {
        let Some(report) = report else {
            ui.small("Coverage unavailable: fix invalid draft; retained RGB is last-good preview.");
            return;
        };
        ui.small(format!("{} · evaluated at {:?}", report.mode.label(), report.evaluated_at));
        ui.small("Scenario/live preview only — not device feedback. Counts are LED positions, including shared/blended contributors. Bottom → top; normal at 100% replaces, including black. Highlight the selected layer above; edit targets, opacity/blend or stack order below.");
        egui::ScrollArea::vertical().id_salt("coverage-rows").max_height(210.0).show(ui, |ui| {
            for layer in &report.layers {
                ui.push_id(&layer.id, |ui| {
                    let badge = match layer.status {
                        CoverageStatus::Disabled => "disabled",
                        CoverageStatus::Inactive => "inactive",
                        CoverageStatus::InputUnavailable => "input unavailable",
                        CoverageStatus::Noncovering => "noncovering",
                        CoverageStatus::Visible => "visible",
                        CoverageStatus::PartiallyObscured => "shared/blended or partly obscured",
                        CoverageStatus::Transparent => "transparent (0% opacity)",
                        CoverageStatus::FullyObscured => "fully obscured",
                    };
                    let label = format!("{}#{} {} · {} · target {} / visible {} / shared {} / obscured {} / overlapping {}",
                        if selected == Some(&layer.id) { "Selected · " } else { "" },
                        layer.index + 1, layer.name, badge, layer.targeted.len(), layer.visible.len(), layer.blended.len(),
                        layer.obscured.len(), layer.overlapping.len());
                    ui.label(label).on_hover_text(&layer.reason);
                    if let Some(transition) = &layer.transition {
                        super::setup_explanation::show_transition(ui, transition);
                    }
                    if !layer.obscured_by.is_empty() {
                        let names: Vec<_> = layer.obscured_by.iter().map(|&index|
                            format!("#{} {}", index + 1, report.layers[index].name)).collect();
                        ui.small(format!("Hidden by {}", names.join(", ")));
                    }
                    if let Some(deadline) = layer.deadline {
                        let kind = match deadline.kind {
                            DeadlineKind::Transient => "Transient expiry",
                            DeadlineKind::SourceLease => "Matching state availability lease",
                        };
                        ui.small(format!("{} · {} · {:.3}s remaining · deadline {:?}",
                            layer.selected_variant.as_deref().unwrap_or("Base / fallback"), kind,
                            deadline.remaining.as_secs_f64(), deadline.at));
                    } else if layer.status == CoverageStatus::Inactive {
                        ui.small(&layer.reason);
                        ui.small("No cached active deadline; no countdown inferred.");
                    }
                });
            }
        });
    });
}
