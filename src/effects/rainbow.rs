use crate::render::{Color, Frame, RenderContext};

pub(super) fn render(ctx: &RenderContext<'_>) -> Frame {
    // 12 degrees/second at 100% speed: a 30-second cycle.
    super::interpolate_steps(ctx, 2.0, 60, render_step)
}

fn render_step(ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    let rows = usize::from(ctx.info.max_rows).max(1);
    let columns = usize::from(ctx.info.max_columns).max(1);

    for row in 0..rows {
        for column in 0..columns {
            let position = ((column * 360) / columns) as u16;
            let vertical = ((row * 90) / rows) as u16;
            let hue = (position + vertical + ((ctx.tick as u16) * 6)) % 360;
            frame.set(row, column, hsv_to_rgb(hue, 255, ctx.brightness));
        }
    }

    frame
}

fn hsv_to_rgb(hue: u16, saturation: u8, value: u8) -> Color {
    if saturation == 0 {
        return Color::new(value, value, value);
    }

    let region = hue / 60;
    let remainder = ((hue % 60) * 255 / 60) as u8;

    let p = scale_down(value, 255 - saturation);
    let q = scale_down(value, 255 - scale_down(saturation, remainder));
    let t = scale_down(value, 255 - scale_down(saturation, 255 - remainder));

    match region {
        0 => Color::new(value, t, p),
        1 => Color::new(q, value, p),
        2 => Color::new(p, value, t),
        3 => Color::new(p, q, value),
        4 => Color::new(t, p, value),
        _ => Color::new(value, p, q),
    }
}

fn scale_down(a: u8, b: u8) -> u8 {
    ((u16::from(a) * u16::from(b)) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{EffectKind, test_support::output_fingerprint};

    #[test]
    fn hue_primary_colors_and_zero_saturation_are_exact() {
        assert_eq!(hsv_to_rgb(0, 255, 96), Color::new(96, 0, 0));
        assert_eq!(hsv_to_rgb(120, 255, 96), Color::new(0, 96, 0));
        assert_eq!(hsv_to_rgb(240, 255, 96), Color::new(0, 0, 96));
        assert_eq!(hsv_to_rgb(180, 0, 96), Color::new(96, 96, 96));
    }

    #[test]
    fn preserves_existing_frame_bytes() {
        assert_eq!(output_fingerprint(EffectKind::Rainbow), 0xe557fd91494ab3a2);
    }
}
