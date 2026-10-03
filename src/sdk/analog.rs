//! Runtime binding to the official Analog SDK 0.9.1 C ABI.
//! The C API supplies HID codes and travel, but does not expose v2 position metadata.
use libloading::Library;
use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub struct AnalogKeyPressure {
    pub key_code: u16,
    pub pressure: f32,
}

#[derive(Debug, thiserror::Error)]
pub enum AnalogError {
    #[error(
        "could not load Analog SDK 0.9.1+; pass --analog-sdk-path or set WOOTING_ANALOG_SDK_PATH to libwooting_analog_sdk_dist.dylib/.so\n{0}"
    )]
    Load(String),
    #[error(
        "Analog SDK {operation} failed ({code}); check SDK version, HID permissions, and keyboard connection"
    )]
    Call { operation: &'static str, code: i32 },
    #[error(
        "ripples currently requires exactly one connected analog keyboard, a Wooting 80HE (found {0})"
    )]
    Device(String),
    #[error("Analog SDK returned an invalid buffer")]
    InvalidBuffer,
    #[error(
        "unsupported Analog SDK version {0}; use the official 0.9.1 (or newer 0.9.x) distributable"
    )]
    Version(String),
}

// Types and layout from includes/wooting-analog-sdk.h, tag v0.9.1.
#[repr(C)]
struct DeviceInfoRaw {
    vendor_id: u16,
    product_id: u16,
    manufacturer_name: *const c_char,
    device_name: *const c_char,
    device_id: u64,
    device_type: i32,
}
type Initialise = unsafe extern "C" fn() -> i32;
type Uninitialise = unsafe extern "C" fn() -> i32;
type SetMode = unsafe extern "C" fn(u32) -> i32;
type Devices = unsafe extern "C" fn(*mut *mut DeviceInfoRaw, u32) -> i32;
type ReadBuffer = unsafe extern "C" fn(*mut u16, *mut f32, u32, u64) -> i32;

pub struct AnalogSdk {
    // Keep the library alive until after uninitialise has stopped SDK worker threads.
    _library: Library,
    uninitialise: Uninitialise,
    read_buffer: ReadBuffer,
    device_id: u64,
}

impl AnalogSdk {
    pub fn open(path: Option<&Path>) -> Result<Self, AnalogError> {
        let mut failures = Vec::new();
        for candidate in library_candidates(path, std::env::var_os("WOOTING_ANALOG_SDK_PATH")) {
            // SAFETY: Loading executable code trusts the user-selected SDK path.
            let library = match unsafe { Library::new(&candidate) } {
                Ok(library) => library,
                Err(error) => {
                    failures.push(format!("{}: {error}", candidate.display()));
                    continue;
                }
            };
            return Self::from_library(library);
        }
        Err(AnalogError::Load(failures.join("\n")))
    }

    fn from_library(library: Library) -> Result<Self, AnalogError> {
        // SAFETY: The official header declares a no-argument function returning a static C string.
        let version = unsafe {
            symbol::<unsafe extern "C" fn() -> *const c_char>(
                &library,
                b"wooting_analog_version_semver\0",
            )?()
        };
        if version.is_null() {
            return Err(AnalogError::Version("null".into()));
        }
        // SAFETY: SDK promises a static, NUL-terminated version string.
        let version = unsafe { CStr::from_ptr(version) }.to_string_lossy();
        if !supported_version(&version) {
            return Err(AnalogError::Version(version.into_owned()));
        }
        // SAFETY: All symbols below use the exact signatures from the official C header.
        let (initialise, uninitialise, mode, devices, read_buffer) = unsafe {
            (
                symbol::<Initialise>(&library, b"wooting_analog_initialise\0")?,
                symbol::<Uninitialise>(&library, b"wooting_analog_uninitialise\0")?,
                symbol::<SetMode>(&library, b"wooting_analog_set_keycode_mode\0")?,
                symbol::<Devices>(&library, b"wooting_analog_get_connected_devices_info\0")?,
                symbol::<ReadBuffer>(&library, b"wooting_analog_read_full_buffer_device\0")?,
            )
        };
        // SAFETY: Exact C ABI, no arguments; initialise starts SDK-owned resources.
        checked("initialise", unsafe { initialise() })?;
        let mut sdk = Self {
            _library: library,
            uninitialise,
            read_buffer,
            device_id: 0,
        };
        // From here Drop also handles partial setup failures.
        // SAFETY: 0 is the documented HID keycode mode.
        checked("set HID mode", unsafe { mode(0) })?;
        let mut buffer = [std::ptr::null_mut(); 16];
        // SAFETY: Buffer has 16 writable pointer slots and stays alive throughout the call.
        let count = checked("enumerate devices", unsafe {
            devices(buffer.as_mut_ptr(), 16)
        })?;
        if count != 1 || buffer[0].is_null() {
            return Err(AnalogError::Device(count.to_string()));
        }
        // SAFETY: The SDK owns this non-null struct until the next enumeration or teardown.
        let info = unsafe { &*buffer[0] };
        if info.vendor_id != 0x31e3 || !(0x1400..=0x1402).contains(&info.product_id) {
            return Err(AnalogError::Device(format!(
                "USB {:04x}:{:04x}",
                info.vendor_id, info.product_id
            )));
        }
        sdk.device_id = info.device_id;
        Ok(sdk)
    }

    pub fn read(&self) -> Result<Vec<AnalogKeyPressure>, AnalogError> {
        let mut codes = [0u16; 256];
        let mut values = [0.0f32; 256];
        // SAFETY: Both buffers have 256 writable elements; the selected device ID is SDK-provided.
        let count = checked("read buffer", unsafe {
            (self.read_buffer)(codes.as_mut_ptr(), values.as_mut_ptr(), 256, self.device_id)
        })? as usize;
        if count > codes.len() || values[..count].iter().any(|v| !v.is_finite()) {
            return Err(AnalogError::InvalidBuffer);
        }
        Ok(codes[..count]
            .iter()
            .zip(&values[..count])
            .map(|(&key_code, &pressure)| AnalogKeyPressure {
                key_code,
                pressure: pressure.clamp(0.0, 1.0),
            })
            .collect())
    }
}

impl Drop for AnalogSdk {
    fn drop(&mut self) {
        // SAFETY: Library is still loaded and initialise succeeded before Self was constructed.
        let code = unsafe { (self.uninitialise)() };
        if code < 0 {
            eprintln!("warning: Analog SDK uninitialise failed ({code})");
        }
    }
}

unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, AnalogError> {
    // SAFETY: Caller guarantees T matches this symbol's C ABI.
    unsafe { library.get::<T>(name) }.map(|s| *s).map_err(|e| {
        AnalogError::Load(format!(
            "{}: {e}",
            CStr::from_bytes_with_nul(name).unwrap().to_string_lossy()
        ))
    })
}

fn checked(operation: &'static str, code: i32) -> Result<i32, AnalogError> {
    if code < 0 {
        Err(AnalogError::Call { operation, code })
    } else {
        Ok(code)
    }
}

fn supported_version(version: &str) -> bool {
    let parts = version.split('.').collect::<Vec<_>>();
    matches!(parts.as_slice(), ["0", "9", patch] if patch.parse::<u32>().is_ok_and(|patch| patch >= 1))
}

fn library_candidates(explicit: Option<&Path>, env: Option<std::ffi::OsString>) -> Vec<PathBuf> {
    // An explicit override is authoritative: don't silently load a different SDK on a typo.
    if let Some(path) = explicit {
        return vec![path.to_owned()];
    }
    if let Some(path) = env {
        return vec![path.into()];
    }
    let name = if cfg!(target_os = "macos") {
        "libwooting_analog_sdk_dist.dylib"
    } else if cfg!(target_os = "windows") {
        "wooting_analog_sdk_dist.dll"
    } else {
        "libwooting_analog_sdk_dist.so"
    };
    vec![
        PathBuf::from(name),
        PathBuf::from("/usr/local/lib").join(name),
        PathBuf::from("/opt/homebrew/lib").join(name),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overrides_do_not_fall_back() {
        assert_eq!(
            library_candidates(Some(Path::new("explicit")), Some("env".into())),
            vec![PathBuf::from("explicit")]
        );
        assert_eq!(
            library_candidates(None, Some("env".into())),
            vec![PathBuf::from("env")]
        );
    }
    #[test]
    fn sdk_errors_are_not_empty_frames() {
        assert!(checked("read", -1999).is_err());
        assert_eq!(checked("read", 0).unwrap(), 0);
    }

    #[test]
    fn reject_old_and_unknown_abis() {
        assert!(supported_version("0.9.1"));
        assert!(supported_version("0.9.2"));
        for version in ["0.8.0", "0.9.0", "1.0.0", "0.10.0", "garbage"] {
            assert!(!supported_version(version));
        }
    }
}
