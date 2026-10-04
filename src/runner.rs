use crate::effects::EffectKind;
use crate::layout::KeyboardLayout;
use crate::render::{PaletteName, RenderContext};
use crate::sdk::rgb::WootingRgb;
use crate::signals::{ProgramResult, SignalProgram, StaticEffectSignal};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub effect: EffectKind,
    pub palette: PaletteName,
    pub brightness: u8,
    pub fps: u32,
    pub seconds: Option<u64>,
    pub continuous: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            effect: EffectKind::Rainbow,
            palette: PaletteName::Wooting,
            brightness: 96,
            fps: 30,
            seconds: Some(10),
            continuous: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SignalRunOptions {
    pub palette: PaletteName,
    pub brightness: u8,
    pub fps: u32,
    pub seconds: Option<u64>,
    pub continuous: bool,
}

impl Default for SignalRunOptions {
    fn default() -> Self {
        let run = RunOptions::default();
        Self {
            palette: run.palette,
            brightness: run.brightness,
            fps: run.fps,
            seconds: run.seconds,
            continuous: run.continuous,
        }
    }
}

impl From<&RunOptions> for SignalRunOptions {
    fn from(options: &RunOptions) -> Self {
        Self {
            palette: options.palette,
            brightness: options.brightness,
            fps: options.fps,
            seconds: options.seconds,
            continuous: options.continuous,
        }
    }
}

/// Own the complete lifecycle for every live mode. Preview paths never call this.
pub fn run_session(
    sdk_path: Option<&Path>,
    options: &SignalRunOptions,
    signal: &mut dyn SignalProgram,
    warn_on_close_error: bool,
    interrupted: &AtomicBool,
) -> ProgramResult {
    if !(1..=120).contains(&options.fps) {
        return Err("fps must be between 1 and 120".into());
    }
    let mut keyboard = None;
    let result = (|| -> ProgramResult {
        if interrupted.load(Ordering::SeqCst) {
            return Ok(());
        }
        // Fail missing mode-specific dependencies before taking lighting control.
        signal.initialize()?;
        if interrupted.load(Ordering::SeqCst) {
            return Ok(());
        }
        keyboard = Some(WootingRgb::open(sdk_path)?);
        let keyboard = keyboard.as_ref().expect("just opened RGB session");
        signal.validate_device(keyboard.info())?;
        eprintln!(
            "keyboard: {} ({:?})",
            keyboard.info().model,
            keyboard.info().layout
        );
        run_frames(keyboard, options, signal, interrupted)
    })();
    if let Some(keyboard) = keyboard.as_mut()
        && let Err(error) = keyboard.close()
        && warn_on_close_error
    {
        eprintln!(
            "warning: lighting restoration was not acknowledged: {error}; stop other RGB writers or reconnect the keyboard if needed"
        );
    }
    // Do this even on initialization, device validation, input, or RGB failures.
    // Commands terminate children; ripples closes its analog SDK; profiles stop all sources.
    signal.shutdown(interrupted.load(Ordering::SeqCst) || result.is_err());
    result
}

pub fn run_effect(
    keyboard: &WootingRgb,
    options: &RunOptions,
    interrupted: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut signal = StaticEffectSignal::new(options.effect);
    run_frames(
        keyboard,
        &SignalRunOptions::from(options),
        &mut signal,
        interrupted,
    )
}

fn run_frames(
    keyboard: &WootingRgb,
    options: &SignalRunOptions,
    signal: &mut dyn SignalProgram,
    interrupted: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    let fps = options.fps.max(1);
    let frame_time = Duration::from_secs_f64(1.0 / f64::from(fps));
    let deadline = options
        .seconds
        .filter(|_| !options.continuous)
        .map(|seconds| {
            Instant::now()
                .checked_add(Duration::from_secs(seconds))
                .ok_or("run duration is too large")
        })
        .transpose()?;
    let layout = KeyboardLayout::for_device(keyboard.info());
    let mut tick = 0;

    while !interrupted.load(Ordering::SeqCst)
        && !signal.finished()
        && deadline.is_none_or(|deadline| Instant::now() < deadline)
    {
        let started = Instant::now();
        signal.tick(interrupted)?;
        let frame = signal.render(&RenderContext {
            info: keyboard.info(),
            layout: &layout,
            brightness: options.brightness,
            palette: options.palette,
            tick,
        });
        keyboard.set_frame(&frame)?;
        keyboard.update()?;
        tick += 1;

        if let Some(remaining) = frame_time.checked_sub(started.elapsed()) {
            sleep_interruptibly(remaining, interrupted);
        }
    }

    Ok(())
}

pub fn sleep_interruptibly(duration: Duration, interrupted: &AtomicBool) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline && !interrupted.load(Ordering::SeqCst) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        thread::sleep(remaining.min(Duration::from_millis(25)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Frame;

    struct LifecycleProbe {
        fail_initialize: bool,
        initialized: bool,
        stopped: bool,
    }

    impl SignalProgram for LifecycleProbe {
        fn initialize(&mut self) -> ProgramResult {
            self.initialized = true;
            if self.fail_initialize {
                Err("input initialization failed".into())
            } else {
                Ok(())
            }
        }
        fn tick(&mut self, _: &AtomicBool) -> ProgramResult {
            panic!("must not tick without a device")
        }
        fn render(&self, _: &RenderContext<'_>) -> Frame {
            panic!("must not render without a device")
        }
        fn finished(&self) -> bool {
            false
        }
        fn shutdown(&mut self, interrupted: bool) {
            assert!(interrupted);
            self.stopped = true;
        }
    }

    #[test]
    fn initialization_and_rgb_open_failures_always_shutdown_the_mode() {
        for fail_initialize in [true, false] {
            let mut signal = LifecycleProbe {
                fail_initialize,
                initialized: false,
                stopped: false,
            };
            let error = run_session(
                Some(Path::new("/missing-test-sdk")),
                &SignalRunOptions::default(),
                &mut signal,
                true,
                &AtomicBool::new(false),
            )
            .unwrap_err();
            assert!(signal.initialized && signal.stopped);
            if fail_initialize {
                assert!(error.to_string().contains("input initialization failed"));
            } else {
                assert!(error.to_string().contains("failed to load Wooting RGB SDK"));
            }
        }
    }
}
