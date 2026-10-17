//! Thin live-engine adapter. Preview construction never observes live sources.
use crate::ingress::SharedSources;
use crate::render::{Frame, RenderContext};
use crate::sdk::rgb::DeviceInfo;
use crate::signals::{ProgramResult, SignalProgram, SignalSnapshot};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use underglow::automation::{AutomationConfig, AutomationRuntime, RuleDiagnostic};
use underglow::compositor::{Layer, compose, winning_layer};

pub(crate) fn wrap(
    base: Box<dyn SignalProgram>,
    config: &AutomationConfig,
    sources: Option<SharedSources>,
) -> Result<Box<dyn SignalProgram>, Box<dyn std::error::Error>> {
    config.validate()?;
    if config.rules.is_empty() && !config.has_named_targets() {
        return Ok(base);
    }
    Ok(Box::new(ExternalRulesProgram {
        base,
        runtime: AutomationRuntime::new(config.clone())?,
        sources: sources.unwrap_or_default(),
        now: Duration::ZERO,
    }))
}

struct ExternalRulesProgram {
    base: Box<dyn SignalProgram>,
    runtime: AutomationRuntime,
    sources: SharedSources,
    now: Duration,
}

impl SignalProgram for ExternalRulesProgram {
    fn setup_runtime(&mut self) -> Option<&mut underglow::layer_runtime::CompiledSetup> {
        self.base.setup_runtime()
    }

    fn set_ripple_colors(&mut self, base: Option<[u8; 3]>, ripple: Option<[u8; 3]>) {
        self.base.set_ripple_colors(base, ripple);
    }

    fn initialize(&mut self) -> ProgramResult {
        self.base.initialize()?;
        let observation = self.sources.observe(self.runtime.event_cursor())?;
        self.now = observation.now;
        self.runtime
            .baseline(&observation.snapshot, observation.now);
        Ok(())
    }

    fn validate_device(&self, info: &DeviceInfo) -> ProgramResult {
        self.base.validate_device(info)?;
        self.runtime.validate_layout(info)?;
        Ok(())
    }

    fn tick(&mut self, interrupted: &AtomicBool) -> ProgramResult {
        if interrupted.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.base.tick(interrupted)?;
        let observation = self.sources.observe(self.runtime.event_cursor())?;
        self.now = observation.now;
        self.runtime
            .observe(&observation.snapshot, &observation.events, observation.now);
        Ok(())
    }

    fn preview_tick(&mut self, tick: u32) {
        // No shared-registry read here, including when previewing an engine profile.
        self.base.preview_tick(tick);
    }

    fn notification_snapshot(&self) -> Option<SignalSnapshot> {
        self.base.notification_snapshot()
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        let (base, layers) = self.composition(ctx);
        compose(base, ctx.layout, &layers, self.now)
    }

    fn composition(&self, ctx: &RenderContext<'_>) -> (Frame, Vec<Layer>) {
        let (base, mut layers) = self.base.composition(ctx);
        layers.extend(self.runtime.layers(ctx).into_iter().map(|mut layer| {
            layer.expires = None;
            layer
        }));
        (base, layers)
    }

    fn automation_diagnostics(&self, ctx: &RenderContext<'_>) -> Vec<RuleDiagnostic> {
        let (_, inherited) = self.base.composition(ctx);
        let mut index = inherited.len();
        let (_, layers) = self.composition(ctx);
        let mut diagnostics = self.runtime.diagnostics(ctx);
        for diagnostic in &mut diagnostics {
            if diagnostic.active {
                diagnostic.visible_keys = ctx
                    .layout
                    .keys()
                    .iter()
                    .filter(|key| winning_layer(&layers, key, self.now) == Some(index))
                    .count();
                index += 1;
            }
        }
        diagnostics
    }

    fn finished(&self) -> bool {
        self.base.finished()
    }

    fn shutdown(&mut self, interrupted: bool) -> ProgramResult {
        self.base.shutdown(interrupted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::StaticEffectSignal;
    use underglow::effects::EffectKind;
    use underglow::external::{Message, Payload};
    use underglow::layout::{KeyboardLayout, Zone};
    use underglow::render::{Color, PaletteName};

    fn config() -> AutomationConfig {
        toml::from_str(
            r#"
[[rules]]
id='working'
source='pi'
mode='while-state'
state='working'
scene='working'
[[rules]]
id='finished'
source='pi'
mode='on-event'
event='finished'
scene='finished'
priority=10
duration_ms=2000
[scenes.working]
color=[0,0,255]
zones=['function']
[scenes.finished]
color=[0,255,0]
zones=['function']
"#,
        )
        .unwrap()
    }

    fn publish(sources: &SharedSources, sequence: u64, payload: Payload) {
        sources
            .apply(Message {
                schema_version: 1,
                source: "pi".into(),
                instance: "a".into(),
                sequence,
                payload,
            })
            .unwrap();
    }

    #[test]
    fn live_wrapper_uses_shared_facts_and_resume_discards_prior_events() {
        let sources = SharedSources::default();
        let config = config();
        publish(
            &sources,
            1,
            Payload::State {
                state: "working".into(),
                ttl_ms: 15000,
            },
        );
        publish(
            &sources,
            2,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 15000,
            },
        );
        let mut program = wrap(
            Box::new(StaticEffectSignal::new(EffectKind::Matrix)),
            &config,
            Some(sources.clone()),
        )
        .unwrap();
        let info = crate::preview::preview_device();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 90,
            palette: PaletteName::Terminal,
            tick: 0,
            animation_seconds: 0.5,
        };
        program.initialize().unwrap();
        program.tick(&AtomicBool::new(false)).unwrap();
        let frame = program.render(&ctx);
        let function = layout
            .keys()
            .iter()
            .find(|key| key.zone == Zone::Function)
            .unwrap();
        assert_eq!(frame.get_coord(function.coord), Color::new(0, 0, 90));
        publish(
            &sources,
            3,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 15000,
            },
        );
        program.tick(&AtomicBool::new(false)).unwrap();
        assert_eq!(
            program.render(&ctx).get_coord(function.coord),
            Color::new(0, 90, 0)
        );
        program.shutdown(true).unwrap();
        program.initialize().unwrap();
        program.tick(&AtomicBool::new(false)).unwrap();
        assert_eq!(
            program.render(&ctx).get_coord(function.coord),
            Color::new(0, 0, 90)
        );
        program.shutdown(true).unwrap();
    }

    #[test]
    fn live_device_validation_rejects_unsupported_named_layout_even_without_rules() {
        for input in [
            "[scenes.x]\nkeys=['F1']",
            "[groups]\nWASD=['W','A','S','D']\n[scenes.x]\ngroups=['WASD']",
        ] {
            let config: AutomationConfig = toml::from_str(input).unwrap();
            assert!(config.rules.is_empty());
            let program = wrap(
                Box::new(StaticEffectSignal::new(EffectKind::Matrix)),
                &config,
                None,
            )
            .unwrap();
            let mut info = crate::preview::preview_device();
            program.validate_device(&info).unwrap();
            info.layout = underglow::device::Layout::Iso;
            let error = program.validate_device(&info).unwrap_err().to_string();
            assert!(error.contains("80HE ANSI 6x17"), "{error}");
            info.layout = underglow::device::Layout::Ansi;
            info.device_type = underglow::device::DeviceType::Keyboard60;
            assert!(program.validate_device(&info).is_err());
        }
        let program = wrap(
            Box::new(StaticEffectSignal::new(EffectKind::Matrix)),
            &config(),
            None,
        )
        .unwrap();
        let mut info = crate::preview::preview_device();
        info.layout = underglow::device::Layout::Iso;
        program.validate_device(&info).unwrap();
    }

    #[test]
    fn preview_never_observes_live_sources_and_keeps_matrix() {
        let sources = SharedSources::default();
        publish(
            &sources,
            1,
            Payload::State {
                state: "working".into(),
                ttl_ms: 15000,
            },
        );
        let mut program = wrap(
            Box::new(StaticEffectSignal::new(EffectKind::Matrix)),
            &config(),
            Some(sources),
        )
        .unwrap();
        program.preview_tick(10);
        let info = crate::preview::preview_device();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 90,
            palette: PaletteName::Terminal,
            tick: 10,
            animation_seconds: 0.5,
        };
        assert_eq!(program.render(&ctx), EffectKind::Matrix.render(&ctx));
    }
}
