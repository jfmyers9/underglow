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

    pub fn quick_action_label(&self) -> Option<&str> {
        self.quick
            .active
            .then(|| {
                self.quick
                    .action
                    .as_ref()
                    .map(|action| action.label.as_str())
            })
            .flatten()
    }

    /// Evaluate controls before painting the board: test identity, RGB and key
    /// explanations must all describe the same canonical capture.
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
            let context = match self.mode {
                Mode::Live => "Live data",
                Mode::InputDemo => "Input demo",
                Mode::Scenario if self.quick_active() => "Private test",
                Mode::Scenario => "Advanced scenario",
            };
            egui::ComboBox::from_id_salt("quick-preview-context")
                .width(118.0)
                .truncate()
                .selected_text(context)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.mode == Mode::Live, "Live data")
                        .on_hover_text("Read-only live connection observations. Missing or expired observations are not idle. Never publishes facts or enables lighting.")
                        .clicked() {
                        self.set_mode(Mode::Live);
                        self.update(Some(document), brightness);
                    }
                    if ui.selectable_label(self.quick.active, "Private test")
                        .on_hover_text("Start a fresh private selected-layer test. Retains and pauses advanced channels; never publishes real source facts.")
                        .clicked() {
                        self.begin_quick(document, layer, brightness);
                        self.set_scenario_playing(true);
                    }
                })
                .response
                .on_hover_text(format!("{context}. Preview input only, independent of editing appearance and physical lighting."));

            // Budget using the configured fonts. The choice is always bounded;
            // custom source labels cannot push playback or tools offscreen.
            let button_width = |label: &str| {
                egui::WidgetText::from(label)
                    .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Button)
                    .size().x + ui.spacing().button_padding.x * 2.0
            };
            // Reserve playback even before the first action: starting a test
            // must not push Tools offscreen for its first painted frame.
            let playback_width = button_width("Pause test").max(button_width("Play test"))
                + ui.spacing().item_spacing.x;
            let reserved = button_width("Preview tools") + playback_width
                + ui.spacing().item_spacing.x * 2.0 + 4.0;
            let width = (ui.available_width() - reserved).clamp(100.0, 300.0);
            let playback = if self.scenario_playing() { "playing" } else { "paused" };
            let title = match self.quick_action_label() {
                Some(label) => format!("Testing: {label} · {playback}"),
                None if self.quick.active => format!("No test facts · {playback}"),
                _ if self.mode == Mode::Scenario => format!("Advanced scenario · {playback}"),
                _ => "Test behavior…".into(),
            };
            let mut picked = None;
            egui::ComboBox::from_id_salt("quick-test-choice")
                .width(width)
                .truncate()
                .selected_text(&title)
                .show_ui(ui, |ui| {
                    if choices.actions.is_empty() {
                        ui.small(if layer.is_none() {
                            "Select a layer to test"
                        } else if choices.warnings.is_empty() {
                            "Always shown · no source test needed"
                        } else {
                            "No declared quick tests"
                        });
                    }
                    for warning in &choices.warnings {
                        ui.colored_label(crate::brand::WARNING, warning);
                    }
                    for action in &choices.actions {
                        let duplicate = choices.actions.iter()
                            .filter(|other| other.label == action.label).count() > 1;
                        let label = if duplicate {
                            format!("{} · {}", action.source_label, action.detail)
                        } else { action.label.clone() };
                        if ui.add_sized([ui.available_width().min(420.0), 28.0],
                            egui::Button::new(label)
                                .selected(self.quick.active && self.quick.action.as_ref() == Some(action))
                                .truncate()
                        ).on_hover_text(action.help()).clicked() {
                            picked = Some(action.clone());
                        }
                    }
                    if let Some(error) = &self.quick.error {
                        ui.colored_label(crate::brand::ERROR, error);
                    }
                })
                .response
                .on_hover_text(format!("{title}\nPrivate selected-layer tests, never real producer facts or hardware output.\n{}", choices.warnings.join("\n")));
            if let (Some(layer), Some(action)) = (layer, picked) {
                self.quick.error = self.run_quick(document, layer, &action, brightness).err();
            }
            if self.quick.active && ui.button(
                if self.scenario_playing() { "Pause test" } else { "Play test" }
            ).on_hover_text("Pause freezes animations and finite leases. Play advances private time without renewing facts. Return to Live data to end the test.").clicked() {
                self.set_scenario_playing(!self.scenario_playing());
            }
            advanced = ui.button("Preview tools")
                .on_hover_text("Advanced simultaneous sources, finite leases, input demo, clock steps, key explanations and coverage.")
                .clicked();
        });
        advanced
    }
}
