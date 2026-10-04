use crate::layout::Zone;
use crate::render::{Color, Frame, RenderContext, pulse_wave};
use crate::signals::SignalProgram;
use crate::signals::external::{ExternalPollState, ExternalSnapshot, ExternalStatus, fetch_json};
use serde::Deserialize;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MarketConfig {
    #[serde(rename = "market_api_url")]
    pub api_url: String,
    #[serde(rename = "market_token_env")]
    pub token_env: String,
    pub watchlist: Vec<String>,
    pub threshold_percent: f64,
    #[serde(rename = "market_poll_seconds")]
    pub poll_seconds: u64,
    #[serde(rename = "market_stale_seconds")]
    pub stale_seconds: u64,
}

impl Default for MarketConfig {
    fn default() -> Self {
        Self {
            api_url: String::new(),
            token_env: "MARKET_API_TOKEN".to_string(),
            watchlist: Vec::new(),
            threshold_percent: 2.0,
            poll_seconds: 300,
            stale_seconds: 900,
        }
    }
}

#[derive(Debug)]
pub struct MarketSignal {
    config: MarketConfig,
    state: ExternalSnapshot,
    poll: ExternalPollState,
}

impl MarketSignal {
    pub fn new(config: MarketConfig) -> Self {
        Self {
            config,
            state: ExternalSnapshot::idle("waiting for market data"),
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
                let snapshot = normalize_market(
                    &value,
                    &self.config.watchlist,
                    self.config.threshold_percent,
                );
                self.poll
                    .mark_success(&snapshot, now, self.config.poll_seconds);
                self.state = snapshot;
            }
            Err(error) => {
                eprintln!("market-pulse poll failed: {error}");
                let snapshot = ExternalSnapshot::error("market poll failed");
                self.poll
                    .mark_error(&snapshot, now, self.config.poll_seconds);
                self.state = snapshot;
            }
        }
    }

    fn color(&self) -> Color {
        match self.state.status {
            ExternalStatus::Positive => Color::new(0, 220, 80),
            ExternalStatus::Negative => Color::new(255, 32, 24),
            ExternalStatus::Alert => Color::new(255, 220, 0),
            ExternalStatus::Stale => Color::new(80, 80, 80),
            ExternalStatus::Error => Color::new(255, 64, 0),
            ExternalStatus::Idle | ExternalStatus::Running => Color::new(0, 90, 180),
        }
    }
}

impl SignalProgram for MarketSignal {
    fn tick(&mut self, _interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        self.poll_if_due();
        Ok(())
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        let mut frame = Frame::black();
        let base = self.color();
        let wave = pulse_wave(ctx.tick, 48);
        let pulse = match self.state.status {
            ExternalStatus::Alert | ExternalStatus::Error => 128 + (wave / 2),
            _ => 128,
        };
        let color = base.scale(((u16::from(ctx.brightness) * u16::from(pulse)) / 255) as u8);
        let dim = base.scale(ctx.brightness / 10);

        for key in ctx.layout.keys() {
            frame.set_coord(key.coord, dim);
        }
        for key in ctx.layout.keys() {
            if matches!(key.zone, Zone::Function | Zone::Navigation) {
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

pub fn normalize_market(
    value: &Value,
    watchlist: &[String],
    threshold_percent: f64,
) -> ExternalSnapshot {
    if value
        .get("market_open")
        .and_then(Value::as_bool)
        .is_some_and(|open| !open)
    {
        return ExternalSnapshot {
            status: ExternalStatus::Idle,
            event_key: "market:closed".to_string(),
            message: "market closed".to_string(),
        };
    }

    let Some(tickers) = value.get("tickers").and_then(Value::as_array) else {
        return ExternalSnapshot::stale("market payload had no tickers");
    };

    let mut selected = None;
    for ticker in tickers {
        let symbol = ticker.get("symbol").and_then(Value::as_str).unwrap_or("");
        if watchlist.is_empty() || watchlist.iter().any(|wanted| wanted == symbol) {
            selected = Some(ticker);
            break;
        }
    }

    let Some(ticker) = selected else {
        return ExternalSnapshot::idle("no watched market tickers found");
    };

    let symbol = ticker
        .get("symbol")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let change = ticker
        .get("change_percent")
        .or_else(|| ticker.get("change"))
        .and_then(Value::as_f64)
        .unwrap_or_else(|| calculate_change_percent(ticker).unwrap_or(0.0));
    let abs_change = change.abs();
    let status = if abs_change >= threshold_percent.max(0.0) {
        ExternalStatus::Alert
    } else if change >= 0.0 {
        ExternalStatus::Positive
    } else {
        ExternalStatus::Negative
    };

    ExternalSnapshot {
        status,
        event_key: format!("market:{symbol}:{change:.2}"),
        message: format!("{symbol} {change:+.2}%"),
    }
}

fn calculate_change_percent(ticker: &Value) -> Option<f64> {
    let price = ticker.get("price")?.as_f64()?;
    let previous = ticker.get("previous_close")?.as_f64()?;
    if previous == 0.0 {
        None
    } else {
        Some(((price - previous) / previous) * 100.0)
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
    fn market_normalizes_up_down_threshold_and_closed() {
        let watchlist = vec!["ABC".to_string()];
        let up = serde_json::json!({ "market_open": true, "tickers": [{ "symbol": "ABC", "change_percent": 1.0 }] });
        let down = serde_json::json!({ "market_open": true, "tickers": [{ "symbol": "ABC", "change_percent": -1.0 }] });
        let threshold = serde_json::json!({ "market_open": true, "tickers": [{ "symbol": "ABC", "change_percent": 3.0 }] });
        let closed = serde_json::json!({ "market_open": false, "tickers": [] });

        assert_eq!(
            normalize_market(&up, &watchlist, 2.0).status,
            ExternalStatus::Positive
        );
        assert_eq!(
            normalize_market(&down, &watchlist, 2.0).status,
            ExternalStatus::Negative
        );
        assert_eq!(
            normalize_market(&threshold, &watchlist, 2.0).status,
            ExternalStatus::Alert
        );
        assert_eq!(
            normalize_market(&closed, &watchlist, 2.0).event_key,
            "market:closed"
        );
    }

    #[test]
    fn market_renders_full_frame() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let signal = MarketSignal::new(MarketConfig::default());
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
