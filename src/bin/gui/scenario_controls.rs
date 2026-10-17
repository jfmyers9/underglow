//! Generic independent source/session input. Only writes the private Scenario.
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
        ui.small("Changing source or identity keeps other channels. Removal drops only this session's state, not still-leased events; it never means finished/success.");
        ui.small(format!(
            "Scenario time: {:.2}s",
            scenario.now().as_secs_f64()
        ));
        ui.label("Simulated facts (not process health)");
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .show(ui, |ui| {
                for state in scenario.snapshot().states {
                    ui.horizontal(|ui| {
                        if ui.button("Edit session").clicked() {
                            self.source = state.identity.source.clone();
                            self.instance = state.identity.instance.clone();
                        }
                        ui.label(format!(
                            "{}/{}: {} · lease until {:.3}s ({:.3}s remaining)",
                            state.identity.source,
                            state.identity.instance,
                            state.state,
                            state.expires_at.as_secs_f64(),
                            state
                                .expires_at
                                .saturating_sub(scenario.now())
                                .as_secs_f64()
                        ));
                    });
                }
                for event in scenario.events_since(0).events {
                    ui.horizontal(|ui| {
                        if ui.button("Edit session").clicked() {
                            self.source = event.identity.source.clone();
                            self.instance = event.identity.instance.clone();
                        }
                        ui.label(format!(
                            "{}/{}: event {} · at {:.3}s · retained until {:.3}s",
                            event.identity.source,
                            event.identity.instance,
                            event.event,
                            event.received_at.as_secs_f64(),
                            event.expires_at.as_secs_f64()
                        ));
                    });
                }
            });
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_sources_and_instances_never_clears_or_renews_other_channels() {
        let mut controls = Controls::default();
        let mut descriptor = underglow::source_capabilities::pi_descriptor();
        descriptor.source = "ci".into();
        descriptor.display_name = "CI".into();
        controls.capabilities.insert(descriptor).unwrap();
        let mut scenario = Scenario::default();
        scenario.publish_state("pi", "a", "working", 500).unwrap();
        scenario.publish_state("pi", "b", "idle", 600).unwrap();
        scenario.publish_state("ci", "a", "busy", 700).unwrap();
        scenario.publish_event("pi", "a", "finished", 1000).unwrap();
        let before = (
            scenario.snapshot(),
            scenario.events_since(0),
            scenario.generation(),
        );
        let ctx = egui::Context::default();
        // Render headlessly: initializing metadata, switching registered sources,
        // and editing identities all used to clear the shared synthetic context.
        for (source, instance) in [("", "a"), ("pi", "b"), ("ci", "a"), ("pi", "a")] {
            controls.source = source.into();
            controls.instance = instance.into();
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| controls.show(ui, &mut scenario));
            });
            assert_eq!(
                (
                    scenario.snapshot(),
                    scenario.events_since(0),
                    scenario.generation()
                ),
                before
            );
        }
        scenario.remove("pi", "a").unwrap();
        assert_eq!(scenario.snapshot().states.len(), 2);
        assert_eq!(scenario.events_since(0), before.1);
    }
}
