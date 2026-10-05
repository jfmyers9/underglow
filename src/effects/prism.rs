use crate::render::{Color, Frame, RenderContext};
use std::f64::consts::TAU;

// Opposing diagonal families move at one and two cycles per twelve seconds.
const PERIOD_SECONDS: f64 = 12.0;

fn bands(x: f64, y: f64, phase: f64) -> (f64, f64) {
    let a = ((TAU * (x + y) - phase).cos() + 1.0) * 0.5;
    let b = ((TAU * (x - y) + 2.0 * phase).cos() + 1.0) * 0.5;
    (a.powi(3), b.powi(3))
}

pub(super) fn render(ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    let phase = ctx.animation_seconds.rem_euclid(PERIOD_SECONDS) / PERIOD_SECONDS * TAU;
    let palette = ctx.palette.palette();
    for key in ctx.layout.keys() {
        if key.coord.row >= ctx.info.max_rows || key.coord.column >= ctx.info.max_columns {
            continue;
        }
        // Equal physical key distances on both axes produce diagonal bands,
        // including the staggered positions of the canonical 80HE geometry.
        let (a, b) = bands(f64::from(key.x) / 5.0, f64::from(key.y) / 5.0, phase);
        let first = palette.gradient((a * 255.0).round() as u8);
        let second = palette.gradient(((1.0 - b) * 255.0).round() as u8);
        let weight = (a + 0.05) / (a + b + 0.1);
        let mix =
            |a: u8, b: u8| (f64::from(a) * weight + f64::from(b) * (1.0 - weight)).round() as u8;
        let light = 0.12 + 0.88 * (a + b) * 0.5;
        frame.set_coord(
            key.coord,
            Color::new(
                mix(first.red, second.red),
                mix(first.green, second.green),
                mix(first.blue, second.blue),
            )
            .scale((light * f64::from(ctx.brightness)).round() as u8),
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

    #[test]
    fn diagonal_families_intersect_and_move_independently() {
        let crossing = bands(0.0, 0.0, 0.0);
        assert_eq!(crossing, (1.0, 1.0));
        let rising_only = bands(0.25, -0.25, 0.0);
        assert_eq!(rising_only, (1.0, 0.0));
        let falling_only = bands(0.25, 0.25, 0.0);
        assert_eq!(falling_only, (0.0, 1.0));
        let moving = bands(0.0, 0.0, TAU / 4.0);
        assert!(moving.0 > moving.1);
        assert!(crossing.0 + crossing.1 > rising_only.0 + rising_only.1);
    }

    #[test]
    fn motion_uses_seconds_and_loops_with_palette_and_zero_support() {
        let info = info(5, 15);
        let layout = KeyboardLayout::for_device(&info);
        let mut ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 255,
            palette: PaletteName::Ocean,
            tick: 0,
            animation_seconds: 0.0,
        };
        let initial = render(&ctx);
        ctx.tick = 999;
        assert_eq!(render(&ctx), initial);
        ctx.animation_seconds = PERIOD_SECONDS;
        assert_eq!(render(&ctx), initial);
        ctx.animation_seconds = PERIOD_SECONDS / 4.0;
        assert_ne!(render(&ctx), initial);
        ctx.animation_seconds = 0.0;
        ctx.palette = PaletteName::Heat;
        assert_ne!(render(&ctx), initial);
        ctx.brightness = 0;
        assert_eq!(render(&ctx), Frame::black());
    }

    #[test]
    fn single_key_shifts_color_and_light() {
        let info = info(1, 1);
        let layout = KeyboardLayout::for_device(&info);
        let mut ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 255,
            palette: PaletteName::Ocean,
            tick: 0,
            animation_seconds: 0.0,
        };
        let initial = render(&ctx);
        assert_ne!(initial.get(0, 0), Color::BLACK);
        ctx.animation_seconds = PERIOD_SECONDS / 4.0;
        assert_ne!(render(&ctx), initial);
    }
}
