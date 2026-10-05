//! Bounded, hardware-independent reactive lighting. Only current pressure and
//! fading per-key intensities are retained; no text, event log, or persistence.
mod afterimage;
mod constellation;
mod heatmap;

use crate::layout::MatrixCoord;
use crate::render::{Frame, RenderContext};
use clap::ValueEnum;
use serde::Deserialize;
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ReactiveKind {
    Constellation,
    Heatmap,
    Afterimage,
}

impl fmt::Display for ReactiveKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            self.to_possible_value()
                .expect("reactive kind has value")
                .get_name(),
        )
    }
}

const COLUMNS: usize = 17;
const KEY_COUNT: usize = 6 * COLUMNS;

#[derive(Clone, Copy, Debug, Default)]
struct KeyState {
    down: bool,
    pressure: f32,
    energy: f32,
}

/// Fixed-size state indexed by the shared 80HE matrix, not by input event count.
#[derive(Clone, Debug)]
pub struct ReactiveSimulation {
    kind: ReactiveKind,
    keys: [KeyState; KEY_COUNT],
}

impl ReactiveSimulation {
    pub fn new(kind: ReactiveKind) -> Self {
        Self {
            kind,
            keys: [KeyState::default(); KEY_COUNT],
        }
    }

    /// `keys` is a complete pressure snapshot; absent keys are released.
    /// Edges occur at 5% travel, with a 2% release threshold to reject chatter.
    /// Duplicate entries use maximum pressure, independently of input order.
    /// Invalid pressure samples are ignored; invalid/negative dt is zero.
    pub fn advance(&mut self, dt: f32, keys: impl IntoIterator<Item = (u16, f32)>) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let mut pressures = [0.0_f32; KEY_COUNT];
        for (code, pressure) in keys {
            if !pressure.is_finite() {
                continue;
            }
            let Some(coord) = crate::ripple::hid_coord(code) else {
                continue;
            };
            let index = usize::from(coord.row) * COLUMNS + usize::from(coord.column);
            pressures[index] = pressures[index].max(pressure.clamp(0.0, 1.0));
        }
        for (key, pressure) in self.keys.iter_mut().zip(pressures) {
            let down = pressure >= if key.down { 0.02 } else { 0.05 };
            let pressed = down && !key.down;
            key.pressure = if down { pressure } else { 0.0 };
            match self.kind {
                ReactiveKind::Constellation => constellation::advance(key, dt, pressed),
                ReactiveKind::Heatmap => heatmap::advance(key, dt, pressed),
                ReactiveKind::Afterimage => afterimage::advance(key, dt, pressed),
            }
            key.down = down;
        }
    }

    /// Rendering is pure and intentionally ignores decorative speed/time.
    pub fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        match self.kind {
            ReactiveKind::Constellation => constellation::render(&self.keys, ctx),
            ReactiveKind::Heatmap => heatmap::render(&self.keys, ctx),
            ReactiveKind::Afterimage => afterimage::render(&self.keys, ctx),
        }
    }

    pub fn clear(&mut self) {
        self.keys = [KeyState::default(); KEY_COUNT];
    }
}

fn index(coord: MatrixCoord) -> Option<usize> {
    (coord.row < 6 && coord.column < COLUMNS as u8)
        .then_some(usize::from(coord.row) * COLUMNS + usize::from(coord.column))
}

fn visible(ctx: &RenderContext<'_>, coord: MatrixCoord) -> bool {
    coord.row < ctx.info.max_rows && coord.column < ctx.info.max_columns
}

fn paint(ctx: &RenderContext<'_>, intensity: f32, position: f32) -> crate::render::Color {
    ctx.palette
        .palette()
        .gradient((position.clamp(0.0, 1.0) * 255.0) as u8)
        .scale((intensity.clamp(0.0, 1.0) * f32::from(ctx.brightness)) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceInfo;
    use crate::layout::KeyboardLayout;
    use crate::render::{FRAME_BYTES, PaletteName};

    const KINDS: [ReactiveKind; 3] = [
        ReactiveKind::Constellation,
        ReactiveKind::Heatmap,
        ReactiveKind::Afterimage,
    ];

    pub(super) fn frame(sim: &ReactiveSimulation, brightness: u8) -> Frame {
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        sim.render(&RenderContext {
            info: &info,
            layout: &layout,
            brightness,
            palette: PaletteName::Heat,
            tick: 0,
            animation_seconds: 0.0,
        })
    }

    #[test]
    fn subdivisions_preserve_decay_and_held_input() {
        for kind in KINDS {
            for held in [false, true] {
                let mut whole = ReactiveSimulation::new(kind);
                whole.advance(0.0, [(0x04, 0.8), (0x0d, 0.7)]);
                let mut split = whole.clone();
                let input = if held {
                    vec![(0x04, 0.8), (0x0d, 0.7)]
                } else {
                    vec![]
                };
                whole.advance(1.0, input.clone());
                for _ in 0..100 {
                    split.advance(0.01, input.clone());
                }
                for (a, b) in whole.keys.iter().zip(split.keys) {
                    assert!((a.energy - b.energy).abs() < 1e-5);
                }
                for (a, b) in frame(&whole, 255)
                    .as_bytes()
                    .iter()
                    .zip(frame(&split, 255).as_bytes())
                {
                    assert!(a.abs_diff(*b) <= 1);
                }
            }
        }
    }

    #[test]
    fn input_is_mapped_sanitized_and_order_independent() {
        for kind in KINDS {
            let mut a = ReactiveSimulation::new(kind);
            a.advance(
                f32::NAN,
                [
                    (0x04, f32::NAN),
                    (0x104, 1.0),
                    (0xffff, 1.0),
                    (0, 1.0),
                    (0x09, -1.0),
                    (0x0d, f32::INFINITY),
                ],
            );
            assert_eq!(frame(&a, 255), Frame::black());
            a.advance(-1.0, [(0x04, 2.0), (0x04, 0.3)]);
            let mut b = ReactiveSimulation::new(kind);
            b.advance(0.0, [(0x04, 0.3), (0x04, 1.0)]);
            assert_eq!(frame(&a, 255), frame(&b, 255));
            a.advance(f32::MAX, []);
            assert_eq!(frame(&a, 255), Frame::black());
        }
    }

    #[test]
    fn brightness_determinism_bounds_and_clear() {
        for kind in KINDS {
            let mut sim = ReactiveSimulation::new(kind);
            sim.advance(0.0, [(0x04, 1.0), (0x0d, 1.0)]);
            assert_ne!(frame(&sim, 255), Frame::black());
            assert_eq!(frame(&sim, 0), Frame::black());
            assert_eq!(frame(&sim, 77), frame(&sim, 77));
            assert_eq!(frame(&sim, 77).as_bytes().len(), FRAME_BYTES);
            assert!(frame(&sim, 77).as_bytes().iter().all(|&value| value <= 77));
            let mut info = DeviceInfo::synthetic_80he();
            info.max_rows = 3;
            info.max_columns = 4;
            let layout = KeyboardLayout::for_device(&DeviceInfo::synthetic_80he());
            let result = sim.render(&RenderContext {
                info: &info,
                layout: &layout,
                brightness: 255,
                palette: PaletteName::Ocean,
                tick: u32::MAX,
                animation_seconds: f64::NAN,
            });
            for row in 0..6 {
                for col in 0..21 {
                    if row >= 3 || col >= 4 {
                        assert_eq!(result.get(row, col), crate::render::Color::BLACK);
                    }
                }
            }
            sim.clear();
            assert_eq!(frame(&sim, 255), Frame::black());
            sim.advance(0.0, [(0x04, 1.0)]);
            assert_ne!(frame(&sim, 255), Frame::black());
        }
    }

    #[test]
    fn memory_and_values_remain_bounded_for_untrusted_input() {
        for kind in KINDS {
            let mut sim = ReactiveSimulation::new(kind);
            let size = std::mem::size_of_val(&sim);
            for _ in 0..30 {
                sim.advance(0.0, (0..=u16::MAX).map(|code| (code, 1.0)));
                sim.advance(0.01, []);
            }
            assert_eq!(std::mem::size_of_val(&sim), size);
            assert!(size < 2048);
            assert_eq!(sim.keys.len(), KEY_COUNT);
            assert!(
                sim.keys
                    .iter()
                    .all(|key| key.energy.is_finite() && (0.0..=1.0).contains(&key.energy))
            );
        }
    }
}
