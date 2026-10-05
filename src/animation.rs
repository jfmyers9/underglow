//! Wall-clock animation time, independent of how often frames are rendered.
use std::time::Duration;

pub const DEFAULT_SPEED: u32 = 100;

pub fn supports_speed(mode: &str) -> bool {
    matches!(mode, "comet" | "rainbow" | "matrix" | "breath")
}

#[derive(Clone, Debug)]
pub struct AnimationClock {
    anchor: Option<Duration>,
    seconds: f64,
    speed: u32,
}

impl Default for AnimationClock {
    fn default() -> Self {
        Self {
            anchor: None,
            seconds: 0.0,
            speed: DEFAULT_SPEED,
        }
    }
}

impl AnimationClock {
    pub fn advance_to(&mut self, now: Duration) -> f64 {
        let anchor = *self.anchor.get_or_insert(now);
        // Measure from the last speed change, not by summing per-frame deltas:
        // this avoids FPS-dependent rounding drift as well as tick-based motion.
        self.seconds + now.saturating_sub(anchor).as_secs_f64() * f64::from(self.speed) / 100.0
    }

    /// Account for time at the old speed before changing it; never reset phase.
    pub fn set_speed(&mut self, now: Duration, speed: u32) -> f64 {
        let seconds = self.advance_to(now);
        let speed = speed.clamp(10, 400);
        if speed != self.speed {
            self.seconds = seconds;
            self.anchor = Some(now);
            self.speed = speed;
        }
        seconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rate_changes_do_not_change_elapsed_animation_time() {
        for fps in [5, 10, 30, 60, 120] {
            let mut clock = AnimationClock::default();
            clock.set_speed(Duration::ZERO, 100);
            for frame in 0..=fps * 4 {
                clock.advance_to(Duration::from_secs_f64(f64::from(frame) / f64::from(fps)));
            }
            assert!((clock.advance_to(Duration::from_secs(4)) - 4.0).abs() < 1e-8);
            assert_eq!(clock.set_speed(Duration::from_secs(4), 200), 4.0);
            assert!((clock.advance_to(Duration::from_secs(5)) - 6.0).abs() < 1e-8);
        }
    }
}
