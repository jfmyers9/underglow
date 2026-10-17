//! Generic single-source scenario input. Only writes the private Scenario.
use eframe::egui;
use underglow::scenarios::Scenario;
use underglow::source_capabilities::CapabilityRegistry;

pub struct Controls {
    pub capabilities: CapabilityRegistry,
    source: String,
    instance: String,
    ttl_ms: u64,
    error: Option<String>,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            capabilities: CapabilityRegistry::with_builtins(),
            source: String::new(),
            instance: "scenario-session".into(),
            ttl_ms: 15_000,
            error: None,
        }
    }
}

impl Controls {
    pub fn show(&mut self, ui: &mut egui::Ui, scenario: &mut Scenario) {
        let previous = self.source.clone();
        if self.capabilities.descriptor(&self.source).is_none() {
            self.source = self
                .capabilities
                .descriptors()
                .next()
                .map(|d| d.source.clone())
                .unwrap_or_default();
        }
        egui::ComboBox::from_id_salt("scenario-source")
            .selected_text(
                self.capabilities
                    .descriptor(&self.source)
                    .map_or("No declared sources", |d| d.display_name.as_str()),
            )
            .show_ui(ui, |ui| {
                for descriptor in self.capabilities.descriptors() {
                    ui.selectable_value(
                        &mut self.source,
                        descriptor.source.clone(),
                        &descriptor.display_name,
                    );
                }
            });
        if previous != self.source {
            scenario.reset();
        }
        let Some(descriptor) = self.capabilities.descriptor(&self.source) else {
            ui.small("Supply source capabilities to offer scenario states/events.");
            return;
        };
        ui.small(format!(
            "Source: {} · {:?} identities · selectors {:?}",
            descriptor.source, descriptor.instances.semantics, descriptor.instances.selectors
        ));
        ui.horizontal(|ui| {
            ui.label("Session identity");
            ui.add(egui::TextEdit::singleline(&mut self.instance).char_limit(128));
            ui.label("TTL ms");
            // Deliberately permit zero/out-of-range edits: protocol validation
            // rejects them atomically with a repairable error, never clamps.
            ui.add(egui::DragValue::new(&mut self.ttl_ms));
        });
        ui.small("Any-instance conditions match every session; exact conditions match this identity. Change identity and publish again to retain independent session leases.");
        ui.horizontal_wrapped(|ui| {
            ui.label("State:");
            for fact in &descriptor.states {
                if ui
                    .button(&fact.label)
                    .on_hover_text(format!(
                        "{} · declared state TTL {:?} · test ingress limit {} ms",
                        fact.id,
                        descriptor.lifecycle.state_ttl,
                        scenario.limits().max_ttl.as_millis()
                    ))
                    .clicked()
                {
                    self.error = scenario
                        .publish_state(&self.source, &self.instance, &fact.id, self.ttl_ms)
                        .err()
                        .map(|e| e.to_string());
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Event:");
            for fact in &descriptor.events {
                if ui
                    .button(&fact.label)
                    .on_hover_text(format!(
                        "{} · occurrence only, does not renew state · declared event TTL {:?}",
                        fact.id, descriptor.lifecycle.event_ttl
                    ))
                    .clicked()
                {
                    self.error = scenario
                        .publish_event(&self.source, &self.instance, &fact.id, self.ttl_ms)
                        .err()
                        .map(|e| e.to_string());
                }
            }
            if ui.button("Disconnect / remove session").clicked() {
                self.error = scenario
                    .remove(&self.source, &self.instance)
                    .err()
                    .map(|e| e.to_string());
            }
        });
        ui.small("Time advances locally: leases expire without renewal. Removal drops state, not still-leased events; it never means finished/success.");
        ui.small(format!(
            "Scenario time: {:.2}s",
            scenario.now().as_secs_f64()
        ));
        for state in scenario.snapshot().states {
            ui.label(format!(
                "{}/{}: {} · expires at {:.2}s",
                state.identity.source,
                state.identity.instance,
                state.state,
                state.expires_at.as_secs_f64()
            ));
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
    }
}
