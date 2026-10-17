//! Declared, selected-layer test choices. No transport or provider handles.
use super::{Mode, Preview};
use eframe::egui;
use std::time::Duration;
use underglow::scenarios::Scenario;
use underglow::setup::SetupDocument;
use underglow::setup::{Activation, Layer};
use underglow::source_capabilities::{CapabilityRegistry, TtlRange};

#[cfg(test)]
#[path = "quick_preview_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "quick_preview_workspace_tests.rs"]
mod workspace_tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub label: String,
    pub detail: String,
    pub source_label: String,
    pub activation: Activation,
    pub ttl_ms: u64,
}

impl Action {
    fn help(&self) -> String {
        let kind = match self.activation {
            Activation::WhileState { .. } => "Persistent state lease",
            Activation::OnEvent { .. } => {
                "Transient occurrence, not a persistent state or success fact"
            }
            Activation::OnTransition { .. } => "Observed state transition",
            Activation::Always {} => unreachable!(),
        };
        format!(
            "{} · {}\n{kind}; finite {} ms lease. Private preview only. Each choice starts a fresh quick test and clears its previous transient.\nConfigured selector: {:?}",
            self.source_label, self.detail, self.ttl_ms, self.activation
        )
    }
}

#[derive(Default)]
pub struct Choices {
    pub actions: Vec<Action>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct Session {
    pub active: bool,
    pub scenario: Scenario,
    pub layer_id: Option<String>,
    pub action: Option<Action>,
    pub picked: usize,
    pub error: Option<String>,
}

fn ttl(range: Option<TtlRange>) -> Option<u64> {
    range.map(|range| 15_000_u64.min(range.max_ms).max(range.min_ms))
}

fn compact(label: &str) -> String {
    // Parenthetical metadata remains in the tooltip, not a giant toolbar button.
    label.split(" (").next().unwrap_or(label).to_owned()
}

pub fn choices(layer: &Layer, registry: &CapabilityRegistry) -> Choices {
    let mut result = Choices::default();
    let conditions = std::iter::once(&layer.activation).chain(
        layer
            .variants
            .iter()
            .filter(|variant| variant.enabled)
            .map(|variant| &variant.activation),
    );
    for activation in conditions {
        let (source, instance) = match activation {
            Activation::Always {} => continue,
            Activation::WhileState {
                source, instance, ..
            }
            | Activation::OnEvent {
                source, instance, ..
            }
            | Activation::OnTransition {
                source, instance, ..
            } => (source, instance),
        };
        match registry.validate_activation(activation) {
            Ok(warnings) if warnings.is_empty() => {}
            _ => {
                let warning = format!(
                    "{source}: a configured condition is undeclared or unsupported by its source metadata. It is preserved, not advertised as a supported quick test. Use advanced tests to investigate."
                );
                if !result.warnings.contains(&warning) {
                    result.warnings.push(warning);
                }
                continue;
            }
        }
        let Some(descriptor) = registry.descriptor(source) else {
            continue;
        };
        let mut push = |label: &str, activation: Activation, ttl_ms: Option<u64>| {
            let Some(ttl_ms) = ttl_ms else { return };
            let action = Action {
                label: compact(label),
                detail: label.into(),
                source_label: descriptor.display_name.clone(),
                activation,
                ttl_ms,
            };
            if !result
                .actions
                .iter()
                .any(|old| old.activation == action.activation && old.ttl_ms == ttl_ms)
            {
                result.actions.push(action);
            }
        };
        match activation {
            Activation::WhileState { state, .. } => {
                for fact in descriptor
                    .states
                    .iter()
                    .filter(|fact| state.as_ref().is_none_or(|state| state == &fact.id))
                {
                    push(
                        &fact.label,
                        Activation::WhileState {
                            source: source.clone(),
                            instance: instance.clone(),
                            state: Some(fact.id.clone()),
                        },
                        ttl(descriptor.lifecycle.state_ttl),
                    );
                }
                if descriptor.states.is_empty() {
                    result.warnings.push(format!("{source}: presence has no declared named states. No synthetic state was invented."));
                }
            }
            Activation::OnEvent { event, .. } => {
                if let Some(fact) = descriptor.events.iter().find(|fact| &fact.id == event) {
                    push(
                        &fact.label,
                        activation.clone(),
                        ttl(descriptor.lifecycle.event_ttl),
                    );
                }
            }
            Activation::OnTransition {
                from_state, state, ..
            } => {
                let label = |state: &str| {
                    descriptor
                        .states
                        .iter()
                        .find(|fact| fact.id == state)
                        .map_or(state, |fact| fact.label.as_str())
                        .to_owned()
                };
                let from = from_state.as_deref().map_or_else(|| "Absent".into(), label);
                push(
                    &format!("{from} → {}", label(state)),
                    activation.clone(),
                    ttl(descriptor.lifecycle.state_ttl),
                );
            }
            Activation::Always {} => unreachable!(),
        }
    }
    result
}

impl Preview {
    pub fn quick_active(&self) -> bool {
        self.quick.active
    }

    fn begin_quick(&mut self, document: &SetupDocument, layer: Option<&Layer>, brightness: u8) {
        // The advanced registry is retained, frozen, and never copied into this
        // context. One runtime is baselined before any new occurrence is added.
        self.set_scenario_playing(false);
        self.quick.active = true;
        self.mode = Mode::Scenario;
        self.quick.scenario.reset();
        self.quick.action = None;
        self.quick.layer_id = layer.map(|layer| layer.id.clone());
        self.quick.error = None;
        self.baseline = true;
        self.update(Some(document), brightness);
    }

    pub fn reset_quick(&mut self) {
        self.quick.scenario.reset();
        self.quick.action = None;
        self.quick.error = None;
        self.baseline = true;
        self.set_scenario_playing(false);
    }

    pub fn run_quick(
        &mut self,
        document: &SetupDocument,
        layer: &Layer,
        action: &Action,
        brightness: u8,
    ) -> Result<(), String> {
        if !document.layers.iter().any(|current| current == layer)
            || !choices(layer, &self.controls.capabilities)
                .actions
                .contains(action)
        {
            return Err(
                "This test no longer matches the selected layer's declared capabilities.".into(),
            );
        }
        self.begin_quick(document, Some(layer), brightness);
        let result = match &action.activation {
            Activation::WhileState {
                source,
                instance,
                state: Some(state),
            } => self.quick.scenario.publish_state(
                source,
                instance.as_deref().unwrap_or("quick-preview"),
                state,
                action.ttl_ms,
            ),
            Activation::OnEvent {
                source,
                instance,
                event,
                ..
            } => self.quick.scenario.publish_event(
                source,
                instance.as_deref().unwrap_or("quick-preview"),
                event,
                action.ttl_ms,
            ),
            Activation::OnTransition {
                source,
                instance,
                from_state,
                state,
                ..
            } => {
                let instance = instance.as_deref().unwrap_or("quick-preview");
                if let Some(from) = from_state {
                    self.quick
                        .scenario
                        .publish_state(source, instance, from, action.ttl_ms)
                        .map_err(|error| error.to_string())?;
                    self.update(Some(document), brightness);
                }
                // Observe the previous state/absence before the transition.
                self.quick
                    .scenario
                    .advance_by(Duration::from_millis(1))
                    .map_err(|error| error.to_string())?;
                self.quick
                    .scenario
                    .publish_state(source, instance, state, action.ttl_ms)
            }
            _ => return Err("This condition has no declared quick-test fact.".into()),
        };
        result.map_err(|error| error.to_string())?;
        self.quick.action = Some(action.clone());
        self.update(Some(document), brightness);
        self.set_scenario_playing(true);
        Ok(())
    }

    /// Two bounded rows, mounted before drawing the board so a quick action and
    /// the painted RGB/explanation always refer to the same canonical capture.
    pub fn show_quick_controls(
        &mut self,
        ui: &mut egui::Ui,
        document: &SetupDocument,
        layer: Option<&Layer>,
        brightness: u8,
    ) -> bool {
        if self.quick.active
            && self.quick.layer_id.as_deref() != layer.map(|layer| layer.id.as_str())
        {
            self.begin_quick(document, layer, brightness);
            self.set_scenario_playing(true);
        }
        let choices = layer
            .map(|layer| choices(layer, &self.controls.capabilities))
            .unwrap_or_default();
        let mut advanced = false;
        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            if ui.selectable_label(self.mode == Mode::Live, "Live")
                .on_hover_text("Read-only live connection facts. Never sends source messages or enables lighting.").clicked() {
                self.set_mode(Mode::Live);
                self.update(Some(document), brightness);
            }
            if ui.selectable_label(self.quick.active, "Try behavior")
                .on_hover_text(format!("Start a private selected-layer test. Advanced scenario channels are retained and paused; no facts are published to real sources.\n{}", choices.warnings.join("\n"))).clicked() {
                self.begin_quick(document, layer, brightness);
                self.set_scenario_playing(true);
            }
            advanced = ui.button("Preview tools").on_hover_text("Advanced simultaneous sources, leases, input demo and deterministic clock steps; key inspection.").clicked();
        });
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            let short = choices.actions.len() <= 3
                && choices.actions.iter().all(|action| action.label.chars().count() <= 16)
                && choices.actions.iter().enumerate().all(|(index, action)|
                    !choices.actions[..index].iter().any(|other| other.label == action.label));
            if choices.actions.is_empty() {
                let message = if layer.is_none() { "Select a layer to test" }
                    else if choices.warnings.is_empty() { "Always shown · no source test needed" }
                    else { "No declared quick tests · see tools" };
                ui.add(egui::Label::new(egui::RichText::new(message).small()).truncate())
                    .on_hover_text(choices.warnings.join("\n"));
            } else if short {
                for action in &choices.actions {
                    if ui.selectable_label(self.quick.active && self.quick.action.as_ref() == Some(action), &action.label)
                        .on_hover_text(action.help()).clicked() {
                        picked = Some(action.clone());
                    }
                }
            } else {
                self.quick.picked = self.quick.picked.min(choices.actions.len() - 1);
                egui::ComboBox::from_id_salt("quick-test-choice")
                    .width((ui.available_width() - 112.0).max(80.0)).truncate()
                    .selected_text(&choices.actions[self.quick.picked].detail)
                    .show_ui(ui, |ui| {
                        for (index, action) in choices.actions.iter().enumerate() {
                            ui.selectable_value(&mut self.quick.picked, index,
                                format!("{} · {}", action.source_label, action.detail)).on_hover_text(action.help());
                        }
                    }).response.on_hover_text(choices.actions[self.quick.picked].help());
                if ui.button("Try").clicked() { picked = Some(choices.actions[self.quick.picked].clone()); }
            }
            if !choices.actions.is_empty() && ui.add_enabled(self.quick.active,
                egui::Button::new(if self.scenario_playing() && self.quick.active { "Pause test" } else { "Play test" }))
                .on_hover_text("Paused freezes animations and finite leases. Play advances private test time without renewing facts.").clicked() {
                self.set_scenario_playing(!self.scenario_playing());
            }
        });
        if let (Some(layer), Some(action)) = (layer, picked) {
            self.quick.error = self.run_quick(document, layer, &action, brightness).err();
        }
        advanced
    }
}
