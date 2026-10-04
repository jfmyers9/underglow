use crate::effects::EffectKind;
use crate::layout::KeyboardLayout;
use crate::render::{Color, Frame, PaletteName, RenderContext};
use crate::runner::SignalRunOptions;
use crate::sdk::rgb::{DeviceInfo, DeviceType, Layout};
use crate::signals::SignalProgram;
use clap::ValueEnum;
use serde_json::json;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum, Default)]
pub enum PreviewFormat {
    #[default]
    Ansi,
    Json,
    Svg,
}

pub fn print_effect_preview(
    effect: EffectKind,
    palette: PaletteName,
    brightness: u8,
    ticks: u32,
    format: PreviewFormat,
) {
    let info = preview_device();
    let layout = KeyboardLayout::for_device(&info);
    let frames = (0..ticks.max(1))
        .map(|tick| {
            effect.render(&RenderContext {
                info: &info,
                layout: &layout,
                brightness,
                palette,
                tick,
            })
        })
        .collect::<Vec<_>>();
    print_frames(&info, &layout, &frames, format);
}

pub fn print_signal_preview(
    signal: &mut dyn SignalProgram,
    options: &SignalRunOptions,
    ticks: u32,
    format: PreviewFormat,
) {
    let info = preview_device();
    let layout = KeyboardLayout::for_device(&info);
    let frames = (0..ticks.max(1))
        .map(|tick| {
            signal.preview_tick(tick);
            signal.render(&RenderContext {
                info: &info,
                layout: &layout,
                brightness: options.brightness,
                palette: options.palette,
                tick,
            })
        })
        .collect::<Vec<_>>();
    if let Err(error) = signal.shutdown(false) {
        eprintln!("warning: {error}");
    }
    print_frames(&info, &layout, &frames, format);
}

pub fn preview_device() -> DeviceInfo {
    DeviceInfo {
        connected: true,
        model: "preview-80he".to_string(),
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

pub(crate) fn print_frames(
    info: &DeviceInfo,
    layout: &KeyboardLayout,
    frames: &[Frame],
    format: PreviewFormat,
) {
    match format {
        PreviewFormat::Ansi => print_ansi(info, frames),
        PreviewFormat::Json => print_json(info, layout, frames),
        PreviewFormat::Svg => print_svg(layout, frames),
    }
}

fn print_ansi(info: &DeviceInfo, frames: &[Frame]) {
    for (tick, frame) in frames.iter().enumerate() {
        println!("tick {tick}");
        for row in 0..usize::from(info.max_rows) {
            for column in 0..usize::from(info.max_columns) {
                let color = frame.get(row, column);
                print!(
                    "\x1b[48;2;{};{};{}m  \x1b[0m",
                    color.red, color.green, color.blue
                );
            }
            println!();
        }
    }
}

fn print_json(info: &DeviceInfo, layout: &KeyboardLayout, frames: &[Frame]) {
    let frames = frames
        .iter()
        .enumerate()
        .map(|(tick, frame)| {
            let rows = (0..usize::from(info.max_rows))
                .map(|row| {
                    (0..usize::from(info.max_columns))
                        .map(|column| hex(frame.get(row, column)))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let lit_keys = frame
                .as_bytes()
                .chunks_exact(3)
                .filter(|chunk| chunk.iter().any(|channel| *channel > 0))
                .count();
            let checksum = frame
                .as_bytes()
                .iter()
                .fold(0u64, |sum, channel| sum + u64::from(*channel));

            json!({
                "tick": tick,
                "lit_keys": lit_keys,
                "checksum": checksum,
                "rows": rows,
            })
        })
        .collect::<Vec<_>>();

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "device": {
                "model": info.model,
                "rows": info.max_rows,
                "columns": info.max_columns,
            },
            "layout": layout.name,
            "frames": frames,
        }))
        .expect("preview JSON serializes")
    );
}

fn print_svg(layout: &KeyboardLayout, frames: &[Frame]) {
    let key = 18.0f32;
    let gap = 4.0f32;
    let frame_height = (layout.height + 1.0) * (key + gap) + 28.0;
    let width = (layout.width + 1.5) * (key + gap);
    let height = frame_height * frames.len() as f32;

    println!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width:.1} {height:.1}\" width=\"{width:.0}\" height=\"{height:.0}\">"
    );
    println!("<rect width=\"100%\" height=\"100%\" fill=\"#111\"/>");
    for (tick, frame) in frames.iter().enumerate() {
        let y_offset = frame_height * tick as f32 + 20.0;
        println!(
            "<text x=\"0\" y=\"{}\" fill=\"#ddd\" font-family=\"monospace\" font-size=\"12\">tick {tick}</text>",
            y_offset - 6.0
        );
        for key_position in layout.keys() {
            let color = frame.get_coord(key_position.coord);
            println!(
                "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{key}\" height=\"{key}\" rx=\"3\" fill=\"{}\"/>",
                key_position.x * (key + gap),
                y_offset + key_position.y * (key + gap),
                hex(color)
            );
        }
    }
    println!("</svg>");
}

fn hex(color: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.red, color.green, color.blue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_device_uses_keyboard_layout() {
        let info = preview_device();
        let layout = KeyboardLayout::for_device(&info);

        assert_eq!(layout.name, "wooting-80he");
        assert_eq!(info.max_rows, 6);
        assert_eq!(info.max_columns, 17);
    }

    #[test]
    fn preview_json_summary_has_lit_keys() {
        let info = preview_device();
        let layout = KeyboardLayout::for_device(&info);
        let frame = EffectKind::Comet.render(&RenderContext {
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Wooting,
            tick: 0,
        });

        assert!(frame.as_bytes().iter().any(|channel| *channel > 0));
    }
}
