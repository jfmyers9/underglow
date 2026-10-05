use crate::render::{Frame, RenderContext};

pub(super) fn render(ctx: &RenderContext<'_>) -> Frame {
    super::interpolate_steps(
        ctx,
        12.0,
        ctx.layout.keys().len().max(1) as u32,
        render_step,
    )
}

fn render_step(ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    let keys = ctx.layout.keys();
    if keys.is_empty() {
        return frame;
    }

    let palette = ctx.palette.palette();
    let head = usize::try_from(ctx.tick).unwrap_or(0) % keys.len();
    let trail = 10.min(keys.len());

    for offset in 0..trail {
        let index = (head + keys.len() - offset) % keys.len();
        let fade = 255u8.saturating_sub(((offset * 255) / trail) as u8);
        let color = palette
            .gradient(fade)
            .scale(((u16::from(ctx.brightness) * u16::from(fade)) / 255) as u8);
        frame.set_coord(keys[index].coord, color);
    }

    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::test_support::info;
    use crate::effects::{EffectKind, test_support::output_fingerprint};
    use crate::layout::KeyboardLayout;
    use crate::render::{Color, PaletteName};

    #[test]
    fn head_wraps_and_trail_has_ten_keys() {
        let info = info(6, 17);
        let layout = KeyboardLayout::for_device(&info);
        let mut ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 255,
            palette: PaletteName::Ocean,
            tick: 0,
            animation_seconds: 0.0,
        };
        let start = render_step(&ctx);
        assert_eq!(
            layout
                .keys()
                .iter()
                .filter(|key| start.get_coord(key.coord) != Color::BLACK)
                .count(),
            10
        );
        assert_eq!(
            start.get_coord(layout.keys()[0].coord),
            ctx.palette.palette().gradient(255)
        );
        ctx.tick = layout.key_count() as u32;
        assert_eq!(render_step(&ctx), start);
    }

    #[test]
    fn preserves_existing_frame_bytes() {
        assert_eq!(output_fingerprint(EffectKind::Comet), 0x4ebb00a59303a3b8);
    }
}
