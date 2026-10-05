use crate::layout::{KeyboardLayout, MatrixCoord, Zone};
use crate::render::{Color, Frame, RenderContext, pulse_wave};

pub fn fill(frame: &mut Frame, layout: &KeyboardLayout, color: Color) {
    for key in layout.keys() {
        frame.set_coord(key.coord, color);
    }
}

pub fn fill_zones(frame: &mut Frame, layout: &KeyboardLayout, zones: &[Zone], color: Color) {
    for key in layout.keys() {
        if zones.is_empty() || zones.contains(&key.zone) {
            frame.set_coord(key.coord, color);
        }
    }
}

pub fn pulse_fill(
    frame: &mut Frame,
    layout: &KeyboardLayout,
    zones: &[Zone],
    color: Color,
    brightness: u8,
    tick: u32,
    period: u32,
) {
    let wave = pulse_wave(tick, period);
    let scale = 96 + (wave / 2);
    let color = color.scale(((u16::from(brightness) * u16::from(scale)) / 255) as u8);
    fill_zones(frame, layout, zones, color);
}

pub fn progress_bar(
    frame: &mut Frame,
    layout: &KeyboardLayout,
    zone: Option<Zone>,
    progress: f32,
    color: Color,
) {
    let mut keys = layout
        .keys()
        .iter()
        .filter(|key| zone.is_none_or(|zone| key.zone == zone))
        .collect::<Vec<_>>();
    if keys.is_empty() {
        keys = layout.keys().iter().collect();
    }
    keys.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));

    let active = ((keys.len() as f32) * progress.clamp(0.0, 1.0))
        .ceil()
        .clamp(0.0, keys.len() as f32) as usize;
    for key in keys.into_iter().take(active) {
        frame.set_coord(key.coord, color);
    }
}

#[allow(dead_code)]
pub fn sweep_trail(
    frame: &mut Frame,
    layout: &KeyboardLayout,
    zones: &[Zone],
    tick: u32,
    trail: usize,
    color: Color,
    brightness: u8,
) {
    let mut keys = layout
        .keys()
        .iter()
        .filter(|key| zones.is_empty() || zones.contains(&key.zone))
        .collect::<Vec<_>>();
    if keys.is_empty() {
        keys = layout.keys().iter().collect();
    }
    keys.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    if keys.is_empty() {
        return;
    }

    let head = usize::try_from(tick).unwrap_or(0) % keys.len();
    let trail = trail.max(1).min(keys.len());
    for offset in 0..trail {
        let index = (head + keys.len() - offset) % keys.len();
        let fade = 255u8.saturating_sub(((offset * 255) / trail) as u8);
        frame.set_coord(keys[index].coord, color.scale(fade).scale(brightness));
    }
}

#[allow(dead_code)]
pub fn split(
    frame: &mut Frame,
    layout: &KeyboardLayout,
    zones: &[Zone],
    left: Color,
    right: Color,
    brightness: u8,
) {
    let midpoint = layout.width / 2.0;
    for key in layout.keys() {
        if zones.is_empty() || zones.contains(&key.zone) {
            let color = if key.x <= midpoint { left } else { right };
            frame.set_coord(key.coord, color.scale(brightness));
        }
    }
}

#[allow(dead_code)]
pub fn heatline(
    frame: &mut Frame,
    layout: &KeyboardLayout,
    zone: Option<Zone>,
    values: &[f32],
    low: Color,
    high: Color,
    brightness: u8,
) {
    if values.is_empty() {
        return;
    }

    let mut keys = layout
        .keys()
        .iter()
        .filter(|key| zone.is_none_or(|zone| key.zone == zone))
        .collect::<Vec<_>>();
    if keys.is_empty() {
        keys = layout.keys().iter().collect();
    }
    keys.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));

    let last_value = values.len().saturating_sub(1).max(1);
    let last_key = keys.len().saturating_sub(1).max(1);
    for (index, key) in keys.into_iter().enumerate() {
        let value_index = (index * last_value) / last_key;
        let amount = (values[value_index].clamp(0.0, 1.0) * 255.0) as u8;
        frame.set_coord(key.coord, blend(low, high, amount).scale(brightness));
    }
}

#[allow(dead_code)]
pub fn overlay_non_black(dst: &mut Frame, src: &Frame, layout: &KeyboardLayout, zones: &[Zone]) {
    for key in layout.keys() {
        if zones.is_empty() || zones.contains(&key.zone) {
            let color = src.get_coord(key.coord);
            if color != Color::BLACK {
                dst.set_coord(key.coord, color);
            }
        }
    }
}

pub fn status_color(status: &str, tick: u32) -> Color {
    match status {
        "success" | "passing" | "approved" | "positive" | "break" => Color::new(0, 220, 80),
        "failure" | "failing" | "error" | "conflict" | "negative" | "overtime" => {
            Color::new(255, 32, 24).scale(160 + (pulse_wave(tick, 24) / 3))
        }
        "timeout" | "alert" | "review-requested" => Color::new(255, 180, 0),
        "interrupted" | "paused" => Color::new(160, 80, 255),
        "focus" | "running" => Color::new(0, 180, 255),
        "meeting" | "meeting-safe" => Color::new(20, 30, 40),
        "terminal" => Color::new(0, 220, 64),
        "recording" => Color::new(255, 32, 24).scale(128 + pulse_wave(tick, 48) / 2),
        "stale" => Color::new(120, 120, 120),
        _ => Color::new(0, 90, 180),
    }
}

pub fn render_status_wash(ctx: &RenderContext<'_>, status: &str, zones: &[Zone]) -> Frame {
    let mut frame = Frame::black();
    let base = status_color(status, ctx.tick);
    fill(&mut frame, ctx.layout, base.scale(ctx.brightness / 10));
    pulse_fill(
        &mut frame,
        ctx.layout,
        zones,
        base,
        ctx.brightness,
        ctx.tick,
        36,
    );
    frame
}

#[allow(dead_code)]
fn blend(left: Color, right: Color, amount: u8) -> Color {
    let blend_channel = |a: u8, b: u8| {
        let a = u16::from(a) * u16::from(255 - amount);
        let b = u16::from(b) * u16::from(amount);
        ((a + b) / 255) as u8
    };
    Color::new(
        blend_channel(left.red, right.red),
        blend_channel(left.green, right.green),
        blend_channel(left.blue, right.blue),
    )
}

#[allow(dead_code)]
pub fn coord(row: u8, column: u8) -> MatrixCoord {
    MatrixCoord { row, column }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceInfo, DeviceType, Layout};
    use crate::render::RenderContext;

    fn info() -> DeviceInfo {
        DeviceInfo {
            connected: true,
            model: "test".to_string(),
            max_rows: 6,
            max_columns: 17,
            led_index_max: 0,
            device_type: DeviceType::Keyboard80,
            layout: Layout::Ansi,
            v2_interface: true,
            uses_small_packets: false,
            uses_multi_report: false,
        }
    }

    #[test]
    fn progress_bar_respects_zone() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let mut frame = Frame::black();

        progress_bar(
            &mut frame,
            &layout,
            Some(Zone::Function),
            0.5,
            Color::new(255, 0, 0),
        );

        assert!(
            layout
                .keys()
                .iter()
                .filter(|key| key.zone == Zone::Function)
                .any(|key| frame.get_coord(key.coord) != Color::BLACK)
        );
        assert!(
            layout
                .keys()
                .iter()
                .filter(|key| key.zone == Zone::Alpha)
                .all(|key| frame.get_coord(key.coord) == Color::BLACK)
        );
    }

    #[test]
    fn sweep_trail_is_deterministic_for_tick() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let mut left = Frame::black();
        let mut right = Frame::black();

        sweep_trail(
            &mut left,
            &layout,
            &[Zone::Function],
            3,
            4,
            Color::new(0, 180, 255),
            96,
        );
        sweep_trail(
            &mut right,
            &layout,
            &[Zone::Function],
            3,
            4,
            Color::new(0, 180, 255),
            96,
        );

        assert_eq!(left, right);
        assert!(left.as_bytes().iter().any(|channel| *channel > 0));
    }

    #[test]
    fn status_wash_renders_full_frame() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let frame = render_status_wash(
            &RenderContext {
                animation_seconds: 0.0,
                info: &info,
                layout: &layout,
                brightness: 96,
                palette: crate::render::PaletteName::Wooting,
                tick: 1,
            },
            "failure",
            &[Zone::Function],
        );

        assert_eq!(frame.as_bytes().len(), crate::render::FRAME_BYTES);
    }
}
