//! Broad, gently folding curtains. One drift takes 24 speed-adjusted seconds.
use crate::render::{Frame, RenderContext};
use std::f64::consts::TAU;

const PERIOD_SECONDS: f64 = 24.0;

pub(super) fn render(ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    let palette = ctx.palette.palette();
    let phase = TAU * ctx.animation_seconds.rem_euclid(PERIOD_SECONDS) / PERIOD_SECONDS;
    for key in ctx.layout.keys() {
        if key.coord.row >= ctx.info.max_rows || key.coord.column >= ctx.info.max_columns {
            continue;
        }
        let x = f64::from(key.x / ctx.layout.width.max(1.0));
        let y = f64::from(key.y / ctx.layout.height.max(1.0));
        // Slow vertical folds bend broad curtains without producing sharp stripes.
        let fold = 0.6 * (y * 3.0 + phase).sin();
        let curtain = (0.5 + 0.5 * (x * TAU * 1.2 + phase + fold).cos()).powi(2);
        let glow = 0.06 + 0.74 * curtain * (0.8 + 0.2 * (y * 2.0 - phase).cos());
        let position = (127.5 + 127.5 * (x * 2.5 + y * 0.7 - phase).sin()) as u8;
        frame.set_coord(
            key.coord,
            palette
                .gradient(position)
                .scale((f64::from(ctx.brightness) * glow) as u8),
        );
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::test_support::info;
    use crate::layout::KeyboardLayout;
    use crate::render::PaletteName;

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
    fn curtains_are_broad_and_fold_across_rows() {
        let frame = sample(0.0, 0, 255, PaletteName::Ocean);
        let peaks: Vec<_> = (0..17)
            .map(|column| {
                let c = frame.get(0, column);
                c.red.max(c.green).max(c.blue)
            })
            .collect();
        assert!(peaks.iter().filter(|&&v| v > 90).count() >= 4);
        assert!(peaks.iter().any(|&v| v < 30));
        assert!(peaks.windows(2).all(|pair| pair[0].abs_diff(pair[1]) < 80));
        assert_ne!(frame.get(0, 4), frame.get(5, 4));
    }

    #[test]
    fn drift_is_elapsed_time_based_and_smooth() {
        let frame = sample(2.0, 0, 255, PaletteName::Ocean);
        assert_eq!(frame, sample(2.0, 900, 255, PaletteName::Ocean));
        assert_ne!(frame, sample(6.0, 0, 255, PaletteName::Ocean));
        let next = sample(2.0 + 1.0 / 120.0, 1, 255, PaletteName::Ocean);
        assert!(
            frame
                .as_bytes()
                .iter()
                .zip(next.as_bytes())
                .all(|(a, b)| a.abs_diff(*b) <= 4)
        );
    }

    #[test]
    fn honors_palette_and_brightness() {
        assert_eq!(sample(2.0, 0, 0, PaletteName::Ocean), Frame::black());
        let ocean = sample(2.0, 0, 96, PaletteName::Ocean);
        assert!(ocean.as_bytes().iter().all(|&v| v <= 96));
        assert_ne!(ocean, sample(2.0, 0, 96, PaletteName::Heat));
    }
}
