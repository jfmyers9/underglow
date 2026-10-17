use crate::render::{Frame, RenderContext};
use crate::signals::SignalProgram;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
#[cfg(test)]
use underglow::focus::FocusPhase;
pub use underglow::focus::FocusState;
use underglow::focus::render_focus;
pub use underglow::focus::{FocusConfig, state_after};

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

impl SignalProgram for FocusSignal {
    fn notification_snapshot(&self) -> Option<crate::signals::SignalSnapshot> {
        let state = self.current_state();
        let mut snapshot = crate::signals::SignalSnapshot::status(
            "focus-cockpit",
            format!("{:?}", state.phase).to_lowercase(),
        );
        snapshot.message = state.cycle.to_string();
        Some(snapshot)
    }
    fn tick(&mut self, _interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        Ok(())
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        render_focus(ctx, self.current_state(), self.config.dim)
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
            animation_seconds: 0.0,
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
            animation_seconds: 0.0,
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
