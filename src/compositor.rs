//! Hardware-independent layer blending on a freshly rendered background.
//!
//! Callers own animation clocks, event identity and activation. Submit persistent
//! layers only while their state is active; timed layers use the caller's monotonic
//! clock. Composition never caches frames, queues alerts, or writes to hardware.
use crate::layout::{KeyPosition, KeyboardLayout, MatrixCoord, Zone};
use crate::render::{Color, Frame};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Software targets, always restricted to keys present in the current layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyMask {
    Whole,
    Zones(Vec<Zone>),
    /// Low-level coordinates for callers implementing custom/named key groups.
    Coordinates(Vec<MatrixCoord>),
}

impl KeyMask {
    pub fn contains(&self, key: &KeyPosition) -> bool {
        match self {
            Self::Whole => true,
            Self::Zones(zones) => zones.contains(&key.zone),
            Self::Coordinates(coords) => coords.contains(&key.coord),
        }
    }
}

/// Channel-linear integer blending, not gamma-corrected or a Photoshop superset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Multiply,
}

/// Blend one RGB contribution. Division by 255 rounds to nearest (n+127)/255.
/// Normal: lerp(dst,src,a). Add: min(255,dst+round(src*a/255)).
/// Multiply: lerp(dst,round(dst*src/255),a). All channels use the same math.
pub fn blend_color(base: Color, source: Color, opacity: u8, mode: BlendMode) -> Color {
    let channel = |d: u8, s: u8| {
        let (d, s, a) = (u32::from(d), u32::from(s), u32::from(opacity));
        let round = |n: u32| (n + 127) / 255;
        (match mode {
            BlendMode::Normal => round(d * (255 - a) + s * a),
            BlendMode::Add => (d + round(s * a)).min(255),
            BlendMode::Multiply => round(d * (255 - a) + round(d * s) * a),
        }) as u8
    };
    Color::new(
        channel(base.red, source.red),
        channel(base.green, source.green),
        channel(base.blue, source.blue),
    )
}

/// Final safety ceiling, NOT another brightness multiplier.
pub fn cap_color(color: Color, brightness: u8) -> Color {
    Color::new(
        color.red.min(brightness),
        color.green.min(brightness),
        color.blue.min(brightness),
    )
}

/// One overlay. Black is a color; transparency uses explicit renderer coverage
/// and layer opacity. Greater priority is on top; first declaration wins ties.
#[derive(Clone, Debug)]
pub struct Layer {
    pub opacity: u8,
    pub blend: BlendMode,
    pub frame: Frame,
    pub mask: KeyMask,
    pub priority: i32,
    /// An absolute deadline on the same monotonic clock as `compose`'s `now`.
    /// `None` is persistent. A timed layer is inactive exactly at its deadline.
    pub expires: Option<Duration>,
}

impl Layer {
    /// Combine layer opacity and renderer coverage once; black never implies a hole.
    pub fn opacity_at(&self, key: &KeyPosition) -> u8 {
        if !self.mask.contains(key) {
            return 0;
        }
        ((u16::from(self.opacity) * u16::from(self.frame.alpha_coord(key.coord)) + 127) / 255) as u8
    }

    pub fn is_active(&self, now: Duration) -> bool {
        self.expires.is_none_or(|expires| now < expires)
    }
}

/// Top eligible nonzero-opacity contribution in declaration order, not necessarily
/// the sole visible contributor. Retains the legacy priority/tie arbitration API.
pub fn winning_layer(layers: &[Layer], key: &KeyPosition, now: Duration) -> Option<usize> {
    let mut winner: Option<usize> = None;
    for (index, layer) in layers.iter().enumerate() {
        if layer.is_active(now)
            && layer.opacity_at(key) != 0
            && winner.is_none_or(|winner| layer.priority > layers[winner].priority)
        {
            winner = Some(index);
        }
    }
    winner
}

/// Eligible indices in bottom-to-top order, preserving signed priorities/ties.
pub fn ordered_layers(layers: &[Layer], now: Duration) -> Vec<usize> {
    let mut order: Vec<_> = (0..layers.len())
        .filter(|&i| layers[i].is_active(now) && layers[i].opacity != 0)
        .collect();
    order.sort_by(|&a, &b| {
        layers[a]
            .priority
            .cmp(&layers[b].priority)
            .then_with(|| b.cmp(&a))
    });
    order
}

/// Structurally visible contributors: lower layers survive partial/add/multiply
/// overlays. RGB coincidences and rounding do not alter coverage diagnostics.
pub fn contributing_layers(layers: &[Layer], key: &KeyPosition, now: Duration) -> Vec<usize> {
    let mut contributors = Vec::new();
    for i in ordered_layers(layers, now) {
        let layer = &layers[i];
        let opacity = layer.opacity_at(key);
        if opacity != 0 {
            if opacity == 255 && layer.blend == BlendMode::Normal {
                contributors.clear();
            }
            contributors.push(i);
        }
    }
    contributors
}

/// Combine active layers onto the current background, leaving every uncovered
/// key (and non-key matrix cell) unchanged. The result is a single RGB frame.
pub fn compose(mut base: Frame, layout: &KeyboardLayout, layers: &[Layer], now: Duration) -> Frame {
    // Composition resolves renderer coverage to physical RGB. With no lower
    // contribution, transparent backdrop positions are off, not stale pixels.
    for key in layout.keys() {
        let alpha = base.alpha_coord(key.coord);
        if alpha != 255 {
            base.set_coord(
                key.coord,
                blend_color(
                    Color::BLACK,
                    base.get_coord(key.coord),
                    alpha,
                    BlendMode::Normal,
                ),
            );
            base.set_alpha_coord(key.coord, 255);
        }
    }
    for i in ordered_layers(layers, now) {
        let layer = &layers[i];
        for key in layout.keys() {
            let opacity = layer.opacity_at(key);
            if opacity != 0 {
                base.set_coord(
                    key.coord,
                    blend_color(
                        base.get_coord(key.coord),
                        layer.frame.get_coord(key.coord),
                        opacity,
                        layer.blend,
                    ),
                );
            }
        }
    }
    base
}

/// Canonical engine/preview pipeline: already brightness-scaled inputs, blend,
/// then clamp final channels to the global safety ceiling without double scaling.
pub fn compose_capped(
    base: Frame,
    layout: &KeyboardLayout,
    layers: &[Layer],
    now: Duration,
    brightness: u8,
) -> Frame {
    let mut frame = compose(base, layout, layers, now);
    for key in layout.keys() {
        frame.set_coord(key.coord, cap_color(frame.get_coord(key.coord), brightness));
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceInfo, DeviceType, Layout};
    use crate::render::Color;

    fn layout() -> KeyboardLayout {
        KeyboardLayout::for_device(&DeviceInfo {
            connected: true,
            model: "test".into(),
            max_rows: 6,
            max_columns: 17,
            led_index_max: 0,
            device_type: DeviceType::Keyboard80,
            layout: Layout::Ansi,
            v2_interface: true,
            uses_small_packets: false,
            uses_multi_report: false,
        })
    }

    fn frame(layout: &KeyboardLayout, red: u8) -> Frame {
        let mut frame = Frame::black();
        for key in layout.keys() {
            frame.set_coord(key.coord, Color::new(red, 0, 0));
        }
        frame
    }

    #[test]
    fn reference_pixels_define_endpoints_rounding_and_saturation() {
        let d = Color::new(100, 200, 255);
        let s = Color::new(200, 100, 0);
        for mode in [BlendMode::Normal, BlendMode::Add, BlendMode::Multiply] {
            assert_eq!(blend_color(d, s, 0, mode), d);
        }
        assert_eq!(blend_color(d, s, 255, BlendMode::Normal), s);
        assert_eq!(
            blend_color(d, s, 128, BlendMode::Normal),
            Color::new(150, 150, 127)
        );
        assert_eq!(
            blend_color(d, Color::BLACK, 255, BlendMode::Normal),
            Color::BLACK
        );
        assert_eq!(
            blend_color(d, Color::BLACK, 128, BlendMode::Normal),
            Color::new(50, 100, 127)
        );
        assert_eq!(
            blend_color(d, s, 128, BlendMode::Add),
            Color::new(200, 250, 255)
        );
        assert_eq!(
            blend_color(d, s, 255, BlendMode::Add),
            Color::new(255, 255, 255)
        );
        assert_eq!(
            blend_color(d, s, 255, BlendMode::Multiply),
            Color::new(78, 78, 0)
        );
        assert_eq!(
            blend_color(d, s, 128, BlendMode::Multiply),
            Color::new(89, 139, 127)
        );
        // Half channel steps round up/down as specified, not by truncation.
        assert_eq!(
            blend_color(Color::BLACK, Color::new(1, 1, 1), 128, BlendMode::Normal),
            Color::new(1, 1, 1)
        );
        assert_eq!(
            blend_color(Color::BLACK, Color::new(1, 1, 1), 127, BlendMode::Normal),
            Color::BLACK
        );
    }

    #[test]
    fn transparent_top_sparse_overlap_ties_and_expiry_keep_advancing_base() {
        let layout = layout();
        let key = &layout.keys()[0];
        let mut layers = [
            layer(&layout, 200, KeyMask::Coordinates(vec![key.coord]), -1),
            layer(&layout, 100, KeyMask::Whole, -1),
            layer(&layout, 255, KeyMask::Whole, i32::MAX),
        ];
        layers[0].opacity = 128;
        layers[0].expires = Some(Duration::from_secs(1));
        layers[2].opacity = 0;
        assert_eq!(winning_layer(&layers, key, Duration::ZERO), Some(0));
        assert_eq!(
            contributing_layers(&layers, key, Duration::ZERO),
            vec![1, 0]
        );
        assert_eq!(
            compose(frame(&layout, 20), &layout, &layers, Duration::ZERO)
                .get_coord(key.coord)
                .red,
            150
        );
        assert_eq!(
            compose(frame(&layout, 30), &layout, &layers, Duration::from_secs(1))
                .get_coord(key.coord)
                .red,
            100
        );
        layers[1].opacity = 0;
        assert_eq!(
            compose(frame(&layout, 20), &layout, &layers, Duration::ZERO)
                .get_coord(key.coord)
                .red,
            110
        );
        assert_eq!(
            compose(frame(&layout, 40), &layout, &layers, Duration::ZERO)
                .get_coord(key.coord)
                .red,
            120
        );
        assert_eq!(
            compose(Frame::black(), &layout, &layers, Duration::from_secs(1)),
            Frame::black()
        );
        layers[1].opacity = 255;
        layers[1].blend = BlendMode::Add;
        let capped = compose_capped(Frame::black(), &layout, &layers, Duration::ZERO, 120);
        assert_eq!(capped.get_coord(key.coord).red, 120);
        assert_eq!(capped.get_coord(layout.keys()[1].coord).red, 100);
    }

    fn layer(layout: &KeyboardLayout, red: u8, mask: KeyMask, priority: i32) -> Layer {
        Layer {
            opacity: 255,
            blend: Default::default(),
            frame: frame(layout, red),
            mask,
            priority,
            expires: None,
        }
    }

    #[test]
    fn disjoint_zones_coexist_and_uncovered_keys_retain_background() {
        let layout = layout();
        let layers = [
            layer(&layout, 10, KeyMask::Zones(vec![Zone::Function]), 1),
            layer(&layout, 20, KeyMask::Zones(vec![Zone::Navigation]), 9),
        ];
        let composed = compose(frame(&layout, 1), &layout, &layers, Duration::ZERO);
        for key in layout.keys() {
            assert_eq!(
                composed.get_coord(key.coord).red,
                match key.zone {
                    Zone::Function => 10,
                    Zone::Navigation => 20,
                    _ => 1,
                }
            );
        }
    }

    #[test]
    fn priority_and_first_declaration_win_only_on_overlapping_keys() {
        let layout = layout();
        let layers = [
            layer(&layout, 10, KeyMask::Whole, 1),
            layer(&layout, 20, KeyMask::Zones(vec![Zone::Function]), 9),
            layer(
                &layout,
                30,
                KeyMask::Zones(vec![Zone::Function, Zone::Navigation]),
                9,
            ),
        ];
        let composed = compose(frame(&layout, 1), &layout, &layers, Duration::ZERO);
        for key in layout.keys() {
            assert_eq!(
                composed.get_coord(key.coord).red,
                match key.zone {
                    Zone::Function => 20,
                    Zone::Navigation => 30,
                    _ => 10,
                }
            );
        }
    }

    #[test]
    fn deadlines_reveal_persistent_layer_then_current_background() {
        let layout = layout();
        let mut layers = [
            layer(&layout, 10, KeyMask::Whole, 1),
            layer(&layout, 20, KeyMask::Whole, 9),
        ];
        layers[1].expires = Some(Duration::from_secs(2));
        let coord = layout.keys()[0].coord;
        assert_eq!(
            compose(frame(&layout, 1), &layout, &layers, Duration::from_secs(1))
                .get_coord(coord)
                .red,
            20
        );
        assert_eq!(
            compose(frame(&layout, 2), &layout, &layers, Duration::from_secs(2))
                .get_coord(coord)
                .red,
            10
        );
        layers[0].expires = Some(Duration::from_secs(3));
        assert_eq!(
            compose(frame(&layout, 3), &layout, &layers, Duration::from_secs(3)),
            frame(&layout, 3)
        );
        assert_eq!(
            compose(
                frame(&layout, 4),
                &layout,
                &layers,
                Duration::from_secs(100)
            ),
            frame(&layout, 4)
        );
    }

    #[test]
    fn diagnostic_winners_share_signed_priority_ties_and_deadline_resolution() {
        let layout = layout();
        let mut layers = [
            layer(&layout, 10, KeyMask::Whole, i32::MIN),
            layer(&layout, 20, KeyMask::Whole, i32::MAX),
            layer(&layout, 30, KeyMask::Zones(vec![Zone::Function]), -1),
            layer(&layout, 40, KeyMask::Zones(vec![Zone::Function]), -1),
        ];
        layers[1].expires = Some(Duration::from_secs(1));
        for key in layout.keys() {
            assert_eq!(winning_layer(&layers, key, Duration::ZERO), Some(1));
            let expected = if key.zone == Zone::Function { 2 } else { 0 };
            assert_eq!(
                winning_layer(&layers, key, Duration::from_secs(1)),
                Some(expected)
            );
        }
        assert_eq!(winning_layer(&[], &layout.keys()[0], Duration::ZERO), None);
    }

    #[test]
    fn coordinate_masks_black_overlays_and_unmapped_cells_are_explicit() {
        let layout = layout();
        let coord = layout.keys()[0].coord;
        let unmapped = MatrixCoord { row: 0, column: 20 };
        let layers = [layer(
            &layout,
            0,
            KeyMask::Coordinates(vec![coord, unmapped]),
            0,
        )];
        let mut base = frame(&layout, 1);
        base.set_coord(unmapped, Color::new(42, 0, 0));
        let composed = compose(base, &layout, &layers, Duration::ZERO);
        assert_eq!(composed.get_coord(coord), Color::BLACK);
        assert_eq!(composed.get_coord(unmapped).red, 42);
        for key in &layout.keys()[1..] {
            assert_eq!(composed.get_coord(key.coord).red, 1);
        }
        let empty = [layer(&layout, 9, KeyMask::Zones(Vec::new()), 0)];
        assert_eq!(
            compose(frame(&layout, 1), &layout, &empty, Duration::ZERO),
            frame(&layout, 1)
        );
    }
}
