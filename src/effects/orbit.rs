use crate::render::{Frame, RenderContext};
use std::f64::consts::TAU;

// An eight-second revolution at the baseline speed. The annulus, rather than
// a ray from the center, keeps Orbit visually distinct from Radar.
const PERIOD_SECONDS: f64 = 8.0;

fn arc_light(x: f64, y: f64, phase: f64) -> f64 {
    let radius = x.hypot(y);
    let ring = (-((radius - 0.78) / 0.25).powi(2)).exp();
    let arc = ((1.0 + (y.atan2(x) - phase).cos()) * 0.5).powi(5);
    ring * arc
}

pub(super) fn render(ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    let phase = ctx.animation_seconds.rem_euclid(PERIOD_SECONDS) / PERIOD_SECONDS * TAU;
    let palette = ctx.palette.palette();
    for key in ctx.layout.keys() {
        if key.coord.row >= ctx.info.max_rows || key.coord.column >= ctx.info.max_columns {
            continue;
        }
        let x = if ctx.info.max_columns <= 1 {
            0.0
        } else {
            2.0 * f64::from(key.x) / f64::from(ctx.layout.width.max(1.0)) - 1.0
        };
        let y = if ctx.info.max_rows <= 1 {
            0.0
        } else {
            2.0 * f64::from(key.y) / f64::from(ctx.layout.height.max(1.0)) - 1.0
        };
        // A single LED cannot depict an arc; retain its rotating light pulse.
        let light = if ctx.layout.key_count() == 1 {
            0.35 + 0.65 * (phase.cos() + 1.0) * 0.5
        } else {
            arc_light(x, y, phase)
        };
        let color = palette
            .gradient((light * 255.0).round() as u8)
            .scale((light * f64::from(ctx.brightness)).round() as u8);
        frame.set_coord(key.coord, color);
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::test_support::info;
    use crate::layout::KeyboardLayout;
    use crate::render::{Color, PaletteName};

    #[test]
    fn arc_is_hollow_localized_and_rotates() {
        assert!(arc_light(0.78, 0.0, 0.0) > 0.99);
        assert!(arc_light(0.0, 0.0, 0.0) < 0.001);
        assert!(arc_light(-0.78, 0.0, 0.0) < 0.001);
        assert!(arc_light(0.0, 0.78, TAU / 4.0) > 0.99);
    }

    #[test]
    fn rendered_arc_leaves_center_dark_and_clips_device_bounds() {
        let info = info(5, 5);
        let layout = KeyboardLayout::for_device(&info);
        let render_info = |info| {
            render(&RenderContext {
                info,
                layout: &layout,
                brightness: 255,
                palette: PaletteName::Ocean,
                tick: 0,
                animation_seconds: 0.0,
            })
        };
        let frame = render_info(&info);
        assert_eq!(frame.get(2, 2), Color::BLACK);
        assert_eq!(frame.get(2, 0), Color::BLACK);
        assert_ne!(frame.get(2, 4), Color::BLACK);
        let mut clipped_info = info.clone();
        clipped_info.max_columns = 4;
        assert_eq!(render_info(&clipped_info).get(2, 4), Color::BLACK);
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
    fn single_key_still_pulses() {
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
        ctx.animation_seconds = PERIOD_SECONDS / 2.0;
        assert_ne!(render(&ctx), initial);
    }
}
