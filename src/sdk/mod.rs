//! Wooting SDK backends used by the signal runtime.
//!
//! RGB output and analog input load the official SDKs at runtime. The analog
//! backend consumes the distributable SDK; it is not an SDK hardware plugin.

pub mod analog;
pub mod rgb;
mod rgb_ffi;
