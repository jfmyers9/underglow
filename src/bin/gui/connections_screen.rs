//! Connections page. The host supplies facts and applies explicit intents; no I/O here.
#[path = "producer_editor.rs"]
pub mod producer_editor;
use eframe::egui;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};
use underglow::connection_registry::{Connection, ConnectionRegistry};
use underglow::external::{EventBatch, EventSnapshot, Identity, Snapshot, StateSnapshot};
use underglow::layer_runtime::CompiledSetup;
use underglow::setup::{Activation, SetupDocument};
use underglow::source_capabilities::SourceDescriptor;

const MAX_REPORT_BYTES: usize = 512 * 1024;
const MAX_HISTORY: usize = 256;

/// Secret-bearing registry stays private to Connections, never generic diagnostics.
#[derive(Deserialize)]
struct Report {
    registry: ConnectionRegistry,
    #[serde(default)]
    runtime_index: BTreeMap<String, Runtime>,
}
#[derive(Deserialize)]
struct Runtime {
    kind: String,
    producer: Option<Health>,
}
#[derive(Deserialize)]
struct Health {
    state: String,
    running: bool,
    healthy: bool,
    restarts: u32,
    retry_in_ms: Option<u64>,
    last_message_age_ms: Option<u64>,
    last_error: Option<String>,
}

/// Borrowed from the shared live-source worker. Never polls or publishes itself.
#[derive(Clone, Copy)]
pub struct SourceSample<'a> {
    pub snapshot: &'a Snapshot,
    pub events: &'a EventBatch,
    pub now: Duration,
    pub stale: bool,
    /// Changes on engine restart/reconnect; history must never cross generations.
    pub generation: (u64, u64),
}

#[derive(Clone, Copy)]
pub struct View<'a> {
    pub sample: Option<SourceSample<'a>>,
    pub setup: Option<&'a SetupDocument>,
    pub runtime: Option<&'a CompiledSetup>,
    pub connected: bool,
    pub busy: bool,
}

pub enum Request {
    Refresh,
    OpenLayer(String),
    Replace {
        revision: u64,
        registry: ConnectionRegistry,
    },
}

#[derive(Default)]
pub struct Panel {
    report: Option<Report>,
    producer_editor: producer_editor::Editor,
    captured: Option<Instant>,
    error: Option<String>,
    generation: Option<(u64, u64)>,
    states: BTreeMap<Identity, StateSnapshot>,
    events: BTreeMap<u64, EventSnapshot>,
}

impl Panel {
    /// Machine-local data: do not log or copy this into setup/manifest exports.
    pub fn registry(&self) -> Option<&ConnectionRegistry> {
        self.report.as_ref().map(|report| &report.registry)
    }

    #[allow(dead_code)] // Extension point for metadata registration/binding repair.
    pub fn producer_editor(&mut self) -> &mut producer_editor::Editor {
        &mut self.producer_editor
    }

    pub fn receive_replacement(
        &mut self,
        submitted: &ConnectionRegistry,
        result: Result<String, String>,
    ) {
        let previous = self.report.take();
        let previous_captured = self.captured;
        self.receive_report(result);
        if self.error.is_none()
            && submitted.revision.checked_add(1).is_some_and(|revision| {
                let mut expected = submitted.clone();
                expected.revision = revision;
                self.registry() == Some(&expected)
            })
        {
            self.producer_editor.accept_saved(submitted);
        } else {
            self.report = previous;
            self.captured = previous_captured;
            self.error = Some("Connections write rejected or unconfirmed; last saved registry and private draft retained. Refresh before retrying.".into());
        }
    }
    /// Metadata-only handoff for condition forms; never exposes producer trust/argv.
    pub fn has_report(&self) -> bool {
        self.report.is_some()
    }

    pub fn capabilities(&self) -> Option<underglow::source_capabilities::CapabilityRegistry> {
        let report = self.report.as_ref()?;
        let mut registry = underglow::source_capabilities::CapabilityRegistry::new();
        for connection in &report.registry.connections {
            registry.insert(connection.descriptor.clone()).ok()?;
        }
        Some(registry)
    }

    pub fn receive_report(&mut self, result: Result<String, String>) {
        match result.and_then(|body| {
            if body.len() > MAX_REPORT_BYTES {
                return Err("Connection health response exceeds 512 KiB".into());
            }
            #[derive(Deserialize)]
            struct Reply {
                ok: bool,
                connections: Option<Report>,
            }
            let reply: Reply = serde_json::from_str(&body).map_err(|_| "Invalid connection response (private details withheld)".to_string())?;
            if !reply.ok {
                return Err(
                    "Connections unavailable; check engine connectivity and refresh".into(),
                );
            }
            let report = reply
                .connections
                .ok_or("Engine returned no connection registry")?;
            report.registry.validate().map_err(|_| "Invalid connection registry (private details withheld)".to_string())?;
            Ok(report)
        }) {
            Ok(report) => {
                self.report = Some(report);
                self.captured = Some(Instant::now());
                self.error = None;
            }
            // Transport stderr/replies may contain argv/env. Never echo them.
            Err(_) => self.error = Some("Connections request failed; last saved registry and private draft retained. Refresh to check connectivity/revision.".into()),
        }
    }

    fn observe(&mut self, sample: &SourceSample<'_>) {
        if self.generation != Some(sample.generation) {
            if self.generation.is_some() {
                if let Some(report) = &mut self.report {
                    report.runtime_index.clear();
                }
                self.error = Some(
                    "Engine generation changed; refresh producer health (registry is last known)"
                        .into(),
                );
            }
            self.states.clear();
            self.events.clear();
            self.generation = Some(sample.generation);
        }
        for state in &sample.snapshot.states {
            self.states.insert(state.identity.clone(), state.clone());
        }
        for event in &sample.events.events {
            self.events.insert(event.cursor, event.clone());
        }
        while self.states.len() > MAX_HISTORY {
            let oldest = self
                .states
                .iter()
                .min_by_key(|(_, s)| s.updated_at)
                .map(|(id, _)| id.clone())
                .expect("nonempty history");
            self.states.remove(&oldest);
        }
        while self.events.len() > MAX_HISTORY {
            self.events.pop_first();
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, view: View<'_>) -> Option<Request> {
        if let Some(sample) = &view.sample {
            self.observe(sample);
        }
        let mut request = None;
        ui.heading("Connections");
        ui.label("Connectors provide facts. Layers turn those facts into visuals.");
        ui.small(
            "Inspection never starts producers. Explicit controls below change process intent, never resume lighting.",
        );
        if ui
            .add_enabled(!view.busy, egui::Button::new("Refresh connections"))
            .clicked()
        {
            request = Some(Request::Refresh);
        }
        if let Some(error) = &self.error {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                format!("Connection health: {error}"),
            );
        }
        if let Some(captured) = self.captured {
            ui.small(format!(
                "Registry/producer health captured {:.1}s ago{}",
                captured.elapsed().as_secs_f32(),
                if view.connected {
                    ""
                } else {
                    " · engine unavailable; health is last known, not current"
                }
            ));
        }
        let now = view.sample.as_ref().map(|s| s.now);
        // The facts endpoint and control/health transport can fail independently.
        let fresh_sample = view.sample.as_ref().is_some_and(|s| !s.stale);
        if !fresh_sample {
            ui.label("Live facts unavailable or delayed. Retained observations are not proof of a live application or process.");
        }
        if view.sample.as_ref().is_some_and(|s| s.events.gap) {
            ui.label(
                "Event history has a gap; some events expired or were evicted before inspection.",
            );
        }
        let sources: BTreeSet<String> = self
            .report
            .iter()
            .flat_map(|r| {
                r.registry
                    .connections
                    .iter()
                    .map(|c| c.descriptor.source.clone())
            })
            .chain(
                view.setup
                    .into_iter()
                    .flat_map(bindings)
                    .map(|b| b.source.to_owned()),
            )
            .chain(self.states.keys().map(|id| id.source.clone()))
            .chain(self.events.values().map(|e| e.identity.source.clone()))
            .collect();
        if sources.is_empty() {
            ui.label("No configured sources yet. Refresh to load the machine-local registry.");
        }
        if let Some(report) = &self.report {
            ui.small(format!(
                "Connection registry revision {}",
                report.registry.revision
            ));
        }
        for source in sources {
            let connection = self.report.as_ref().and_then(|r| {
                r.registry
                    .connections
                    .iter()
                    .find(|c| c.descriptor.source == source)
            });
            let label = connection.map_or_else(
                || source.clone(),
                |c| format!("{} · {}", c.descriptor.display_name, source),
            );
            ui.push_id(&source, |ui| {
                egui::CollapsingHeader::new(label).default_open(true).show(ui, |ui| {
                    match connection {
                        Some(connection) => {
                            show_process(ui, connection, self.report.as_ref().and_then(|r| r.runtime_index.get(&source)),
                                self.error.is_some() || !view.connected);
                            show_capabilities(ui, &connection.descriptor);
                        }
                        None if self.report.is_some() => {
                            ui.strong("Missing connection binding");
                            ui.label("Repair: restore/register a connection with this source ID, or edit the affected layer binding. Layers are preserved.");
                        }
                        None => { ui.label("Connection registration unknown — refresh the registry to inspect binding health."); }
                    }
                    let current: Vec<_> = view.sample.as_ref().filter(|_| fresh_sample).into_iter()
                        .flat_map(|s| &s.snapshot.states)
                        .filter(|s| s.identity.source == source && s.expires_at > s_now(&view)).collect();
                    ui.strong(if current.is_empty() { "Application state: Not reporting (no fresh state lease)" }
                        else { "Application state: Fresh" });
                    if current.is_empty() {
                        ui.small("A running/healthy connector can still have stale state. Check publication, source/instance IDs and lease renewal; external process health remains unknown.");
                    }
                    for state in self.states.values().filter(|s| s.identity.source == source) {
                        let active = current.iter().any(|s| s.identity == state.identity && s.sequence == state.sequence);
                        ui.label(format!("Instance {} · state {} · {} · last update {} · lease {}",
                            state.identity.instance, state.state, if active { "fresh" } else { "last seen / not reporting" },
                            age(now, state.updated_at), lease(now, state.expires_at, active)));
                    }
                    ui.collapsing("Observed events (bounded history)", |ui| {
                        for event in self.events.values().filter(|e| e.identity.source == source) {
                            let active = fresh_sample && now.is_some_and(|n| event.expires_at > n);
                            ui.label(format!("{} / {} · {} · lease {}", event.identity.instance, event.event,
                                age(now, event.received_at), lease(now, event.expires_at, active)));
                        }
                        ui.small("Events are occurrences, not persistent application state or task success.");
                    });
                    ui.strong("Layer binding impact (current draft; not proof of hardware output)");
                    let bound: Vec<_> = view.setup.into_iter().flat_map(bindings).filter(|b| b.source == source).collect();
                    if bound.is_empty() { ui.label("No layers reference this source. Fresh facts alone do not create visuals."); }
                    for binding in bound {
                        if ui.button(binding.label()).clicked() { request = Some(Request::OpenLayer(binding.layer_id.into())); }
                        ui.label(binding.explanation(&view, &current));
                    }
                });
            });
        }
        if let Some(report) = &self.report
            && let Some(replacement) = self.producer_editor.show(
                ui,
                &report.registry,
                view.connected && !view.busy && self.error.is_none(),
            )
        {
            request = Some(Request::Replace {
                revision: replacement.revision,
                registry: replacement.registry,
            });
        }
        request
    }
}

fn s_now(view: &View<'_>) -> Duration {
    view.sample.as_ref().map_or(Duration::ZERO, |s| s.now)
}
fn age(now: Option<Duration>, updated: Duration) -> String {
    now.map_or_else(
        || "unknown".into(),
        |n| format!("{:.1}s ago", n.saturating_sub(updated).as_secs_f32()),
    )
}
fn lease(now: Option<Duration>, expires: Duration, active: bool) -> String {
    if !active {
        return "not current / expired".into();
    }
    now.map_or_else(
        || "unknown".into(),
        |n| format!("{}ms remaining", expires.saturating_sub(n).as_millis()),
    )
}

fn show_process(
    ui: &mut egui::Ui,
    connection: &Connection,
    runtime: Option<&Runtime>,
    outdated: bool,
) {
    let Some(intent) = &connection.producer else {
        ui.label("External publisher · process health: Unknown (not managed by this engine)");
        ui.small("No traffic does not prove process death or loss of application focus.");
        return;
    };
    ui.label(format!(
        "Managed producer · enabled={} · explicitly trusted={}",
        intent.enabled, intent.trusted
    ));
    if outdated {
        ui.label(
            "Process health: Unknown/current health unavailable; values below are last known.",
        );
    }
    if let Some(health) = runtime
        .filter(|r| r.kind == "managed")
        .and_then(|r| r.producer.as_ref())
    {
        ui.label(format!(
            "Process {} · running={} · protocol healthy={} · restarts={}",
            health.state, health.running, health.healthy, health.restarts
        ));
        if let Some(ms) = health.retry_in_ms {
            ui.label(format!("Next restart retry in {ms}ms (at capture)"));
        }
        if let Some(ms) = health.last_message_age_ms {
            ui.label(format!(
                "Last valid connector message {ms}ms before health capture"
            ));
        }
        if health.last_error.is_some() {
            ui.label("Connector error: producer failed or sent invalid protocol data (private details withheld)");
        }
        ui.small("Protocol healthy means recent valid traffic, not fresh application state or a matching visual layer.");
    } else {
        ui.label("Process health: Unknown — supervisor has not reported this producer yet.");
    }
    if !intent.enabled || !intent.trusted {
        ui.small("Repair: review and save the local producer configuration below, then explicitly enable the reviewed producer.");
    }
}

fn show_capabilities(ui: &mut egui::Ui, descriptor: &SourceDescriptor) {
    ui.collapsing(
        "Declared capabilities (not inferred from observations)",
        |ui| {
            ui.label(format!(
                "Instance identity: {:?}; selectors: {:?}",
                descriptor.instances.semantics, descriptor.instances.selectors
            ));
            let ttl = |range: Option<underglow::source_capabilities::TtlRange>| {
                range.map_or_else(
                    || "not declared".into(),
                    |r| format!("{}–{}ms", r.min_ms, r.max_ms),
                )
            };
            ui.label(format!(
                "State lease TTL: {}; event lease TTL: {}; expected renewal: {}",
                ttl(descriptor.lifecycle.state_ttl),
                ttl(descriptor.lifecycle.event_ttl),
                descriptor
                    .lifecycle
                    .renewal_interval_ms
                    .map_or_else(|| "not promised".into(), |ms| format!("{ms}ms"))
            ));
            for (kind, facts) in [("State", &descriptor.states), ("Event", &descriptor.events)] {
                if facts.is_empty() {
                    ui.label(format!("{kind}: no declared vocabulary"));
                }
                for fact in facts {
                    ui.label(format!("{kind}: {} · {}", fact.id, fact.label));
                }
            }
            ui.small(
                "Observed undeclared values do not expand the source's declared capabilities.",
            );
        },
    );
}

struct Binding<'a> {
    source: &'a str,
    layer_id: &'a str,
    layer_name: &'a str,
    variant: Option<(&'a str, &'a str)>,
    enabled: bool,
    activation: &'a Activation,
}
fn bindings(setup: &SetupDocument) -> Vec<Binding<'_>> {
    setup
        .layers
        .iter()
        .flat_map(|layer| {
            std::iter::once((&layer.activation, None, layer.enabled))
                .chain(layer.variants.iter().map(|v| {
                    (
                        &v.activation,
                        Some((v.id.as_str(), v.name.as_str())),
                        layer.enabled && v.enabled,
                    )
                }))
                .filter_map(|(activation, variant, enabled)| {
                    let source = match activation {
                        Activation::Always {} => return None,
                        Activation::WhileState { source, .. }
                        | Activation::OnEvent { source, .. }
                        | Activation::OnTransition { source, .. } => source,
                    };
                    Some(Binding {
                        source,
                        layer_id: &layer.id,
                        layer_name: &layer.name,
                        variant,
                        enabled,
                        activation,
                    })
                })
        })
        .collect()
}
impl Binding<'_> {
    fn label(&self) -> String {
        let mut label = format!("{} [{}]", self.layer_name, self.layer_id);
        if let Some((id, name)) = self.variant {
            label.push_str(&format!(" → {name} [{id}]"));
        }
        label
    }
    fn explanation(&self, view: &View<'_>, states: &[&StateSnapshot]) -> String {
        if !self.enabled {
            return "Disabled layer/variant; fresh source facts do not enable it.".into();
        }
        if let Some(runtime) = view.runtime
            && runtime
                .selected_appearance(self.layer_id)
                .is_some_and(|a| a.variant_id == self.variant.map(|v| v.0))
        {
            return "Selected in the composition preview. Higher layers may cover it; hardware activation is separate.".into();
        }
        match self.activation {
            Activation::WhileState {
                instance, state, ..
            } => {
                let matched = states.iter().any(|s| {
                    instance.as_ref().is_none_or(|i| i == &s.identity.instance)
                        && state.as_ref().is_none_or(|value| value == &s.state)
                });
                if matched {
                    "Fresh state matches this binding, but another variant may take precedence (or preview is unavailable).".into()
                } else {
                    format!(
                        "No matching fresh state: requires instance {} / state {}. A fresh source can still fail this selector.",
                        instance.as_deref().unwrap_or("any"),
                        state.as_deref().unwrap_or("any (presence)")
                    )
                }
            }
            Activation::OnEvent { event, .. } => format!(
                "Not selected: waits for event '{event}' and its active window; state freshness alone does not trigger it."
            ),
            Activation::OnTransition {
                state, from_state, ..
            } => format!(
                "Not selected: waits for transition {} → {state}; an initial snapshot is not a transition.",
                from_state.as_deref().unwrap_or("any state")
            ),
            Activation::Always {} => unreachable!(),
        }
    }
}

#[cfg(test)]
#[path = "connections_screen_tests.rs"]
mod tests;
