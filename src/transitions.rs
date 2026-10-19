//! Bounded, hardware-free appearance crossfades on the evaluator's clock.
//! Only explicit preparation changes history; diagnostics and rendering are reads.
use crate::render::{Color, Frame, MAX_COLUMNS, MAX_ROWS};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const MAX_DURATION_MS: u64 = 5_000;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum TransitionCurve {
    #[default]
    Linear,
    SmoothStep,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Transition {
    pub duration_ms: u64,
    pub curve: TransitionCurve,
}

impl Transition {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn validate(self) -> Result<(), String> {
        if self.duration_ms > MAX_DURATION_MS {
            return Err(format!(
                "transition duration_ms must be 0..={MAX_DURATION_MS}"
            ));
        }
        Ok(())
    }

    pub fn progress(self, started: Duration, now: Duration) -> f64 {
        let duration = self.duration_ms.min(MAX_DURATION_MS);
        if duration == 0 {
            return 1.0;
        }
        let t = (now.saturating_sub(started).as_secs_f64() / (duration as f64 / 1000.0))
            .clamp(0.0, 1.0);
        match self.curve {
            TransitionCurve::Linear => t,
            TransitionCurve::SmoothStep => t * t * (3.0 - 2.0 * t),
        }
    }
}

/// Interpolate renderer coverage and premultiplied RGB to avoid dark/color halos.
/// Opaque frames retain the original round-nearest RGB interpolation exactly.
pub fn interpolate(from: &Frame, to: &Frame, progress: f64) -> Frame {
    let t = if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        1.0
    };
    if t == 0.0 {
        return from.clone();
    }
    if t == 1.0 {
        return to.clone();
    }
    let mut frame = Frame::black();
    for row in 0..MAX_ROWS {
        for column in 0..MAX_COLUMNS {
            let a = from.get(row, column);
            let b = to.get(row, column);
            let from_alpha = f64::from(from.alpha(row, column));
            let to_alpha = f64::from(to.alpha(row, column));
            let alpha = from_alpha * (1.0 - t) + to_alpha * t;
            let channel = |a: u8, b: u8| {
                if from_alpha == 255.0 && to_alpha == 255.0 {
                    (f64::from(a) * (1.0 - t) + f64::from(b) * t).round() as u8
                } else if alpha == 0.0 {
                    0
                } else {
                    ((f64::from(a) * from_alpha * (1.0 - t) + f64::from(b) * to_alpha * t) / alpha)
                        .round() as u8
                }
            };
            frame.set_alpha(row, column, alpha.round() as u8);
            frame.set(
                row,
                column,
                Color::new(
                    channel(a.red, b.red),
                    channel(a.green, b.green),
                    channel(a.blue, b.blue),
                ),
            );
        }
    }
    frame
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransitionInspection {
    /// Appearance IDs; the empty string denotes the base/fallback appearance.
    pub from: String,
    pub to: String,
    /// Eased progress, not wall-clock time or a new activation deadline.
    pub progress: f64,
}

/// At most two frames per logical layer, even during arbitrarily fast retargets.
/// The outgoing frame is frozen; the incoming renderer continues animating.
#[derive(Default)]
pub struct FrameTransition {
    selected: Option<String>,
    outgoing: Option<Frame>,
    visible: Option<Frame>,
    started: Duration,
    inspection: Option<TransitionInspection>,
    // Provenance, not a nested transition tree. Valid setups have at most 17
    // appearances; retain these IDs so an interrupted ancestor can expire too.
    outgoing_ids: Vec<String>,
}

impl FrameTransition {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn prepare(&mut self, now: Duration, selected: &str, incoming: Frame, config: Transition) {
        if self.selected.as_deref() != Some(selected) {
            let previous = self.selected.replace(selected.to_owned());
            self.outgoing = self.visible.take().filter(|_| config.duration_ms != 0);
            if self.outgoing.is_none() {
                self.outgoing_ids.clear();
            } else if let Some(id) = &previous
                && !self.outgoing_ids.contains(id)
            {
                if self.outgoing_ids.len() < 17 {
                    self.outgoing_ids.push(id.clone());
                } else {
                    // Defensive bound for direct helper callers outside setup validation.
                    self.outgoing = None;
                    self.outgoing_ids.clear();
                }
            }
            self.started = now;
            self.inspection =
                previous
                    .filter(|_| self.outgoing.is_some())
                    .map(|from| TransitionInspection {
                        from,
                        to: selected.to_owned(),
                        progress: 0.0,
                    });
        }
        let progress = config.progress(self.started, now);
        self.visible = Some(match self.outgoing.as_ref() {
            Some(outgoing) if progress < 1.0 => interpolate(outgoing, &incoming, progress),
            _ => incoming,
        });
        if progress >= 1.0 {
            self.outgoing = None;
            self.inspection = None;
            self.outgoing_ids.clear();
        } else if let Some(inspection) = &mut self.inspection {
            inspection.progress = progress;
        }
    }

    pub fn frame(&self) -> Option<&Frame> {
        self.visible.as_ref()
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn history_ids(&self) -> impl Iterator<Item = &str> {
        self.selected
            .iter()
            .map(String::as_str)
            .chain(self.outgoing_ids.iter().map(String::as_str))
    }

    pub fn inspection(&self) -> Option<&TransitionInspection> {
        self.inspection.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(red: u8) -> Frame {
        let mut frame = Frame::black();
        frame.set(0, 0, Color::new(red, 0, 0));
        frame
    }

    fn config(curve: TransitionCurve) -> Transition {
        Transition {
            duration_ms: 1000,
            curve,
        }
    }

    #[test]
    fn endpoints_midpoint_curves_and_bounds() {
        let linear = config(TransitionCurve::Linear);
        assert_eq!(linear.progress(Duration::from_secs(1), Duration::ZERO), 0.0);
        assert_eq!(
            linear.progress(Duration::ZERO, Duration::from_millis(500)),
            0.5
        );
        assert_eq!(linear.progress(Duration::ZERO, Duration::MAX), 1.0);
        assert_eq!(
            Transition::default().progress(Duration::ZERO, Duration::ZERO),
            1.0
        );
        assert_eq!(
            config(TransitionCurve::SmoothStep)
                .progress(Duration::ZERO, Duration::from_millis(250)),
            0.15625
        );
        assert!(
            Transition {
                duration_ms: 5001,
                ..linear
            }
            .validate()
            .is_err()
        );
        assert!(
            Transition {
                duration_ms: 5000,
                ..linear
            }
            .validate()
            .is_ok()
        );
        for (t, red) in [
            (-1.0, 0),
            (0.0, 0),
            (0.5, 128),
            (1.0, 255),
            (2.0, 255),
            (f64::NAN, 255),
        ] {
            assert_eq!(interpolate(&frame(0), &frame(255), t).get(0, 0).red, red);
        }
    }

    #[test]
    fn retarget_is_continuous_bounded_and_reads_do_not_advance() {
        let mut fade = FrameTransition::default();
        let config = config(TransitionCurve::Linear);
        fade.prepare(Duration::ZERO, "idle", frame(0), config);
        fade.prepare(Duration::ZERO, "working", frame(200), config);
        fade.prepare(Duration::from_millis(500), "working", frame(200), config);
        assert_eq!(fade.frame(), Some(&frame(100)));
        for _ in 0..10 {
            assert_eq!(fade.frame(), Some(&frame(100)));
            assert_eq!(fade.inspection().unwrap().progress, 0.5);
        }
        fade.prepare(Duration::from_millis(500), "finished", frame(0), config);
        assert_eq!(fade.frame(), Some(&frame(100)));
        fade.prepare(Duration::from_millis(1000), "finished", frame(0), config);
        assert_eq!(fade.frame(), Some(&frame(50)));
        fade.prepare(Duration::from_millis(1500), "finished", frame(0), config);
        assert_eq!(fade.frame(), Some(&frame(0)));
        assert!(fade.inspection().is_none());
        fade.clear();
        assert!(fade.frame().is_none());
        fade.prepare(Duration::from_secs(9), "idle", frame(90), config);
        assert_eq!(fade.frame(), Some(&frame(90)));
        assert!(fade.inspection().is_none());
    }

    #[test]
    fn instant_default_never_retains_old_pixels() {
        let mut fade = FrameTransition::default();
        fade.prepare(Duration::ZERO, "a", frame(255), Transition::default());
        fade.prepare(Duration::ZERO, "b", frame(0), Transition::default());
        assert_eq!(fade.frame(), Some(&frame(0)));
        assert!(fade.inspection().is_none());
    }

    #[test]
    fn interrupted_provenance_is_bounded_and_retains_older_expiry_dependencies() {
        let mut fade = FrameTransition::default();
        let config = config(TransitionCurve::Linear);
        for id in 0..1000 {
            fade.prepare(
                Duration::from_millis(id),
                &id.to_string(),
                frame(100),
                config,
            );
            assert!(fade.history_ids().count() <= 18);
        }
        fade.clear();
        for id in ["a", "b", "c"] {
            fade.prepare(Duration::ZERO, id, frame(100), config);
        }
        assert_eq!(fade.history_ids().collect::<Vec<_>>(), ["c", "a", "b"]);
        fade.prepare(Duration::from_secs(1), "c", frame(100), config);
        assert_eq!(fade.history_ids().collect::<Vec<_>>(), ["c"]);
    }

    #[test]
    fn same_clock_preparation_is_idempotent_and_incoming_keeps_animating() {
        let mut fade = FrameTransition::default();
        let config = config(TransitionCurve::Linear);
        fade.prepare(Duration::ZERO, "idle", frame(0), config);
        fade.prepare(Duration::ZERO, "working", frame(100), config);
        for _ in 0..20 {
            fade.prepare(Duration::from_millis(500), "working", frame(100), config);
            assert_eq!(fade.frame(), Some(&frame(50)));
            assert_eq!(fade.inspection().unwrap().progress, 0.5);
        }
        fade.prepare(Duration::from_millis(750), "working", frame(200), config);
        assert_eq!(fade.frame(), Some(&frame(150)));
    }

    #[test]
    fn strict_declarative_settings_round_trip_without_executable_fields() {
        assert_eq!(
            serde_json::from_str::<Transition>("{}").unwrap(),
            Transition::default()
        );
        let transition = config(TransitionCurve::SmoothStep);
        let value = serde_json::to_value(transition).unwrap();
        assert_eq!(value["curve"], "smooth-step");
        assert_eq!(
            serde_json::from_value::<Transition>(value).unwrap(),
            transition
        );
        let toml = toml::to_string_pretty(&transition).unwrap();
        assert_eq!(toml::from_str::<Transition>(&toml).unwrap(), transition);
        assert!(serde_json::from_str::<Transition>(r#"{"duration_ms":-1}"#).is_err());
        assert!(serde_json::from_str::<Transition>(r#"{"curve":"bounce"}"#).is_err());
        assert!(serde_json::from_str::<Transition>(r#"{"command":"do-not-run"}"#).is_err());
    }
}
