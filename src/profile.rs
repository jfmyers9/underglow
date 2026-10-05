use crate::config::{AppConfig, SceneConfig, SceneZone, SourceConfig, SourceKind};
use crate::layout::Zone;
use crate::render::{Frame, RenderContext};
use crate::scenes;
use crate::signals::{
    CommandPulseSignal, FixtureSignal, SignalProgram, SignalSnapshot, StaticEffectSignal,
    build_signal,
};
use std::sync::atomic::AtomicBool;

pub struct ProfileRuntimeSignal {
    config: AppConfig,
    sources: Vec<RuntimeSource>,
}

enum RuntimeSource {
    Static {
        id: String,
        signal: StaticEffectSignal,
    },
    CommandPulse {
        id: String,
        signal: CommandPulseSignal,
    },
    Fixture {
        id: String,
        signal: FixtureSignal,
    },
    Generic {
        id: String,
        signal: Box<dyn SignalProgram>,
    },
}

impl ProfileRuntimeSignal {
    pub fn new(config: AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        config.validate()?;
        let sources = config
            .sources
            .iter()
            .map(|source| RuntimeSource::new(source, config.effect))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { config, sources })
    }

    pub fn is_profile_runtime_config(config: &AppConfig) -> bool {
        config.signal.is_none()
            && !config.sources.is_empty()
            && (!config.rules.is_empty() || !config.scenes.is_empty())
    }

    fn snapshots(&self) -> Vec<SignalSnapshot> {
        self.sources.iter().map(RuntimeSource::snapshot).collect()
    }
}

impl SignalProgram for ProfileRuntimeSignal {
    fn set_ripple_colors(&mut self, base: Option<[u8; 3]>, ripple: Option<[u8; 3]>) {
        for source in &mut self.sources {
            source.program_mut().set_ripple_colors(base, ripple);
        }
    }

    fn initialize(&mut self) -> crate::signals::ProgramResult {
        for source in &mut self.sources {
            source.program_mut().initialize()?;
        }
        Ok(())
    }

    fn validate_device(&self, info: &crate::sdk::rgb::DeviceInfo) -> crate::signals::ProgramResult {
        for source in &self.sources {
            source.program().validate_device(info)?;
        }
        Ok(())
    }

    fn preview_tick(&mut self, tick: u32) {
        for source in &mut self.sources {
            source.program_mut().preview_tick(tick);
        }
    }

    fn tick(&mut self, interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        for source in &mut self.sources {
            source.tick(interrupted)?;
        }
        Ok(())
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        let snapshots = self.snapshots();
        let mut selected = snapshots
            .iter()
            .filter_map(|snapshot| {
                self.config
                    .select_scene(&snapshot.source_id, &snapshot.status)
                    .map(|selected| (selected.rule.priority, snapshot, selected.scene))
            })
            .collect::<Vec<_>>();
        selected.sort_by_key(|(priority, _, _)| *priority);

        if selected.is_empty() {
            return self.render_fallback(ctx);
        }

        let mut frame = Frame::black();
        for (_, snapshot, scene) in selected {
            let zones = scene_zones(&scene.zones);
            let scene_frame = render_scene(ctx, scene, snapshot, &zones);
            scenes::overlay_non_black(&mut frame, &scene_frame, ctx.layout, &zones);
        }
        frame
    }

    fn finished(&self) -> bool {
        !self.sources.is_empty() && self.sources.iter().all(RuntimeSource::finished)
    }

    fn shutdown(&mut self, interrupted: bool) -> crate::signals::ProgramResult {
        let mut errors = Vec::new();
        for source in &mut self.sources {
            if let Err(error) = source.shutdown(interrupted) {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; ").into())
        }
    }
}

impl RuntimeSource {
    fn program(&self) -> &dyn SignalProgram {
        match self {
            Self::Static { signal, .. } => signal,
            Self::CommandPulse { signal, .. } => signal,
            Self::Fixture { signal, .. } => signal,
            Self::Generic { signal, .. } => &**signal,
        }
    }

    fn program_mut(&mut self) -> &mut dyn SignalProgram {
        match self {
            Self::Static { signal, .. } => signal,
            Self::CommandPulse { signal, .. } => signal,
            Self::Fixture { signal, .. } => signal,
            Self::Generic { signal, .. } => &mut **signal,
        }
    }

    fn new(
        source: &SourceConfig,
        fallback_effect: crate::effects::EffectKind,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let id = if source.id.is_empty() {
            format!("{:?}", source.kind).to_lowercase()
        } else {
            source.id.clone()
        };

        Ok(match source.kind {
            SourceKind::StaticEffect => RuntimeSource::Static {
                id,
                signal: StaticEffectSignal::new(source.effect.unwrap_or(fallback_effect)),
            },
            SourceKind::CommandPulse => RuntimeSource::CommandPulse {
                id,
                signal: CommandPulseSignal::new(source.command_pulse.clone())?,
            },
            SourceKind::FixtureReplay => RuntimeSource::Fixture {
                id,
                signal: FixtureSignal::new(source.fixture.clone()),
            },
            _ => {
                let signal_config = source
                    .signal_config(fallback_effect)
                    .expect("source kind has signal config");
                RuntimeSource::Generic {
                    id,
                    signal: build_signal(&signal_config, fallback_effect)?,
                }
            }
        })
    }

    fn tick(&mut self, interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        match self {
            RuntimeSource::Static { signal, .. } => signal.tick(interrupted),
            RuntimeSource::CommandPulse { signal, .. } => signal.tick(interrupted),
            RuntimeSource::Fixture { signal, .. } => signal.tick(interrupted),
            RuntimeSource::Generic { signal, .. } => signal.tick(interrupted),
        }
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        match self {
            RuntimeSource::Static { signal, .. } => signal.render(ctx),
            RuntimeSource::CommandPulse { signal, .. } => signal.render(ctx),
            RuntimeSource::Fixture { signal, .. } => signal.render(ctx),
            RuntimeSource::Generic { signal, .. } => signal.render(ctx),
        }
    }

    fn snapshot(&self) -> SignalSnapshot {
        match self {
            RuntimeSource::Static { id, .. } => SignalSnapshot::status(id, "running"),
            RuntimeSource::CommandPulse { id, signal } => signal.snapshot(id),
            RuntimeSource::Fixture { id, signal } => signal.snapshot(id),
            RuntimeSource::Generic { id, .. } => SignalSnapshot::status(id, "running"),
        }
    }

    fn finished(&self) -> bool {
        match self {
            RuntimeSource::Static { signal, .. } => signal.finished(),
            RuntimeSource::CommandPulse { signal, .. } => signal.finished(),
            RuntimeSource::Fixture { signal, .. } => signal.finished(),
            RuntimeSource::Generic { signal, .. } => signal.finished(),
        }
    }

    fn shutdown(&mut self, interrupted: bool) -> crate::signals::ProgramResult {
        match self {
            RuntimeSource::Static { signal, .. } => signal.shutdown(interrupted),
            RuntimeSource::CommandPulse { signal, .. } => signal.shutdown(interrupted),
            RuntimeSource::Fixture { signal, .. } => signal.shutdown(interrupted),
            RuntimeSource::Generic { signal, .. } => signal.shutdown(interrupted),
        }
    }
}

fn render_scene(
    ctx: &RenderContext<'_>,
    scene: &SceneConfig,
    snapshot: &SignalSnapshot,
    zones: &[Zone],
) -> Frame {
    let brightness = scene.brightness.unwrap_or(ctx.brightness);
    let palette = scene.palette.unwrap_or(ctx.palette);
    let scene_ctx = RenderContext {
        animation_seconds: ctx.animation_seconds,
        info: ctx.info,
        layout: ctx.layout,
        brightness,
        palette,
        tick: ctx.tick,
    };
    let mut frame = scene
        .effect
        .map(|effect| effect.render(&scene_ctx))
        .unwrap_or_else(|| scenes::render_status_wash(&scene_ctx, &snapshot.status, zones));

    if let Some(progress) = snapshot.progress {
        scenes::progress_bar(
            &mut frame,
            ctx.layout,
            Some(Zone::Function),
            progress,
            scenes::status_color(&snapshot.status, ctx.tick).scale(brightness),
        );
    }

    if zones.is_empty() {
        frame
    } else {
        let mut masked = Frame::black();
        scenes::overlay_non_black(&mut masked, &frame, ctx.layout, zones);
        masked
    }
}

fn scene_zones(zones: &[SceneZone]) -> Vec<Zone> {
    zones
        .iter()
        .map(|zone| match zone {
            SceneZone::Function => Zone::Function,
            SceneZone::Alpha => Zone::Alpha,
            SceneZone::Navigation => Zone::Navigation,
            SceneZone::Arrows => Zone::Arrows,
            SceneZone::System => Zone::System,
        })
        .collect()
}

impl ProfileRuntimeSignal {
    fn render_fallback(&self, ctx: &RenderContext<'_>) -> Frame {
        let mut frame = Frame::black();
        for source in &self.sources {
            let source_frame = source.render(ctx);
            scenes::overlay_non_black(&mut frame, &source_frame, ctx.layout, &[]);
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::KeyboardLayout;
    use crate::preview;
    use crate::render::{PaletteName, RenderContext};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn detects_profile_runtime_configs() {
        let config: AppConfig = toml::from_str(
            r#"
[[sources]]
id = "demo"
type = "fixture-replay"

[[rules]]
when = "demo.status == 'running'"
scene = "demo-running"
priority = 1

[scenes.demo-running]
effect = "breath"
zones = ["function"]
"#,
        )
        .unwrap();

        assert!(ProfileRuntimeSignal::is_profile_runtime_config(&config));
    }

    #[test]
    fn profile_runtime_renders_selected_scene() {
        let config: AppConfig = toml::from_str(
            r#"
[[sources]]
id = "demo"
type = "fixture-replay"

[[sources.steps]]
status = "running"
progress = 0.5
hold_ticks = 10

[[rules]]
when = "demo.status == 'running'"
scene = "demo-running"
priority = 1

[scenes.demo-running]
effect = "breath"
palette = "heat"
zones = ["function"]
"#,
        )
        .unwrap();
        let mut runtime = ProfileRuntimeSignal::new(config).unwrap();
        let info = preview::preview_device();
        let layout = KeyboardLayout::for_device(&info);
        let interrupted = AtomicBool::new(false);

        runtime.tick(&interrupted).unwrap();
        let frame = runtime.render(&RenderContext {
            animation_seconds: 0.0,
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Wooting,
            tick: 1,
        });

        assert_eq!(frame.as_bytes().len(), crate::render::FRAME_BYTES);
        assert!(frame.as_bytes().iter().any(|channel| *channel > 0));
    }
}
