//! Read-only connector reports and isolated synthetic previews. Never opens an SDK.
use crate::config::AppConfig;
use crate::ingress::Observation;
use crate::signals::ProgramResult;
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;
use underglow::automation::{AutomationRuntime, RuleDiagnostic};
use underglow::compositor::compose;
use underglow::device::DeviceInfo;
use underglow::external::{Message, Payload, Registry};
use underglow::layout::KeyboardLayout;
use underglow::render::RenderContext;

pub(crate) fn report(
    observation: &Observation,
    rules: Vec<RuleDiagnostic>,
    evaluation: &str,
    lighting_applied: bool,
    layout: &str,
) -> Value {
    let now = observation.now;
    let instances: Vec<_> = observation
        .snapshot
        .states
        .iter()
        .map(|s| {
            json!({
                "source":s.identity.source, "instance":s.identity.instance, "state":s.state,
                "sequence":s.sequence, "age_ms":now.saturating_sub(s.updated_at).as_millis(),
                "lease_remaining_ms":s.expires_at.saturating_sub(now).as_millis(),
                "available":s.expires_at > now,
            })
        })
        .collect();
    let events: Vec<_> = observation
        .events
        .events
        .iter()
        .map(|e| {
            json!({
                "source":e.identity.source, "instance":e.identity.instance, "event":e.event,
                "age_ms":now.saturating_sub(e.received_at).as_millis(),
                "lease_remaining_ms":e.expires_at.saturating_sub(now).as_millis(),
            })
        })
        .collect();
    let rules: Vec<_> = rules
        .into_iter()
        .map(|r| {
            let obscured = r.active && r.visible_keys < r.targeted_keys;
            let mut value = serde_json::to_value(r).expect("rule diagnostics serialize");
            value["obscured"] = json!(obscured);
            value
        })
        .collect();
    json!({"schema_version":1,"now":now,"evaluation":evaluation,
        "lighting_applied":lighting_applied,"layout":layout,
        "instances":instances,"events":events,"rules":rules,
        "visibility_scope":"live reports include legacy overlay competition; potential/synthetic reports include external automation only",
        "availability_note":"Available means an unexpired reported state, not terminal focus or process liveness"})
}

#[derive(Debug, Args)]
pub struct DiagnosticsOptions {
    #[command(subcommand)]
    command: DiagnosticsCommand,
}

#[derive(Debug, Subcommand)]
enum DiagnosticsCommand {
    /// Render an isolated synthetic source over the configured base effect. No IPC or commands.
    Preview {
        #[arg(long)]
        config: PathBuf,
        #[arg(long, default_value = "pi")]
        source: String,
        #[arg(long, default_value = "preview")]
        instance: String,
        #[arg(long, default_value = "working")]
        state: String,
        #[arg(long)]
        event: Option<String>,
        /// Sample time after the synthetic messages; bounded to one minute.
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=60000))]
        elapsed_ms: u64,
    },
}

pub fn run(options: DiagnosticsOptions) -> ProgramResult {
    let DiagnosticsCommand::Preview {
        config,
        source,
        instance,
        state,
        event,
        elapsed_ms,
    } = options.command;
    let mut text = String::new();
    std::fs::File::open(config)?
        .take(256 * 1024 + 1)
        .read_to_string(&mut text)?;
    if text.len() > 256 * 1024 {
        return Err("configuration exceeds 256 KiB".into());
    }
    // Deserialize only: do not build/initialize configured providers (which may execute commands).
    let config: AppConfig = toml::from_str(&text)?;
    config.validate()?;
    let mut runtime = AutomationRuntime::new(config.automation.clone())?;
    let mut registry = Registry::default();
    runtime.baseline(&registry.snapshot(), Duration::ZERO);
    let message = |sequence, payload| Message {
        schema_version: 1,
        source: source.clone(),
        instance: instance.clone(),
        sequence,
        payload,
    };
    registry.apply(
        message(
            1,
            Payload::State {
                state,
                ttl_ms: 60000,
            },
        ),
        Duration::ZERO,
    )?;
    if let Some(event) = event {
        registry.apply(
            message(
                2,
                Payload::Event {
                    event,
                    ttl_ms: 60000,
                },
            ),
            Duration::ZERO,
        )?;
    }
    // Consume at receipt time, then advance so transient expiry uses the real evaluator.
    runtime.observe(
        &registry.snapshot(),
        &registry.events_since(0),
        Duration::ZERO,
    );
    let now = Duration::from_millis(elapsed_ms);
    registry.expire(now)?;
    let observation = Observation {
        now,
        snapshot: registry.snapshot(),
        events: registry.events_since(0),
    };
    runtime.observe(&observation.snapshot, &observation.events, now);
    let info = DeviceInfo::synthetic_80he();
    let layout = KeyboardLayout::for_device(&info);
    let ctx = RenderContext {
        info: &info,
        layout: &layout,
        brightness: config.brightness,
        palette: config.palette,
        tick: 0,
        animation_seconds: now.as_secs_f64() * f64::from(config.speed) / 100.0,
    };
    let frame = compose(
        config.effect.render(&ctx),
        &layout,
        &runtime.layers(&ctx),
        now,
    );
    let mut diagnostics = report(
        &observation,
        runtime.diagnostics(&ctx),
        "synthetic",
        false,
        layout.name,
    );
    diagnostics["preview_note"] = json!(
        "Isolated 80HE simulation using configured base effect; no configured signal providers run"
    );
    let keys: Vec<_> = layout
        .keys()
        .iter()
        .map(|key| {
            let c = frame.get_coord(key.coord);
            json!({"x":key.x,"y":key.y,"rgb":[c.red,c.green,c.blue]})
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"ok":true,"diagnostics":diagnostics,"preview":{"keys":keys}})
        )?
    );
    Ok(())
}
