//! Pure focus visualization shared by the timer and synthetic previews.
//! This module does not own a clock, provider, notification, or device.

use crate::layout::Zone;
use crate::render::{Color, Frame, RenderContext, pulse_wave};
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FocusConfig {
    pub focus_minutes: u64,
    pub break_minutes: u64,
    pub cycles: u32,
    pub start_paused: bool,
    pub meeting_safe: bool,
    pub dim: bool,
}

impl Default for FocusConfig {
    fn default() -> Self {
        Self {
            focus_minutes: 25,
            break_minutes: 5,
            cycles: 1,
            start_paused: false,
            meeting_safe: false,
            dim: false,
        }
    }
}

pub fn state_after(config: &FocusConfig, elapsed: Duration) -> FocusState {
    if config.meeting_safe {
        return FocusState {
            phase: FocusPhase::MeetingSafe,
            progress: 1.0,
            cycle: 0,
        };
    }
    if config.start_paused {
        return FocusState {
            phase: FocusPhase::Paused,
            progress: 0.0,
            cycle: 0,
        };
    }

    let focus = Duration::from_secs(config.focus_minutes.max(1) * 60);
    let break_duration = Duration::from_secs(config.break_minutes * 60);
    let cycle_duration = focus + break_duration;
    let cycles = config.cycles.max(1);
    let total = cycle_duration * cycles;

    if elapsed >= total {
        return FocusState {
            phase: FocusPhase::Overtime,
            progress: 1.0,
            cycle: cycles,
        };
    }

    let cycle_index = (elapsed.as_secs() / cycle_duration.as_secs().max(1)) as u32;
    let cycle_elapsed = elapsed.saturating_sub(cycle_duration * cycle_index);
    if cycle_elapsed < focus {
        FocusState {
            phase: FocusPhase::Focus,
            progress: duration_progress(cycle_elapsed, focus),
            cycle: cycle_index + 1,
        }
    } else {
        let break_elapsed = cycle_elapsed.saturating_sub(focus);
        FocusState {
            phase: FocusPhase::Break,
            progress: duration_progress(break_elapsed, break_duration.max(Duration::from_secs(1))),
            cycle: cycle_index + 1,
        }
    }
}

fn duration_progress(elapsed: Duration, total: Duration) -> f32 {
    (elapsed.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusPhase {
    Focus,
    Break,
    Overtime,
    Paused,
    MeetingSafe,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusState {
    pub phase: FocusPhase,
    pub progress: f32,
    pub cycle: u32,
}

pub fn render_focus(ctx: &RenderContext<'_>, state: FocusState, dim_mode: bool) -> Frame {
    let mut frame = Frame::black();
    let brightness = if dim_mode {
        ctx.brightness / 3
    } else {
        ctx.brightness
    };
    let base = phase_color(state.phase, ctx.tick);
    let dim = base.scale(brightness / 10);

    for key in ctx.layout.keys() {
        frame.set_coord(key.coord, dim);
    }

    let zone = match state.phase {
        FocusPhase::Focus | FocusPhase::Break | FocusPhase::Paused | FocusPhase::MeetingSafe => {
            Zone::Function
        }
        FocusPhase::Overtime => Zone::Alpha,
    };
    let mut keys = ctx
        .layout
        .keys()
        .iter()
        .filter(|key| key.zone == zone)
        .collect::<Vec<_>>();
    if keys.is_empty() {
        keys = ctx.layout.keys().iter().collect();
    }
    keys.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));

    let active = match state.phase {
        FocusPhase::Paused | FocusPhase::MeetingSafe => keys.len(),
        FocusPhase::Overtime => keys.len(),
        FocusPhase::Focus | FocusPhase::Break => ((keys.len() as f32) * state.progress)
            .ceil()
            .clamp(1.0, keys.len() as f32)
            as usize,
    };
    let color = base.scale(brightness);
    for key in keys.into_iter().take(active) {
        frame.set_coord(key.coord, color);
    }

    frame
}

pub fn phase_color(phase: FocusPhase, tick: u32) -> Color {
    match phase {
        FocusPhase::Focus => Color::new(0, 180, 255),
        FocusPhase::Break => Color::new(0, 220, 80),
        FocusPhase::Overtime => Color::new(255, 32, 24).scale(128 + (pulse_wave(tick, 24) / 2)),
        FocusPhase::Paused => Color::new(160, 80, 255),
        FocusPhase::MeetingSafe => Color::new(20, 30, 40),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceInfo, DeviceType, Layout};
    use crate::layout::KeyboardLayout;
    use crate::render::PaletteName;

    #[test]
    fn phase_colors_brightness_and_dim_mode_are_shared_contracts() {
        let info = DeviceInfo {
            connected: false,
            model: "Synthetic focus test".into(),
            max_rows: 6,
            max_columns: 17,
            led_index_max: 0,
            device_type: DeviceType::Keyboard80,
            layout: Layout::Ansi,
            v2_interface: true,
            uses_small_packets: false,
            uses_multi_report: false,
        };
        let layout = KeyboardLayout::for_device(&info);
        for phase in [
            FocusPhase::Focus,
            FocusPhase::Break,
            FocusPhase::Overtime,
            FocusPhase::Paused,
            FocusPhase::MeetingSafe,
        ] {
            for tick in [0, 12, 24, 100] {
                for brightness in [0, 96, 255] {
                    let mut ctx = RenderContext {
                        animation_seconds: 100.0,
                        info: &info,
                        layout: &layout,
                        brightness,
                        palette: PaletteName::Ocean,
                        tick,
                    };
                    let state = FocusState {
                        phase,
                        progress: 0.55,
                        cycle: 1,
                    };
                    let frame = render_focus(&ctx, state, false);
                    ctx.palette = PaletteName::Heat;
                    assert_eq!(frame, render_focus(&ctx, state, false));
                    assert!(
                        frame
                            .as_bytes()
                            .iter()
                            .all(|channel| *channel <= brightness)
                    );
                    if brightness == 0 {
                        assert_eq!(frame, Frame::black());
                    }
                    let dim = render_focus(&ctx, state, true);
                    ctx.brightness /= 3;
                    assert_eq!(dim, render_focus(&ctx, state, false));
                    assert!(
                        dim.as_bytes()
                            .iter()
                            .zip(frame.as_bytes())
                            .all(|(dim, full)| dim <= full)
                    );
                }
            }
        }
    }
}
