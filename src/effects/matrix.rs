use crate::render::{Frame, PaletteName, RenderContext};

pub(super) fn render(ctx: &RenderContext<'_>) -> Frame {
    super::interpolate_steps(ctx, 7.5, u32::from(ctx.info.max_rows.max(1)), render_step)
}

fn render_step(ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    let palette = PaletteName::Terminal.palette();
    let rows = ctx.info.max_rows.max(1);

    for key in ctx.layout.keys() {
        let column_seed = u32::from(key.coord.column) * 3;
        let head = ((ctx.tick + column_seed) % u32::from(rows)) as i16;
        let distance = (head - i16::from(key.coord.row)).rem_euclid(i16::from(rows)) as u8;
        if distance <= 3 {
            let fade = 255u8.saturating_sub(distance * 70);
            frame.set_coord(key.coord, palette.gradient(fade).scale(ctx.brightness));
        }
    }

    frame
}

#[cfg(test)]
mod tests {
    use crate::effects::{EffectKind, test_support::output_fingerprint};

    #[test]
    fn preserves_existing_frame_bytes() {
        assert_eq!(output_fingerprint(EffectKind::Matrix), 0x63ad376c5346762c);
    }
    use super::*;
    use crate::effects::test_support::info;
    use crate::layout::KeyboardLayout;

    #[test]
    fn matrix_default_is_seven_and_a_half_rows_per_second_with_substep_blending() {
        let info = info(6, 17);
        let layout = KeyboardLayout::for_device(&info);
        let mut context = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 255,
            palette: PaletteName::Ocean,
            tick: 3,
            animation_seconds: 0.4,
        };
        assert_eq!(EffectKind::Matrix.render(&context), render_step(&context));
        context.tick = 0;
        let start = render_step(&context);
        context.tick = 1;
        let end = render_step(&context);
        context.animation_seconds = 0.5 / 7.5;
        let between = EffectKind::Matrix.render(&context);
        assert_ne!(between, start);
        assert_ne!(between, end);
        for ((&a, &b), &c) in start
            .as_bytes()
            .iter()
            .zip(end.as_bytes())
            .zip(between.as_bytes())
        {
            assert!((a.min(b)..=a.max(b)).contains(&c));
        }
    }
}
