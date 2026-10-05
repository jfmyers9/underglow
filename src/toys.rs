//! Interactive toys sharing the signal runtime and hardware-free previews.
use crate::layout::KeyboardLayout;
#[cfg(test)]
use crate::layout::MatrixCoord;
use crate::preview::{self, PreviewFormat};
use crate::render::{Color, Frame, PaletteName};
use crate::runner::{SignalRunOptions, run_session};
use crate::sdk::analog::{AnalogKeyPressure, AnalogSdk};
use crate::sdk::rgb::{DeviceInfo, DeviceType, Layout};
use crate::signals::{ProgramResult, SignalProgram};
use clap::{Args, Subcommand};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

#[derive(Debug, Subcommand)]
pub enum ToyCommand {
    /// Key travel launches expanding rings of color (80HE ANSI).
    Ripples(RippleOptions),
}

#[derive(Debug, Args)]
pub struct RippleOptions {
    /// Official Analog SDK distributable library (0.9.1 or newer 0.9.x).
    #[arg(long)]
    analog_sdk_path: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = PaletteName::Ocean)]
    palette: PaletteName,
    /// Maximum RGB channel value (0–255); independent of Wootility brightness.
    #[arg(long, default_value_t = underglow::DEFAULT_BRIGHTNESS)]
    brightness: u8,
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..=120))]
    fps: u32,
    /// Stop after this many seconds; otherwise run until Ctrl-C.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=86400))]
    seconds: Option<u64>,
    /// Render synthetic key presses without loading either SDK or touching hardware.
    #[arg(long)]
    pub(crate) preview: bool,
    #[arg(long, requires = "preview", default_value_t = 12, value_parser = clap::value_parser!(u32).range(1..=600))]
    ticks: u32,
    #[arg(long, requires = "preview", value_enum, default_value_t = PreviewFormat::Ansi)]
    format: PreviewFormat,
}

pub fn run(
    command: ToyCommand,
    rgb_path: Option<&Path>,
    interrupted: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        ToyCommand::Ripples(options) => run_ripples(&options, rgb_path, interrupted),
    }
}

fn run_ripples(
    options: &RippleOptions,
    rgb_path: Option<&Path>,
    interrupted: &AtomicBool,
) -> ProgramResult {
    let mut signal = RippleSignal::new(RippleConfig {
        analog_sdk_path: options.analog_sdk_path.clone(),
        ..RippleConfig::default()
    });
    let run = SignalRunOptions {
        palette: options.palette,
        brightness: options.brightness,
        fps: options.fps,
        speed: underglow::animation::DEFAULT_SPEED,
        seconds: options.seconds,
        continuous: options.seconds.is_none(),
    };
    if options.preview {
        preview::print_signal_preview(&mut signal, &run, options.ticks, options.format);
        Ok(())
    } else {
        run_session(rgb_path, &run, &mut signal, true, interrupted)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RippleConfig {
    pub analog_sdk_path: Option<PathBuf>,
    #[serde(deserialize_with = "deserialize_rgb")]
    pub base_color: Option<[u8; 3]>,
    #[serde(deserialize_with = "deserialize_rgb")]
    pub ripple_color: Option<[u8; 3]>,
}

fn deserialize_rgb<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<[u8; 3]>, D::Error> {
    let channels = Option::<Vec<u8>>::deserialize(deserializer)?;
    channels
        .map(|v| {
            v.try_into()
                .map_err(|_| serde::de::Error::custom("RGB color requires exactly three channels"))
        })
        .transpose()
}

/// Construction is side-effect-free. Only initialize opens the analog SDK.
pub struct RippleSignal {
    config: RippleConfig,
    analog: Option<AnalogSdk>,
    ripples: Ripples,
    previous: Instant,
}

impl RippleSignal {
    pub fn new(config: RippleConfig) -> Self {
        Self {
            config,
            analog: None,
            ripples: Ripples::default(),
            previous: Instant::now(),
        }
    }
}

impl SignalProgram for RippleSignal {
    fn set_ripple_colors(&mut self, base: Option<[u8; 3]>, ripple: Option<[u8; 3]>) {
        self.config.base_color = base;
        self.config.ripple_color = ripple;
    }

    fn initialize(&mut self) -> ProgramResult {
        if self.analog.is_some() {
            return Err("ripples is already initialized".into());
        }
        self.analog = Some(AnalogSdk::open(self.config.analog_sdk_path.as_deref())?);
        self.ripples = Ripples::default();
        self.previous = Instant::now();
        eprintln!(
            "Ripples owns RGB while running. Ctrl-C attempts to restore the current keyboard profile's lighting. Key presses still reach your apps."
        );
        Ok(())
    }

    fn validate_device(&self, info: &DeviceInfo) -> ProgramResult {
        if info.device_type != DeviceType::Keyboard80
            || info.layout != Layout::Ansi
            || info.max_rows != 6
            || info.max_columns != 17
        {
            return Err("ripples currently supports the 80HE ANSI layout only".into());
        }
        Ok(())
    }

    fn tick(&mut self, _interrupted: &AtomicBool) -> ProgramResult {
        let analog = self
            .analog
            .as_ref()
            .ok_or("ripples has not been initialized")?;
        let now = Instant::now();
        let keys = analog.read()?;
        self.ripples
            .advance(now.duration_since(self.previous).as_secs_f32(), &keys);
        self.previous = now;
        Ok(())
    }

    fn preview_tick(&mut self, tick: u32) {
        // Same synthetic travel sequence for both CLI entry points and profiles.
        // Fixed 100ms steps do not depend on wall-clock time or live input.
        let keys = if tick < 5 {
            vec![AnalogKeyPressure {
                key_code: 0x09,
                pressure: (tick + 1) as f32 / 5.0,
            }]
        } else {
            vec![]
        };
        self.ripples.advance(0.1, &keys);
    }

    fn render(&self, ctx: &crate::render::RenderContext<'_>) -> Frame {
        self.ripples
            .render(ctx.layout, ctx.palette, ctx.brightness, &self.config)
    }

    fn finished(&self) -> bool {
        false
    }

    fn shutdown(&mut self, _interrupted: bool) -> ProgramResult {
        let result = self
            .analog
            .take()
            .map(|mut analog| analog.close())
            .transpose();
        self.ripples = Ripples::default();
        result?;
        Ok(())
    }
}

/// Adapter from SDK/layout types to the shared pure renderer.
#[derive(Default)]
struct Ripples(underglow::ripple::RippleSimulation);

impl Ripples {
    fn advance(&mut self, dt: f32, keys: &[AnalogKeyPressure]) {
        self.0
            .advance(dt, keys.iter().map(|key| (key.key_code, key.pressure)));
    }

    fn render(
        &self,
        layout: &KeyboardLayout,
        palette: PaletteName,
        brightness: u8,
        config: &RippleConfig,
    ) -> Frame {
        let geometry: Vec<_> = layout
            .keys()
            .iter()
            .map(|key| underglow::ripple::KeyGeometry {
                row: key.coord.row,
                column: key.coord.column,
                x: key.x,
                y: key.y,
            })
            .collect();
        let palette = palette.palette();
        let colors = self.0.render(
            &geometry,
            |position| {
                let color = palette.gradient(position);
                [color.red, color.green, color.blue]
            },
            brightness,
            config.base_color,
            config.ripple_color,
        );
        let mut frame = Frame::black();
        for (key, [red, green, blue]) in layout.keys().iter().zip(colors) {
            frame.set_coord(key.coord, Color::new(red, green, blue));
        }
        frame
    }
}

#[cfg(test)]
fn hid_coord(code: u16) -> Option<MatrixCoord> {
    underglow::ripple::hid_coord(code).map(|coord| MatrixCoord {
        row: coord.row,
        column: coord.column,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    const LIFETIME: f32 = 2.5;
    const MAX_RIPPLES: usize = 128;

    #[test]
    fn two_tone_idle_peak_decay_and_live_update() {
        let layout = KeyboardLayout::for_device(&preview::preview_device());
        let mut signal = RippleSignal::new(RippleConfig::default());
        signal.set_ripple_colors(Some([0, 32, 64]), Some([120, 255, 255]));
        let render = |signal: &RippleSignal, brightness| {
            signal
                .ripples
                .render(&layout, PaletteName::Ocean, brightness, &signal.config)
        };
        let origin = hid_coord(0x09).unwrap();
        assert_eq!(
            render(&signal, 255).get_coord(origin),
            Color::new(0, 32, 64)
        );
        signal.ripples.advance(0.0, &[key(1.0)]);
        assert_eq!(
            render(&signal, 255).get_coord(origin),
            Color::new(120, 255, 255)
        );
        assert_eq!(
            render(&signal, 96).get_coord(origin),
            Color::new(120, 255, 255).scale(96)
        );
        assert_eq!(render(&signal, 0), Frame::black());
        signal.set_ripple_colors(Some([0; 3]), Some([255, 0, 0]));
        assert_eq!(signal.ripples.0.wave_count(), 1); // no restart on edit
        assert_eq!(
            render(&signal, 255).get_coord(origin),
            Color::new(255, 0, 0)
        );
        signal.ripples.advance(LIFETIME, &[]);
        assert_eq!(render(&signal, 255), Frame::black());
        signal.set_ripple_colors(Some([0, 32, 64]), Some([120, 255, 255]));
        assert_eq!(
            render(&signal, 255).get_coord(origin),
            Color::new(0, 32, 64)
        );
        signal.set_ripple_colors(None, None);
        assert_eq!(render(&signal, 255), Frame::black());
    }

    #[test]
    fn brighter_default_preserves_explicit_brightness() {
        for (extra, expected) in [(vec![], 255), (vec!["--brightness", "96"], 96)] {
            let cli = crate::Cli::try_parse_from(
                ["underglow", "toy", "ripples"].into_iter().chain(extra),
            )
            .unwrap();
            let crate::Command::Toy {
                command: ToyCommand::Ripples(options),
            } = cli.command
            else {
                panic!("expected ripples");
            };
            assert_eq!(options.brightness, expected);
        }
    }

    fn key(pressure: f32) -> AnalogKeyPressure {
        AnalogKeyPressure {
            key_code: 0x09,
            pressure,
        }
    }
    fn frame(pressure: f32, age: f32) -> Frame {
        let mut ripples = Ripples::default();
        ripples.advance(0.0, &[key(pressure)]);
        ripples.advance(age, &[]);
        ripples.render(
            &KeyboardLayout::for_device(&preview::preview_device()),
            PaletteName::Ocean,
            96,
            &RippleConfig::default(),
        )
    }
    #[test]
    fn mapping_preserves_gaps_and_rejects_namespaces() {
        assert_eq!(hid_coord(0x04), Some(MatrixCoord { row: 3, column: 1 }));
        assert_eq!(hid_coord(0x1d), Some(MatrixCoord { row: 4, column: 2 }));
        assert_eq!(hid_coord(0x2c), Some(MatrixCoord { row: 5, column: 6 }));
        assert_eq!(hid_coord(0x0204), None);
        assert_eq!(hid_coord(0), None);
    }
    #[test]
    fn deeper_travel_is_brighter_and_rings_expand_then_expire() {
        let energy = |frame: Frame| frame.as_bytes().iter().map(|&v| u32::from(v)).sum::<u32>();
        assert!(energy(frame(1.0, 0.0)) > energy(frame(0.2, 0.0)));
        assert_ne!(frame(1.0, 0.0), frame(1.0, 0.4));
        assert_eq!(frame(1.0, LIFETIME), Frame::black());
        assert!(frame(1.0, 0.4).as_bytes().iter().all(|&v| v <= 96));
    }
    #[test]
    fn idle_invalid_and_unmapped_input_stay_dark() {
        assert_eq!(frame(0.0, 0.0), Frame::black());
        assert_eq!(frame(f32::NAN, 0.0), Frame::black());
        let mut ripples = Ripples::default();
        ripples.advance(
            0.1,
            &[AnalogKeyPressure {
                key_code: 0x0204,
                pressure: 1.0,
            }],
        );
        assert!(ripples.0.wave_count() == 0);
    }
    #[test]
    fn holds_are_rate_limited_release_stops_emission_and_state_is_bounded() {
        let mut ripples = Ripples::default();
        for _ in 0..10 {
            ripples.advance(0.01, &[key(1.0)]);
        }
        assert_eq!(ripples.0.wave_count(), 1);
        ripples.advance(0.2, &[key(1.0)]);
        assert_eq!(ripples.0.wave_count(), 2);
        ripples.advance(3.0, &[]);
        assert!(ripples.0.wave_count() == 0);
        let keys = (1..256)
            .map(|key_code| AnalogKeyPressure {
                key_code,
                pressure: 1.0,
            })
            .collect::<Vec<_>>();
        for _ in 0..100 {
            ripples.advance(0.2, &keys);
        }
        assert!(ripples.0.wave_count() <= MAX_RIPPLES);
    }
}
