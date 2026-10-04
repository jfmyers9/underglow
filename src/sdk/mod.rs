//! Wooting SDK backends used by the signal runtime.
//!
//! RGB output and analog input load the official SDKs at runtime. The analog
//! backend consumes the distributable SDK; it is not an SDK hardware plugin.

pub mod analog;
pub mod rgb;
mod rgb_ffi;

/// Fail closed even when a development profile specifies explicit SDK paths.
pub fn hardware_disabled() -> bool {
    std::env::var_os("WOOTING_DEV_SIMULATION").is_some()
}

pub const SIMULATION_NOTICE: &str = "Hardware is disabled in the development simulator; use make dev DEV_HARDWARE=1 for explicit hardware testing";
