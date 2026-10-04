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

/// A live mode's RGB session. Call close even after a failed frame.
pub struct Session {
    keyboard: WootingRgb,
    options: SignalRunOptions,
    deadline: Option<Instant>,
    tick: u32,
    closed: bool,
}
impl Session {
    pub fn set_visuals(&mut self, options: &SignalRunOptions) {
        self.options.brightness = options.brightness;
        self.options.palette = options.palette;
        self.options.fps = options.fps;
    }
    pub fn open(
        path: Option<&Path>,
        options: &SignalRunOptions,
        signal: &mut dyn SignalProgram,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if !(1..=120).contains(&options.fps) {
            return Err("fps must be between 1 and 120".into());
        }
        let deadline = options
            .seconds
            .filter(|_| !options.continuous)
            .map(|s| {
                Instant::now()
                    .checked_add(Duration::from_secs(s))
                    .ok_or("run duration is too large")
            })
            .transpose()?;
        let mut keyboard = None;
        let result = (|| -> ProgramResult {
            signal.initialize()?;
            keyboard = Some(WootingRgb::open(path)?);
            signal.validate_device(keyboard.as_ref().expect("opened").info())?;
            Ok(())
        })();
        if let Err(error) = result {
            if let Some(keyboard) = keyboard.as_mut()
                && let Err(close) = keyboard.close()
            {
                eprintln!("warning: {close}");
            }
            if let Err(close) = signal.shutdown(true) {
                eprintln!("warning: {close}");
            }
            return Err(error);
        }
        // The configured duration measures animation time, not SDK startup time.
        let deadline = deadline.map(|_| {
            Instant::now() + Duration::from_secs(options.seconds.expect("finite duration"))
        });
        Ok(Self {
            keyboard: keyboard.expect("opened"),
            options: options.clone(),
            deadline,
            tick: 0,
            closed: false,
        })
    }
    pub fn step(
        &mut self,
        signal: &mut dyn SignalProgram,
        interrupted: &AtomicBool,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if self.closed
            || interrupted.load(Ordering::SeqCst)
            || signal.finished()
            || self.deadline.is_some_and(|d| Instant::now() >= d)
        {
            return Ok(false);
        }
        render_frame(
            &self.keyboard,
            &self.options,
            signal,
            self.tick,
            interrupted,
        )?;
        self.tick = self.tick.wrapping_add(1);
        Ok(true)
    }
    pub fn close(&mut self, signal: &mut dyn SignalProgram, interrupted: bool) -> ProgramResult {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let rgb = self.keyboard.close().map_err(|e| e.to_string());
        let input = signal.shutdown(interrupted).map_err(|e| e.to_string());
        match (rgb, input) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(a), Err(b)) => Err(format!("{a}; {b}").into()),
            (Err(error), _) | (_, Err(error)) => Err(error.into()),
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
    if interrupted.load(Ordering::SeqCst) {
        return signal.shutdown(true);
    }
    let mut session = Session::open(sdk_path, options, signal)?;
    let frame_time = Duration::from_secs_f64(1.0 / f64::from(options.fps));
    let result = (|| -> ProgramResult {
        loop {
            let start = Instant::now();
            if !session.step(signal, interrupted)? {
                break;
            }
            if let Some(remaining) = frame_time.checked_sub(start.elapsed()) {
                sleep_interruptibly(remaining, interrupted);
            }
        }
        Ok(())
    })();
    if let Err(error) = session.close(
        signal,
        interrupted.load(Ordering::SeqCst) || result.is_err(),
    ) && warn_on_close_error
    {
        eprintln!(
            "warning: lighting restoration was not acknowledged, or input shutdown failed: {error}; stop other RGB writers or reconnect the keyboard if needed"
        );
    }
    result
}

fn render_frame(
    keyboard: &WootingRgb,
    options: &SignalRunOptions,
    signal: &mut dyn SignalProgram,
    tick: u32,
    interrupted: &AtomicBool,
) -> ProgramResult {
    signal.tick(interrupted)?;
    let layout = KeyboardLayout::for_device(keyboard.info());
    let frame = signal.render(&RenderContext {
        info: keyboard.info(),
        layout: &layout,
        brightness: options.brightness,
        palette: options.palette,
        tick,
    });
    keyboard.set_frame(&frame)?;
    keyboard.update()?;
    Ok(())
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
        fn shutdown(&mut self, interrupted: bool) -> ProgramResult {
            assert!(interrupted);
            self.stopped = true;
            Ok(())
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
