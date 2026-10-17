//! Complete draft preview and read-only facts transport. Never owns hardware,
//! starts a provider, submits an engine command, or publishes source messages.
use eframe::egui;
use serde::Deserialize;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use underglow::explanations::{
    CoverageReport, ExplanationContext, ExplanationMode, analyze_coverage,
};
use underglow::external::{Error as SourceError, EventBatch, Snapshot};
use underglow::layer_runtime::CompiledSetup;
use underglow::render::{Frame, PaletteName, RenderContext};
use underglow::scenarios::Scenario;
use underglow::setup::SetupDocument;
use underglow::source_capabilities::CapabilityRegistry;

#[path = "scenario_controls.rs"]
mod scenario_controls;

#[cfg(test)]
#[path = "scenario_clock_tests.rs"]
mod clock_tests;

const LIMIT: usize = 512 * 1024;
const STALE: Duration = Duration::from_millis(750);

/// Raw engine-clock facts shared by preview, Connections and explanations.
/// Consumers own their cursors; a new consumer must baseline the snapshot.
#[derive(Clone, Debug)]
pub struct LiveSample {
    /// Endpoint device/inode: a new engine must not inherit old deadlines/cursors.
    pub generation: (u64, u64),
    pub now: Duration,
    pub snapshot: Snapshot,
    pub events: EventBatch,
    pub received_at: Instant,
    pub connected: bool,
    pub stale: bool,
}
impl LiveSample {
    pub fn extrapolated_now(&self) -> Duration {
        self.now.saturating_add(self.received_at.elapsed())
    }

    pub fn is_stale(&self) -> bool {
        self.stale || !self.connected || self.received_at.elapsed() >= STALE
    }
}

#[derive(Deserialize)]
struct InspectReply {
    schema_version: u32,
    ok: bool,
    now: Duration,
    snapshot: Snapshot,
    events: EventBatch,
}

fn parse(bytes: &[u8]) -> Result<LiveSample, String> {
    if bytes.len() > LIMIT {
        return Err("Source snapshot too large".into());
    }
    let reply: InspectReply = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if !reply.ok
        || reply.schema_version != 1
        || reply.snapshot.states.len() > 256
        || reply.events.events.len() > 1024
        || reply.snapshot.event_cursor != reply.events.cursor
    {
        return Err("Incompatible source snapshot".into());
    }
    Ok(LiveSample {
        generation: (0, 0),
        now: reply.now,
        snapshot: reply.snapshot,
        events: reply.events,
        received_at: Instant::now(),
        connected: true,
        stale: false,
    })
}

fn request(path: &Path) -> Result<LiveSample, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let generation = (metadata.dev(), metadata.ino());
    let mut stream = UnixStream::connect(path).map_err(|e| e.to_string())?;
    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
    let request = b"{\"schema_version\":1,\"kind\":\"inspect\"}\n";
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut sent = 0;
    let mut bytes = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err("Source inspection timed out".into());
        }
        if sent < request.len() {
            match stream.write(&request[sent..]) {
                Ok(0) => return Err("Source connection closed".into()),
                Ok(n) => {
                    sent += n;
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.to_string()),
            }
        } else {
            let mut chunk = [0; 16 * 1024];
            match stream.read(&mut chunk) {
                Ok(0) => return Err("Source connection closed".into()),
                Ok(n) => {
                    bytes.extend_from_slice(&chunk[..n]);
                    if bytes.len() > LIMIT {
                        return Err("Source snapshot too large".into());
                    }
                    if let Some(offset) = chunk[..n].iter().position(|b| *b == b'\n') {
                        let end = bytes.len() - n + offset;
                        let current = std::fs::metadata(path).map_err(|e| e.to_string())?;
                        if generation != (current.dev(), current.ino()) {
                            return Err("Source engine restarted".into());
                        }
                        let mut sample = parse(&bytes[..end])?;
                        sample.generation = generation;
                        return Ok(sample);
                    }
                    // Drain ready bytes without artificial sleeps or repeatedly
                    // scanning the accumulated response. Keep the total deadline.
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
}

pub struct LiveSourceWorker {
    latest: Arc<Mutex<Option<LiveSample>>>,
    stop: Arc<AtomicBool>,
}
impl LiveSourceWorker {
    pub fn start(directory: PathBuf, context: egui::Context) -> Self {
        let latest = Arc::new(Mutex::new(None::<LiveSample>));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = Self {
            latest: latest.clone(),
            stop: stop.clone(),
        };
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let result = request(&directory.join("events.sock"));
                if let Ok(mut slot) = latest.lock() {
                    match result {
                        Ok(sample) => *slot = Some(sample),
                        Err(_) => {
                            if let Some(sample) = slot.as_mut() {
                                sample.connected = false;
                                sample.stale = true;
                            }
                        }
                    }
                }
                context.request_repaint();
                thread::sleep(Duration::from_millis(100));
            }
        });
        worker
    }

    /// Latest-value only, bounded memory, no UI-thread socket I/O or lock waits.
    pub fn sample(&self) -> Option<LiveSample> {
        let mut sample = self.latest.try_lock().ok()?.clone()?;
        sample.stale = sample.is_stale();
        Some(sample)
    }
}
impl Drop for LiveSourceWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Live,
    InputDemo,
    Scenario,
}

pub struct Preview {
    document: Option<SetupDocument>,
    runtime: Option<CompiledSetup>,
    frame: Frame,
    coverage: Option<CoverageReport>,
    sample: Option<LiveSample>,
    pub mode: Mode,
    pub scenario: Scenario,
    controls: scenario_controls::Controls,
    scenario_tick: Option<Instant>,
    scenario_playing: bool,
    step_ms: u64,
    evaluation_now: Duration,
    evaluation_elapsed: Duration,
    evaluation_brightness: u8,
    evaluation_snapshot: Snapshot,
    evaluation_events: EventBatch,
    last_scenario_generation: u64,
    previous_mode: Mode,
    started: Instant,
    last_tick: Option<u32>,
    baseline: bool,
    last_source_now: Duration,
    last_generation: Option<(u64, u64)>,
    pub error: Option<String>,
}
impl Default for Preview {
    fn default() -> Self {
        Self {
            document: None,
            runtime: None,
            frame: Frame::black(),
            coverage: None,
            sample: None,
            mode: Mode::Live,
            scenario: Scenario::default(),
            controls: scenario_controls::Controls::default(),
            scenario_tick: None,
            scenario_playing: false,
            step_ms: 100,
            evaluation_now: Duration::ZERO,
            evaluation_elapsed: Duration::ZERO,
            evaluation_brightness: 255,
            evaluation_snapshot: Snapshot {
                revision: 0,
                event_cursor: 0,
                states: vec![],
            },
            evaluation_events: EventBatch {
                cursor: 0,
                gap: false,
                events: vec![],
            },
            last_scenario_generation: 0,
            previous_mode: Mode::Live,
            started: Instant::now(),
            last_tick: None,
            baseline: true,
            last_source_now: Duration::ZERO,
            last_generation: None,
            error: None,
        }
    }
}
impl Preview {
    // Host metadata binding is mounted by the independent behavior editor.
    #[allow(dead_code)]
    pub fn set_capabilities(&mut self, capabilities: CapabilityRegistry) {
        self.controls.capabilities = capabilities;
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode != self.mode {
            self.reset_scenario();
            self.mode = mode;
            self.baseline = true;
        }
    }

    pub fn reset_scenario(&mut self) {
        self.scenario.reset();
        self.scenario_playing = false;
        self.scenario_tick = None;
        self.baseline = true;
    }

    pub fn scenario_playing(&self) -> bool {
        self.scenario_playing
    }

    /// Play follows wall time from this point, never catching up paused time.
    /// Pause freezes at the last evaluated scenario instant.
    pub fn set_scenario_playing(&mut self, playing: bool) {
        self.scenario_playing = playing && self.mode == Mode::Scenario;
        self.scenario_tick = self.scenario_playing.then(Instant::now);
    }

    /// Manual steps auto-pause, discarding unevaluated wall-clock drift. An
    /// invalid step changes neither facts/time nor the play/pause state.
    pub fn advance_scenario_by(&mut self, delta: Duration) -> Result<(), SourceError> {
        self.scenario.advance_by(delta)?;
        self.set_scenario_playing(false);
        Ok(())
    }

    /// Clock and effective facts from the last evaluation, not the real-only
    /// transport sample. Consumers must not use these as a live event cursor.
    #[allow(dead_code)] // Shared with the composition explanation pane.
    pub fn evaluation_now(&self) -> Duration {
        self.evaluation_now
    }

    #[allow(dead_code)]
    pub fn evaluation_elapsed(&self) -> Duration {
        self.evaluation_elapsed
    }

    #[allow(dead_code)]
    pub fn evaluation_brightness(&self) -> u8 {
        self.evaluation_brightness
    }

    #[allow(dead_code)]
    pub fn evaluation_facts(&self) -> (&Snapshot, &EventBatch) {
        (&self.evaluation_snapshot, &self.evaluation_events)
    }

    /// Pure controls: no engine intent or command channel is available here.
    pub fn show_controls(&mut self, ui: &mut egui::Ui) {
        let mut mode = self.mode;
        ui.horizontal_wrapped(|ui| {
            ui.label("Composition preview:");
            ui.selectable_value(&mut mode, Mode::Live, "Live facts");
            ui.selectable_value(&mut mode, Mode::InputDemo, "Isolated input demo");
            ui.selectable_value(&mut mode, Mode::Scenario, "Scenario");
        });
        self.set_mode(mode);
        match self.mode {
            Mode::Live => {
                ui.small(if self.sample().is_some_and(|s| !s.is_stale()) {
                    "Read-only live facts · physical input unavailable · no hardware output"
                } else {
                    "Live facts disconnected/stale · conditional layers hidden · no hardware output"
                });
            }
            Mode::InputDemo => {
                ui.small(
                    "Private simulated keystrokes only · no live source facts or provider activity",
                );
            }
            Mode::Scenario => {
                ui.strong("Scenario · isolated source facts · no hardware output");
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(if self.scenario_playing() {
                            "Pause"
                        } else {
                            "Play"
                        })
                        .clicked()
                    {
                        self.set_scenario_playing(!self.scenario_playing());
                    }
                    for (label, ms) in [("+100 ms", 100), ("+1 s", 1000)] {
                        if ui.button(label).clicked() {
                            self.error = self
                                .advance_scenario_by(Duration::from_millis(ms))
                                .err()
                                .map(|e| e.to_string());
                        }
                    }
                    ui.add(egui::DragValue::new(&mut self.step_ms));
                    if ui.button("Advance ms").clicked() {
                        self.error = self
                            .advance_scenario_by(Duration::from_millis(self.step_ms))
                            .err()
                            .map(|e| e.to_string());
                    }
                });
                ui.small("Paused freezes leases and animations. Steps auto-pause; only reset moves time backwards.");
                ui.horizontal(|ui| {
                    if ui.button("Reset scenario").clicked() {
                        self.reset_scenario();
                    }
                    if ui.button("Return to Live").clicked() {
                        self.set_mode(Mode::Live);
                    }
                });
                if self.mode == Mode::Scenario {
                    self.controls.show(ui, &mut self.scenario);
                }
            }
        }
    }

    pub fn runtime(&self) -> Option<&CompiledSetup> {
        self.runtime.as_ref()
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Exact capture used for the displayed frame. Invalid drafts are never
    /// presented as explanations of a retained last-good frame.
    pub fn coverage(&self) -> Option<&CoverageReport> {
        self.coverage.as_ref().filter(|_| self.error.is_none())
    }
    pub fn sample(&self) -> Option<&LiveSample> {
        self.sample.as_ref()
    }
    pub fn set_sample(&mut self, sample: Option<LiveSample>) {
        self.sample = sample;
    }

    /// Save/Discard/reload never replay old completion events.
    pub fn rebaseline(&mut self) {
        self.baseline = true;
    }

    pub fn update(&mut self, document: Option<&SetupDocument>, brightness: u8) {
        let tick = Instant::now();
        self.advance_playback_at(tick);
        if document.map(|d| &d.id) != self.document.as_ref().map(|d| &d.id) {
            self.started = Instant::now();
            self.last_tick = None;
            self.runtime = None;
            self.document = None;
            self.baseline = true;
        }
        self.update_at(document, brightness, self.started.elapsed());
    }

    fn advance_playback_at(&mut self, tick: Instant) {
        if self.mode == Mode::Scenario && self.scenario_playing {
            if let Some(previous) = self.scenario_tick
                && let Err(error) = self
                    .scenario
                    .advance_by(tick.saturating_duration_since(previous))
            {
                self.error = Some(error.to_string());
                self.set_scenario_playing(false);
                return;
            }
            self.scenario_tick = Some(tick);
        } else {
            self.scenario_tick = None;
        }
    }

    fn update_at(&mut self, document: Option<&SetupDocument>, brightness: u8, elapsed: Duration) {
        if self.previous_mode == Mode::Scenario && self.mode != Mode::Scenario {
            self.reset_scenario();
        }
        let snapshot = self
            .sample
            .as_ref()
            .filter(|s| !s.is_stale() && self.mode == Mode::Live);
        let empty = Snapshot {
            revision: 0,
            event_cursor: 0,
            states: vec![],
        };
        let empty_events = EventBatch {
            cursor: 0,
            gap: false,
            events: vec![],
        };
        let synthetic = self.scenario.snapshot();
        let synthetic_events = self.scenario.events_since(0);
        let (facts, events, now) = if self.mode == Mode::Scenario {
            (&synthetic, &synthetic_events, self.scenario.now())
        } else {
            snapshot.map_or((&empty, &empty_events, elapsed), |s| {
                (&s.snapshot, &s.events, s.extrapolated_now())
            })
        };
        let changed_mode = self.previous_mode != self.mode;
        let generation = snapshot.map(|s| s.generation);
        if changed_mode
            || (self.mode == Mode::Scenario
                && self.scenario.generation() != self.last_scenario_generation)
            || (snapshot.is_none() && self.mode != Mode::Scenario)
            || now < self.last_source_now
            || generation != self.last_generation
            || snapshot.is_some_and(|s| {
                self.runtime
                    .as_ref()
                    .is_some_and(|r| s.events.cursor < r.event_cursor())
            })
        {
            self.baseline = true;
        }
        if document != self.document.as_ref() {
            match document.map(CompiledSetup::compile).transpose() {
                Ok(Some(candidate)) if self.runtime.is_some() => {
                    if let Some(runtime) = &mut self.runtime {
                        // Reconfigure preserves unrelated simulations and condition deadlines.
                        let no_replay = EventBatch {
                            cursor: facts.event_cursor,
                            gap: false,
                            events: vec![],
                        };
                        runtime.reconfigure(
                            candidate,
                            facts,
                            if self.baseline { &no_replay } else { events },
                            now,
                        );
                    }
                    self.document = document.cloned();
                    self.error = None;
                }
                Ok(candidate) => {
                    self.runtime = candidate;
                    self.baseline = true;
                    self.document = document.cloned();
                    self.error = None;
                }
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            }
        }
        let elapsed = if self.mode == Mode::Scenario {
            now
        } else {
            elapsed
        };
        self.evaluation_now = now;
        self.evaluation_elapsed = elapsed;
        self.evaluation_brightness = brightness;
        self.evaluation_snapshot = facts.clone();
        self.evaluation_events = events.clone();
        let Some(runtime) = &mut self.runtime else {
            self.frame = Frame::black();
            self.coverage = None;
            return;
        };
        if changed_mode {
            runtime.reset_input();
            self.last_tick = None;
        }
        if self.baseline {
            runtime.baseline(facts, now);
        } else {
            runtime.observe(facts, events, now);
        }
        // A stale connection baselines again on recovery. No past events replay.
        self.baseline = snapshot.is_none() && self.mode != Mode::Scenario;
        self.previous_mode = self.mode;
        self.last_scenario_generation = self.scenario.generation();
        self.last_source_now = now;
        self.last_generation = generation;
        // The exact same clock drives source leases, condition deadlines, and
        // all animation (including Always layers) in isolated scenarios.
        let tick = (elapsed.as_secs_f64() * 30.0) as u32;
        if self.mode == Mode::InputDemo {
            if self.last_tick != Some(tick) {
                let dt = self
                    .last_tick
                    .map_or(1.0 / 30.0, |old| tick.saturating_sub(old) as f32 / 30.0)
                    .min(0.25);
                runtime.preview_tick_at(tick, dt);
                self.last_tick = Some(tick);
            }
        } else {
            runtime.input_unavailable();
        }
        let info = underglow::device::DeviceInfo::synthetic_80he();
        let layout = underglow::layout::KeyboardLayout::for_device(&info);
        let render = RenderContext {
            animation_seconds: elapsed.as_secs_f64(),
            info: &info,
            layout: &layout,
            brightness,
            palette: PaletteName::Wooting,
            tick,
        };
        runtime.prepare_render(&render, elapsed);
        let coverage = analyze_coverage(
            runtime,
            &ExplanationContext {
                render: &render,
                elapsed,
                snapshot: &self.evaluation_snapshot,
                now,
                mode: match self.mode {
                    Mode::Live => ExplanationMode::Live,
                    Mode::InputDemo => ExplanationMode::InputDemo,
                    Mode::Scenario => ExplanationMode::Scenario,
                },
            },
        );
        self.frame = coverage.frame.clone();
        self.coverage = Some(coverage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use underglow::external::{Limits, Message, Payload, Registry};
    use underglow::render::Color;

    fn conditional_document() -> SetupDocument {
        serde_json::from_value(serde_json::json!({"schema_version":1,"id":"scenario","name":"Generic scenario","layers":[
            {"id":"background","name":"Background","renderer":{"kind":"solid","color":[8,8,16]},"target":{"kind":"whole"}},
            {"id":"status","name":"Status","renderer":{"kind":"solid","color":[0,20,40]},"target":{"kind":"whole"},"activation":{"kind":"while-state","source":"custom"},"variants":[
                {"id":"busy","name":"Busy","activation":{"kind":"while-state","source":"custom","state":"busy"},"renderer":{"kind":"solid","color":[0,0,255]}},
                {"id":"done","name":"Done","activation":{"kind":"on-event","source":"custom","event":"done","duration_ms":1000},"renderer":{"kind":"solid","color":[0,255,0]}}
            ]}
        ]})).unwrap()
    }

    #[test]
    fn synthetic_facts_use_real_evaluator_and_compositor_without_mixing_live_sample() {
        let doc = conditional_document();
        let mut live = Registry::default();
        event(&mut live, 999, 0);
        live.apply(
            Message {
                schema_version: 1,
                source: "custom".into(),
                instance: "real-session".into(),
                sequence: 1,
                payload: Payload::State {
                    state: "busy".into(),
                    ttl_ms: 10_000,
                },
            },
            Duration::ZERO,
        )
        .unwrap();
        let raw = sample(&live, 0);
        let original = (raw.snapshot.clone(), raw.events.clone());
        let mut preview = Preview::default();
        preview.set_sample(Some(raw));
        preview.set_mode(Mode::Scenario);
        preview.update_at(Some(&doc), 255, Duration::ZERO);
        assert_eq!(
            color(&preview, "A"),
            Color::new(8, 8, 16),
            "matching live state cannot leak into Scenario"
        );
        let mut pure = CompiledSetup::compile(&doc).unwrap();
        pure.baseline(&preview.scenario.snapshot(), Duration::ZERO);
        for (now, state, emit) in [
            (0, "idle", false),
            (100, "busy", false),
            (200, "idle", true),
        ] {
            preview
                .scenario
                .advance_to(Duration::from_millis(now))
                .unwrap();
            preview
                .scenario
                .publish_state("custom", "scenario-session", state, 1000)
                .unwrap();
            if emit {
                preview
                    .scenario
                    .publish_event("custom", "scenario-session", "done", 1000)
                    .unwrap();
            }
            pure.observe(
                &preview.scenario.snapshot(),
                &preview.scenario.events_since(0),
                preview.scenario.now(),
            );
            let elapsed = Duration::from_millis(now);
            preview.update_at(Some(&doc), 255, elapsed);
            assert_eq!(
                preview.frame().as_bytes(),
                expected(&pure, elapsed, 255).as_bytes()
            );
        }
        assert_eq!(color(&preview, "A"), Color::new(0, 255, 0));
        preview
            .scenario
            .remove("custom", "scenario-session")
            .unwrap();
        preview.update_at(Some(&doc), 255, Duration::from_millis(200));
        assert_eq!(color(&preview, "A"), Color::new(8, 8, 16));
        assert_eq!(preview.sample().unwrap().snapshot, original.0);
        assert_eq!(preview.sample().unwrap().events, original.1);
        assert_eq!(live.snapshot(), original.0);
        assert_eq!(live.events_since(0), original.1);
        preview
            .scenario
            .publish_state("custom", "short-session", "busy", 100)
            .unwrap();
        preview.update_at(Some(&doc), 255, Duration::from_millis(200));
        assert_eq!(color(&preview, "A"), Color::new(0, 0, 255));
        preview
            .scenario
            .advance_by(Duration::from_millis(100))
            .unwrap();
        preview.update_at(Some(&doc), 255, Duration::from_millis(300));
        assert_eq!(
            color(&preview, "A"),
            Color::new(8, 8, 16),
            "exclusive state lease expiry"
        );
    }

    #[test]
    fn scenario_reset_save_discard_reload_and_return_live_do_not_replay_completions() {
        let mut doc = conditional_document();
        // Both live and synthetic contexts deliberately use the same selectors.
        let mut live = Registry::default();
        live.apply(
            Message {
                schema_version: 1,
                source: "custom".into(),
                instance: "scenario-session".into(),
                sequence: 1,
                payload: Payload::Event {
                    event: "done".into(),
                    ttl_ms: 10_000,
                },
            },
            Duration::ZERO,
        )
        .unwrap();
        let mut preview = Preview::default();
        preview.set_sample(Some(sample(&live, 0)));
        preview.set_mode(Mode::Scenario);
        preview.update_at(Some(&doc), 255, Duration::ZERO);
        for index in 0..3 {
            preview
                .scenario
                .publish_event("custom", "scenario-session", "done", 10_000)
                .unwrap();
            preview.update_at(Some(&doc), 255, Duration::ZERO);
            assert_eq!(color(&preview, "A"), Color::new(0, 255, 0));
            preview.rebaseline(); // Save / Discard / reload hook.
            if index == 2 {
                doc.name = "Reloaded".into();
            }
            preview.update_at(Some(&doc), 255, Duration::ZERO);
            assert_eq!(color(&preview, "A"), Color::new(8, 8, 16));
        }
        preview.reset_scenario();
        preview.update_at(Some(&doc), 255, Duration::ZERO);
        assert_eq!(preview.scenario.snapshot().event_cursor, 0);
        preview
            .scenario
            .publish_event("custom", "scenario-session", "done", 10_000)
            .unwrap();
        preview.update_at(Some(&doc), 255, Duration::ZERO);
        assert_eq!(color(&preview, "A"), Color::new(0, 255, 0));
        // Direct library reset (as future scenario controls may use) must also
        // reset evaluator history even when clock and event cursor coincide.
        preview.scenario.reset();
        preview
            .scenario
            .publish_event("custom", "scenario-session", "done", 10_000)
            .unwrap();
        preview.update_at(Some(&doc), 255, Duration::ZERO);
        assert_eq!(color(&preview, "A"), Color::new(8, 8, 16));
        preview.set_mode(Mode::Live);
        preview.update_at(Some(&doc), 255, Duration::ZERO);
        assert_eq!(
            color(&preview, "A"),
            Color::new(8, 8, 16),
            "old real event must not replay"
        );
        assert!(preview.scenario.events_since(0).events.is_empty());
        preview.set_mode(Mode::Scenario);
        preview.update_at(Some(&doc), 255, Duration::ZERO);
        assert_eq!(color(&preview, "A"), Color::new(8, 8, 16));
    }

    #[test]
    fn capability_driven_controls_offer_custom_not_pi_only_facts() {
        let mut descriptor = underglow::source_capabilities::pi_descriptor();
        descriptor.source = "custom".into();
        descriptor.display_name = "Custom source".into();
        descriptor.states[0].id = "resting".into();
        descriptor.states[0].label = "Resting custom".into();
        descriptor.states[1].id = "busy".into();
        descriptor.states[1].label = "Busy custom".into();
        descriptor.events[0].id = "done".into();
        descriptor.events[0].label = "Done custom".into();
        let mut capabilities = CapabilityRegistry::new();
        capabilities.insert(descriptor).unwrap();
        let mut preview = Preview::default();
        preview.set_capabilities(capabilities);
        preview.set_mode(Mode::Scenario);
        let ctx = egui::Context::default();
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| preview.show_controls(ui));
        });
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        for label in [
            "Custom source",
            "Resting custom",
            "Busy custom",
            "Done custom",
            "Scenario · isolated source facts · no hardware output",
            "Reset scenario",
            "Play",
            "+100 ms",
            "+1 s",
            "Advance ms",
            "Return to Live",
        ] {
            assert!(
                labels.iter().any(|text| text == label),
                "missing {label}: {labels:?}"
            );
        }
        assert!(!labels.iter().any(|label| label == "Pi sessions"));
    }

    fn document() -> SetupDocument {
        serde_json::from_value(serde_json::json!({"schema_version":1,"id":"preview","name":"Preview","layers":[
            {"id":"matrix","name":"Matrix","renderer":{"kind":"effect","effect":"matrix","speed":230},"brightness":180,"target":{"kind":"whole"}},
            {"id":"arrows","name":"Cyan arrows","renderer":{"kind":"solid","color":[0,255,255]},"target":{"kind":"selection","keys":["ArrowUp","ArrowLeft","ArrowDown","ArrowRight"]}},
            {"id":"functions","name":"Animated function row","renderer":{"kind":"pulse","color":[200,40,10],"period_ms":1000},"target":{"kind":"selection","zones":["function"]}},
            {"id":"black","name":"Black hole","renderer":{"kind":"solid","color":[0,0,0]},"target":{"kind":"selection","keys":["A"]}}
        ]})).unwrap()
    }

    fn color(preview: &Preview, key: &str) -> Color {
        preview
            .frame()
            .get_coord(underglow::key_targets::canonical_coord(key).unwrap())
    }

    fn expected(runtime: &CompiledSetup, elapsed: Duration, cap: u8) -> Frame {
        let info = underglow::device::DeviceInfo::synthetic_80he();
        let layout = underglow::layout::KeyboardLayout::for_device(&info);
        runtime.render(
            &RenderContext {
                animation_seconds: elapsed.as_secs_f64(),
                info: &info,
                layout: &layout,
                brightness: cap,
                palette: PaletteName::Wooting,
                tick: (elapsed.as_secs_f64() * 30.0) as u32,
            },
            elapsed,
        )
    }

    #[test]
    fn complete_preview_is_byte_identical_to_shared_renderer_including_black_and_cap() {
        let mut doc = document();
        let mut preview = Preview::default();
        for cap in [255, 90, 0] {
            for millis in [0, 250, 500, 1500] {
                let elapsed = Duration::from_millis(millis);
                preview.update_at(Some(&doc), cap, elapsed);
                assert_eq!(
                    preview.frame().as_bytes(),
                    expected(&CompiledSetup::compile(&doc).unwrap(), elapsed, cap).as_bytes()
                );
                assert_eq!(color(&preview, "A"), Color::BLACK);
                assert_eq!(color(&preview, "ArrowUp"), Color::new(0, cap, cap));
            }
        }
        // Reorder and palette/speed/brightness changes use exactly the same compiler.
        doc.layers.swap(0, 3);
        doc.layers[3].renderer = super::super::setup_inspector::default_renderer("aurora").unwrap();
        preview.update_at(Some(&doc), 130, Duration::from_secs(3));
        assert_eq!(
            preview.frame().as_bytes(),
            expected(
                &CompiledSetup::compile(&doc).unwrap(),
                Duration::from_secs(3),
                130
            )
            .as_bytes()
        );
    }

    #[test]
    fn isolated_input_and_focus_use_shared_simulations_without_live_facts() {
        for renderer in ["ripples", "afterimage", "focus-cockpit"] {
            let mut doc = document();
            doc.layers.truncate(1);
            doc.layers[0].renderer =
                super::super::setup_inspector::default_renderer(renderer).unwrap();
            let mut pure = CompiledSetup::compile(&doc).unwrap();
            let mut preview = Preview {
                mode: Mode::InputDemo,
                ..Default::default()
            };
            for tick in 0..40 {
                pure.preview_tick_at(tick, 1.0 / 30.0);
                let elapsed = Duration::from_secs_f64((f64::from(tick) + 0.001) / 30.0);
                preview.update_at(Some(&doc), 170, elapsed);
                assert_eq!(
                    preview.frame().as_bytes(),
                    expected(&pure, elapsed, 170).as_bytes(),
                    "{renderer} tick {tick}"
                );
            }
            assert!(preview.sample().is_none());
            preview.mode = Mode::Live;
            preview.update_at(Some(&doc), 170, Duration::from_secs(2));
            if renderer != "focus-cockpit" {
                assert!(preview.frame().as_bytes().iter().all(|b| *b == 0));
            }
        }
    }

    fn sample(registry: &Registry, millis: u64) -> LiveSample {
        LiveSample {
            generation: (1, 10),
            now: Duration::from_millis(millis),
            snapshot: registry.snapshot(),
            events: registry.events_since(0),
            received_at: Instant::now(),
            connected: true,
            stale: false,
        }
    }
    fn event(registry: &mut Registry, sequence: u64, millis: u64) {
        registry
            .apply(
                Message {
                    schema_version: 1,
                    source: "ci".into(),
                    instance: "build".into(),
                    sequence,
                    payload: Payload::Event {
                        event: "done".into(),
                        ttl_ms: 10_000,
                    },
                },
                Duration::from_millis(millis),
            )
            .unwrap();
    }

    #[test]
    fn baselines_reconnect_restart_save_discard_and_edits_never_replay_completions() {
        let mut doc = document();
        doc.layers.truncate(1);
        doc.layers[0].renderer = underglow::setup::Renderer::Solid {
            color: [0, 255, 255],
        };
        doc.layers[0].activation = underglow::setup::Activation::OnEvent {
            source: "ci".into(),
            instance: None,
            event: "done".into(),
            duration_ms: 1000,
        };
        let mut registry = Registry::new(Limits::default()).unwrap();
        event(&mut registry, 1, 10);
        let mut preview = Preview::default();
        preview.set_sample(Some(sample(&registry, 20)));
        preview.update_at(Some(&doc), 255, Duration::ZERO);
        assert_eq!(
            color(&preview, "A"),
            Color::BLACK,
            "old events baseline on load"
        );
        event(&mut registry, 2, 30);
        preview.set_sample(Some(sample(&registry, 30)));
        preview.update_at(Some(&doc), 255, Duration::from_millis(30));
        assert_ne!(color(&preview, "A"), Color::BLACK);
        let deadline = preview
            .runtime()
            .unwrap()
            .selected_appearance("matrix")
            .unwrap()
            .expires;
        doc.layers[0].name = "Renamed without restarting completion".into();
        preview.update_at(Some(&doc), 255, Duration::from_millis(40));
        assert_eq!(
            preview
                .runtime()
                .unwrap()
                .selected_appearance("matrix")
                .unwrap()
                .expires,
            deadline
        );
        preview.rebaseline();
        preview.update_at(Some(&doc), 255, Duration::from_millis(50));
        assert_eq!(color(&preview, "A"), Color::BLACK, "Save/Discard baseline");
        event(&mut registry, 3, 60);
        preview.set_sample(Some(sample(&registry, 60)));
        preview.update_at(Some(&doc), 255, Duration::from_millis(60));
        assert_ne!(color(&preview, "A"), Color::BLACK);
        let mut restarted = sample(&registry, 60);
        restarted.generation = (1, 11);
        preview.set_sample(Some(restarted));
        preview.update_at(Some(&doc), 255, Duration::from_millis(70));
        assert_eq!(
            color(&preview, "A"),
            Color::BLACK,
            "same clock/cursor on new endpoint still baselines"
        );
        let mut stale = sample(&registry, 80);
        stale.stale = true;
        preview.set_sample(Some(stale));
        preview.update_at(Some(&doc), 255, Duration::from_millis(80));
        assert_eq!(color(&preview, "A"), Color::BLACK);
        preview.set_sample(Some(sample(&registry, 90)));
        preview.update_at(Some(&doc), 255, Duration::from_millis(90));
        assert_eq!(color(&preview, "A"), Color::BLACK, "reconnect baselines");
        preview.mode = Mode::InputDemo;
        preview.update_at(Some(&doc), 255, Duration::from_millis(100));
        assert_eq!(
            color(&preview, "A"),
            Color::BLACK,
            "demo never mixes live facts"
        );
    }

    #[test]
    fn source_decode_is_bounded_and_preserves_raw_clock_snapshot_and_events() {
        let mut registry = Registry::new(Limits::default()).unwrap();
        event(&mut registry, 1, 10);
        let bytes = serde_json::to_vec(&serde_json::json!({"schema_version":1,"ok":true,
            "now":Duration::from_millis(12),"snapshot":registry.snapshot(),"events":registry.events_since(0)})).unwrap();
        let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.snapshot, registry.snapshot());
        assert_eq!(parsed.events, registry.events_since(0));
        assert_eq!(parsed.now, Duration::from_millis(12));
        assert!(parsed.extrapolated_now() >= parsed.now);
        assert!(!parsed.is_stale());
        assert!(parse(&vec![b' '; LIMIT + 1]).is_err());
        assert!(parse(br#"{"schema_version":2,"ok":true}"#).is_err());
    }

    #[test]
    fn readonly_inspection_has_total_deadline_and_never_sends_publish_or_control() {
        use std::os::unix::net::UnixListener;
        struct SocketPath(PathBuf);
        impl Drop for SocketPath {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        for oversized in [false, true] {
            let path = std::env::temp_dir()
                .join(format!("ug-source-{}-{oversized}.sock", std::process::id()));
            let _cleanup = SocketPath(path.clone());
            let listener = UnixListener::bind(&path).unwrap();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut b = [0];
                    socket.read_exact(&mut b).unwrap();
                    bytes.push(b[0]);
                    if b[0] == b'\n' {
                        break;
                    }
                }
                assert_eq!(bytes, b"{\"schema_version\":1,\"kind\":\"inspect\"}\n");
                if oversized {
                    let _ = socket.write_all(&vec![b' '; LIMIT + 1]);
                } else {
                    thread::sleep(Duration::from_millis(350));
                }
            });
            let started = Instant::now();
            let error = request(&path).unwrap_err();
            assert!(started.elapsed() < Duration::from_secs(1));
            assert!(
                error.contains(if oversized { "too large" } else { "timed out" }),
                "{error}"
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn unchanged_live_sample_extrapolates_lease_expiration_without_replaying_events() {
        let mut doc = document();
        doc.layers.truncate(1);
        doc.layers[0].renderer = underglow::setup::Renderer::Solid { color: [255; 3] };
        doc.layers[0].activation = underglow::setup::Activation::WhileState {
            source: "ci".into(),
            instance: None,
            state: Some("working".into()),
        };
        let mut registry = Registry::new(Limits::default()).unwrap();
        registry
            .apply(
                Message {
                    schema_version: 1,
                    source: "ci".into(),
                    instance: "build".into(),
                    sequence: 1,
                    payload: Payload::State {
                        state: "working".into(),
                        ttl_ms: 100,
                    },
                },
                Duration::ZERO,
            )
            .unwrap();
        let mut preview = Preview::default();
        preview.set_sample(Some(sample(&registry, 10)));
        preview.update_at(Some(&doc), 255, Duration::ZERO);
        assert_ne!(color(&preview, "A"), Color::BLACK);
        preview.sample.as_mut().unwrap().received_at = Instant::now() - Duration::from_millis(200);
        preview.update_at(Some(&doc), 255, Duration::from_millis(200));
        assert_eq!(color(&preview, "A"), Color::BLACK);
    }
}
