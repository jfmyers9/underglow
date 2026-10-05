//! Repeated press edges accumulate warmth; holding never adds repeat events.
use super::{KEY_COUNT, KeyState, index, paint, visible};
use crate::render::{Frame, RenderContext};

const HALF_LIFE: f32 = 20.0;
const HEAT_PER_PRESS: f32 = 0.2;

pub(super) fn advance(key: &mut KeyState, dt: f32, pressed: bool) {
    key.energy *= (-std::f32::consts::LN_2 * dt / HALF_LIFE).exp();
    if pressed {
        key.energy = (key.energy + HEAT_PER_PRESS).min(1.0);
    }
}

pub(super) fn render(keys: &[KeyState; KEY_COUNT], ctx: &RenderContext<'_>) -> Frame {
    let mut frame = Frame::black();
    for key in ctx
        .layout
        .keys()
        .iter()
        .filter(|key| visible(ctx, key.coord))
    {
        let Some(index) = index(key.coord) else {
            continue;
        };
        let heat = keys[index].energy;
        frame.set_coord(key.coord, paint(ctx, heat.sqrt(), heat));
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::{ReactiveKind, ReactiveSimulation, tests::frame};

    #[test]
    fn repeated_presses_warm_but_a_held_key_cools() {
        let mut sim = ReactiveSimulation::new(ReactiveKind::Heatmap);
        sim.advance(0.0, [(0x04, 1.0)]);
        let first = sim.keys[52].energy;
        sim.advance(HALF_LIFE, [(0x04, 1.0)]);
        assert!((sim.keys[52].energy - first * 0.5).abs() < 1e-6);
        sim.advance(0.0, []);
        sim.advance(0.0, [(0x04, 1.0)]);
        assert!(sim.keys[52].energy > first);
        for _ in 0..100 {
            sim.advance(0.0, []);
            sim.advance(0.0, [(0x04, 1.0)]);
        }
        assert_eq!(sim.keys[52].energy, 1.0);
        assert_eq!(frame(&sim, 255).get(3, 2), crate::render::Color::BLACK);
        sim.advance(1000.0, []);
        assert_eq!(frame(&sim, 255), Frame::black());
    }

    #[test]
    fn hysteresis_rejects_chatter_and_pressure_does_not_count_as_repeats() {
        let mut sim = ReactiveSimulation::new(ReactiveKind::Heatmap);
        sim.advance(0.0, [(0x04, 0.05)]);
        for pressure in [0.04, 0.03, 0.08, 1.0, 0.02] {
            sim.advance(0.0, [(0x04, pressure)]);
        }
        assert_eq!(sim.keys[52].energy, HEAT_PER_PRESS);
        sim.advance(0.0, [(0x04, 0.01)]);
        sim.advance(0.0, [(0x04, 0.05)]);
        assert_eq!(sim.keys[52].energy, 2.0 * HEAT_PER_PRESS);
    }
}
