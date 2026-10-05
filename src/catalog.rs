//! Built-in visualization metadata shared by CLI presets and the controller.
//! Register a visualization here once; its implementation belongs in its own module.
use crate::effects::EffectKind;
use crate::render::PaletteName;

pub const DEFAULT_PRESET: &str = "ripples";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererKind {
    Static(EffectKind),
    Ripples,
    Focus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaletteControl {
    Selectable,
    Fixed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualDefaults {
    pub palette: PaletteName,
    pub brightness: u8,
    pub fps: u32,
    pub speed: u32,
}

impl VisualDefaults {
    const fn with_palette(palette: PaletteName) -> Self {
        Self {
            palette,
            brightness: crate::DEFAULT_BRIGHTNESS,
            fps: 30,
            speed: crate::animation::DEFAULT_SPEED,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Visualization {
    /// Stable CLI/config/IPC identity; display names can change independently.
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub renderer: RendererKind,
    pub palette: PaletteControl,
    pub supports_speed: bool,
    pub supports_two_tone: bool,
    pub fixed_color_note: Option<&'static str>,
    pub defaults: VisualDefaults,
    /// Diagnostic renderers may be available through profiles but not everyday presets.
    pub selectable: bool,
}

impl Visualization {
    pub fn palette_available(&self, two_tone: bool) -> bool {
        self.palette == PaletteControl::Selectable && !(self.supports_two_tone && two_tone)
    }

    pub fn preview_label(&self) -> &'static str {
        match self.renderer {
            RendererKind::Static(_) => "EFFECT SIMULATION",
            RendererKind::Ripples => "RIPPLE SIMULATION",
            RendererKind::Focus => "FOCUS SIMULATION",
        }
    }

    pub fn preview_description(&self) -> &'static str {
        match self.renderer {
            RendererKind::Static(_) => {
                "80HE LED matrix · shared effect renderer and draft settings · simulated time, not device feedback"
            }
            RendererKind::Ripples => {
                "80HE LED matrix · actual ripple math and draft settings · synthetic input, not device feedback"
            }
            RendererKind::Focus => {
                "80HE LED matrix · shared Focus renderer with synthetic phase and progress · not the live timer"
            }
        }
    }

    /// Fresh preset only. Never merge these defaults into an existing saved profile.
    pub fn preset_config(&self) -> String {
        let signal = match self.renderer {
            RendererKind::Static(effect) => {
                format!("kind = 'static-effect'\neffect = '{effect}'\n")
            }
            RendererKind::Ripples => "kind = 'ripples'\n".into(),
            RendererKind::Focus => "kind = 'focus-cockpit'\n".into(),
        };
        format!(
            "schema_version = 1\ncontinuous = true\npalette = '{}'\nbrightness = {}\nfps = {}\nspeed = {}\n[signal]\n{signal}",
            self.defaults.palette, self.defaults.brightness, self.defaults.fps, self.defaults.speed
        )
    }
}

const STANDARD: VisualDefaults = VisualDefaults::with_palette(PaletteName::Wooting);
static VISUALIZATIONS: &[Visualization] = &[
    Visualization {
        id: "ripples",
        title: "Ripples",
        description: "Light that follows your touch",
        renderer: RendererKind::Ripples,
        palette: PaletteControl::Selectable,
        supports_speed: false,
        supports_two_tone: true,
        fixed_color_note: None,
        defaults: VisualDefaults::with_palette(PaletteName::Ocean),
        selectable: true,
    },
    Visualization {
        id: "comet",
        title: "Comet",
        description: "A quiet trail across your keys",
        renderer: RendererKind::Static(EffectKind::Comet),
        palette: PaletteControl::Selectable,
        supports_speed: true,
        supports_two_tone: false,
        fixed_color_note: None,
        defaults: STANDARD,
        selectable: true,
    },
    Visualization {
        id: "rainbow",
        title: "Spectrum",
        description: "A continuous flow of color",
        renderer: RendererKind::Static(EffectKind::Rainbow),
        palette: PaletteControl::Fixed,
        supports_speed: true,
        supports_two_tone: false,
        fixed_color_note: Some("Spectrum uses a fixed rainbow, not a palette."),
        defaults: STANDARD,
        selectable: true,
    },
    Visualization {
        id: "breath",
        title: "Breathe",
        description: "Slow down. Fade in, fade out.",
        renderer: RendererKind::Static(EffectKind::Breath),
        palette: PaletteControl::Selectable,
        supports_speed: true,
        supports_two_tone: false,
        fixed_color_note: None,
        defaults: STANDARD,
        selectable: true,
    },
    Visualization {
        id: "matrix",
        title: "Matrix",
        description: "A little digital rainfall",
        renderer: RendererKind::Static(EffectKind::Matrix),
        palette: PaletteControl::Fixed,
        supports_speed: true,
        supports_two_tone: false,
        fixed_color_note: Some("Matrix uses the fixed Terminal green palette."),
        defaults: STANDARD,
        selectable: true,
    },
    Visualization {
        id: "focus-cockpit",
        title: "Focus",
        description: "Keep time, without the noise",
        renderer: RendererKind::Focus,
        palette: PaletteControl::Fixed,
        supports_speed: false,
        supports_two_tone: false,
        fixed_color_note: Some(
            "Focus uses phase colors; the preview illustrates the blue focus phase, not the live timer.",
        ),
        defaults: STANDARD,
        selectable: true,
    },
    Visualization {
        id: "row-test",
        title: "Row test",
        description: "Diagnostic row colors",
        renderer: RendererKind::Static(EffectKind::RowTest),
        palette: PaletteControl::Fixed,
        supports_speed: false,
        supports_two_tone: false,
        fixed_color_note: Some("Row test uses fixed diagnostic row colors."),
        defaults: STANDARD,
        selectable: false,
    },
];

pub fn all() -> &'static [Visualization] {
    VISUALIZATIONS
}

pub fn find(id: &str) -> Option<&'static Visualization> {
    all().iter().find(|visualization| visualization.id == id)
}

pub fn presets() -> impl Iterator<Item = &'static Visualization> {
    all()
        .iter()
        .filter(|visualization| visualization.selectable)
}

pub fn default_preset() -> &'static Visualization {
    find(DEFAULT_PRESET).expect("default visualization must be registered")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;
    use std::collections::HashSet;

    #[test]
    fn stable_ids_order_and_diagnostic_visibility_are_preserved() {
        assert_eq!(
            presets().map(|v| v.id).collect::<Vec<_>>(),
            [
                "ripples",
                "comet",
                "rainbow",
                "breath",
                "matrix",
                "focus-cockpit"
            ]
        );
        assert_eq!(
            all().iter().map(|v| v.id).collect::<HashSet<_>>().len(),
            all().len()
        );
        assert!(find("row-test").is_some_and(|v| !v.selectable));
        assert!(find("unknown").is_none());
    }

    #[test]
    fn every_static_effect_has_one_matching_catalog_entry() {
        for effect in EffectKind::value_variants() {
            let matches = all()
                .iter()
                .filter(|v| v.renderer == RendererKind::Static(*effect))
                .collect::<Vec<_>>();
            assert_eq!(matches.len(), 1, "{effect:?}");
            assert_eq!(matches[0].id, effect.to_string());
        }
    }

    #[test]
    fn capabilities_and_fresh_defaults_match_existing_modes() {
        for v in all() {
            assert!(!v.title.is_empty() && !v.description.is_empty());
            assert_eq!(v.defaults.brightness, 255);
            assert_eq!(v.defaults.fps, 30);
            assert_eq!(v.defaults.speed, 100);
            assert_eq!(
                v.supports_speed,
                matches!(v.id, "comet" | "rainbow" | "matrix" | "breath")
            );
            assert_eq!(
                v.palette_available(false),
                matches!(v.id, "comet" | "breath" | "ripples")
            );
            assert_eq!(
                v.palette_available(true),
                matches!(v.id, "comet" | "breath")
            );
            assert_eq!(v.supports_two_tone, v.id == "ripples");
            let config: toml::Value = toml::from_str(&v.preset_config()).unwrap();
            assert_eq!(config["continuous"].as_bool(), Some(true));
            assert!(config.get("sdk_path").is_none());
        }
        assert_eq!(
            find("ripples").unwrap().defaults.palette,
            PaletteName::Ocean
        );
        assert_eq!(
            find("comet").unwrap().defaults.palette,
            PaletteName::Wooting
        );
    }
}
