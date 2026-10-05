//! Hardware-independent effect rendering.
pub mod animation;
pub mod catalog;
pub mod device;
pub mod effects;
pub mod focus;
pub mod layout;
pub mod reactive;
pub mod render;
pub mod ripple;
pub mod scenes;

/// Full brightness (100%); CLI/config values use RGB channels, not percentages.
pub const DEFAULT_BRIGHTNESS: u8 = 255;
