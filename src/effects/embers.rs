//! Independent, deterministic points glow and fade over six to twelve seconds.
use crate::render::{Frame, RenderContext};
use std::f64::consts::TAU;

// An address hash, not a random source: replaying any elapsed time gives the same
// frame, regardless of frame rate, render order, or the previously selected mode.
fn address_hash(row: u8, column: u8) -> u32 {
    let mut value = u32::from(row) * 256 + u32::from(column) + 1;
    value = (value ^ (value >> 16)).wrapping_mul(0x7feb352d);
    value = (value ^ (value >> 15)).wrapping_mul(0x846ca68b);
    value ^ (value >> 16)
}

fn glow(seed: u32, seconds: f64) -> f64 {
    let period = 6.0 + 6.0 * f64::from(seed & 0xffff) / 65535.0;
    let offset = f64::from(seed >> 16) / 65535.0;
    let phase = seconds.rem_euclid(period) / period + offset;
    // The flat dark interval keeps the field sparse; squared shoulders make
    // ignition and extinction continuous rather than switching a key on/off.
    (((TAU * phase).cos() - 0.3) / 0.7).max(0.0).powi(2)
}

pub(super) fn render(ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    let palette = ctx.palette.palette();
    for key in ctx.layout.keys() {
        if key.coord.row >= ctx.info.max_rows || key.coord.column >= ctx.info.max_columns {
            continue;
        }
        let seed = address_hash(key.coord.row, key.coord.column);
        let glow = glow(seed, ctx.animation_seconds);
        let position = (80.0 + 175.0 * glow) as u8;
        frame.set_coord(
            key.coord,
            palette
                .gradient(position)
                .scale((f64::from(ctx.brightness) * 0.85 * glow) as u8),
        );
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::test_support::info;
    use crate::layout::KeyboardLayout;
    use crate::render::{Color, PaletteName};

    fn sample(seconds: f64, tick: u32, brightness: u8, palette: PaletteName) -> Frame {
        let info = info(6, 17);
        let layout = KeyboardLayout::for_device(&info);
        render(&RenderContext {
            info: &info,
            layout: &layout,
            brightness,
            palette,
            tick,
            animation_seconds: seconds,
        })
    }

    #[test]
    fn embers_are_sparse_and_scattered() {
        for seconds in [0.0, 2.0, 5.0, 11.0] {
            let frame = sample(seconds, 0, 255, PaletteName::Heat);
            let lit = (0..6)
                .flat_map(|r| (0..17).map(move |c| (r, c)))
                .filter(|&(r, c)| frame.get(r, c) != Color::BLACK)
                .count();
            assert!((10..65).contains(&lit), "lit={lit} at {seconds}");
            assert!(
                (0..6)
                    .filter(|&r| (0..17).any(|c| frame.get(r, c) != Color::BLACK))
                    .count()
                    >= 4
            );
        }
    }

    #[test]
    fn each_point_ignites_fades_and_rests() {
        let seed = address_hash(2, 7);
        let samples: Vec<_> = (0..1200)
            .map(|n| glow(seed, f64::from(n) / 100.0))
            .collect();
        assert!(samples.contains(&0.0));
        assert!(samples.iter().any(|&v| v > 0.99));
        assert!(samples.windows(2).any(|v| v[1] > v[0]));
        assert!(samples.windows(2).any(|v| v[1] < v[0]));
        assert!(samples.windows(2).all(|v| (v[1] - v[0]).abs() < 0.04));
    }

    #[test]
    fn replay_is_deterministic_and_independent_of_tick() {
        let frame = sample(2.0, 0, 255, PaletteName::Heat);
        assert_ne!(frame, sample(5.0, 0, 255, PaletteName::Heat));
        assert_eq!(frame, sample(2.0, 900, 255, PaletteName::Heat));
    }

    #[test]
    fn honors_palette_and_brightness() {
        assert_eq!(sample(2.0, 0, 0, PaletteName::Heat), Frame::black());
        let heat = sample(2.0, 0, 96, PaletteName::Heat);
        assert!(heat.as_bytes().iter().all(|&v| v <= 96));
        assert_ne!(heat, sample(2.0, 0, 96, PaletteName::Ocean));
    }
}
