//! A soft band advances and retreats in a twelve-second, eased cycle.
use crate::render::{Frame, RenderContext};
use std::f64::consts::{PI, TAU};

const PERIOD_SECONDS: f64 = 12.0;
const BAND_WIDTH: f64 = 0.25;

pub(super) fn render(ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    let palette = ctx.palette.palette();
    let phase = TAU * ctx.animation_seconds.rem_euclid(PERIOD_SECONDS) / PERIOD_SECONDS;
    let center = 0.5 - 0.5 * phase.cos();
    for key in ctx.layout.keys() {
        if key.coord.row >= ctx.info.max_rows || key.coord.column >= ctx.info.max_columns {
            continue;
        }
        let x = f64::from(key.x / ctx.layout.width.max(1.0));
        let distance = ((x - center) / BAND_WIDTH).abs();
        if distance >= 1.0 {
            continue;
        }
        // A raised cosine has zero slope at the edge, avoiding hard on/off steps.
        let glow = 0.5 + 0.5 * (PI * distance).cos();
        let position = (180.0 * x + 75.0 * glow).clamp(0.0, 255.0) as u8;
        frame.set_coord(
            key.coord,
            palette
                .gradient(position)
                .scale((f64::from(ctx.brightness) * 0.8 * glow) as u8),
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
    fn band_moves_from_left_to_right_and_retreats() {
        let left = sample(0.0, 0, 255, PaletteName::Ocean);
        let middle = sample(3.0, 0, 255, PaletteName::Ocean);
        let right = sample(6.0, 0, 255, PaletteName::Ocean);
        assert_ne!(left.get(2, 0), Color::BLACK);
        assert_eq!(left.get(2, 16), Color::BLACK);
        assert_ne!(right.get(2, 16), Color::BLACK);
        assert_eq!(right.get(2, 0), Color::BLACK);
        assert_ne!(middle.get(2, 8), Color::BLACK);
        assert_eq!(middle.get(2, 0), Color::BLACK);
        assert_eq!(middle.get(2, 16), Color::BLACK);
        let retreat = sample(9.0, 0, 255, PaletteName::Ocean);
        assert!(
            middle
                .as_bytes()
                .iter()
                .zip(retreat.as_bytes())
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
        assert_eq!(left, sample(PERIOD_SECONDS, 0, 255, PaletteName::Ocean));
    }

    #[test]
    fn band_has_soft_shoulders_and_spans_rows() {
        let frame = sample(3.0, 0, 255, PaletteName::Ocean);
        let peak = |c| {
            let color = frame.get(2, c);
            color.red.max(color.green).max(color.blue)
        };
        assert!(peak(8) > peak(6));
        assert!(peak(6) > peak(5));
        assert!(peak(5) > peak(4));
        assert_eq!(frame.get(0, 8), frame.get(5, 8));
    }

    #[test]
    fn honors_palette_brightness_and_elapsed_time() {
        assert_eq!(sample(2.0, 0, 0, PaletteName::Ocean), Frame::black());
        let ocean = sample(2.0, 0, 96, PaletteName::Ocean);
        assert!(ocean.as_bytes().iter().all(|&v| v <= 96));
        assert_ne!(ocean, sample(2.0, 0, 96, PaletteName::Heat));
        assert_eq!(ocean, sample(2.0, 900, 96, PaletteName::Ocean));
    }
}
