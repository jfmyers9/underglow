use crate::layout::Zone;
use crate::render::{Color, Frame, RenderContext, pulse_wave};
use crate::signals::SignalProgram;
use serde::Deserialize;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

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

#[derive(Clone, Debug)]
pub struct FocusSignal {
    config: FocusConfig,
    started: Instant,
}

impl FocusSignal {
    pub fn new(config: FocusConfig) -> Self {
        Self {
            config,
            started: Instant::now(),
        }
    }

    pub fn state_after(&self, elapsed: Duration) -> FocusState {
        state_after(&self.config, elapsed)
    }

    fn current_state(&self) -> FocusState {
        self.state_after(self.started.elapsed())
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

impl SignalProgram for FocusSignal {
    fn tick(&mut self, _interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        Ok(())
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        render_focus(ctx, self.current_state(), self.config.dim)
    }

    fn finished(&self) -> bool {
        false
    }

    fn shutdown(&mut self, _interrupted: bool) {}
}

fn render_focus(ctx: &RenderContext<'_>, state: FocusState, dim_mode: bool) -> Frame {
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

fn phase_color(phase: FocusPhase, tick: u32) -> Color {
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

    fn config() -> FocusConfig {
        FocusConfig {
            focus_minutes: 1,
            break_minutes: 1,
            cycles: 1,
            ..FocusConfig::default()
        }
    }

    #[test]
    fn focus_state_transitions_through_focus_break_overtime() {
        let config = config();

        assert_eq!(
            state_after(&config, Duration::from_secs(0)).phase,
            FocusPhase::Focus
        );
        assert_eq!(
            state_after(&config, Duration::from_secs(60)).phase,
            FocusPhase::Break
        );
        assert_eq!(
            state_after(&config, Duration::from_secs(120)).phase,
            FocusPhase::Overtime
        );
    }

    #[test]
    fn focus_state_supports_paused_and_meeting_safe_modes() {
        assert_eq!(
            state_after(
                &FocusConfig {
                    start_paused: true,
                    ..config()
                },
                Duration::from_secs(60)
            )
            .phase,
            FocusPhase::Paused
        );
        assert_eq!(
            state_after(
                &FocusConfig {
                    meeting_safe: true,
                    ..config()
                },
                Duration::from_secs(60)
            )
            .phase,
            FocusPhase::MeetingSafe
        );
    }

    #[test]
    fn focus_progress_advances_during_phase() {
        let state = state_after(&config(), Duration::from_secs(30));

        assert_eq!(state.phase, FocusPhase::Focus);
        assert!((0.49..=0.51).contains(&state.progress));
    }

    #[test]
    fn focus_renders_full_frame() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let signal = FocusSignal::new(config());

        let frame = signal.render(&RenderContext {
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Wooting,
            tick: 0,
        });

        assert_eq!(frame.as_bytes().len(), crate::render::FRAME_BYTES);
    }

    #[test]
    fn focus_progress_lights_more_keys_over_time() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let context = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Wooting,
            tick: 0,
        };

        let early = render_focus(
            &context,
            FocusState {
                phase: FocusPhase::Focus,
                progress: 0.1,
                cycle: 1,
            },
            false,
        );
        let late = render_focus(
            &context,
            FocusState {
                phase: FocusPhase::Focus,
                progress: 0.9,
                cycle: 1,
            },
            false,
        );

        assert!(lit_channels(late.as_bytes()) > lit_channels(early.as_bytes()));
    }

    fn lit_channels(bytes: &[u8]) -> usize {
        bytes.iter().filter(|channel| **channel > 20).count()
    }
}
