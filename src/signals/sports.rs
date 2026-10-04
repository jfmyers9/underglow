use crate::layout::Zone;
use crate::render::{Color, Frame, RenderContext, pulse_wave};
use crate::signals::SignalProgram;
use crate::signals::external::{ExternalPollState, ExternalSnapshot, ExternalStatus, fetch_json};
use serde::Deserialize;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SportsConfig {
    #[serde(rename = "sports_api_url")]
    pub api_url: String,
    #[serde(rename = "sports_token_env")]
    pub token_env: String,
    pub favorites: Vec<String>,
    #[serde(rename = "sports_poll_seconds")]
    pub poll_seconds: u64,
    #[serde(rename = "sports_stale_seconds")]
    pub stale_seconds: u64,
}

impl Default for SportsConfig {
    fn default() -> Self {
        Self {
            api_url: String::new(),
            token_env: "SPORTS_API_TOKEN".to_string(),
            favorites: Vec::new(),
            poll_seconds: 60,
            stale_seconds: 300,
        }
    }
}

#[derive(Debug)]
pub struct SportsSignal {
    config: SportsConfig,
    state: ExternalSnapshot,
    poll: ExternalPollState,
}

impl SportsSignal {
    pub fn new(config: SportsConfig) -> Self {
        Self {
            config,
            state: ExternalSnapshot::idle("waiting for sports data"),
            poll: ExternalPollState::default(),
        }
    }

    fn poll_if_due(&mut self) {
        let now = Instant::now();
        if !self.poll.should_poll(now) {
            if let Some(snapshot) = self.poll.stale_snapshot(now, self.config.stale_seconds) {
                self.state = snapshot;
            }
            return;
        }

        match fetch_json(&self.config.api_url, &self.config.token_env) {
            Ok(value) => {
                let snapshot = normalize_sports(&value, &self.config.favorites);
                self.poll
                    .mark_success(&snapshot, now, self.config.poll_seconds);
                self.state = snapshot;
            }
            Err(error) => {
                eprintln!("sports-alerts poll failed: {error}");
                let snapshot = ExternalSnapshot::error("sports poll failed");
                self.poll
                    .mark_error(&snapshot, now, self.config.poll_seconds);
                self.state = snapshot;
            }
        }
    }

    fn color(&self) -> Color {
        match self.state.status {
            ExternalStatus::Running => Color::new(0, 180, 255),
            ExternalStatus::Positive => Color::new(0, 220, 80),
            ExternalStatus::Alert => Color::new(255, 220, 0),
            ExternalStatus::Negative | ExternalStatus::Error => Color::new(255, 32, 24),
            ExternalStatus::Stale => Color::new(80, 80, 80),
            ExternalStatus::Idle => Color::new(0, 48, 96),
        }
    }
}

impl SignalProgram for SportsSignal {
    fn tick(&mut self, _interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        self.poll_if_due();
        Ok(())
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        let mut frame = Frame::black();
        let base = self.color();
        let wave = pulse_wave(ctx.tick, 24);
        let pulse = match self.state.status {
            ExternalStatus::Alert | ExternalStatus::Positive => 128 + (wave / 2),
            _ => 128,
        };
        let color = base.scale(((u16::from(ctx.brightness) * u16::from(pulse)) / 255) as u8);
        let dim = base.scale(ctx.brightness / 10);

        for key in ctx.layout.keys() {
            frame.set_coord(key.coord, dim);
        }
        for key in ctx.layout.keys() {
            if matches!(key.zone, Zone::Navigation | Zone::Arrows | Zone::Function) {
                frame.set_coord(key.coord, color);
            }
        }
        frame
    }

    fn finished(&self) -> bool {
        false
    }

    fn shutdown(&mut self, _interrupted: bool) {}
}

pub fn normalize_sports(value: &Value, favorites: &[String]) -> ExternalSnapshot {
    let Some(events) = value.get("events").and_then(Value::as_array) else {
        return ExternalSnapshot::stale("sports payload had no events");
    };

    let mut selected = None;
    for event in events {
        let favorite = event
            .get("favorite")
            .and_then(Value::as_str)
            .or_else(|| event.get("team").and_then(Value::as_str))
            .unwrap_or("");
        if favorites.is_empty() || favorites.iter().any(|wanted| wanted == favorite) {
            selected = Some(event);
            break;
        }
    }

    let Some(event) = selected else {
        return ExternalSnapshot::idle("no favorite sports events found");
    };

    let id = event.get("id").and_then(Value::as_str).unwrap_or("unknown");
    let favorite = event
        .get("favorite")
        .and_then(Value::as_str)
        .or_else(|| event.get("team").and_then(Value::as_str))
        .unwrap_or("favorite");
    let status_text = event
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let score = event
        .get("score")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let opponent_score = event
        .get("opponent_score")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let previous_score = event
        .get("previous_score")
        .and_then(Value::as_i64)
        .unwrap_or(score);

    let (status, key_status, message) = if score > previous_score
        || event
            .get("favorite_scored")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        (
            ExternalStatus::Alert,
            "scored",
            format!("{favorite} scored"),
        )
    } else if score > opponent_score {
        (
            ExternalStatus::Positive,
            "leading",
            format!("{favorite} leading"),
        )
    } else if matches!(status_text, "live" | "in_progress" | "started") {
        (ExternalStatus::Running, "live", format!("{favorite} live"))
    } else if matches!(status_text, "final" | "complete") {
        (ExternalStatus::Idle, "final", format!("{favorite} final"))
    } else {
        (ExternalStatus::Idle, "idle", format!("{favorite} idle"))
    };

    ExternalSnapshot {
        status,
        event_key: format!("sports:{id}:{key_status}:{score}-{opponent_score}"),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::KeyboardLayout;
    use crate::render::{PaletteName, RenderContext};
    use crate::sdk::rgb::{DeviceInfo, DeviceType, Layout};

    fn info() -> DeviceInfo {
        DeviceInfo {
            connected: true,
            model: "test".to_string(),
            max_rows: 6,
            max_columns: 17,
            led_index_max: 0,
            device_type: DeviceType::Keyboard80,
            layout: Layout::Ansi,
            v2_interface: true,
            uses_small_packets: false,
            uses_multi_report: false,
        }
    }

    #[test]
    fn sports_normalizes_live_score_lead_and_final() {
        let favorites = vec!["WOO".to_string()];
        let live = serde_json::json!({ "events": [{ "id": "1", "favorite": "WOO", "status": "live", "score": 1, "opponent_score": 1, "previous_score": 1 }] });
        let scored = serde_json::json!({ "events": [{ "id": "1", "favorite": "WOO", "status": "live", "score": 2, "opponent_score": 1, "previous_score": 1 }] });
        let leading = serde_json::json!({ "events": [{ "id": "1", "favorite": "WOO", "status": "live", "score": 2, "opponent_score": 1, "previous_score": 2 }] });
        let final_event = serde_json::json!({ "events": [{ "id": "1", "favorite": "WOO", "status": "final", "score": 2, "opponent_score": 1, "previous_score": 2 }] });

        assert_eq!(
            normalize_sports(&live, &favorites).status,
            ExternalStatus::Running
        );
        assert_eq!(
            normalize_sports(&scored, &favorites).status,
            ExternalStatus::Alert
        );
        assert_eq!(
            normalize_sports(&leading, &favorites).status,
            ExternalStatus::Positive
        );
        assert_eq!(
            normalize_sports(&final_event, &favorites).event_key,
            "sports:1:leading:2-1"
        );
    }

    #[test]
    fn sports_event_keys_are_stable_for_repeated_events() {
        let favorites = vec!["WOO".to_string()];
        let event = serde_json::json!({ "events": [{ "id": "1", "favorite": "WOO", "status": "live", "score": 2, "opponent_score": 1, "previous_score": 1 }] });

        assert_eq!(
            normalize_sports(&event, &favorites).event_key,
            normalize_sports(&event, &favorites).event_key
        );
    }

    #[test]
    fn sports_renders_full_frame() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let signal = SportsSignal::new(SportsConfig::default());
        let frame = signal.render(&RenderContext {
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Wooting,
            tick: 0,
        });

        assert_eq!(frame.as_bytes().len(), crate::render::FRAME_BYTES);
    }
}
