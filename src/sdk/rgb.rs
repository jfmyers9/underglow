use super::rgb_ffi::{RgbSdk, SdkLoadError};
use crate::render::{Color, Frame, MAX_COLUMNS, MAX_ROWS};
use std::ffi::CStr;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub connected: bool,
    pub model: String,
    pub max_rows: u8,
    pub max_columns: u8,
    pub led_index_max: u8,
    pub device_type: DeviceType,
    pub layout: Layout,
    pub v2_interface: bool,
    pub uses_small_packets: bool,
    pub uses_multi_report: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceType {
    KeyboardTkl,
    KeyboardFullSize,
    Keyboard60,
    Keypad3Key,
    Keyboard80,
    Unknown(i32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layout {
    Unknown,
    Ansi,
    Iso,
    Jis,
    AnsiSplit,
    IsoSplit,
    Other(i32),
}

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

        let info = match DeviceInfo::from_sdk(&sdk) {
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

impl DeviceInfo {
    fn from_sdk(sdk: &RgbSdk) -> Result<Self, WootingError> {
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

        Ok(Self {
            connected: raw.connected,
            model,
            max_rows: raw.max_rows.min(MAX_ROWS as u8),
            max_columns: raw.max_columns.min(MAX_COLUMNS as u8),
            led_index_max: raw.led_index_max,
            device_type: DeviceType::from_raw(raw.device_type),
            layout: Layout::from_raw(sdk.device_layout()),
            v2_interface: raw.v2_interface,
            uses_small_packets: raw.uses_small_packets,
            uses_multi_report: raw.uses_multi_report,
        })
    }
}

impl DeviceType {
    fn from_raw(value: i32) -> Self {
        match value {
            1 => Self::KeyboardTkl,
            2 => Self::KeyboardFullSize,
            3 => Self::Keyboard60,
            4 => Self::Keypad3Key,
            5 => Self::Keyboard80,
            other => Self::Unknown(other),
        }
    }
}

impl Layout {
    fn from_raw(value: i32) -> Self {
        match value {
            -1 => Self::Unknown,
            0 => Self::Ansi,
            1 => Self::Iso,
            2 => Self::Jis,
            3 => Self::AnsiSplit,
            4 => Self::IsoSplit,
            other => Self::Other(other),
        }
    }
}
