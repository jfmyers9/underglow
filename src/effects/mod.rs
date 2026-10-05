//! Hardware-independent decorative effects with shared elapsed-time interpolation.

mod breath;
mod comet;
mod matrix;
mod rainbow;
mod row_test;

#[cfg(test)]
mod test_support;

use crate::render::{Color, Frame, RenderContext};
use clap::ValueEnum;
use serde::Deserialize;
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum EffectKind {
    RowTest,
    #[default]
    Rainbow,
    Comet,
    Matrix,
    Breath,
}

impl fmt::Display for EffectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            self.to_possible_value()
                .expect("effect has value")
                .get_name()
        )
    }
}

impl EffectKind {
    pub fn render(self, ctx: &RenderContext<'_>) -> Frame {
        match self {
            Self::RowTest => row_test::render(ctx),
            Self::Rainbow => rainbow::render(ctx),
            Self::Comet => comet::render(ctx),
            Self::Matrix => matrix::render(ctx),
            Self::Breath => breath::render(ctx),
        }
    }
}

/// Interpolate logical steps so render FPS improves smoothness, not pace.
/// Each effect owns its baseline rate and loop period.
fn interpolate_steps(
    ctx: &RenderContext<'_>,
    steps_per_second: f64,
    period: u32,
    render_step: fn(&RenderContext<'_>) -> Frame,
) -> Frame {
    let phase = (ctx.animation_seconds * steps_per_second).rem_euclid(f64::from(period));
    let mut sample = ctx.clone();
    sample.tick = phase.floor() as u32;
    let mut frame = render_step(&sample);
    let fraction = phase.fract();
    if fraction > 0.0 {
        sample.tick = (sample.tick + 1) % period;
        let next = render_step(&sample);
        for row in 0..crate::render::MAX_ROWS {
            for column in 0..crate::render::MAX_COLUMNS {
                let a = frame.get(row, column);
                let b = next.get(row, column);
                let blend = |a: u8, b: u8| {
                    (f64::from(a) + (f64::from(b) - f64::from(a)) * fraction).round() as u8
                };
                frame.set(
                    row,
                    column,
                    Color::new(
                        blend(a.red, b.red),
                        blend(a.green, b.green),
                        blend(a.blue, b.blue),
                    ),
                );
            }
        }
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::test_support::info;
    use super::*;
    use crate::layout::KeyboardLayout;
    use crate::render::{FRAME_BYTES, PaletteName, RenderContext};

    #[test]
    fn effect_pace_is_elapsed_time_not_render_tick() {
        let info = info(6, 17);
        let layout = KeyboardLayout::for_device(&info);
        for effect in [
            EffectKind::Rainbow,
            EffectKind::Comet,
            EffectKind::Matrix,
            EffectKind::Breath,
        ] {
            let mut context = RenderContext {
                info: &info,
                layout: &layout,
                brightness: 255,
                palette: PaletteName::Ocean,
                tick: 0,
                animation_seconds: 1.25,
            };
            let expected = effect.render(&context);
            for fps in [5, 10, 30, 60, 120] {
                context.tick = (fps as f64 * 1.25) as u32;
                assert_eq!(effect.render(&context), expected, "{effect:?} at {fps} FPS");
            }
            context.animation_seconds = 0.0;
            assert_ne!(effect.render(&context), expected, "{effect:?} must animate");
        }
    }

    #[test]
    fn all_effects_obey_frame_bounds_brightness_and_determinism_contract() {
        use crate::device::DeviceType;
        use crate::render::{MAX_COLUMNS, MAX_ROWS};

        for (rows, columns, device_type) in [
            (1, 1, DeviceType::Keyboard60),
            (1, 3, DeviceType::Keypad3Key),
            (5, 15, DeviceType::Keyboard60),
            (6, 21, DeviceType::KeyboardFullSize),
            (6, 17, DeviceType::Keyboard80),
        ] {
            let mut info = info(rows, columns);
            info.device_type = device_type;
            let layout = KeyboardLayout::for_device(&info);
            for &effect in EffectKind::value_variants() {
                for &palette in PaletteName::value_variants() {
                    for brightness in [0, 1, 96, 255] {
                        for animation_seconds in [0.0, 0.05, 1.25, 1_000_000.125] {
                            let context = RenderContext {
                                animation_seconds,
                                info: &info,
                                layout: &layout,
                                brightness,
                                palette,
                                tick: 5,
                            };
                            let frame = effect.render(&context);
                            assert_eq!(frame.as_bytes().len(), FRAME_BYTES);
                            assert_eq!(frame, effect.render(&context), "{effect}: deterministic");
                            assert!(
                                frame.as_bytes().iter().all(|&c| c <= brightness),
                                "{effect}: brightness {brightness}"
                            );
                            for row in 0..MAX_ROWS {
                                for column in 0..MAX_COLUMNS {
                                    if row >= usize::from(rows) || column >= usize::from(columns) {
                                        assert_eq!(
                                            frame.get(row, column),
                                            Color::BLACK,
                                            "{effect}: outside {rows}x{columns}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn persisted_and_cli_ids_remain_compatible() {
        // Preserve old identities without preventing additional variants.
        for (effect, id) in [
            (EffectKind::RowTest, "row-test"),
            (EffectKind::Rainbow, "rainbow"),
            (EffectKind::Comet, "comet"),
            (EffectKind::Matrix, "matrix"),
            (EffectKind::Breath, "breath"),
        ] {
            assert_eq!(effect.to_string(), id);
            assert_eq!(EffectKind::from_str(id, false).unwrap(), effect);
            assert_eq!(
                serde_json::from_str::<EffectKind>(&format!("\"{id}\"")).unwrap(),
                effect
            );
        }
    }
}
