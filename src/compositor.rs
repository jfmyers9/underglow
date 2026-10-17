//! Hardware-independent, opaque overlays on a freshly rendered background.
//!
//! Callers own animation clocks, event identity and activation. Submit persistent
//! layers only while their state is active; timed layers use the caller's monotonic
//! clock. Composition never caches frames, queues alerts, or writes to hardware.
use crate::layout::{KeyPosition, KeyboardLayout, MatrixCoord, Zone};
use crate::render::Frame;
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

/// One opaque overlay. Black is a color, not transparency; the mask defines
/// which keys are covered. Greater priority wins; the first submitted layer
/// wins equal-priority conflicts, independently for each key.
#[derive(Clone, Debug)]
pub struct Layer {
    pub frame: Frame,
    pub mask: KeyMask,
    pub priority: i32,
    /// An absolute deadline on the same monotonic clock as `compose`'s `now`.
    /// `None` is persistent. A timed layer is inactive exactly at its deadline.
    pub expires: Option<Duration>,
}

impl Layer {
    pub fn is_active(&self, now: Duration) -> bool {
        self.expires.is_none_or(|expires| now < expires)
    }
}

/// Resolve one key to its index in the submitted declaration order. Diagnostics
/// can use the same arbitration as rendering without comparing RGB values.
pub fn winning_layer(layers: &[Layer], key: &KeyPosition, now: Duration) -> Option<usize> {
    let mut winner: Option<usize> = None;
    for (index, layer) in layers.iter().enumerate() {
        if layer.is_active(now)
            && layer.mask.contains(key)
            && winner.is_none_or(|winner| layer.priority > layers[winner].priority)
        {
            winner = Some(index);
        }
    }
    winner
}

/// Combine active layers onto the current background, leaving every uncovered
/// key (and non-key matrix cell) unchanged. The result is a single RGB frame.
pub fn compose(mut base: Frame, layout: &KeyboardLayout, layers: &[Layer], now: Duration) -> Frame {
    for key in layout.keys() {
        if let Some(index) = winning_layer(layers, key, now) {
            base.set_coord(key.coord, layers[index].frame.get_coord(key.coord));
        }
    }
    base
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

    fn layer(layout: &KeyboardLayout, red: u8, mask: KeyMask, priority: i32) -> Layer {
        Layer {
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
