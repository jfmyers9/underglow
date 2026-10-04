use crate::layout::{KeyboardLayout, MatrixCoord};
use crate::sdk::rgb::DeviceInfo;
use clap::ValueEnum;
use serde::Deserialize;
use std::fmt;

pub const MAX_ROWS: usize = 6;
pub const MAX_COLUMNS: usize = 21;
pub const RGB_CHANNELS: usize = 3;
pub const FRAME_BYTES: usize = MAX_ROWS * MAX_COLUMNS * RGB_CHANNELS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Color {
    pub const BLACK: Self = Self::new(0, 0, 0);

    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    pub fn scale(self, max_channel: u8) -> Self {
        let scale = |value: u8| ((u16::from(value) * u16::from(max_channel)) / 255) as u8;
        Self::new(scale(self.red), scale(self.green), scale(self.blue))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    bytes: [u8; FRAME_BYTES],
}

impl Frame {
    pub fn black() -> Self {
        Self {
            bytes: [0; FRAME_BYTES],
        }
    }

    pub fn set(&mut self, row: usize, column: usize, color: Color) {
        if row >= MAX_ROWS || column >= MAX_COLUMNS {
            return;
        }

        let offset = ((row * MAX_COLUMNS) + column) * RGB_CHANNELS;
        self.bytes[offset] = color.red;
        self.bytes[offset + 1] = color.green;
        self.bytes[offset + 2] = color.blue;
    }

    pub fn set_coord(&mut self, coord: MatrixCoord, color: Color) {
        self.set(usize::from(coord.row), usize::from(coord.column), color);
    }

    pub fn get(&self, row: usize, column: usize) -> Color {
        if row >= MAX_ROWS || column >= MAX_COLUMNS {
            return Color::BLACK;
        }

        let offset = ((row * MAX_COLUMNS) + column) * RGB_CHANNELS;
        Color::new(
            self.bytes[offset],
            self.bytes[offset + 1],
            self.bytes[offset + 2],
        )
    }

    pub fn get_coord(&self, coord: MatrixCoord) -> Color {
        self.get(usize::from(coord.row), usize::from(coord.column))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum PaletteName {
    #[default]
    Wooting,
    Cyberpunk,
    Ocean,
    Heat,
    Terminal,
}

impl fmt::Display for PaletteName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            self.to_possible_value()
                .expect("palette has value")
                .get_name()
        )
    }
}

#[derive(Clone, Debug)]
pub struct Palette {
    colors: &'static [[u8; 3]],
}

impl PaletteName {
    pub fn palette(self) -> Palette {
        let name = match self {
            Self::Wooting => "wooting",
            Self::Cyberpunk => "cyberpunk",
            Self::Ocean => "ocean",
            Self::Heat => "heat",
            Self::Terminal => "terminal",
        };
        Palette {
            colors: wooting_signals::ripple::palette_colors(name),
        }
    }
}

impl Palette {
    pub fn sample(&self, tick: u32) -> Color {
        let [r, g, b] = self.colors[usize::try_from(tick).unwrap_or(0) % self.colors.len()];
        Color::new(r, g, b)
    }

    pub fn gradient(&self, position: u8) -> Color {
        let [r, g, b] = wooting_signals::ripple::color_gradient(self.colors, position);
        Color::new(r, g, b)
    }
}

#[derive(Clone, Debug)]
pub struct RenderContext<'a> {
    pub info: &'a DeviceInfo,
    pub layout: &'a KeyboardLayout,
    pub brightness: u8,
    pub palette: PaletteName,
    pub tick: u32,
}

pub fn pulse_wave(tick: u32, period: u32) -> u8 {
    let period = period.max(2);
    let phase = tick % period;
    let half = period / 2;
    if phase < half {
        ((phase * 255) / half) as u8
    } else {
        (((period - phase) * 255) / half) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_uses_official_full_matrix_size() {
        assert_eq!(Frame::black().as_bytes().len(), 6 * 21 * 3);
    }

    #[test]
    fn frame_ignores_out_of_bounds_writes() {
        let mut frame = Frame::black();

        frame.set(MAX_ROWS, 0, Color::new(255, 0, 0));
        frame.set(0, MAX_COLUMNS, Color::new(0, 255, 0));

        assert!(frame.as_bytes().iter().all(|channel| *channel == 0));
        assert_eq!(frame.get(MAX_ROWS, 0), Color::BLACK);
    }

    #[test]
    fn palette_gradient_samples_endpoints() {
        let palette = PaletteName::Heat.palette();
        assert_eq!(palette.gradient(0), Color::new(80, 0, 0));
        assert_eq!(palette.gradient(255), Color::new(255, 220, 64));
    }
}
