//! Interactive toys sharing the signal runtime and hardware-free previews.
use crate::layout::{KeyboardLayout, MatrixCoord};
use crate::preview::{self, PreviewFormat};
use crate::render::{Frame, PaletteName};
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
    #[arg(long, default_value_t = 180)]
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
    });
    let run = SignalRunOptions {
        palette: options.palette,
        brightness: options.brightness,
        fps: options.fps,
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
        self.ripples.render(ctx.layout, ctx.palette, ctx.brightness)
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

const LIFETIME: f32 = 2.5;
const MAX_RIPPLES: usize = 128;

struct Ripple {
    coord: MatrixCoord,
    strength: f32,
    age: f32,
}

struct Ripples {
    waves: Vec<Ripple>,
    previous: [f32; 256],
    cooldown: [f32; 256],
}

impl Default for Ripples {
    fn default() -> Self {
        Self {
            waves: Vec::new(),
            previous: [0.0; 256],
            cooldown: [0.0; 256],
        }
    }
}

impl Ripples {
    fn advance(&mut self, dt: f32, keys: &[AnalogKeyPressure]) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        for wave in &mut self.waves {
            wave.age += dt;
        }
        self.waves.retain(|wave| wave.age < LIFETIME);
        for cooldown in &mut self.cooldown {
            *cooldown = (*cooldown - dt).max(0.0);
        }
        let mut current = [0.0f32; 256];
        for key in keys {
            // Do not truncate namespaces: custom/Fn keys are not ordinary USB HID keys.
            if key.key_code < 256 && key.pressure.is_finite() {
                let index = usize::from(key.key_code);
                current[index] = current[index].max(key.pressure.clamp(0.0, 1.0));
            }
        }
        for (index, &pressure) in current.iter().enumerate() {
            if pressure < 0.05 {
                continue;
            }
            let Some(coord) = hid_coord(index as u16) else {
                continue;
            };
            // A new touch is immediate. Held keys emit at most five rings/second;
            // their strength tracks key travel, rather than typing repeat events.
            if self.previous[index] < 0.05 || self.cooldown[index] == 0.0 {
                if self.waves.len() == MAX_RIPPLES {
                    self.waves.remove(0);
                }
                self.waves.push(Ripple {
                    coord,
                    strength: pressure,
                    age: 0.0,
                });
                self.cooldown[index] = 0.2;
            }
        }
        self.previous = current;
    }

    fn render(&self, layout: &KeyboardLayout, palette: PaletteName, brightness: u8) -> Frame {
        let mut frame = Frame::black();
        let palette = palette.palette();
        let mut intensities = vec![0.0f32; layout.keys().len()];
        for wave in &self.waves {
            let Some(origin) = layout.keys().iter().find(|key| key.coord == wave.coord) else {
                continue;
            };
            let amplitude = wave.strength * (1.0 - wave.age / LIFETIME).powi(2);
            for (key, intensity) in layout.keys().iter().zip(&mut intensities) {
                let distance = (key.x - origin.x).hypot(key.y - origin.y);
                let ring = (1.0 - (distance - wave.age * 6.0).abs() / 1.1).max(0.0);
                *intensity += ring * amplitude;
            }
        }
        for (key, intensity) in layout.keys().iter().zip(intensities) {
            let intensity = intensity.min(1.0);
            frame.set_coord(
                key.coord,
                palette
                    .gradient((intensity * 255.0) as u8)
                    .scale((intensity * f32::from(brightness)) as u8),
            );
        }
        frame
    }
}

/// Standard ANSI HID positions in the RGB SDK's 6x21 matrix (not OS text layout).
/// Restrict the first toy to the typing block; Fn/custom namespaces and navigation
/// are intentionally unmapped until validated on hardware. See the SDK's
/// resources/keyboard-matrix-rows-columns.png. Gaps are not compressed.
fn hid_coord(code: u16) -> Option<MatrixCoord> {
    const ROWS: [&[u16]; 4] = [
        &[
            0x35, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2d, 0x2e, 0x2a,
        ],
        &[
            0x2b, 0x14, 0x1a, 0x08, 0x15, 0x17, 0x1c, 0x18, 0x0c, 0x12, 0x13, 0x2f, 0x30, 0x31,
        ],
        &[
            0x39, 0x04, 0x16, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x0f, 0x33, 0x34, 0, 0x28,
        ],
        &[
            0xe1, 0, 0x1d, 0x1b, 0x06, 0x19, 0x05, 0x11, 0x10, 0x36, 0x37, 0x38, 0, 0xe5,
        ],
    ];
    if code == 0 {
        return None;
    }
    if code == 0x2c {
        return Some(MatrixCoord { row: 5, column: 6 });
    }
    ROWS.iter().enumerate().find_map(|(row, keys)| {
        keys.iter()
            .position(|&key| key == code)
            .map(|column| MatrixCoord {
                row: row as u8 + 1,
                column: column as u8,
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn brighter_default_preserves_explicit_brightness() {
        for (extra, expected) in [(vec![], 180), (vec!["--brightness", "96"], 96)] {
            let cli = crate::Cli::try_parse_from(
                ["wooting-signals", "toy", "ripples"]
                    .into_iter()
                    .chain(extra),
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
        assert!(ripples.waves.is_empty());
    }
    #[test]
    fn holds_are_rate_limited_release_stops_emission_and_state_is_bounded() {
        let mut ripples = Ripples::default();
        for _ in 0..10 {
            ripples.advance(0.01, &[key(1.0)]);
        }
        assert_eq!(ripples.waves.len(), 1);
        ripples.advance(0.2, &[key(1.0)]);
        assert_eq!(ripples.waves.len(), 2);
        ripples.advance(3.0, &[]);
        assert!(ripples.waves.is_empty());
        let keys = (1..256)
            .map(|key_code| AnalogKeyPressure {
                key_code,
                pressure: 1.0,
            })
            .collect::<Vec<_>>();
        for _ in 0..100 {
            ripples.advance(0.2, &keys);
        }
        assert!(ripples.waves.len() <= MAX_RIPPLES);
    }
}
