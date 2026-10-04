use crate::layout::Zone;
use crate::render::{Color, Frame, RenderContext, pulse_wave};
use crate::signals::SignalProgram;
use serde::Deserialize;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AppAuraConfig {
    pub profile: AppAuraProfile,
    pub accessibility_enabled: bool,
    pub dim: bool,
}

impl Default for AppAuraConfig {
    fn default() -> Self {
        Self {
            profile: AppAuraProfile::Manual,
            accessibility_enabled: false,
            dim: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum AppAuraProfile {
    #[default]
    Manual,
    Ide,
    Terminal,
    Meeting,
    Game,
    Recording,
    LateNight,
}

#[derive(Clone, Debug)]
pub struct AppAuraSignal {
    config: AppAuraConfig,
}

impl AppAuraSignal {
    pub fn new(config: AppAuraConfig) -> Self {
        Self { config }
    }
}

impl SignalProgram for AppAuraSignal {
    fn tick(&mut self, _interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        Ok(())
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        let mut frame = Frame::black();
        let base = profile_color(self.config.profile, ctx.tick);
        let brightness = if self.config.dim {
            ctx.brightness / 3
        } else {
            ctx.brightness
        };
        let color = base.scale(brightness);
        let dim = base.scale(brightness / 8);

        for key in ctx.layout.keys() {
            frame.set_coord(key.coord, dim);
        }
        for key in ctx.layout.keys() {
            let active = match self.config.profile {
                AppAuraProfile::Meeting | AppAuraProfile::Recording => key.zone == Zone::Function,
                AppAuraProfile::Game => key.zone == Zone::Alpha || key.zone == Zone::Arrows,
                _ => true,
            };
            if active {
                frame.set_coord(key.coord, color);
            }
        }
        frame
    }

    fn finished(&self) -> bool {
        false
    }

    fn shutdown(&mut self, _interrupted: bool) {}
}

fn profile_color(profile: AppAuraProfile, tick: u32) -> Color {
    match profile {
        AppAuraProfile::Manual => Color::new(0, 90, 180),
        AppAuraProfile::Ide => Color::new(0, 180, 255),
        AppAuraProfile::Terminal => Color::new(0, 220, 64),
        AppAuraProfile::Meeting => Color::new(20, 30, 40),
        AppAuraProfile::Game => Color::new(255, 0, 120),
        AppAuraProfile::Recording => Color::new(255, 32, 24).scale(128 + pulse_wave(tick, 48) / 2),
        AppAuraProfile::LateNight => Color::new(80, 0, 160),
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
    fn app_aura_defaults_to_manual_fallback() {
        let config = AppAuraConfig::default();

        assert_eq!(config.profile, AppAuraProfile::Manual);
        assert!(!config.accessibility_enabled);
    }

    #[test]
    fn app_aura_renders_full_frame() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let signal = AppAuraSignal::new(AppAuraConfig {
            profile: AppAuraProfile::Ide,
            ..AppAuraConfig::default()
        });
        let frame = signal.render(&RenderContext {
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Wooting,
            tick: 0,
        });

        assert_eq!(frame.as_bytes().len(), crate::render::FRAME_BYTES);
    }
}
