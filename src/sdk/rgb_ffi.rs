use libloading::Library;
use std::env;
use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};

#[repr(C)]
pub struct WootingUsbMetaRaw {
    pub connected: bool,
    pub model: *const c_char,
    pub max_rows: u8,
    pub max_columns: u8,
    pub led_index_max: u8,
    pub device_type: i32,
    pub v2_interface: bool,
    pub layout: i32,
    pub uses_small_packets: bool,
    pub uses_multi_report: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SdkLoadError {
    #[error("failed to load Wooting RGB SDK; set WOOTING_RGB_SDK_PATH or pass --sdk-path\n{0}")]
    NotFound(String),
    #[error("failed to load symbol {symbol}: {source}")]
    Symbol {
        symbol: &'static str,
        source: libloading::Error,
    },
}

type KbdConnected = unsafe extern "C" fn() -> bool;
type Close = unsafe extern "C" fn() -> bool;
type DirectSetKey = unsafe extern "C" fn(u8, u8, u8, u8, u8) -> bool;
type ArrayUpdateKeyboard = unsafe extern "C" fn() -> bool;
type ArrayAutoUpdate = unsafe extern "C" fn(bool);
type ArraySetFull = unsafe extern "C" fn(*const u8) -> bool;
type DeviceInfo = unsafe extern "C" fn() -> *const WootingUsbMetaRaw;
type DeviceLayout = unsafe extern "C" fn() -> i32;

pub struct RgbSdk {
    _library: Library,
    kbd_connected: KbdConnected,
    close: Close,
    direct_set_key: DirectSetKey,
    array_update_keyboard: ArrayUpdateKeyboard,
    array_auto_update: ArrayAutoUpdate,
    array_set_full: ArraySetFull,
    device_info: DeviceInfo,
    device_layout: DeviceLayout,
}

impl RgbSdk {
    pub fn load(explicit_path: Option<&Path>) -> Result<Self, SdkLoadError> {
        let mut errors = Vec::new();

        for candidate in library_candidates(explicit_path, env::var_os("WOOTING_RGB_SDK_PATH")) {
            // SAFETY: Loading a dynamic library is inherently unsafe because it
            // trusts the file at `candidate`. The path is either user-supplied,
            // from WOOTING_RGB_SDK_PATH, or a documented platform default.
            match unsafe { Library::new(&candidate) } {
                Ok(library) => return Self::from_library(library),
                Err(err) => errors.push(format!("{}: {err}", candidate.display())),
            }
        }

        Err(SdkLoadError::NotFound(errors.join("\n")))
    }

    fn from_library(library: Library) -> Result<Self, SdkLoadError> {
        let kbd_connected = load_symbol(&library, b"wooting_rgb_kbd_connected\0")?;
        let close = load_symbol(&library, b"wooting_rgb_close\0")?;
        let direct_set_key = load_symbol(&library, b"wooting_rgb_direct_set_key\0")?;
        let array_update_keyboard = load_symbol(&library, b"wooting_rgb_array_update_keyboard\0")?;
        let array_auto_update = load_symbol(&library, b"wooting_rgb_array_auto_update\0")?;
        let array_set_full = load_symbol(&library, b"wooting_rgb_array_set_full\0")?;
        let device_info = load_symbol(&library, b"wooting_rgb_device_info\0")?;
        let device_layout = load_symbol(&library, b"wooting_rgb_device_layout\0")?;

        Ok(Self {
            _library: library,
            kbd_connected,
            close,
            direct_set_key,
            array_update_keyboard,
            array_auto_update,
            array_set_full,
            device_info,
            device_layout,
        })
    }

    pub fn kbd_connected(&self) -> bool {
        // SAFETY: Function pointer comes from the loaded Wooting RGB SDK and
        // has the exact C ABI declared in the official header.
        unsafe { (self.kbd_connected)() }
    }

    pub fn close(&self) -> bool {
        // SAFETY: See `kbd_connected`.
        unsafe { (self.close)() }
    }

    pub fn direct_set_key(&self, row: u8, column: u8, red: u8, green: u8, blue: u8) -> bool {
        // SAFETY: See `kbd_connected`. Scalar arguments are range-checked by
        // the safe wrapper before this is called.
        unsafe { (self.direct_set_key)(row, column, red, green, blue) }
    }

    pub fn array_update_keyboard(&self) -> bool {
        // SAFETY: See `kbd_connected`.
        unsafe { (self.array_update_keyboard)() }
    }

    pub fn array_auto_update(&self, auto_update: bool) {
        // SAFETY: See `kbd_connected`.
        unsafe { (self.array_auto_update)(auto_update) }
    }

    pub fn array_set_full(&self, colors: &[u8]) -> bool {
        debug_assert_eq!(colors.len(), 6 * 21 * 3);
        // SAFETY: The SDK reads a 6*21*3 byte buffer. `Frame` always provides
        // that length, and the pointer remains valid for the duration of call.
        unsafe { (self.array_set_full)(colors.as_ptr()) }
    }

    pub fn device_info(&self) -> Option<&WootingUsbMetaRaw> {
        // SAFETY: See `kbd_connected`. Null is handled; non-null is treated as
        // an SDK-owned immutable struct valid until close/disconnect.
        let ptr = unsafe { (self.device_info)() };
        if ptr.is_null() {
            None
        } else {
            // SAFETY: Checked for null above; SDK owns the pointee.
            Some(unsafe { &*ptr })
        }
    }

    pub fn device_layout(&self) -> i32 {
        // SAFETY: See `kbd_connected`.
        unsafe { (self.device_layout)() }
    }
}

fn load_symbol<T: Copy>(library: &Library, symbol: &'static [u8]) -> Result<T, SdkLoadError> {
    // SAFETY: `from_library` requests each symbol with the function-pointer
    // type declared by the official Wooting RGB SDK C header.
    unsafe { library.get::<T>(symbol) }
        .map(|symbol| *symbol)
        .map_err(|source| SdkLoadError::Symbol {
            symbol: CStr::from_bytes_with_nul(symbol)
                .ok()
                .and_then(|s| s.to_str().ok())
                .unwrap_or("<invalid>"),
            source,
        })
}

fn library_candidates(
    explicit_path: Option<&Path>,
    environment_path: Option<std::ffi::OsString>,
) -> Vec<PathBuf> {
    // Overrides are authoritative. Falling back on a typo could silently load
    // an old SDK or touch real hardware during an isolated diagnostic test.
    if let Some(path) = explicit_path {
        return vec![path.to_path_buf()];
    }
    if let Some(path) = environment_path {
        return vec![PathBuf::from(path)];
    }

    let mut paths = Vec::new();

    #[cfg(target_os = "macos")]
    {
        paths.extend([
            PathBuf::from("external/wooting-rgb-sdk/mac/libwooting-rgb-sdk.dylib"),
            PathBuf::from("libwooting-rgb-sdk.dylib"),
            PathBuf::from("/opt/homebrew/lib/libwooting-rgb-sdk.dylib"),
            PathBuf::from("/usr/local/lib/libwooting-rgb-sdk.dylib"),
        ]);
    }

    #[cfg(target_os = "linux")]
    {
        paths.extend([
            PathBuf::from("external/wooting-rgb-sdk/linux/libwooting-rgb-sdk.so"),
            PathBuf::from("libwooting-rgb-sdk.so"),
            PathBuf::from("/usr/local/lib/libwooting-rgb-sdk.so"),
            PathBuf::from("/usr/lib/libwooting-rgb-sdk.so"),
        ]);
    }

    #[cfg(target_os = "windows")]
    {
        paths.extend([
            PathBuf::from("external/wooting-rgb-sdk/windows/x64/Release/wooting-rgb-sdk.dll"),
            PathBuf::from("wooting-rgb-sdk.dll"),
        ]);
    }

    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_path_overrides_are_authoritative() {
        assert_eq!(
            library_candidates(Some(Path::new("explicit")), Some("env".into())),
            vec![PathBuf::from("explicit")]
        );
        assert_eq!(
            library_candidates(None, Some("env".into())),
            vec![PathBuf::from("env")]
        );
    }
}
