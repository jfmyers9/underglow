//! Fading stars joined by thin links to their nearest visible star.
use super::{KEY_COUNT, KeyState, index, paint, visible};
use crate::layout::KeyPosition;
use crate::render::{Frame, RenderContext};

const HALF_LIFE: f32 = 2.4;

pub(super) fn advance(key: &mut KeyState, dt: f32, _pressed: bool) {
    key.energy = (key.energy * (-std::f32::consts::LN_2 * dt / HALF_LIFE).exp()).max(key.pressure);
}

pub(super) fn render(keys: &[KeyState; KEY_COUNT], ctx: &RenderContext<'_>) -> Frame {
    // Connections are derived from current fading intensities, never an ordered
    // history of key presses. At most one link per matrix slot, no allocation.
    let mut links = [None; KEY_COUNT];
    let energy = |key: &KeyPosition| index(key.coord).map_or(0.0, |i| keys[i].energy);
    for origin in ctx.layout.keys() {
        let Some(i) = index(origin.coord) else {
            continue;
        };
        if energy(origin) < 0.004 {
            continue;
        }
        let mut nearest_distance = f32::INFINITY;
        for other in ctx.layout.keys() {
            if other.coord == origin.coord || energy(other) < 0.004 {
                continue;
            }
            let distance = (origin.x - other.x).hypot(origin.y - other.y);
            if distance < nearest_distance {
                nearest_distance = distance;
                links[i] = Some(other);
            }
        }
    }
    let mut frame = Frame::black();
    for key in ctx
        .layout
        .keys()
        .iter()
        .filter(|key| visible(ctx, key.coord))
    {
        let mut intensity = energy(key);
        for origin in ctx.layout.keys() {
            let Some(i) = index(origin.coord) else {
                continue;
            };
            let Some(other) = links[i] else {
                continue;
            };
            let distance = segment_distance(key, origin, other);
            let line = (1.0 - distance / 0.65).max(0.0);
            intensity = intensity.max(0.3 * energy(origin).min(energy(other)) * line);
        }
        frame.set_coord(key.coord, paint(ctx, intensity, 0.8));
    }
    frame
}

fn segment_distance(point: &KeyPosition, a: &KeyPosition, b: &KeyPosition) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f32::EPSILON {
        return (point.x - a.x).hypot(point.y - a.y);
    }
    let t = (((point.x - a.x) * dx + (point.y - a.y) * dy) / length_squared).clamp(0.0, 1.0);
    (point.x - a.x - t * dx).hypot(point.y - a.y - t * dy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::{ReactiveKind, ReactiveSimulation, tests::frame};
    use crate::render::Color;

    #[test]
    fn two_stars_create_a_fading_link_not_a_flood() {
        let mut sim = ReactiveSimulation::new(ReactiveKind::Constellation);
        sim.advance(0.0, [(0x04, 1.0)]);
        assert_eq!(frame(&sim, 255).get(3, 4), Color::BLACK);
        sim.advance(0.0, [(0x0d, 1.0)]);
        let bright = frame(&sim, 255);
        assert_ne!(bright.get(3, 4), Color::BLACK);
        assert!(bright.get(3, 1).red > bright.get(3, 4).red);
        assert_eq!(bright.get(0, 4), Color::BLACK);
        sim.advance(HALF_LIFE, []);
        assert!(frame(&sim, 255).get(3, 4).red < bright.get(3, 4).red);
        sim.advance(100.0, []);
        assert_eq!(frame(&sim, 255), Frame::black());
    }

    #[test]
    fn held_stars_do_not_accumulate_and_repress_restores_brightness() {
        let mut sim = ReactiveSimulation::new(ReactiveKind::Constellation);
        sim.advance(0.0, [(0x04, 0.7)]);
        let initial = frame(&sim, 255);
        for _ in 0..100 {
            sim.advance(0.02, [(0x04, 0.7)]);
        }
        assert_eq!(frame(&sim, 255), initial);
        sim.advance(2.0, []);
        assert_ne!(frame(&sim, 255), initial);
        sim.advance(0.0, [(0x04, 0.7)]);
        assert_eq!(frame(&sim, 255), initial);
    }
}
