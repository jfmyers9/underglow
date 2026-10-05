use crate::render::{Frame, RenderContext};
use std::f64::consts::{PI, TAU};

// One sweep per six speed-adjusted seconds; the angular trail fades over
// roughly a quarter turn, with a soft leading edge to avoid a hard flash.
const PERIOD_SECONDS: f64 = 6.0;

fn sweep_light(x: f64, y: f64, phase: f64) -> f64 {
    let lag = (phase - y.atan2(x)).rem_euclid(TAU);
    let angle = lag.min(TAU - lag);
    let head = (-(angle / 0.16).powi(2)).exp();
    let trail = (-lag / 0.75).exp() * (1.0 - lag / PI).max(0.0).powi(2);
    head.max(trail)
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
        let light = sweep_light(x, y, phase);
        frame.set_coord(
            key.coord,
            palette
                .gradient((light * 255.0).round() as u8)
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
    use crate::render::{Color, PaletteName};

    #[test]
    fn sweep_lights_a_ray_and_trail_fades_behind_it() {
        assert_eq!(sweep_light(0.2, 0.0, 0.0), 1.0);
        assert_eq!(sweep_light(1.0, 0.0, 0.0), 1.0);
        let behind = |angle: f64| sweep_light(angle.cos(), -angle.sin(), 0.0);
        assert!(behind(0.2) > behind(0.6));
        assert!(behind(0.6) > behind(1.2));
        assert!(sweep_light(0.0, 1.0, 0.0) < 0.001);
        assert_eq!(sweep_light(0.0, 1.0, TAU / 4.0), 1.0);
    }

    #[test]
    fn sweep_is_continuous_across_angular_wrap() {
        let before = sweep_light(1.0, 0.0, TAU - 0.0001);
        let after = sweep_light(1.0, 0.0, 0.0001);
        assert!((before - after).abs() < 0.001);
    }

    #[test]
    fn rendered_sweep_reaches_from_center_to_edge_and_clips_bounds() {
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
        let head = PaletteName::Ocean.palette().gradient(255);
        assert_eq!(frame.get(2, 2), head);
        assert_eq!(frame.get(2, 4), head);
        assert_eq!(frame.get(2, 0), Color::BLACK);
        assert_eq!(frame.get(4, 2), Color::BLACK);
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
    fn single_key_receives_the_sweep() {
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
