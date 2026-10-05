//! A stationary soft glow, not a propagating ring.
use super::{KEY_COUNT, KeyState, index, paint, visible};
use crate::render::{Frame, RenderContext};

const HALF_LIFE: f32 = 1.4;
const RADIUS: f32 = 1.65;

pub(super) fn advance(key: &mut KeyState, dt: f32, _pressed: bool) {
    key.energy = (key.energy * (-std::f32::consts::LN_2 * dt / HALF_LIFE).exp()).max(key.pressure);
}

pub(super) fn render(keys: &[KeyState; KEY_COUNT], ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    for key in ctx
        .layout
        .keys()
        .iter()
        .filter(|key| visible(ctx, key.coord))
    {
        let mut intensity = 0.0_f32;
        for origin in ctx.layout.keys() {
            let Some(index) = index(origin.coord) else {
                continue;
            };
            let distance = (key.x - origin.x).hypot(key.y - origin.y);
            let glow = (1.0 - distance / RADIUS).max(0.0).powi(2);
            intensity = intensity.max(keys[index].energy * glow);
        }
        frame.set_coord(key.coord, paint(ctx, intensity, 0.7));
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::{ReactiveKind, ReactiveSimulation, tests::frame};
    use crate::render::Color;

    #[test]
    fn glow_lingers_locally_without_expanding_and_repress_restores_it() {
        let mut sim = ReactiveSimulation::new(ReactiveKind::Afterimage);
        sim.advance(0.0, [(0x09, 1.0)]);
        let bright = frame(&sim, 255);
        assert!(bright.get(3, 4).red > bright.get(3, 5).red);
        assert_ne!(bright.get(3, 5), Color::BLACK);
        assert_eq!(bright.get(3, 6), Color::BLACK);
        for _ in 0..10 {
            sim.advance(0.2, []);
            assert_eq!(frame(&sim, 255).get(3, 6), Color::BLACK);
        }
        assert!(frame(&sim, 255).get(3, 4).red < bright.get(3, 4).red);
        sim.advance(0.0, [(0x09, 1.0)]);
        assert_eq!(frame(&sim, 255), bright);
        sim.advance(100.0, []);
        assert_eq!(frame(&sim, 255), Frame::black());
    }

    #[test]
    fn held_glow_tracks_pressure_without_accumulating() {
        let mut sim = ReactiveSimulation::new(ReactiveKind::Afterimage);
        sim.advance(0.0, [(0x09, 0.4)]);
        let start = frame(&sim, 255);
        for _ in 0..60 {
            sim.advance(1.0 / 60.0, [(0x09, 0.4)]);
        }
        assert_eq!(frame(&sim, 255), start);
        sim.advance(0.0, [(0x09, 0.9)]);
        assert!(frame(&sim, 255).get(3, 4).red > start.get(3, 4).red);
    }
}
