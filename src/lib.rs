//! Hardware-independent effect rendering.
pub mod ripple;

/// Full brightness (100%); CLI/config values use RGB channels, not percentages.
pub const DEFAULT_BRIGHTNESS: u8 = 255;
