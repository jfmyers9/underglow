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
use underglow::external::{EventBatch, Snapshot};
use underglow::layer_runtime::CompiledSetup;
use underglow::render::{Frame, PaletteName, RenderContext};
use underglow::setup::SetupDocument;

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
                Ok(n) => sent += n,
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
                    if let Some(end) = bytes.iter().position(|b| *b == b'\n') {
                        let current = std::fs::metadata(path).map_err(|e| e.to_string())?;
                        if generation != (current.dev(), current.ino()) {
                            return Err("Source engine restarted".into());
                        }
                        let mut sample = parse(&bytes[..end])?;
                        sample.generation = generation;
                        return Ok(sample);
                    }
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
}

pub struct Preview {
    document: Option<SetupDocument>,
    runtime: Option<CompiledSetup>,
    frame: Frame,
    sample: Option<LiveSample>,
    pub mode: Mode,
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
            sample: None,
            mode: Mode::Live,
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
    pub fn runtime(&self) -> Option<&CompiledSetup> {
        self.runtime.as_ref()
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
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
        if document.map(|d| &d.id) != self.document.as_ref().map(|d| &d.id) {
            self.started = Instant::now();
            self.last_tick = None;
            self.runtime = None;
            self.document = None;
            self.baseline = true;
        }
        self.update_at(document, brightness, self.started.elapsed());
    }

    fn update_at(&mut self, document: Option<&SetupDocument>, brightness: u8, elapsed: Duration) {
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
        let (facts, events, now) = snapshot.map_or((&empty, &empty_events, elapsed), |s| {
            (&s.snapshot, &s.events, s.extrapolated_now())
        });
        let changed_mode = self.previous_mode != self.mode;
        let generation = snapshot.map(|s| s.generation);
        if changed_mode
            || snapshot.is_none()
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
        let Some(runtime) = &mut self.runtime else {
            self.frame = Frame::black();
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
        self.baseline = snapshot.is_none();
        self.previous_mode = self.mode;
        self.last_source_now = now;
        self.last_generation = generation;
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
        self.frame = runtime.render(
            &RenderContext {
                animation_seconds: elapsed.as_secs_f64(),
                info: &info,
                layout: &layout,
                brightness,
                palette: PaletteName::Wooting,
                tick,
            },
            elapsed,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use underglow::external::{Limits, Message, Payload, Registry};
    use underglow::render::Color;

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
        for oversized in [false, true] {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                ".jim23-source-{}-{oversized}.sock",
                std::process::id()
            ));
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
            std::fs::remove_file(path).unwrap();
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
