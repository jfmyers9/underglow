use crate::layout::Zone;
use crate::render::{Frame, RenderContext};
use crate::scenes;
use crate::signals::{SignalProgram, SignalSnapshot};
use serde::Deserialize;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FixtureConfig {
    pub steps: Vec<FixtureStep>,
    pub loop_steps: bool,
}

impl Default for FixtureConfig {
    fn default() -> Self {
        Self {
            steps: vec![FixtureStep::default()],
            loop_steps: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FixtureStep {
    pub status: String,
    pub message: String,
    pub progress: Option<f32>,
    pub intensity: Option<f32>,
    pub hold_ticks: u32,
    pub hold_seconds: Option<u64>,
}

impl Default for FixtureStep {
    fn default() -> Self {
        Self {
            status: "running".to_string(),
            message: "fixture running".to_string(),
            progress: Some(0.5),
            intensity: Some(1.0),
            hold_ticks: 30,
            hold_seconds: None,
        }
    }
}

impl FixtureStep {
    fn duration_ticks(&self) -> u32 {
        self.hold_seconds
            .map(|seconds| (seconds as u32).saturating_mul(30))
            .unwrap_or(self.hold_ticks)
            .max(1)
    }
}

#[derive(Clone, Debug)]
pub struct FixtureSignal {
    config: FixtureConfig,
    index: usize,
    tick_in_step: u32,
    finished: bool,
}

impl FixtureSignal {
    pub fn new(config: FixtureConfig) -> Self {
        Self {
            config,
            index: 0,
            tick_in_step: 0,
            finished: false,
        }
    }

    pub fn snapshot(&self, source_id: &str) -> SignalSnapshot {
        let step = self.current_step();
        SignalSnapshot {
            source_id: source_id.to_string(),
            status: step.status.clone(),
            message: step.message.clone(),
            progress: step.progress,
            intensity: step.intensity,
        }
    }

    fn current_step(&self) -> &FixtureStep {
        self.config
            .steps
            .get(self.index)
            .or_else(|| self.config.steps.first())
            .expect("fixture config always has at least one step")
    }

    fn advance(&mut self) {
        if self.finished {
            return;
        }
        self.tick_in_step += 1;
        if self.tick_in_step < self.current_step().duration_ticks() {
            return;
        }

        self.tick_in_step = 0;
        if self.index + 1 < self.config.steps.len() {
            self.index += 1;
        } else if self.config.loop_steps {
            self.index = 0;
        } else {
            self.finished = true;
        }
    }
}

impl SignalProgram for FixtureSignal {
    fn tick(&mut self, _interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        self.advance();
        Ok(())
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        let step = self.current_step();
        let zones = [Zone::Function, Zone::Navigation, Zone::Arrows];
        let mut frame = scenes::render_status_wash(ctx, &step.status, &zones);
        if let Some(progress) = step.progress {
            scenes::progress_bar(
                &mut frame,
                ctx.layout,
                Some(Zone::Function),
                progress,
                scenes::status_color(&step.status, ctx.tick).scale(ctx.brightness),
            );
        }
        frame
    }

    fn finished(&self) -> bool {
        self.finished
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
    use std::sync::atomic::AtomicBool;

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
    fn fixture_config_accepts_scripted_steps() {
        let config: FixtureConfig = toml::from_str(
            r#"
loop_steps = false

[[steps]]
status = "running"
message = "tests running"
progress = 0.25
hold_ticks = 1

[[steps]]
status = "failure"
message = "tests failed"
intensity = 0.9
hold_seconds = 1
"#,
        )
        .unwrap();

        assert_eq!(config.steps.len(), 2);
        assert!(!config.loop_steps);
        assert_eq!(config.steps[0].status, "running");
        assert_eq!(config.steps[1].duration_ticks(), 30);
    }

    #[test]
    fn fixture_advances_statuses() {
        let mut signal = FixtureSignal::new(FixtureConfig {
            steps: vec![
                FixtureStep {
                    status: "running".to_string(),
                    hold_ticks: 1,
                    ..FixtureStep::default()
                },
                FixtureStep {
                    status: "success".to_string(),
                    hold_ticks: 1,
                    ..FixtureStep::default()
                },
            ],
            loop_steps: false,
        });
        let interrupted = AtomicBool::new(false);

        assert_eq!(signal.snapshot("demo").status, "running");
        signal.tick(&interrupted).unwrap();
        assert_eq!(signal.snapshot("demo").status, "success");
    }

    #[test]
    fn fixture_renders_full_frame() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let signal = FixtureSignal::new(FixtureConfig::default());
        let frame = signal.render(&RenderContext {
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
