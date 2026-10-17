//! Physical ANSI key names in the Wooting RGB matrix, not OS text/remap names.
//!
//! Typing positions share the reactive-input map. Function/navigation positions
//! follow external/wooting-rgb-sdk/resources/keyboard-matrix-rows-columns.png;
//! in particular F1 is column 2, not column 1. Fn, custom remaps, and the light
//! bar have no named selector. Other models/layouts must not reuse this map.
use crate::device::{DeviceInfo, DeviceType, Layout};
use crate::layout::MatrixCoord;

pub fn validate_device(info: &DeviceInfo) -> Result<(), String> {
    if info.device_type == DeviceType::Keyboard80
        && info.layout == Layout::Ansi
        && info.max_rows == 6
        && info.max_columns == 17
    {
        Ok(())
    } else {
        Err(format!(
            "named key targets require an 80HE ANSI 6x17 layout; got {:?} {:?} {}x{}",
            info.device_type, info.layout, info.max_rows, info.max_columns
        ))
    }
}

/// Case-insensitive physical key names. No whitespace, ranges, or aliases.
pub fn canonical_coord(name: &str) -> Option<MatrixCoord> {
    let name = name.to_ascii_lowercase();
    if name.len() == 1 {
        let code = match name.as_bytes()[0] {
            b'a'..=b'z' => 0x04 + u16::from(name.as_bytes()[0] - b'a'),
            b'1'..=b'9' => 0x1e + u16::from(name.as_bytes()[0] - b'1'),
            b'0' => 0x27,
            _ => return None,
        };
        return typing_coord(code);
    }
    if let Some(column) = [
        "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
    ]
    .iter()
    .position(|key| *key == name)
    {
        return Some(MatrixCoord {
            row: 0,
            column: column as u8 + 2,
        });
    }
    let hid = match name.as_str() {
        "grave" => 0x35,
        "minus" => 0x2d,
        "equal" => 0x2e,
        "backspace" => 0x2a,
        "tab" => 0x2b,
        "bracketleft" => 0x2f,
        "bracketright" => 0x30,
        "backslash" => 0x31,
        "capslock" => 0x39,
        "semicolon" => 0x33,
        "quote" => 0x34,
        "enter" => 0x28,
        "shiftleft" => 0xe1,
        "shiftright" => 0xe5,
        "comma" => 0x36,
        "period" => 0x37,
        "slash" => 0x38,
        "space" => 0x2c,
        _ => 0,
    };
    if hid != 0 {
        return typing_coord(hid);
    }
    let (row, column) = match name.as_str() {
        "escape" => (0, 0),
        "printscreen" => (0, 14),
        "pause" => (0, 15),
        "insert" => (1, 14),
        "home" => (1, 15),
        "pageup" => (1, 16),
        "delete" => (2, 14),
        "end" => (2, 15),
        "pagedown" => (2, 16),
        "arrowup" => (4, 15),
        "arrowleft" => (5, 14),
        "arrowdown" => (5, 15),
        "arrowright" => (5, 16),
        "controlleft" => (5, 0),
        "metaleft" => (5, 1),
        "altleft" => (5, 2),
        "altright" => (5, 10),
        "controlright" => (5, 13),
        _ => return None,
    };
    Some(MatrixCoord { row, column })
}

fn typing_coord(code: u16) -> Option<MatrixCoord> {
    crate::ripple::hid_coord(code).map(|coord| MatrixCoord {
        row: coord.row,
        column: coord.column,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_map_keeps_matrix_gaps_and_typing_map() {
        for (name, row, column) in [
            ("Escape", 0, 0),
            ("F1", 0, 2),
            ("F4", 0, 5),
            ("f12", 0, 13),
            ("W", 2, 2),
            ("A", 3, 1),
            ("S", 3, 2),
            ("D", 3, 3),
            ("Space", 5, 6),
            ("ArrowUp", 4, 15),
            ("ArrowRight", 5, 16),
        ] {
            assert_eq!(
                canonical_coord(name),
                Some(MatrixCoord { row, column }),
                "{name}"
            );
        }
        for code in 0x04..=0x1d {
            let name = char::from(b'A' + (code - 0x04) as u8).to_string();
            assert_eq!(canonical_coord(&name), typing_coord(code));
        }
        for unsupported in [
            "Fn",
            "Mode",
            "ScrollLock",
            "LightBar",
            "F13",
            "F01",
            "F1-F4",
            "WASD",
            " A",
            "é",
            "Numpad1",
        ] {
            assert_eq!(canonical_coord(unsupported), None, "{unsupported}");
        }
    }

    #[test]
    fn named_targets_reject_other_layouts_models_and_dimensions() {
        let mut info = DeviceInfo::synthetic_80he();
        assert!(validate_device(&info).is_ok());
        for layout in [Layout::Iso, Layout::Jis, Layout::AnsiSplit, Layout::Unknown] {
            info.layout = layout;
            assert!(
                validate_device(&info)
                    .unwrap_err()
                    .contains("80HE ANSI 6x17")
            );
        }
        info.layout = Layout::Ansi;
        info.device_type = DeviceType::Keyboard60;
        assert!(validate_device(&info).is_err());
        info.device_type = DeviceType::Keyboard80;
        info.max_rows = 5;
        assert!(validate_device(&info).is_err());
        info.max_rows = 6;
        info.max_columns = 21;
        assert!(validate_device(&info).is_err());
    }
}
