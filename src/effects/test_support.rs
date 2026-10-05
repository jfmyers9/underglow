//! Shared fixtures and byte-level compatibility checks for effect-local tests.
use super::EffectKind;
use crate::device::{DeviceInfo, DeviceType, Layout};
use crate::layout::KeyboardLayout;
use crate::render::{PaletteName, RenderContext};
use clap::ValueEnum;

pub(super) fn info(rows: u8, columns: u8) -> DeviceInfo {
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

pub(super) fn output_fingerprint(effect: EffectKind) -> u64 {
    // FNV-1a over 225 representative complete frames, including interpolation.
    // Mode-local expected values were captured before splitting the renderer:
    // these deliberately lock down existing bytes, not a second implementation.
    let mut hash = 0xcbf29ce484222325u64;
    for (device_type, rows, columns) in [
        (DeviceType::Keyboard80, 6, 17),
        (DeviceType::Keyboard60, 5, 15),
        (DeviceType::Keypad3Key, 1, 3),
    ] {
        let info = DeviceInfo {
            connected: true,
            model: "fixture".into(),
            max_rows: rows,
            max_columns: columns,
            led_index_max: 0,
            device_type,
            layout: Layout::Ansi,
            v2_interface: true,
            uses_small_packets: false,
            uses_multi_report: false,
        };
        let layout = KeyboardLayout::for_device(&info);
        for &palette in PaletteName::value_variants() {
            for brightness in [0, 96, 255] {
                for animation_seconds in [0.0, 0.125, 1.25, 7.875, 55.5] {
                    let frame = effect.render(&RenderContext {
                        info: &info,
                        layout: &layout,
                        brightness,
                        palette,
                        tick: 1234,
                        animation_seconds,
                    });
                    for &byte in frame.as_bytes() {
                        hash ^= u64::from(byte);
                        hash = hash.wrapping_mul(0x100000001b3);
                    }
                }
            }
        }
    }
    hash
}
