//! Source-neutral condition/behavior forms. Declarative edits only: no transport,
//! producers, engine writes, SDK or trust changes.
use super::setup_inspector;
use eframe::egui::{self, Ui};
use std::collections::{BTreeMap, BTreeSet};
use underglow::external::{EventBatch, Snapshot};
use underglow::setup::{Activation, BehaviorVariant, Layer, MAX_BEHAVIOR_VARIANTS};
use underglow::source_capabilities::{
    CapabilityChoice, CapabilityRegistry, ChoiceOrigin, FactKind, InstanceSelector, MAX_FACTS,
    MAX_SOURCES,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Always,
    State,
    Event,
    Transition,
}
impl Mode {
    fn of(a: &Activation) -> Self {
        match a {
            Activation::Always {} => Self::Always,
            Activation::WhileState { .. } => Self::State,
            Activation::OnEvent { .. } => Self::Event,
            Activation::OnTransition { .. } => Self::Transition,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Always => "Always",
            Self::State => "While a source matches",
            Self::Event => "When an event occurs",
            Self::Transition => "When a state changes",
        }
    }
}

fn selector(a: &Activation) -> Option<(&str, &Option<String>)> {
    match a {
        Activation::Always {} => None,
        Activation::WhileState {
            source, instance, ..
        }
        | Activation::OnEvent {
            source, instance, ..
        }
        | Activation::OnTransition {
            source, instance, ..
        } => Some((source, instance)),
    }
}
fn valid_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
}

/// One frame's bounded observations. Unknown values stay explicitly unverified;
/// they do not upgrade the descriptor or create new supported facts.
struct Choices<'a> {
    registry: &'a CapabilityRegistry,
    snapshot: Option<&'a Snapshot>,
    events: Option<&'a EventBatch>,
}
impl Choices<'_> {
    fn sources(&self) -> Vec<(String, String)> {
        let mut sources: BTreeMap<_, _> = self
            .registry
            .descriptors()
            .map(|d| (d.source.clone(), d.display_name.clone()))
            .collect();
        let observed = self
            .snapshot
            .into_iter()
            .flat_map(|s| s.states.iter().map(|v| &v.identity.source))
            .chain(
                self.events
                    .into_iter()
                    .flat_map(|s| s.events.iter().map(|v| &v.identity.source)),
            );
        for source in observed {
            if valid_token(source) && !sources.contains_key(source) && sources.len() < MAX_SOURCES {
                sources.insert(source.clone(), format!("{source} (unknown/observed)"));
            }
        }
        sources.into_iter().collect()
    }
    fn facts(&self, source: &str, kind: FactKind) -> Vec<CapabilityChoice> {
        let observed: BTreeSet<String> = match kind {
            FactKind::State => self
                .snapshot
                .into_iter()
                .flat_map(|s| &s.states)
                .filter(|v| v.identity.source == source)
                .map(|v| v.state.clone())
                .collect(),
            FactKind::Event => self
                .events
                .into_iter()
                .flat_map(|s| &s.events)
                .filter(|v| v.identity.source == source)
                .map(|v| v.event.clone())
                .collect(),
        };
        let observed: Vec<_> = observed
            .into_iter()
            .filter(|s| valid_token(s))
            .take(MAX_FACTS)
            .collect();
        self.registry
            .choices(source, kind, &observed)
            .unwrap_or_default()
    }
    fn instances(&self, source: &str) -> Vec<String> {
        self.snapshot
            .into_iter()
            .flat_map(|s| &s.states)
            .map(|v| &v.identity)
            .chain(
                self.events
                    .into_iter()
                    .flat_map(|s| &s.events)
                    .map(|v| &v.identity),
            )
            .filter(|i| i.source == source && valid_token(&i.instance))
            .map(|i| i.instance.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(MAX_FACTS)
            .collect()
    }
    fn supports(&self, source: &str, mode: Mode) -> bool {
        match mode {
            Mode::Always => true,
            Mode::State => {
                self.registry
                    .descriptor(source)
                    .is_some_and(|d| d.lifecycle.state_ttl.is_some())
                    || !self.facts(source, FactKind::State).is_empty()
            }
            Mode::Event => !self.facts(source, FactKind::Event).is_empty(),
            Mode::Transition => !self.facts(source, FactKind::State).is_empty(),
        }
    }
    fn activation(&self, old: &Activation, mode: Mode, source: &str) -> Option<Activation> {
        if mode == Mode::Always {
            return Some(Activation::Always {});
        }
        if !self.supports(source, mode) {
            return None;
        }
        let selectors = self
            .registry
            .descriptor(source)
            .map(|d| &d.instances.selectors);
        let previous = selector(old)
            .filter(|(s, _)| *s == source)
            .and_then(|(_, i)| i.clone());
        let instance = if previous.is_some()
            && selectors.is_none_or(|s| s.contains(&InstanceSelector::Exact))
        {
            previous
        } else if selectors.is_some_and(|s| !s.contains(&InstanceSelector::Any)) {
            Some(
                self.instances(source)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| "replace-me".into()),
            )
        } else {
            None
        };
        Some(match mode {
            Mode::Always => unreachable!(),
            Mode::State => Activation::WhileState {
                source: source.into(),
                instance,
                state: None,
            },
            Mode::Event => Activation::OnEvent {
                source: source.into(),
                instance,
                event: self.facts(source, FactKind::Event).first()?.id.clone(),
                duration_ms: 2000,
            },
            Mode::Transition => Activation::OnTransition {
                source: source.into(),
                instance,
                from_state: None,
                state: self.facts(source, FactKind::State).first()?.id.clone(),
                duration_ms: 2000,
            },
        })
    }
}
fn fact_picker(
    ui: &mut Ui,
    salt: &str,
    selected: &mut Option<String>,
    choices: &[CapabilityChoice],
    any: Option<&str>,
) {
    let label = selected
        .as_ref()
        .map(|id| {
            choices
                .iter()
                .find(|c| c.id == *id)
                .map(choice_label)
                .unwrap_or_else(|| format!("{id} (saved; unavailable/undeclared)"))
        })
        .unwrap_or_else(|| any.unwrap_or("Choose a value").into());
    egui::ComboBox::from_id_salt(salt)
        .selected_text(label)
        .show_ui(ui, |ui| {
            if let Some(label) = any {
                ui.selectable_value(selected, None, label);
            }
            for choice in choices {
                ui.selectable_value(selected, Some(choice.id.clone()), choice_label(choice));
            }
        });
}
fn choice_label(c: &CapabilityChoice) -> String {
    if c.origin == ChoiceOrigin::UnknownObserved {
        format!("{} (unknown/observed)", c.label)
    } else {
        c.label.clone()
    }
}

fn condition(ui: &mut Ui, activation: &mut Activation, choices: &Choices<'_>) {
    let mut mode = Mode::of(activation);
    let old_mode = mode;
    let sources = choices.sources();
    egui::ComboBox::from_id_salt("condition-mode")
        .selected_text(mode.label())
        .show_ui(ui, |ui| {
            for m in [Mode::Always, Mode::State, Mode::Event, Mode::Transition] {
                let available =
                    m == Mode::Always || sources.iter().any(|(s, _)| choices.supports(s, m));
                ui.add_enabled_ui(available, |ui| {
                    ui.selectable_value(&mut mode, m, m.label());
                });
            }
        });
    if mode != old_mode {
        let source = selector(activation)
            .map(|(s, _)| s.to_owned())
            .filter(|s| choices.supports(s, mode))
            .or_else(|| {
                sources
                    .iter()
                    .find(|(s, _)| choices.supports(s, mode))
                    .map(|(s, _)| s.clone())
            })
            .unwrap_or_default();
        if let Some(next) = choices.activation(activation, mode, &source) {
            *activation = next;
        }
    }
    let Some((saved_source, _)) = selector(activation) else {
        return;
    };
    let mut source = saved_source.to_string();
    egui::ComboBox::from_id_salt("source")
        .selected_text(
            sources
                .iter()
                .find(|(s, _)| s == &source)
                .map(|(_, l)| l.clone())
                .unwrap_or_else(|| format!("{source} (saved; descriptor missing)")),
        )
        .show_ui(ui, |ui| {
            for (id, label) in &sources {
                ui.add_enabled_ui(choices.supports(id, mode), |ui| {
                    ui.selectable_value(&mut source, id.clone(), label);
                });
            }
        });
    if source != saved_source
        && let Some(next) = choices.activation(activation, mode, &source)
    {
        *activation = next;
    }
    let instance = match activation {
        Activation::Always {} => return,
        Activation::WhileState { instance, .. }
        | Activation::OnEvent { instance, .. }
        | Activation::OnTransition { instance, .. } => instance,
    };
    let selectors = choices
        .registry
        .descriptor(&source)
        .map(|d| &d.instances.selectors);
    let mut exact = instance.is_some();
    let old_exact = exact;
    egui::ComboBox::from_id_salt("instance-mode")
        .selected_text(if exact {
            "Exact instance"
        } else {
            "Any instance"
        })
        .show_ui(ui, |ui| {
            for (value, selector, label) in [
                (false, InstanceSelector::Any, "Any instance"),
                (true, InstanceSelector::Exact, "Exact instance"),
            ] {
                ui.add_enabled_ui(selectors.is_none_or(|s| s.contains(&selector)), |ui| {
                    ui.selectable_value(&mut exact, value, label);
                });
            }
        });
    if exact != old_exact {
        *instance = exact.then(|| {
            choices
                .instances(&source)
                .into_iter()
                .next()
                .unwrap_or_else(|| "replace-me".into())
        });
    }
    if let Some(id) = instance {
        ui.add(
            egui::TextEdit::singleline(id)
                .char_limit(128)
                .hint_text("Exact instance ID"),
        );
        egui::ComboBox::from_id_salt("live-instance")
            .selected_text("Observed instances")
            .show_ui(ui, |ui| {
                for live in choices.instances(&source) {
                    ui.selectable_value(id, live.clone(), live);
                }
            });
        ui.small("Instance IDs may be ephemeral producer sessions, not focus/window IDs. Type an ID or choose an observation; replace-me is a placeholder.");
    }
    let states = choices.facts(&source, FactKind::State);
    match activation {
        Activation::WhileState { state, .. } => {
            if state.is_none()
                && choices
                    .registry
                    .descriptor(&source)
                    .is_some_and(|d| d.lifecycle.state_ttl.is_none())
            {
                ui.colored_label(egui::Color32::YELLOW, "Saved source-presence selector is unsupported by this descriptor; retained until you change it.");
            }
            fact_picker(
                ui,
                "state",
                state,
                &states,
                choices
                    .registry
                    .descriptor(&source)
                    .is_none_or(|d| d.lifecycle.state_ttl.is_some())
                    .then_some("Source is present (any live state)"),
            );
        }
        Activation::OnEvent {
            event, duration_ms, ..
        } => {
            let mut value = Some(event.clone());
            fact_picker(
                ui,
                "event",
                &mut value,
                &choices.facts(&source, FactKind::Event),
                None,
            );
            if let Some(value) = value {
                *event = value;
            }
            duration(ui, duration_ms);
        }
        Activation::OnTransition {
            from_state,
            state,
            duration_ms,
            ..
        } => {
            ui.small("Requires two consecutive observed states of the same instance. Initial presence, heartbeats, expiry and removal do not trigger transitions.");
            ui.label("From state");
            fact_picker(ui, "from", from_state, &states, Some("Any previous state"));
            ui.label("To state");
            let mut value = Some(state.clone());
            fact_picker(ui, "to", &mut value, &states, None);
            if let Some(value) = value {
                *state = value;
            }
            duration(ui, duration_ms);
        }
        Activation::Always {} => {}
    }
    ui.small("Unavailable, removed or expired state is not idle/success. Conditional appearances hide when unmatched; transients end after their duration. Events do not establish presence.");
    match choices.registry.validate_activation(activation) {
        Ok(warnings) => {
            for warning in warnings {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!("Repair selector: {warning:?}. Saved value retained."),
                );
            }
        }
        Err(error) => {
            ui.colored_label(egui::Color32::YELLOW, format!("Repair condition: {error}"));
        }
    }
}
fn duration(ui: &mut Ui, ms: &mut u64) {
    ui.add(egui::Slider::new(ms, 1..=60_000).text("Transient duration (ms)"));
    ui.small("Maximum lifetime: source/event lease expiry or removal can end an appearance sooner. This does not change source TTL or heartbeat settings.");
}

/// Selection is view state, keyed by stable logical layer/variant IDs.
#[derive(Default)]
pub struct BehaviorEditor {
    selected: BTreeMap<String, Option<String>>,
}
impl BehaviorEditor {
    /// The host supplies only fresh, read-only real-source observations here;
    /// synthetic preview scenarios must not become observed selector choices.
    /// Metadata and saved selectors remain editable when both samples are absent.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        layer: &mut Layer,
        registry: &CapabilityRegistry,
        snapshot: Option<&Snapshot>,
        events: Option<&EventBatch>,
    ) -> bool {
        let before = layer.clone();
        let choices = Choices {
            registry,
            snapshot,
            events,
        };
        let selected = self.selected.entry(layer.id.clone()).or_default();
        if selected
            .as_ref()
            .is_some_and(|id| !layer.variants.iter().any(|v| &v.id == id))
        {
            *selected = None;
        }
        ui.push_id(("behavior", layer.id.clone()), |ui| {
            ui.heading("Behavior appearances");
            if snapshot.is_some() || events.is_some() {
                ui.small("Observed choices are read-only source samples, separate from preview simulation. Selecting a fact does not emit it.");
            }
            ui.small("Transient appearances outrank persistent ones. Within each class, later appearances win. All inherit this layer's target; base is the final fallback.");
            ui.selectable_value(selected, None, "Base / fallback appearance");
            for (index, variant) in layer.variants.iter().enumerate() {
                ui.selectable_value(selected, Some(variant.id.clone()), format!("{} · {}", index + 1, variant.name));
            }
            if ui.add_enabled(layer.variants.len() < MAX_BEHAVIOR_VARIANTS, egui::Button::new("Add behavior appearance")).clicked() {
                let id = (1..).map(|i| format!("appearance-{i}")).find(|id| !layer.variants.iter().any(|v| &v.id == id)).unwrap();
                layer.variants.push(BehaviorVariant { id: id.clone(), name: "New appearance".into(), enabled: true,
                    activation: Activation::Always {}, renderer: layer.renderer.clone(), brightness: None });
                *selected = Some(id);
            }
            if let Some(index) = selected.as_ref().and_then(|id| layer.variants.iter().position(|v| &v.id == id)) {
                let mut deleted = false;
                ui.horizontal(|ui| {
                    if ui.add_enabled(index > 0, egui::Button::new("Lower priority")).clicked() { layer.variants.swap(index, index - 1); }
                    if ui.add_enabled(index + 1 < layer.variants.len(), egui::Button::new("Higher priority")).clicked() { layer.variants.swap(index, index + 1); }
                    deleted = ui.button("Delete appearance").clicked();
                });
                if deleted { layer.variants.remove(index); *selected = None; return; }
                let variant = layer.variants.iter_mut().find(|v| Some(&v.id) == selected.as_ref()).unwrap();
                ui.push_id(&variant.id, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut variant.name).char_limit(128));
                    ui.checkbox(&mut variant.enabled, "Appearance enabled");
                    condition(ui, &mut variant.activation, &choices);
                    let mut inherit = variant.brightness.is_none();
                    if ui.checkbox(&mut inherit, "Inherit layer brightness").changed() {
                        variant.brightness = (!inherit).then_some(layer.brightness);
                    }
                    if let Some(value) = &mut variant.brightness { ui.add(egui::Slider::new(value, 0..=255).text("Appearance brightness")); }
                    setup_inspector::show_renderer(ui, &mut variant.renderer);
                });
            } else {
                ui.label("Base visibility / unavailable and expiry fallback");
                ui.small("Always keeps the base visible without a source. While source is present hides it when unavailable/expired; this is the default for source-conditional layers.");
                condition(ui, &mut layer.activation, &choices);
                setup_inspector::show(ui, layer);
            }
        });
        before != *layer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use underglow::external::{EventSnapshot, Identity, StateSnapshot};
    use underglow::setup::{SetupDocument, Target};
    use underglow::source_capabilities::{FactDescriptor, pi_descriptor};

    struct Form {
        ctx: egui::Context,
        editor: BehaviorEditor,
        layer: Layer,
        registry: CapabilityRegistry,
        snapshot: Snapshot,
        events: EventBatch,
    }
    impl Form {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                editor: BehaviorEditor::default(),
                layer: Layer {
                    id: "stable-layer".into(),
                    name: "Status".into(),
                    enabled: true,
                    renderer: setup_inspector::default_renderer("aurora").unwrap(),
                    brightness: 73,
                    target: Target::Selection {
                        keys: vec!["F1".into()],
                        zones: vec![],
                        groups: vec!["alerts".into()],
                    },
                    activation: Activation::Always {},
                    variants: vec![],
                },
                registry: CapabilityRegistry::with_builtins(),
                snapshot: Snapshot {
                    revision: 0,
                    event_cursor: 0,
                    states: vec![],
                },
                events: EventBatch {
                    cursor: 0,
                    gap: false,
                    events: vec![],
                },
            }
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            self.ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 2000.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        self.editor.show(
                            ui,
                            &mut self.layer,
                            &self.registry,
                            Some(&self.snapshot),
                            Some(&self.events),
                        );
                    });
                },
            )
        }
        fn click_n(&mut self, label: &str, nth: usize) {
            self.frame(vec![]);
            let output = self.frame(vec![]);
            let pos = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .nth(nth)
                .unwrap_or_else(|| panic!("missing label {label:?}; text={:?}", texts(&output)));
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]);
            }
        }
        fn click(&mut self, label: &str) {
            self.click_n(label, 0);
        }
        fn replace_text(&mut self, label: &str, value: &str) {
            self.click(label);
            self.frame(vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        command: true,
                        ctrl: true,
                        ..Default::default()
                    },
                },
                egui::Event::Text(value.into()),
            ]);
        }
        fn mode(&mut self, label: &str) {
            self.click(Mode::of(&self.layer.activation).label());
            self.click(label);
        }
    }
    fn texts(out: &egui::FullOutput) -> Vec<String> {
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect()
    }
    fn state(source: &str, instance: &str, value: &str) -> StateSnapshot {
        StateSnapshot {
            identity: Identity {
                source: source.into(),
                instance: instance.into(),
            },
            sequence: 1,
            state: value.into(),
            updated_at: Duration::ZERO,
            expires_at: Duration::from_secs(10),
            revision: 1,
        }
    }

    #[test]
    fn mounted_forms_edit_all_modes_and_preserve_identity_target_and_appearance() {
        let mut f = Form::new();
        let original = f.layer.clone();
        f.mode("While a source matches");
        assert_eq!(
            f.layer.activation,
            Activation::WhileState {
                source: "pi".into(),
                instance: None,
                state: None
            }
        );
        f.click("Source is present (any live state)");
        f.click("Working");
        assert!(
            matches!(&f.layer.activation, Activation::WhileState { state: Some(s), .. } if s == "working")
        );
        f.mode("When an event occurs");
        assert!(
            matches!(&f.layer.activation, Activation::OnEvent { event, duration_ms: 2000, .. } if event == "finished")
        );
        f.mode("When a state changes");
        f.click("Any previous state");
        f.click("Working");
        // To-state initially Idle, set it to Working too; dropdown uses actual metadata.
        f.click("Idle");
        f.click_n("Working", 1); // From selector also displays Working.
        assert!(
            matches!(&f.layer.activation, Activation::OnTransition { from_state: Some(from), state, .. } if from == "working" && state == "working")
        );
        f.mode("Always");
        assert_eq!(f.layer, original);
    }

    #[test]
    fn pi_choices_are_accurate_and_observed_unknowns_never_become_declared() {
        let mut f = Form::new();
        f.snapshot.states.push(state("pi", "a", "approval"));
        f.events.events.push(EventSnapshot {
            identity: Identity {
                source: "pi".into(),
                instance: "a".into(),
            },
            event: "surprise".into(),
            cursor: 1,
            sequence: 2,
            received_at: Duration::ZERO,
            expires_at: Duration::from_secs(1),
            revision: 1,
        });
        let choices = Choices {
            registry: &f.registry,
            snapshot: Some(&f.snapshot),
            events: Some(&f.events),
        };
        let states = choices.facts("pi", FactKind::State);
        assert_eq!(
            states
                .iter()
                .filter(|c| c.origin == ChoiceOrigin::Declared)
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            ["idle", "working"]
        );
        assert_eq!(states.last().unwrap().origin, ChoiceOrigin::UnknownObserved);
        assert_eq!(choices.facts("pi", FactKind::Event)[0].id, "finished");
        f.mode("While a source matches");
        f.click("Source is present (any live state)");
        let labels = texts(&f.frame(vec![]));
        assert!(labels.contains(&"approval (unknown/observed)".into()));
        assert!(!labels.iter().any(|l| l == "success" || l == "focus"));
        f.click("approval (unknown/observed)");
        assert!(
            texts(&f.frame(vec![]))
                .iter()
                .any(|t| t.contains("UndeclaredState"))
        );
    }

    #[test]
    fn missing_saved_selectors_are_retained_with_repair_warnings() {
        let mut f = Form::new();
        f.layer.activation = Activation::OnTransition {
            source: "offline".into(),
            instance: Some("old-session".into()),
            from_state: Some("custom-from".into()),
            state: "custom-to".into(),
            duration_ms: 500,
        };
        let original = f.layer.clone();
        let labels = texts(&f.frame(vec![]));
        f.frame(vec![]);
        assert_eq!(f.layer, original);
        assert!(labels.iter().any(|t| t.contains("MissingDescriptor")));
        assert!(
            labels
                .iter()
                .any(|t| t.contains("custom-to (saved; unavailable/undeclared)"))
        );
        f.layer.activation = Activation::WhileState {
            source: "pi".into(),
            instance: None,
            state: Some("old-state".into()),
        };
        let before = f.layer.clone();
        let labels = texts(&f.frame(vec![]));
        assert_eq!(f.layer, before);
        assert!(labels.iter().any(|t| t.contains("UndeclaredState")));
        let mut descriptor = pi_descriptor();
        descriptor.instances.selectors = vec![InstanceSelector::Any];
        f.registry = CapabilityRegistry::new();
        f.registry.insert(descriptor).unwrap();
        f.layer.activation = Activation::OnEvent {
            source: "pi".into(),
            instance: Some("saved-session".into()),
            event: "old-event".into(),
            duration_ms: 800,
        };
        let before = f.layer.clone();
        let labels = texts(&f.frame(vec![]));
        assert_eq!(f.layer, before);
        assert!(
            labels
                .iter()
                .any(|t| t.contains("UnsupportedInstanceSelector"))
        );
        assert!(labels.iter().any(|t| t.contains("UndeclaredEvent")));
    }

    #[test]
    fn exact_any_and_live_instances_follow_descriptor_and_preserve_saved_ids() {
        let mut f = Form::new();
        f.snapshot.states = vec![state("pi", "session-a", "working")];
        f.mode("While a source matches");
        f.click("Any instance");
        f.click("Exact instance");
        assert!(
            matches!(&f.layer.activation, Activation::WhileState { instance: Some(id), .. } if id == "session-a")
        );
        f.replace_text("session-a", "typed-session");
        assert!(
            matches!(&f.layer.activation, Activation::WhileState { instance: Some(id), .. } if id == "typed-session")
        );
        f.snapshot.states.clear(); // ephemeral instance disappears, saved exact selection stays
        f.frame(vec![]);
        assert!(
            matches!(&f.layer.activation, Activation::WhileState { instance: Some(id), .. } if id == "typed-session")
        );
        f.click("Exact instance");
        f.click("Any instance");
        assert!(matches!(
            &f.layer.activation,
            Activation::WhileState { instance: None, .. }
        ));
        let mut d = pi_descriptor();
        d.source = "only-exact".into();
        d.display_name = "Only exact".into();
        d.instances.selectors = vec![InstanceSelector::Exact];
        let mut registry = CapabilityRegistry::new();
        registry.insert(d).unwrap();
        f.registry = registry;
        f.layer.activation = Activation::Always {};
        f.mode("While a source matches");
        assert!(matches!(
            &f.layer.activation,
            Activation::WhileState {
                instance: Some(_),
                ..
            }
        ));
        f.click("Exact instance");
        f.click("Any instance"); // disabled, not an unsupported new choice
        assert!(matches!(
            &f.layer.activation,
            Activation::WhileState {
                instance: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn custom_descriptors_and_observed_sources_require_no_connector_specific_code() {
        let mut f = Form::new();
        let mut descriptor = pi_descriptor();
        descriptor.source = "build".into();
        descriptor.display_name = "Build server".into();
        descriptor.states = vec![FactDescriptor {
            id: "compiling".into(),
            label: "Compiling".into(),
        }];
        descriptor.events = vec![FactDescriptor {
            id: "artifact".into(),
            label: "Artifact ready".into(),
        }];
        f.registry.insert(descriptor).unwrap();
        f.snapshot.states = vec![state("unknown-source", "ephemeral", "new-state")];
        f.mode("When an event occurs"); // alphabetically build before pi
        assert!(
            matches!(&f.layer.activation, Activation::OnEvent { source, event, .. } if source == "build" && event == "artifact")
        );
        f.mode("While a source matches");
        f.click("Build server");
        assert!(texts(&f.frame(vec![])).contains(&"unknown-source (unknown/observed)".into()));
        f.click("unknown-source (unknown/observed)");
        f.click("Source is present (any live state)");
        f.click("new-state (unknown/observed)");
        assert!(
            matches!(&f.layer.activation, Activation::WhileState { source, state: Some(state), .. } if source == "unknown-source" && state == "new-state")
        );
    }

    #[test]
    fn mounted_variant_crud_priority_and_shared_inspector_keep_inherited_brightness() {
        let mut f = Form::new();
        let original = f.layer.clone();
        f.click("Add behavior appearance");
        let first = f.layer.variants[0].id.clone();
        assert_eq!(f.layer.variants[0].brightness, None);
        f.replace_text("New appearance", "Renamed appearance");
        assert_eq!(f.layer.variants[0].name, "Renamed appearance");
        f.click("Always");
        f.click("When an event occurs");
        assert!(
            matches!(&f.layer.variants[0].activation, Activation::OnEvent { event, .. } if event == "finished")
        );
        f.click("Aurora");
        f.click("Matrix");
        assert_eq!(f.layer.variants[0].renderer.catalog_id(), Some("matrix"));
        assert_eq!(f.layer.variants[0].brightness, None);
        f.click("Appearance enabled");
        assert!(!f.layer.variants[0].enabled);
        f.click("Inherit layer brightness");
        assert_eq!(f.layer.variants[0].brightness, Some(73));
        f.click("Inherit layer brightness");
        f.click("Add behavior appearance");
        let second = f.layer.variants[1].id.clone();
        f.click("Lower priority");
        assert_eq!(f.layer.variants[0].id, second);
        assert_eq!(f.editor.selected[&f.layer.id], Some(second.clone()));
        f.click("Higher priority");
        assert_eq!(f.layer.variants[0].id, first);
        f.click("Delete appearance");
        assert_eq!(f.layer.variants.len(), 1);
        f.click("1 · Renamed appearance");
        f.click("Delete appearance");
        assert_eq!(f.layer, original); // all base/reference fields untouched
    }

    #[test]
    fn choices_variant_capacity_and_duration_controls_are_bounded() {
        let mut f = Form::new();
        for _ in 0..MAX_BEHAVIOR_VARIANTS {
            f.click("Add behavior appearance");
        }
        f.click("Add behavior appearance"); // disabled
        assert_eq!(f.layer.variants.len(), MAX_BEHAVIOR_VARIANTS);
        f.layer.variants.clear();
        f.layer.activation = Activation::OnEvent {
            source: "pi".into(),
            instance: None,
            event: "finished".into(),
            duration_ms: 60_001,
        };
        f.frame(vec![]);
        assert!(matches!(
            f.layer.activation,
            Activation::OnEvent {
                duration_ms: 60_000,
                ..
            }
        ));
        f.layer.activation = Activation::OnEvent {
            source: "pi".into(),
            instance: None,
            event: "finished".into(),
            duration_ms: 0,
        };
        f.frame(vec![]);
        f.layer.activation.validate().unwrap();
        f.snapshot.states = (0..300)
            .map(|i| state(&format!("s{i}"), "session", &format!("v{i}")))
            .collect();
        let choices = Choices {
            registry: &f.registry,
            snapshot: Some(&f.snapshot),
            events: None,
        };
        assert!(choices.sources().len() <= MAX_SOURCES);
        f.snapshot.states = (0..300)
            .map(|i| state("pi", &format!("i{i}"), &format!("v{i}")))
            .collect();
        let choices = Choices {
            registry: &f.registry,
            snapshot: Some(&f.snapshot),
            events: None,
        };
        assert_eq!(choices.instances("pi").len(), MAX_FACTS);
        assert_eq!(choices.facts("pi", FactKind::State).len(), MAX_FACTS + 2);
        let mut doc = SetupDocument {
            schema_version: 1,
            revision: 0,
            id: "test".into(),
            name: "Test".into(),
            layers: vec![f.layer],
            groups: BTreeMap::from([("alerts".into(), vec!["F2".into()])]),
        };
        doc.validate().unwrap();
        doc.layers[0].activation = Activation::Always {};
        doc.validate().unwrap();
    }
}
