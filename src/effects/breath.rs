use crate::render::{Frame, RenderContext};

pub(super) fn render(ctx: &RenderContext<'_>) -> Frame {
    super::interpolate_steps(
        ctx,
        16.0,
        96 * ctx.palette.palette().color_count() as u32,
        render_step,
    )
}

fn render_step(ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    let palette = ctx.palette.palette();
    let phase = crate::render::pulse_wave(ctx.tick, 96);
    let brightness = ((u16::from(ctx.brightness) * u16::from(phase)) / 255) as u8;
    let color = palette.sample(ctx.tick / 96).scale(brightness);

    crate::scenes::fill(&mut frame, ctx.layout, color);

    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::test_support::info;
    use crate::effects::{EffectKind, test_support::output_fingerprint};
    use crate::layout::KeyboardLayout;
    use crate::render::PaletteName;

    #[test]
    fn pulse_starts_black_peaks_and_advances_palette_after_one_cycle() {
        let info = info(6, 17);
        let layout = KeyboardLayout::for_device(&info);
        let mut ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Ocean,
            tick: 0,
            animation_seconds: 0.0,
        };
        assert_eq!(render_step(&ctx), Frame::black());
        for (tick, palette_index) in [(48, 0), (144, 1)] {
            ctx.tick = tick;
            let frame = render_step(&ctx);
            let expected = ctx
                .palette
                .palette()
                .sample(palette_index)
                .scale(ctx.brightness);
            assert!(
                layout
                    .keys()
                    .iter()
                    .all(|key| frame.get_coord(key.coord) == expected)
            );
        }
    }

    #[test]
    fn preserves_existing_frame_bytes() {
        assert_eq!(output_fingerprint(EffectKind::Breath), 0x6043abdbf00b5467);
    }
}
