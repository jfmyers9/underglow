//! Pure device metadata shared by hardware adapters and renderers.

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

impl DeviceInfo {
    /// Shared offline fixture for CLI and GUI previews; this never discovers a device.
    pub fn synthetic_80he() -> Self {
        Self {
            connected: false,
            model: "preview-80he".into(),
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
