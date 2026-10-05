use crate::layout::Zone;
use crate::render::{Color, Frame, RenderContext, pulse_wave};
use crate::signals::{SignalProgram, SignalSnapshot};
use clap::ValueEnum;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct CommandPulseConfig {
    pub command: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub output: CommandPulseOutput,
    pub summary: bool,
    pub timeout_seconds: u64,
    pub success_hold_seconds: u64,
    pub failure_hold_seconds: u64,
    pub interrupted_hold_seconds: u64,
    pub state_colors: CommandPulseStateColors,
}

impl Default for CommandPulseConfig {
    fn default() -> Self {
        Self {
            command: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            output: CommandPulseOutput::Inherit,
            summary: false,
            timeout_seconds: 600,
            success_hold_seconds: 3,
            failure_hold_seconds: 6,
            interrupted_hold_seconds: 2,
            state_colors: CommandPulseStateColors::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum CommandPulseOutput {
    #[default]
    Inherit,
    Quiet,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CommandPulseStateColors {
    pub pending: [u8; 3],
    pub running: [u8; 3],
    pub success: [u8; 3],
    pub failure: [u8; 3],
    pub timeout: [u8; 3],
    pub interrupted: [u8; 3],
}

impl Default for CommandPulseStateColors {
    fn default() -> Self {
        Self {
            pending: [0, 80, 180],
            running: [0, 180, 255],
            success: [0, 220, 80],
            failure: [255, 32, 24],
            timeout: [255, 128, 0],
            interrupted: [160, 80, 255],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommandPulseError {
    #[error("command-pulse requires a command")]
    EmptyCommand,
}

#[derive(Debug)]
pub struct CommandPulseSignal {
    config: CommandPulseConfig,
    child: Option<Child>,
    state: CommandPulseState,
    summary_reported: bool,
}

#[derive(Debug)]
enum CommandPulseState {
    Pending,
    Running {
        started: Instant,
    },
    Success {
        completed: Instant,
        elapsed: Duration,
    },
    Failure {
        completed: Instant,
        elapsed: Duration,
    },
    TimedOut {
        completed: Instant,
        elapsed: Duration,
    },
    Interrupted {
        completed: Instant,
        elapsed: Duration,
    },
}

impl CommandPulseSignal {
    pub fn new(config: CommandPulseConfig) -> Result<Self, CommandPulseError> {
        if config.command.is_empty() {
            return Err(CommandPulseError::EmptyCommand);
        }

        Ok(Self {
            config,
            child: None,
            state: CommandPulseState::Pending,
            summary_reported: false,
        })
    }

    fn start(&mut self) {
        let mut command = Command::new(&self.config.command[0]);
        command.args(&self.config.command[1..]);
        command.envs(&self.config.env);
        if let Some(cwd) = &self.config.cwd {
            command.current_dir(cwd);
        }
        command.stdin(Stdio::null());
        match self.config.output {
            CommandPulseOutput::Inherit => {
                command.stdout(Stdio::inherit());
                command.stderr(Stdio::inherit());
            }
            CommandPulseOutput::Quiet => {
                command.stdout(Stdio::null());
                command.stderr(Stdio::null());
            }
        }

        match command.spawn() {
            Ok(child) => {
                self.child = Some(child);
                self.summary_reported = false;
                self.state = CommandPulseState::Running {
                    started: Instant::now(),
                };
            }
            Err(error) => {
                eprintln!("command-pulse failed to start: {error}");
                self.state = CommandPulseState::Failure {
                    completed: Instant::now(),
                    elapsed: Duration::ZERO,
                };
            }
        }
    }

    fn poll_child(&mut self, interrupted: &AtomicBool) {
        let CommandPulseState::Running { started } = self.state else {
            return;
        };

        if interrupted.load(Ordering::SeqCst) {
            self.kill_child();
            self.state = CommandPulseState::Interrupted {
                completed: Instant::now(),
                elapsed: started.elapsed(),
            };
            return;
        }

        if started.elapsed() > Duration::from_secs(self.config.timeout_seconds) {
            self.kill_child();
            self.state = CommandPulseState::TimedOut {
                completed: Instant::now(),
                elapsed: started.elapsed(),
            };
            return;
        }

        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.child = None;
                    let completed = Instant::now();
                    let elapsed = started.elapsed();
                    if status.success() {
                        self.state = CommandPulseState::Success { completed, elapsed };
                    } else {
                        self.state = CommandPulseState::Failure { completed, elapsed };
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    eprintln!("command-pulse failed to poll child: {error}");
                    self.kill_child();
                    self.state = CommandPulseState::Failure {
                        completed: Instant::now(),
                        elapsed: started.elapsed(),
                    };
                }
            }
        }
    }

    fn kill_child(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn terminal_hold_elapsed(&self, completed: Instant, hold_seconds: u64) -> bool {
        completed.elapsed() >= Duration::from_secs(hold_seconds)
    }

    fn terminal_color(&self) -> Color {
        let color = match self.state {
            CommandPulseState::Pending => self.config.state_colors.pending,
            CommandPulseState::Running { .. } => self.config.state_colors.running,
            CommandPulseState::Success { .. } => self.config.state_colors.success,
            CommandPulseState::Failure { .. } => self.config.state_colors.failure,
            CommandPulseState::TimedOut { .. } => self.config.state_colors.timeout,
            CommandPulseState::Interrupted { .. } => self.config.state_colors.interrupted,
        };
        Color::new(color[0], color[1], color[2])
    }

    pub fn snapshot(&self, source_id: &str) -> SignalSnapshot {
        SignalSnapshot {
            source_id: source_id.to_string(),
            status: self.status_name().to_string(),
            message: self.config.command.join(" "),
            progress: None,
            intensity: None,
        }
    }

    fn status_name(&self) -> &'static str {
        match self.state {
            CommandPulseState::Pending => "pending",
            CommandPulseState::Running { .. } => "running",
            CommandPulseState::Success { .. } => "success",
            CommandPulseState::Failure { .. } => "failure",
            CommandPulseState::TimedOut { .. } => "timeout",
            CommandPulseState::Interrupted { .. } => "interrupted",
        }
    }

    fn terminal_status(&self) -> Option<(&'static str, Duration)> {
        match self.state {
            CommandPulseState::Pending | CommandPulseState::Running { .. } => None,
            CommandPulseState::Success { elapsed, .. } => Some(("success", elapsed)),
            CommandPulseState::Failure { elapsed, .. } => Some(("failure", elapsed)),
            CommandPulseState::TimedOut { elapsed, .. } => Some(("timeout", elapsed)),
            CommandPulseState::Interrupted { elapsed, .. } => Some(("interrupted", elapsed)),
        }
    }

    fn maybe_print_summary(&mut self) {
        if !self.config.summary || self.summary_reported {
            return;
        }

        if let Some((status, elapsed)) = self.terminal_status() {
            eprintln!(
                "command-pulse: {status} after {:.2}s: {}",
                elapsed.as_secs_f64(),
                self.config.command.join(" ")
            );
            self.summary_reported = true;
        }
    }
}

impl SignalProgram for CommandPulseSignal {
    fn notification_snapshot(&self) -> Option<SignalSnapshot> {
        Some(self.snapshot("command-pulse"))
    }
    fn tick(&mut self, interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        match self.state {
            CommandPulseState::Pending => self.start(),
            CommandPulseState::Running { .. } => self.poll_child(interrupted),
            CommandPulseState::Success { .. }
            | CommandPulseState::Failure { .. }
            | CommandPulseState::TimedOut { .. }
            | CommandPulseState::Interrupted { .. } => {}
        }
        self.maybe_print_summary();
        Ok(())
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        let mut frame = Frame::black();
        let wave = pulse_wave(ctx.tick, 24);
        let base = self.terminal_color();
        let scale = match self.state {
            CommandPulseState::Pending => 48,
            CommandPulseState::Running { .. } => 72 + (wave / 4),
            CommandPulseState::Success { .. } => 160 + (wave / 4),
            CommandPulseState::Failure { .. }
            | CommandPulseState::TimedOut { .. }
            | CommandPulseState::Interrupted { .. } => 128 + (wave / 2),
        };
        let color = base.scale(((u16::from(ctx.brightness) * u16::from(scale)) / 255) as u8);
        let dim = base.scale(ctx.brightness / 8);

        for key in ctx.layout.keys() {
            frame.set_coord(key.coord, dim);
        }

        let mut status_keys = ctx
            .layout
            .keys()
            .iter()
            .filter(|key| key.zone == Zone::Function)
            .collect::<Vec<_>>();
        if status_keys.is_empty() {
            status_keys = ctx.layout.keys().iter().collect();
        }
        status_keys.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));

        match self.state {
            CommandPulseState::Running { .. } => {
                if !status_keys.is_empty() {
                    let head = usize::try_from(ctx.tick).unwrap_or(0) % status_keys.len();
                    for offset in 0..6.min(status_keys.len()) {
                        let index = (head + status_keys.len() - offset) % status_keys.len();
                        let fade = 255u8.saturating_sub((offset * 40) as u8);
                        frame.set_coord(
                            status_keys[index].coord,
                            base.scale(fade).scale(ctx.brightness),
                        );
                    }
                }
            }
            _ => {
                for key in status_keys {
                    frame.set_coord(key.coord, color);
                }
            }
        }

        frame
    }

    fn finished(&self) -> bool {
        match self.state {
            CommandPulseState::Pending | CommandPulseState::Running { .. } => false,
            CommandPulseState::Success { completed, .. } => {
                self.terminal_hold_elapsed(completed, self.config.success_hold_seconds)
            }
            CommandPulseState::Failure { completed, .. }
            | CommandPulseState::TimedOut { completed, .. } => {
                self.terminal_hold_elapsed(completed, self.config.failure_hold_seconds)
            }
            CommandPulseState::Interrupted { completed, .. } => {
                self.terminal_hold_elapsed(completed, self.config.interrupted_hold_seconds)
            }
        }
    }

    fn shutdown(&mut self, interrupted: bool) -> crate::signals::ProgramResult {
        if self.child.is_some() {
            let elapsed = match self.state {
                CommandPulseState::Running { started } => started.elapsed(),
                _ => Duration::ZERO,
            };
            self.kill_child();
            if interrupted {
                self.state = CommandPulseState::Interrupted {
                    completed: Instant::now(),
                    elapsed,
                };
            }
        }
        self.maybe_print_summary();
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
    use std::thread;

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

    fn config(command: &str) -> CommandPulseConfig {
        config_args(&[command])
    }

    fn config_args(command: &[&str]) -> CommandPulseConfig {
        CommandPulseConfig {
            command: command.iter().map(|part| (*part).to_string()).collect(),
            success_hold_seconds: 0,
            failure_hold_seconds: 0,
            interrupted_hold_seconds: 0,
            timeout_seconds: 5,
            ..CommandPulseConfig::default()
        }
    }

    #[test]
    fn command_pulse_rejects_empty_command() {
        assert!(CommandPulseSignal::new(CommandPulseConfig::default()).is_err());
    }

    #[test]
    fn command_pulse_config_accepts_options() {
        let config: CommandPulseConfig = toml::from_str(
            r#"
command = ["make", "check"]
cwd = "/tmp"
env = { RUST_LOG = "debug" }
output = "quiet"
summary = true

[state_colors]
success = [1, 2, 3]
failure = [4, 5, 6]
"#,
        )
        .unwrap();

        assert_eq!(config.cwd, Some(PathBuf::from("/tmp")));
        assert_eq!(config.env["RUST_LOG"], "debug");
        assert_eq!(config.output, CommandPulseOutput::Quiet);
        assert!(config.summary);
        assert_eq!(config.state_colors.success, [1, 2, 3]);
        assert_eq!(config.state_colors.failure, [4, 5, 6]);
        assert_eq!(config.state_colors.timeout, [255, 128, 0]);
    }

    #[test]
    fn terminal_color_uses_state_colors() {
        let mut config = config("true");
        config.state_colors.success = [1, 2, 3];
        let mut signal = CommandPulseSignal::new(config).unwrap();
        signal.state = CommandPulseState::Success {
            completed: Instant::now(),
            elapsed: Duration::from_millis(1),
        };

        assert_eq!(signal.terminal_color(), Color::new(1, 2, 3));
    }

    #[test]
    fn command_pulse_finishes_success() {
        let interrupted = AtomicBool::new(false);
        let mut signal = CommandPulseSignal::new(config("true")).unwrap();
        for _ in 0..100 {
            signal.tick(&interrupted).unwrap();
            if signal.finished() {
                assert!(matches!(signal.state, CommandPulseState::Success { .. }));
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("command did not finish");
    }

    #[test]
    fn command_pulse_marks_failed_exit() {
        let interrupted = AtomicBool::new(false);
        let mut signal = CommandPulseSignal::new(config("false")).unwrap();
        for _ in 0..100 {
            signal.tick(&interrupted).unwrap();
            if signal.finished() {
                assert!(matches!(signal.state, CommandPulseState::Failure { .. }));
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("command did not finish");
    }

    #[cfg(unix)]
    #[test]
    fn command_pulse_applies_env_and_cwd() {
        let interrupted = AtomicBool::new(false);
        let cwd =
            std::env::temp_dir().join(format!("wooting-command-pulse-test-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        let mut config = config_args(&[
            "sh",
            "-c",
            "test \"$WOOTING_TEST_ENV\" = works && touch command-pulse-cwd-ok",
        ]);
        config.cwd = Some(cwd.clone());
        config
            .env
            .insert("WOOTING_TEST_ENV".to_string(), "works".to_string());
        config.output = CommandPulseOutput::Quiet;
        let mut signal = CommandPulseSignal::new(config).unwrap();

        for _ in 0..100 {
            signal.tick(&interrupted).unwrap();
            if signal.finished() {
                assert!(cwd.join("command-pulse-cwd-ok").exists());
                std::fs::remove_dir_all(&cwd).unwrap();
                assert!(matches!(signal.state, CommandPulseState::Success { .. }));
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = std::fs::remove_dir_all(&cwd);
        panic!("command did not finish");
    }

    #[cfg(unix)]
    #[test]
    fn command_pulse_marks_timeout_and_clears_child() {
        let interrupted = AtomicBool::new(false);
        let mut config = config_args(&["sleep", "1"]);
        config.timeout_seconds = 0;
        let mut signal = CommandPulseSignal::new(config).unwrap();

        signal.tick(&interrupted).unwrap();
        thread::sleep(Duration::from_millis(10));
        signal.tick(&interrupted).unwrap();

        assert!(matches!(signal.state, CommandPulseState::TimedOut { .. }));
        assert!(signal.child.is_none());
        assert!(signal.finished());
    }

    #[cfg(unix)]
    #[test]
    fn command_pulse_marks_interrupt_and_clears_child() {
        let interrupted = AtomicBool::new(false);
        let mut signal = CommandPulseSignal::new(config_args(&["sleep", "1"])).unwrap();

        signal.tick(&interrupted).unwrap();
        interrupted.store(true, std::sync::atomic::Ordering::SeqCst);
        signal.tick(&interrupted).unwrap();

        assert!(matches!(
            signal.state,
            CommandPulseState::Interrupted { .. }
        ));
        assert!(signal.child.is_none());
        assert!(signal.finished());
    }

    #[test]
    fn command_pulse_renders_full_frame() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let signal = CommandPulseSignal::new(config("true")).unwrap();
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
}
