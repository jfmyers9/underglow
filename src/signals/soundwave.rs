use crate::layout::Zone;
use crate::render::{Color, Frame, RenderContext, pulse_wave};
use crate::signals::SignalProgram;
use serde::Deserialize;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SoundwaveConfig {
    pub enabled: bool,
    pub level: f32,
    pub bass: f32,
    pub cpu_limit_percent: u8,
}

impl Default for SoundwaveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            level: 0.0,
            bass: 0.0,
            cpu_limit_percent: 10,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SoundwaveSignal {
    config: SoundwaveConfig,
}

impl SoundwaveSignal {
    pub fn new(config: SoundwaveConfig) -> Self {
        Self { config }
    }
}

impl SignalProgram for SoundwaveSignal {
    fn tick(&mut self, _interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        Ok(())
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        let mut frame = Frame::black();
        if !self.config.enabled {
            return frame;
        }

        let level = self.config.level.clamp(0.0, 1.0);
        let bass = self.config.bass.clamp(0.0, 1.0);
        let wave = f32::from(pulse_wave(ctx.tick, 24)) / 255.0;
        let bars = ((ctx.layout.keys().len() as f32) * level).ceil() as usize;
        let bass_color =
            Color::new(255, 0, 120).scale((ctx.brightness as f32 * bass.max(wave * bass)) as u8);
        let level_color =
            Color::new(0, 180, 255).scale((ctx.brightness as f32 * level.max(0.1)) as u8);

        let mut keys = ctx.layout.keys().iter().collect::<Vec<_>>();
        keys.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        for key in keys.into_iter().take(bars) {
            frame.set_coord(key.coord, level_color);
        }
        for key in ctx.layout.keys() {
            if key.zone == Zone::Function && bass > 0.0 {
                frame.set_coord(key.coord, bass_color);
            }
        }
        frame
    }

    fn finished(&self) -> bool {
        false
    }

    fn shutdown(&mut self, _interrupted: bool) -> crate::signals::ProgramResult {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::KeyboardLayout;
    use crate::render::{PaletteName, RenderContext};
    use crate::sdk::rgb::{DeviceInfo, DeviceType, Layout};

    fn info() -> DeviceInfo {
        DeviceInfo {
            connected: true,
            model: "test".to_string(),
            max_rows: 6,
            max_columns: 17,
            led_index_max: 0,
            device_type: DeviceType::Keyboard80,
            layout: Layout::Ansi,
            v2_interface: true,
            uses_small_packets: false,
            uses_multi_report: false,
        }
    }

    #[test]
    fn soundwave_is_disabled_by_default() {
        let config = SoundwaveConfig::default();

        assert!(!config.enabled);
    }

    #[test]
    fn disabled_soundwave_renders_black_frame() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let signal = SoundwaveSignal::new(SoundwaveConfig::default());
        let frame = signal.render(&RenderContext {
            animation_seconds: 0.0,
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Wooting,
            tick: 0,
        });

        assert!(frame.as_bytes().iter().all(|channel| *channel == 0));
    }

    #[test]
    fn enabled_soundwave_renders_full_frame() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let signal = SoundwaveSignal::new(SoundwaveConfig {
            enabled: true,
            level: 0.5,
            bass: 0.5,
            ..SoundwaveConfig::default()
        });
        let frame = signal.render(&RenderContext {
            animation_seconds: 0.0,
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Wooting,
            tick: 0,
        });

        assert_eq!(frame.as_bytes().len(), crate::render::FRAME_BYTES);
        assert!(frame.as_bytes().iter().any(|channel| *channel > 0));
    }
}
