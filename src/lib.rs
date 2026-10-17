//! Hardware-independent effect rendering.
pub mod animation;
pub mod automation;
pub mod binding_repair;
pub mod capability_manifest;
pub mod catalog;
pub mod compositor;
pub mod connection_registry;
pub mod device;
pub mod effects;
pub mod explanations;
pub mod external;
pub mod focus;
pub mod key_targets;
pub mod layer_runtime;
pub mod layout;
pub mod reactive;
pub mod render;
pub mod ripple;
pub mod scenarios;
pub mod scenes;
pub mod setup;
pub mod setup_edits;
pub mod setup_library;
pub mod source_capabilities;
pub mod templates;

/// Full brightness (100%); CLI/config values use RGB channels, not percentages.
pub const DEFAULT_BRIGHTNESS: u8 = 255;
