//! Live analog adapter for the pure reactive simulations. Construction and
//! previews are offline; the existing session owns hardware activation.
use super::{ProgramResult, SignalProgram};
use crate::render::{Frame, RenderContext};
use crate::sdk::analog::AnalogSdk;
use crate::sdk::rgb::{DeviceInfo, DeviceType, Layout};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use underglow::reactive::{ReactiveKind, ReactiveSimulation};

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ReactiveConfig {
    pub analog_sdk_path: Option<PathBuf>,
}

pub struct ReactiveSignal {
    kind: ReactiveKind,
    config: ReactiveConfig,
    analog: Option<AnalogSdk>,
    simulation: ReactiveSimulation,
    previous: Option<Instant>,
}

impl ReactiveSignal {
    pub fn new(kind: ReactiveKind, config: ReactiveConfig) -> Self {
        Self {
            kind,
            config,
            analog: None,
            simulation: ReactiveSimulation::new(kind),
            previous: None,
        }
    }
}

impl SignalProgram for ReactiveSignal {
    fn initialize(&mut self) -> ProgramResult {
        if self.analog.is_some() {
            return Err(format!("{} is already initialized", self.kind).into());
        }
        self.analog = Some(AnalogSdk::open(self.config.analog_sdk_path.as_deref())?);
        self.simulation.clear();
        self.previous = Some(Instant::now());
        eprintln!(
            "{} owns RGB while running. Ctrl-C attempts to restore the current keyboard profile's lighting. Key presses still reach your apps; activity stays in memory only.",
            self.kind
        );
        Ok(())
    }

    fn validate_device(&self, info: &DeviceInfo) -> ProgramResult {
        if info.device_type != DeviceType::Keyboard80
            || info.layout != Layout::Ansi
            || info.max_rows != 6
            || info.max_columns != 17
        {
            return Err(
                format!("{} currently supports the 80HE ANSI layout only", self.kind).into(),
            );
        }
        Ok(())
    }

    fn tick(&mut self, _interrupted: &AtomicBool) -> ProgramResult {
        let analog = self
            .analog
            .as_ref()
            .ok_or("reactive signal has not been initialized")?;
        let keys = analog.read()?;
        let now = Instant::now();
        let dt = self
            .previous
            .map_or(0.0, |previous| now.duration_since(previous).as_secs_f32());
        self.simulation
            .advance(dt, keys.iter().map(|key| (key.key_code, key.pressure)));
        self.previous = Some(now);
        Ok(())
    }

    fn preview_tick(&mut self, tick: u32) {
        if tick == 0 {
            self.simulation.clear();
        }
        // Fixed synthetic presses include releases and repeated keys to show
        // warming, links and trails without reading either SDK or a wall clock.
        const KEYS: [u16; 8] = [0x04, 0x09, 0x0d, 0x09, 0x04, 0x09, 0x0f, 0x09];
        let input = (tick % 4 < 2).then_some((KEYS[((tick / 4) % 8) as usize], 0.9));
        self.simulation.advance(0.1, input);
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        self.simulation.render(ctx)
    }

    fn finished(&self) -> bool {
        false
    }

    fn shutdown(&mut self, _interrupted: bool) -> ProgramResult {
        // Clear activity even if closing the SDK reports a failure.
        self.simulation.clear();
        self.previous = None;
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
    use crate::render::PaletteName;

    const KINDS: [ReactiveKind; 3] = [
        ReactiveKind::Constellation,
        ReactiveKind::Heatmap,
        ReactiveKind::Afterimage,
    ];

    fn frame(signal: &ReactiveSignal) -> Frame {
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        signal.render(&RenderContext {
            info: &info,
            layout: &layout,
            brightness: 255,
            palette: PaletteName::Heat,
            tick: 0,
            animation_seconds: 0.0,
        })
    }

    #[test]
    fn constructor_preview_and_shutdown_are_offline_and_deterministic() {
        for kind in KINDS {
            let config = ReactiveConfig {
                analog_sdk_path: Some("/not/a/real/analog-sdk".into()),
            };
            let mut a = ReactiveSignal::new(kind, config.clone());
            let mut b = ReactiveSignal::new(kind, config);
            assert!(a.analog.is_none());
            assert!(a.previous.is_none());
            assert_eq!(frame(&a), Frame::black());
            assert!(a.tick(&AtomicBool::new(false)).is_err());
            for tick in 0..40 {
                a.preview_tick(tick);
                b.preview_tick(tick);
                assert_eq!(frame(&a), frame(&b));
            }
            assert_ne!(frame(&a), Frame::black());
            assert!(a.analog.is_none());
            assert!(a.previous.is_none());
            assert!(a.notification_snapshot().is_none());
            a.shutdown(false).unwrap();
            a.shutdown(true).unwrap();
            assert_eq!(frame(&a), Frame::black());
            a.preview_tick(0);
            b.preview_tick(0);
            assert_eq!(frame(&a), frame(&b));
        }
    }

    #[test]
    fn device_validation_requires_exact_80he_ansi_matrix() {
        for kind in KINDS {
            let signal = ReactiveSignal::new(kind, ReactiveConfig::default());
            let info = DeviceInfo::synthetic_80he();
            assert!(signal.validate_device(&info).is_ok());
            let mut invalid = info.clone();
            invalid.max_rows = 5;
            assert!(signal.validate_device(&invalid).is_err());
            invalid = info.clone();
            invalid.max_columns = 21;
            assert!(signal.validate_device(&invalid).is_err());
            invalid = info.clone();
            invalid.layout = Layout::Iso;
            assert!(signal.validate_device(&invalid).is_err());
            invalid = info;
            invalid.device_type = DeviceType::Keyboard60;
            assert!(signal.validate_device(&invalid).is_err());
        }
    }

    #[test]
    fn config_rejects_unknown_fields() {
        assert_eq!(
            toml::from_str::<ReactiveConfig>("").unwrap(),
            ReactiveConfig::default()
        );
        assert!(toml::from_str::<ReactiveConfig>("record_keys = true").is_err());
        let config: ReactiveConfig = toml::from_str("analog_sdk_path = '/offline/sdk'").unwrap();
        assert_eq!(config.analog_sdk_path, Some(PathBuf::from("/offline/sdk")));
    }
}
