use crate::render::{Color, Frame, RenderContext};

pub(super) fn render(ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    let palette = [
        Color::new(255, 0, 0),
        Color::new(255, 128, 0),
        Color::new(255, 255, 0),
        Color::new(0, 255, 0),
        Color::new(0, 128, 255),
        Color::new(128, 0, 255),
    ];

    for row in 0..usize::from(ctx.info.max_rows) {
        for column in 0..usize::from(ctx.info.max_columns) {
            frame.set(
                row,
                column,
                palette[row % palette.len()].scale(ctx.brightness),
            );
        }
    }

    frame
}

#[cfg(test)]
mod tests {
    use crate::effects::{EffectKind, test_support::output_fingerprint};

    #[test]
    fn preserves_existing_frame_bytes() {
        assert_eq!(output_fingerprint(EffectKind::RowTest), 0xd6193d861060d07d);
    }
    use super::*;
    use crate::effects::test_support::info;
    use crate::layout::KeyboardLayout;
    use crate::render::PaletteName;

    #[test]
    fn row_test_respects_device_bounds() {
        let info = info(1, 2);
        let layout = KeyboardLayout::for_device(&info);
        let frame = EffectKind::RowTest.render(&RenderContext {
            animation_seconds: 0.0,
            info: &info,
            layout: &layout,
            brightness: 10,
            palette: PaletteName::Wooting,
            tick: 0,
        });
        assert!(frame.as_bytes()[0..6].iter().any(|channel| *channel > 0));
        assert!(frame.as_bytes()[6..].iter().all(|channel| *channel == 0));
    }
}
