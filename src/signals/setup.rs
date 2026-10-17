//! Live adapter for the same pure setup runtime used by previews.
//! Construction and preview never initialize SDKs or publish synthetic facts.
use super::{ProgramResult, SignalProgram};
use crate::ingress::SharedSources;
use crate::render::{Frame, RenderContext};
use crate::sdk::analog::AnalogSdk;
use crate::sdk::rgb::DeviceInfo;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use underglow::layer_runtime::CompiledSetup;
use underglow::setup::SetupDocument;

pub struct SetupProgram {
    runtime: CompiledSetup,
    sources: SharedSources,
    analog: Option<AnalogSdk>,
    started: Option<Instant>,
    previous: Option<Instant>,
    elapsed: Duration,
    sources_available: bool,
    preview_fps: u32,
}

impl SetupProgram {
    pub(crate) fn new(
        document: &SetupDocument,
        sources: Option<SharedSources>,
        preview_fps: u32,
    ) -> Result<Self, String> {
        Ok(Self {
            runtime: CompiledSetup::compile(document)?,
            sources: sources.unwrap_or_default(),
            analog: None,
            started: None,
            previous: None,
            elapsed: Duration::ZERO,
            sources_available: false,
            preview_fps: preview_fps.max(1),
        })
    }

    fn observe(&mut self, baseline: bool) {
        match self.sources.observe(self.runtime.event_cursor()) {
            Ok(observation) => {
                if baseline || !self.sources_available {
                    self.runtime
                        .baseline(&observation.snapshot, observation.now);
                } else {
                    self.runtime.observe(
                        &observation.snapshot,
                        &observation.events,
                        observation.now,
                    );
                }
                self.sources_available = true;
            }
            Err(error) => {
                // Fail only conditional contributions, not unrelated renderers.
                self.runtime.clear_conditions(Duration::ZERO);
                self.sources_available = false;
                eprintln!("warning: setup sources unavailable: {error}");
            }
        }
    }
}

impl SignalProgram for SetupProgram {
    fn setup_runtime(&mut self) -> Option<&mut CompiledSetup> {
        Some(&mut self.runtime)
    }

    fn initialize(&mut self) -> ProgramResult {
        if self.started.is_some() {
            return Err("setup is already initialized".into());
        }
        self.runtime.reset_input();
        self.elapsed = Duration::ZERO;
        self.observe(true);
        if self.runtime.requirements().analog_input {
            match AnalogSdk::open(None) {
                Ok(analog) => self.analog = Some(analog),
                Err(error) => {
                    self.runtime.input_unavailable();
                    eprintln!("warning: setup input layer unavailable: {error}");
                }
            }
        }
        let now = Instant::now();
        self.started = Some(now);
        self.previous = Some(now);
        Ok(())
    }

    fn validate_device(&self, info: &DeviceInfo) -> ProgramResult {
        self.runtime.validate_device(info)?;
        Ok(())
    }

    fn tick(&mut self, interrupted: &AtomicBool) -> ProgramResult {
        if interrupted.load(Ordering::SeqCst) {
            return Ok(());
        }
        let started = self.started.ok_or("setup has not been initialized")?;
        let now = Instant::now();
        self.elapsed = now.duration_since(started);
        let dt = self
            .previous
            .map_or(0.0, |previous| now.duration_since(previous).as_secs_f32());
        self.previous = Some(now);
        if let Some(analog) = &self.analog {
            match analog.read() {
                Ok(keys) => {
                    let keys: Vec<_> = keys
                        .iter()
                        .map(|key| (key.key_code, key.pressure))
                        .collect();
                    self.runtime.advance_input(dt, &keys);
                }
                Err(error) => {
                    self.runtime.input_unavailable();
                    if let Some(mut analog) = self.analog.take() {
                        let _ = analog.close();
                    }
                    eprintln!("warning: setup input layer unavailable until resume: {error}");
                }
            }
        }
        self.observe(false);
        Ok(())
    }

    fn preview_tick(&mut self, tick: u32) {
        // Unlike the engine's live-source preview, ordinary config previews have
        // no live facts. Only this private simulation receives demo input.
        self.runtime
            .preview_tick_at(tick, 1.0 / self.preview_fps as f32);
        self.elapsed = Duration::from_secs_f64(f64::from(tick) / f64::from(self.preview_fps));
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        // Session's legacy animation clock is already globally speed-adjusted.
        // The canonical runtime needs unscaled elapsed for per-layer speeds.
        self.runtime.render(ctx, self.elapsed)
    }

    fn composition(&self, ctx: &RenderContext<'_>) -> (Frame, Vec<underglow::compositor::Layer>) {
        // Keep canonical arbitration inside the base; legacy notifications and
        // automation remain overlays with their original priorities.
        (self.render(ctx), Vec::new())
    }

    fn automation_diagnostics(
        &self,
        ctx: &RenderContext<'_>,
    ) -> Vec<underglow::automation::RuleDiagnostic> {
        self.runtime.diagnostics(ctx)
    }

    fn finished(&self) -> bool {
        false
    }

    fn shutdown(&mut self, _interrupted: bool) -> ProgramResult {
        self.started = None;
        self.previous = None;
        self.elapsed = Duration::ZERO;
        self.runtime.reset_input();
        self.runtime.clear_conditions(Duration::ZERO);
        if let Some(mut analog) = self.analog.take() {
            analog.close()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::KeyboardLayout;
    use crate::render::{Color, PaletteName};
    use underglow::external::{Message, Payload};

    fn document() -> SetupDocument {
        SetupDocument::from_toml(include_str!("../../examples/layer-setup.toml")).unwrap()
    }

    #[test]
    fn constructor_and_preview_match_shared_runtime_without_sdk_or_live_facts() {
        let mut doc = document();
        doc.layers[1].activation = underglow::setup::Activation::Always {};
        doc.layers[1].renderer = underglow::setup::Renderer::Reactive {
            effect: underglow::reactive::ReactiveKind::Afterimage,
            palette: Some(PaletteName::Ocean),
        };
        let sources = SharedSources::default();
        let before = sources.observe(0).unwrap().snapshot;
        let mut program = SetupProgram::new(&doc, Some(sources.clone()), 40).unwrap();
        let mut pure = CompiledSetup::compile(&doc).unwrap();
        assert!(program.analog.is_none());
        assert!(program.started.is_none());
        assert!(program.tick(&AtomicBool::new(false)).is_err());
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 90,
            palette: PaletteName::Heat,
            tick: 0,
            animation_seconds: 999.0,
        };
        for tick in 0..20 {
            program.preview_tick(tick);
            pure.preview_tick_at(tick, 1.0 / 40.0);
            let elapsed = Duration::from_secs_f64(f64::from(tick) / 40.0);
            assert_eq!(program.render(&ctx), pure.render(&ctx, elapsed));
        }
        assert!(program.analog.is_none());
        assert!(program.started.is_none());
        assert_eq!(before, sources.observe(0).unwrap().snapshot);
        assert!(program.notification_snapshot().is_none());
        program.shutdown(false).unwrap();
        program.shutdown(true).unwrap();
        assert!(program.analog.is_none());
        assert!(program.started.is_none());
    }

    #[test]
    fn live_noninput_adapter_baselines_sources_and_shutdown_discards_transients() {
        let mut doc = document();
        doc.layers[1].renderer = underglow::setup::Renderer::Solid { color: [255, 0, 0] };
        doc.layers[1].brightness = 255;
        doc.layers[1].target = underglow::setup::Target::Whole {};
        doc.layers[1].activation = underglow::setup::Activation::OnEvent {
            source: "pi".into(),
            instance: None,
            event: "done".into(),
            duration_ms: 1000,
        };
        let sources = SharedSources::default();
        let publish = |sequence| {
            sources
                .apply(Message {
                    schema_version: 1,
                    source: "pi".into(),
                    instance: "a".into(),
                    sequence,
                    payload: Payload::Event {
                        event: "done".into(),
                        ttl_ms: 1000,
                    },
                })
                .unwrap()
        };
        publish(1);
        let mut program = SetupProgram::new(&doc, Some(sources.clone()), 30).unwrap();
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 255,
            palette: PaletteName::Heat,
            tick: 0,
            animation_seconds: 0.0,
        };
        // Purely noninput layers: initialize/tick never call either SDK.
        program.initialize().unwrap();
        assert_ne!(
            program.render(&ctx).get_coord(layout.keys()[0].coord),
            Color::new(255, 0, 0)
        );
        assert!(program.initialize().is_err());
        publish(2);
        program.tick(&AtomicBool::new(false)).unwrap();
        assert_eq!(
            program.render(&ctx).get_coord(layout.keys()[0].coord),
            Color::new(255, 0, 0)
        );
        program.shutdown(true).unwrap();
        assert!(program.started.is_none());
        program.initialize().unwrap();
        program.tick(&AtomicBool::new(false)).unwrap();
        assert_ne!(
            program.render(&ctx).get_coord(layout.keys()[0].coord),
            Color::new(255, 0, 0)
        );
        program.shutdown(false).unwrap();
    }

    #[test]
    fn build_config_signal_uses_canonical_setup_not_legacy_effect() {
        let mut config: crate::config::AppConfig =
            toml::from_str(include_str!("../../examples/layers-profile.toml")).unwrap();
        config.sdk_path = Some("/not/a/real/rgb-sdk".into());
        config.speed = 400; // must not multiply canonical effect speeds again
        config.validate().unwrap();
        let mut mounted = crate::build_config_signal(&config).unwrap();
        let mut expected = CompiledSetup::compile(config.setup.as_ref().unwrap()).unwrap();
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        for tick in 0..6 {
            mounted.preview_tick(tick);
            expected.preview_tick_at(tick, 1.0 / config.fps as f32);
            let ctx = RenderContext {
                info: &info,
                layout: &layout,
                brightness: config.brightness,
                palette: config.palette,
                tick,
                animation_seconds: f64::from(tick) / f64::from(config.fps) * 4.0,
            };
            assert_eq!(
                mounted.render(&ctx),
                expected.render(
                    &ctx,
                    Duration::from_secs_f64(f64::from(tick) / f64::from(config.fps))
                )
            );
        }
        mounted.shutdown(false).unwrap();
    }
}
