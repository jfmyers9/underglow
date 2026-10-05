use super::rgb_ffi::{RgbSdk, SdkLoadError};
use crate::render::{Color, Frame, MAX_COLUMNS, MAX_ROWS};
use std::ffi::CStr;
use std::path::Path;

pub use underglow::device::{DeviceInfo, DeviceType, Layout};

#[derive(Debug, thiserror::Error)]
pub enum WootingError {
    #[error(transparent)]
    SdkLoad(#[from] SdkLoadError),
    #[error("no Wooting RGB keyboard found")]
    NoKeyboard,
    #[error("Wooting RGB SDK returned no device info")]
    MissingDeviceInfo,
    #[error(
        "coordinate ({row}, {column}) outside connected device bounds {max_rows}x{max_columns}"
    )]
    OutOfBounds {
        row: u8,
        column: u8,
        max_rows: u8,
        max_columns: u8,
    },
    #[error("Wooting RGB SDK call failed: {0}")]
    SdkCall(&'static str),
}

pub struct WootingRgb {
    sdk: RgbSdk,
    info: DeviceInfo,
    closed: bool,
}

impl WootingRgb {
    pub fn open(sdk_path: Option<&Path>) -> Result<Self, WootingError> {
        let sdk = RgbSdk::load(sdk_path)?;
        if !sdk.kbd_connected() {
            return Err(WootingError::NoKeyboard);
        }

        sdk.array_auto_update(false);

        let info = match device_info_from_sdk(&sdk) {
            Ok(info) => info,
            Err(error) => {
                // The SDK has already initialized RGB. Self does not exist yet,
                // so Drop cannot release the session on a metadata failure.
                if !sdk.close() {
                    eprintln!(
                        "warning: RGB setup failed and SDK restore/close was not acknowledged"
                    );
                }
                return Err(error);
            }
        };
        Ok(Self {
            sdk,
            info,
            closed: false,
        })
    }

    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    pub fn direct_set_key(&self, row: u8, column: u8, color: Color) -> Result<(), WootingError> {
        self.check_bounds(row, column)?;
        if self
            .sdk
            .direct_set_key(row, column, color.red, color.green, color.blue)
        {
            Ok(())
        } else {
            Err(WootingError::SdkCall("wooting_rgb_direct_set_key"))
        }
    }

    pub fn set_frame(&self, frame: &Frame) -> Result<(), WootingError> {
        if self.sdk.array_set_full(frame.as_bytes()) {
            Ok(())
        } else {
            Err(WootingError::SdkCall("wooting_rgb_array_set_full"))
        }
    }

    pub fn update(&self) -> Result<(), WootingError> {
        if self.sdk.array_update_keyboard() {
            Ok(())
        } else {
            Err(WootingError::SdkCall("wooting_rgb_array_update_keyboard"))
        }
    }

    pub fn close(&mut self) -> Result<(), WootingError> {
        if self.closed {
            return Ok(());
        }

        self.closed = true;
        if self.sdk.close() {
            Ok(())
        } else {
            Err(WootingError::SdkCall("wooting_rgb_close"))
        }
    }

    fn check_bounds(&self, row: u8, column: u8) -> Result<(), WootingError> {
        if row < self.info.max_rows && column < self.info.max_columns {
            Ok(())
        } else {
            Err(WootingError::OutOfBounds {
                row,
                column,
                max_rows: self.info.max_rows,
                max_columns: self.info.max_columns,
            })
        }
    }
}

impl Drop for WootingRgb {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn device_info_from_sdk(sdk: &RgbSdk) -> Result<DeviceInfo, WootingError> {
    let raw = sdk.device_info().ok_or(WootingError::MissingDeviceInfo)?;
    let model = if raw.model.is_null() {
        "N/A".to_string()
    } else {
        // SAFETY: The SDK exposes a null-terminated static model string in
        // WOOTING_USB_META. Null was checked above.
        unsafe { CStr::from_ptr(raw.model) }
            .to_string_lossy()
            .into_owned()
    };

    Ok(DeviceInfo {
        connected: raw.connected,
        model,
        max_rows: raw.max_rows.min(MAX_ROWS as u8),
        max_columns: raw.max_columns.min(MAX_COLUMNS as u8),
        led_index_max: raw.led_index_max,
        device_type: device_type_from_raw(raw.device_type),
        layout: layout_from_raw(sdk.device_layout()),
        v2_interface: raw.v2_interface,
        uses_small_packets: raw.uses_small_packets,
        uses_multi_report: raw.uses_multi_report,
    })
}

fn device_type_from_raw(value: i32) -> DeviceType {
    match value {
        1 => DeviceType::KeyboardTkl,
        2 => DeviceType::KeyboardFullSize,
        3 => DeviceType::Keyboard60,
        4 => DeviceType::Keypad3Key,
        5 => DeviceType::Keyboard80,
        other => DeviceType::Unknown(other),
    }
}

fn layout_from_raw(value: i32) -> Layout {
    match value {
        -1 => Layout::Unknown,
        0 => Layout::Ansi,
        1 => Layout::Iso,
        2 => Layout::Jis,
        3 => Layout::AnsiSplit,
        4 => Layout::IsoSplit,
        other => Layout::Other(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_device_type_values_preserve_known_and_unknown_metadata() {
        for (raw, expected) in [
            (1, DeviceType::KeyboardTkl),
            (2, DeviceType::KeyboardFullSize),
            (3, DeviceType::Keyboard60),
            (4, DeviceType::Keypad3Key),
            (5, DeviceType::Keyboard80),
            (-1, DeviceType::Unknown(-1)),
            (99, DeviceType::Unknown(99)),
        ] {
            assert_eq!(device_type_from_raw(raw), expected);
        }
    }

    #[test]
    fn sdk_layout_values_preserve_known_and_unknown_metadata() {
        for (raw, expected) in [
            (-1, Layout::Unknown),
            (0, Layout::Ansi),
            (1, Layout::Iso),
            (2, Layout::Jis),
            (3, Layout::AnsiSplit),
            (4, Layout::IsoSplit),
            (99, Layout::Other(99)),
        ] {
            assert_eq!(layout_from_raw(raw), expected);
        }
    }
}
