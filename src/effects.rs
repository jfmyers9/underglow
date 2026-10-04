use crate::render::{Color, Frame, PaletteName, RenderContext};
use clap::ValueEnum;
use serde::Deserialize;
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum EffectKind {
    RowTest,
    #[default]
    Rainbow,
    Comet,
    Matrix,
    Breath,
}

impl fmt::Display for EffectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            self.to_possible_value()
                .expect("effect has value")
                .get_name()
        )
    }
}

impl EffectKind {
    pub fn render(self, ctx: &RenderContext<'_>) -> Frame {
        match self {
            Self::RowTest => row_test(ctx),
            Self::Rainbow => rainbow(ctx),
            Self::Comet => comet(ctx),
            Self::Matrix => matrix(ctx),
            Self::Breath => breath(ctx),
        }
    }
}

fn row_test(ctx: &RenderContext<'_>) -> Frame {
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

fn rainbow(ctx: &RenderContext<'_>) -> Frame {
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

fn comet(ctx: &RenderContext<'_>) -> Frame {
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

fn matrix(ctx: &RenderContext<'_>) -> Frame {
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

fn breath(ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    let palette = ctx.palette.palette();
    let phase = crate::render::pulse_wave(ctx.tick, 96);
    let brightness = ((u16::from(ctx.brightness) * u16::from(phase)) / 255) as u8;
    let color = palette.sample(ctx.tick / 96).scale(brightness);

    crate::scenes::fill(&mut frame, ctx.layout, color);

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
    use crate::layout::KeyboardLayout;
    use crate::render::{FRAME_BYTES, RenderContext};
    use crate::sdk::rgb::{DeviceInfo, DeviceType, Layout};

    fn info(rows: u8, columns: u8) -> DeviceInfo {
        DeviceInfo {
            connected: true,
            model: "test".to_string(),
            max_rows: rows,
            max_columns: columns,
            led_index_max: 0,
            device_type: DeviceType::Keyboard60,
            layout: Layout::Ansi,
            v2_interface: true,
            uses_small_packets: false,
            uses_multi_report: false,
        }
    }

    #[test]
    fn row_test_respects_device_bounds() {
        let info = info(1, 2);
        let layout = KeyboardLayout::for_device(&info);
        let frame = EffectKind::RowTest.render(&RenderContext {
            info: &info,
            layout: &layout,
            brightness: 10,
            palette: PaletteName::Wooting,
            tick: 0,
        });
        assert!(frame.as_bytes()[0..6].iter().any(|channel| *channel > 0));
        assert!(frame.as_bytes()[6..].iter().all(|channel| *channel == 0));
    }

    #[test]
    fn all_effects_render_full_frames() {
        let info = info(6, 17);
        let layout = KeyboardLayout::for_device(&info);
        for effect in [
            EffectKind::RowTest,
            EffectKind::Rainbow,
            EffectKind::Comet,
            EffectKind::Matrix,
            EffectKind::Breath,
        ] {
            let frame = effect.render(&RenderContext {
                info: &info,
                layout: &layout,
                brightness: 96,
                palette: PaletteName::Cyberpunk,
                tick: 5,
            });
            assert_eq!(frame.as_bytes().len(), FRAME_BYTES);
        }
    }
}
