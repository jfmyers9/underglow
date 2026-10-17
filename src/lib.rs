//! Hardware-independent effect rendering.
pub mod animation;
pub mod automation;
pub mod catalog;
pub mod compositor;
pub mod device;
pub mod effects;
pub mod external;
pub mod focus;
pub mod key_targets;
pub mod layer_runtime;
pub mod layout;
pub mod reactive;
pub mod render;
pub mod ripple;
pub mod scenes;
pub mod setup;
pub mod setup_edits;
pub mod source_capabilities;

/// Full brightness (100%); CLI/config values use RGB channels, not percentages.
pub const DEFAULT_BRIGHTNESS: u8 = 255;
