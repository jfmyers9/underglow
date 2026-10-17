//! User-local engine. No TCP listener; one bounded JSON request per connection.
use crate::config::AppConfig;
use crate::ownership::{self, Error, Lease};
use crate::render::PaletteName;
use crate::runner::Session;
use crate::signals::{ProgramResult, SignalProgram};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use underglow::connection_registry::ConnectionRegistry;
use underglow::layer_runtime::CompiledSetup;
use underglow::setup::SetupDocument;
use underglow::setup_edits::{self, LayerEdit};

const LIMIT: u64 = 1024 * 1024;
const RETRIES: u32 = 5;
#[derive(Debug, Args)]
pub struct EngineOptions {
    /// Only used to initialize a new state directory; saved state wins on restart.
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    state_dir: Option<PathBuf>,
    /// Start paused even when saved lighting was enabled (useful for development).
    #[arg(long)]
    paused: bool,
}
#[derive(Debug, Args)]
pub struct ControlOptions {
    #[arg(long)]
    state_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: ControlCommand,
}
#[derive(Debug, Subcommand)]
enum ControlCommand {
    Status,
    /// Read or explicitly replace local connections (may authorize executables).
    Connections {
        #[command(subcommand)]
        command: ConnectionCommand,
    },
    /// Read or atomically edit canonical layers without restarting providers.
    Setup {
        #[command(subcommand)]
        command: SetupCommand,
    },
    /// Read-only source freshness and actual/potential rule visibility. Never resumes lighting.
    Diagnostics,
    /// Hardware-free animation with live external overlays; never resumes lighting.
    Preview,
    /// Add development Pi defaults without replacing existing lighting settings.
    DevSetup,
    /// Manage explicitly trusted connector executables, independently of lighting.
    Connectors {
        #[command(subcommand)]
        command: ConnectorCommand,
    },
    Pause,
    Resume,
    Stop,
    /// Reviewed profiles can run enabled/trusted connectors even while lighting is paused.
    Select {
        #[arg(long, required_unless_present = "preset", conflicts_with = "preset")]
        config: Option<PathBuf>,
        #[arg(long, value_parser = preset_values())]
        preset: Option<String>,
    },
    Settings {
        #[arg(long)]
        brightness: Option<u8>,
        #[arg(long, value_enum)]
        palette: Option<PaletteName>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=120))]
        fps: Option<u32>,
        /// Static-effect speed in percent, independent of rendering FPS.
        #[arg(long, value_parser = clap::value_parser!(u32).range(10..=400))]
        speed: Option<u32>,
        /// Background RGB color, six hexadecimal digits (optional #).
        #[arg(long, value_parser = parse_hex_color, conflicts_with = "ripple_palette")]
        ripple_base_color: Option<[u8; 3]>,
        /// Wave RGB color, six hexadecimal digits (optional #).
        #[arg(long, value_parser = parse_hex_color, conflicts_with = "ripple_palette")]
        ripple_color: Option<[u8; 3]>,
        /// Restore black background and palette-driven ripples.
        #[arg(long)]
        ripple_palette: bool,
    },
}
#[derive(Debug, Subcommand)]
enum ConnectorCommand {
    Status,
    /// Enable an already trusted connector; does not resume lighting.
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
}
#[derive(Debug, Subcommand)]
enum ConnectionCommand {
    Show,
    /// Read local machine configuration JSON from stdin (256 KiB); not a setup import.
    Replace {
        #[arg(long)]
        expected_revision: u64,
    },
}
#[derive(Debug, Subcommand)]
enum SetupCommand {
    Show,
    /// Read a JSON array of layer operations from stdin (256 KiB maximum).
    Edit {
        #[arg(long)]
        expected_revision: u64,
    },
    /// Read a complete declarative setup JSON document from stdin.
    Replace {
        #[arg(long)]
        expected_revision: u64,
    },
}
#[derive(Serialize, Deserialize)]
struct Request {
    schema_version: u32,
    #[serde(flatten)]
    action: Action,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
enum Action {
    Status,
    ConnectionsShow,
    ConnectionsReplace {
        expected_revision: u64,
        registry: ConnectionRegistry,
    },
    SetupShow,
    SetupEdit {
        expected_revision: u64,
        edits: Vec<LayerEdit>,
    },
    SetupReplace {
        expected_revision: u64,
        document: SetupDocument,
    },
    Diagnostics,
    Preview,
    DevSetup,
    Connectors,
    ConnectorEnable {
        id: String,
    },
    ConnectorDisable {
        id: String,
    },
    Pause,
    Resume,
    Stop,
    Select {
        config: String,
    },
    Settings {
        brightness: Option<u8>,
        palette: Option<String>,
        fps: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        speed: Option<u32>,
        ripple_base_color: Option<[u8; 3]>,
        ripple_color: Option<[u8; 3]>,
        #[serde(default)]
        ripple_palette: bool,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    schema_version: u32,
    enabled: bool,
    config: String,
    #[serde(default)]
    dev_starter_migrated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    migration_warning: Option<String>,
}

/// Only private snapshots migrate. Source paths and TOML files are never written.
fn migrate_saved(saved: &mut Saved, development: bool) -> ProgramResult {
    let before: AppConfig = toml::from_str(&saved.config)?;
    if development && !saved.dev_starter_migrated {
        saved.config = crate::dev_setup::config(&saved.config)?;
        // Compatibility mode has no setup marker; remember the attempted upgrade
        // privately rather than adding unknown fields to application TOML.
        saved.dev_starter_migrated = true;
    }
    let outcome = crate::migration::migrate(&saved.config)?;
    saved.config = outcome.config;
    saved.migration_warning = outcome.warning;
    let after: AppConfig = toml::from_str(&saved.config)?;
    if before.setup.is_none() && after.setup.is_some() {
        saved.enabled = false;
    }
    if let Some(warning) = &saved.migration_warning {
        eprintln!("warning: {warning}");
    }
    Ok(())
}
#[derive(Serialize)]
struct Status {
    schema_version: u32,
    state: &'static str,
    enabled: bool,
    mode: String,
    brightness: u8,
    palette: String,
    fps: u32,
    speed: u32,
    ripple_base_color: Option<[u8; 3]>,
    ripple_color: Option<[u8; 3]>,
    last_error: Option<String>,
    retry_attempt: u32,
    migration_warning: Option<String>,
}
#[derive(Serialize)]
struct Response {
    ok: bool,
    error: Option<String>,
    status: Status,
    connectors: Vec<crate::managed_connectors::Health>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    setup: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    connections: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<&'static str>,
}

fn parse_hex_color(value: &str) -> Result<[u8; 3], String> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("expected six hexadecimal digits, optionally prefixed with #".into());
    }
    Ok(std::array::from_fn(|i| {
        u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("validated hex")
    }))
}

/// Edit only the selected ripple table, retaining paths, notifications and scenes.
fn update_ripple_config(
    value: &mut toml::Value,
    config: &AppConfig,
    base: Option<[u8; 3]>,
    ripple: Option<[u8; 3]>,
    reset: bool,
) -> ProgramResult {
    if reset && (base.is_some() || ripple.is_some()) {
        return Err("ripple-palette conflicts with explicit ripple colors".into());
    }
    if config.signal_config().kind != crate::signals::SignalKind::Ripples
        || (config.signal.is_none() && config.sources.len() != 1)
    {
        return Err("ripple colors require a single ripple mode".into());
    }
    let root = value
        .as_table_mut()
        .ok_or("configuration must be a table")?;
    let selected = if config.signal.is_some() {
        let name = if root.contains_key("signal") {
            "signal"
        } else {
            "extension"
        };
        root.get_mut(name).and_then(toml::Value::as_table_mut)
    } else {
        root.get_mut("sources")
            .and_then(toml::Value::as_array_mut)
            .and_then(|sources| sources.first_mut())
            .and_then(toml::Value::as_table_mut)
    }
    .ok_or("missing ripple configuration")?;
    let colors = selected
        .entry("ripples")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or("ripples must be a table")?;
    for (key, color) in [("base_color", base), ("ripple_color", ripple)] {
        if reset {
            colors.remove(key);
        } else if let Some(color) = color {
            colors.insert(
                key.into(),
                toml::Value::Array(
                    color
                        .into_iter()
                        .map(|v| toml::Value::Integer(v.into()))
                        .collect(),
                ),
            );
        }
    }
    Ok(())
}

fn read_limited(path: &Path) -> Result<String, Error> {
    let mut text = String::new();
    File::open(path)?
        .take(LIMIT + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > LIMIT {
        return Err("configuration exceeds 1 MiB".into());
    }
    Ok(text)
}
fn read_setup_json<T: serde::de::DeserializeOwned>(input: impl Read) -> Result<T, Error> {
    let limit = underglow::setup::MAX_DOCUMENT_BYTES;
    let mut bytes = Vec::new();
    input.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("setup input exceeds 256 KiB".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn validate(text: &str) -> Result<AppConfig, Error> {
    if text.len() as u64 > LIMIT / 4 {
        return Err("configuration exceeds 256 KiB".into());
    }
    let config: AppConfig = toml::from_str(text)?;
    config.validate()?;
    let _ = crate::build_config_signal(&config)?;
    Ok(config)
}
fn preset_values() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(underglow::catalog::presets().map(|v| v.id))
}

fn preset(name: &str) -> Result<String, Error> {
    underglow::catalog::find(name)
        .map(|visualization| visualization.preset_config())
        .ok_or_else(|| format!("unknown visualization preset: {name}").into())
}
fn save(dir: &Path, state: &Saved) -> ProgramResult {
    // Create a new private file, sync contents, atomically rename, then sync the directory.
    // Never truncate the last valid snapshot on a failed edit or process crash.
    let temp = dir.join(format!(".state-{}.tmp", std::process::id()));
    let result = (|| -> ProgramResult {
        let directory = File::open(dir)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        serde_json::to_writer(&mut file, state)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temp, dir.join("state.json"))?;
        // Rename is the commit point. Reporting a rejection after it would leave
        // saved/running state inconsistent. A directory-sync failure affects
        // crash durability, not whether this edit has already been committed.
        if let Err(error) = directory.sync_all() {
            eprintln!("warning: state committed, directory sync failed: {error}");
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

struct Active {
    session: Session,
    signal: Box<dyn SignalProgram>,
    _lease: Lease,
}

fn editable_runtime<'a>(
    signal: &'a mut dyn SignalProgram,
    candidate: &CompiledSetup,
) -> Result<&'a mut CompiledSetup, Error> {
    let runtime = signal.setup_runtime().ok_or(
        "live legacy program cannot adopt a setup safely; Pause first, replace, then explicitly Resume",
    )?;
    if runtime.requirements().analog_input != candidate.requirements().analog_input {
        return Err("changing live analog-input requirements requires Pause first; explicitly Resume after editing".into());
    }
    Ok(runtime)
}
impl Active {
    fn open(
        config: &AppConfig,
        path: Option<&Path>,
        sources: crate::ingress::SharedSources,
    ) -> Result<Self, Error> {
        let lease = Lease::acquire("hardware")?;
        let mut signal = crate::build_config_signal_with_sources(config, Some(sources))?;
        let session = Session::open(
            path.or(config.sdk_path.as_deref()),
            &config.signal_run_options(),
            &mut *signal,
        )?;
        Ok(Self {
            session,
            signal,
            _lease: lease,
        })
    }
    fn close(&mut self) -> ProgramResult {
        self.session.close(&mut *self.signal, true)
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("warning: {error}");
        }
    }
}
/// Independent of the hardware session and configured providers. Preview requests
/// must not initialize a command, audio capture, network provider, or RGB SDK.
struct LivePreview {
    automation: underglow::automation::AutomationRuntime,
    animation: underglow::animation::AnimationClock,
    setup: Option<underglow::layer_runtime::CompiledSetup>,
    started: Duration,
    previous_preview: Duration,
    tick: u32,
}
impl LivePreview {
    fn new(config: &AppConfig, observation: &crate::ingress::Observation) -> Result<Self, Error> {
        let mut automation =
            underglow::automation::AutomationRuntime::new(config.automation.clone())?;
        automation.baseline(&observation.snapshot, observation.now);
        let mut animation = underglow::animation::AnimationClock::default();
        animation.set_speed(observation.now, config.speed);
        let mut setup = config
            .setup
            .as_ref()
            .map(underglow::layer_runtime::CompiledSetup::compile)
            .transpose()?;
        if let Some(setup) = &mut setup {
            setup.baseline(&observation.snapshot, observation.now);
        }
        Ok(Self {
            automation,
            animation,
            setup,
            started: observation.now,
            previous_preview: observation.now,
            tick: 0,
        })
    }

    fn observe(&mut self, observation: &crate::ingress::Observation) {
        if let Some(setup) = &mut self.setup {
            setup.observe(&observation.snapshot, &observation.events, observation.now);
        }
        self.automation
            .observe(&observation.snapshot, &observation.events, observation.now);
    }

    fn frame(&mut self, config: &AppConfig, now: Duration) -> serde_json::Value {
        let info = underglow::device::DeviceInfo::synthetic_80he();
        let layout = underglow::layout::KeyboardLayout::for_device(&info);
        let seconds = self.animation.set_speed(now, config.speed);
        let ctx = underglow::render::RenderContext {
            info: &info,
            layout: &layout,
            brightness: config.brightness,
            palette: config.palette,
            tick: self.tick,
            animation_seconds: seconds,
        };
        let selected = config.signal_config();
        let effect = if selected.kind == crate::signals::SignalKind::StaticEffect {
            selected.effect.unwrap_or(config.effect)
        } else {
            // Provider-driven backgrounds are deliberately not evaluated here.
            config.effect
        };
        let layers = self.automation.layers(&ctx);
        let overlay_keys: Vec<_> = layout
            .keys()
            .iter()
            .filter_map(|key| {
                underglow::compositor::winning_layer(&layers, key, now)
                    .map(|_| [key.coord.row, key.coord.column])
            })
            .collect();
        let mut conditional_keys = overlay_keys.clone();
        let base = if let Some(setup) = &mut self.setup {
            setup.preview_tick_at(
                self.tick,
                now.saturating_sub(self.previous_preview).as_secs_f32(),
            );
            let contributions = setup.composition(&ctx, now.saturating_sub(self.started));
            conditional_keys = layout
                .keys()
                .iter()
                .filter(|key| {
                    contributions.iter().any(|layer| {
                        config
                            .setup
                            .as_ref()
                            .and_then(|document| document.layers.get(layer.priority as usize))
                            .is_some_and(|definition| {
                                setup.selected_appearance(&definition.id).is_some_and(
                                    |appearance| {
                                        !matches!(
                                            appearance.activation,
                                            underglow::setup::Activation::Always {}
                                        )
                                    },
                                )
                            })
                            && layer.mask.contains(key)
                    })
                })
                .map(|key| [key.coord.row, key.coord.column])
                .collect();
            underglow::compositor::compose(
                underglow::render::Frame::black(),
                &layout,
                &contributions,
                now,
            )
        } else {
            effect.render(&ctx)
        };
        self.tick = self.tick.wrapping_add(1);
        self.previous_preview = now;
        let frame = underglow::compositor::compose(base, &layout, &layers, now);
        // A canonical stack owns the entire frame, including uncovered black.
        // Keep conditional targets separate for lifecycle/debugging consumers.
        let overlay_keys = if self.setup.is_some() {
            layout
                .keys()
                .iter()
                .map(|key| [key.coord.row, key.coord.column])
                .collect()
        } else {
            overlay_keys
        };
        serde_json::json!({
            "schema_version": 1,
            "kind": "simulated-live-sources",
            "layout": layout.name,
            "rgb": frame.as_bytes(),
            "overlay_keys": overlay_keys,
            "conditional_keys": conditional_keys,
            "base_effect": if self.setup.is_some() { "setup".to_string() } else { effect.to_string() },
        })
    }
}

struct Runtime {
    sources: crate::ingress::SharedSources,
    preview: LivePreview,
    next_preview: Instant,
    connectors: Option<crate::managed_connectors::Supervisor>,
    connections: ConnectionRegistry,
    dir: PathBuf,
    saved: Saved,
    config: AppConfig,
    sdk: Option<PathBuf>,
    active: Option<Active>,
    last_error: Option<String>,
    retry_attempt: u32,
    retry_at: Option<Instant>,
    next_frame: Instant,
    healthy_since: Instant,
}
impl Runtime {
    fn connection_report(&self) -> serde_json::Value {
        let health = self
            .connectors
            .as_ref()
            .map(|c| c.status())
            .unwrap_or_default();
        let runtime_index: std::collections::BTreeMap<_, _> = self
            .connections
            .connections
            .iter()
            .map(|c| {
                let producer = health.iter().find(|h| h.source() == c.descriptor.source);
                (
                    c.descriptor.source.clone(),
                    serde_json::json!({
                        "kind": if c.producer.is_some() { "managed" } else { "external" },
                        "producer": producer,
                    }),
                )
            })
            .collect();
        serde_json::json!({
            "registry": self.connections,
            "runtime_index": runtime_index,
            "missing_sources": self.config.setup.as_ref().map(|s| self.connections.missing_sources(s)).unwrap_or_default(),
        })
    }

    fn replace_connections(
        &mut self,
        expected_revision: u64,
        registry: ConnectionRegistry,
    ) -> ProgramResult {
        let candidate = self.connections.replace(expected_revision, &registry)?;
        crate::connection_store::save(&self.dir, &candidate)?;
        if let Some(supervisor) = &self.connectors {
            supervisor.replace(candidate.managed_configs());
        }
        self.connections = candidate;
        Ok(())
    }

    fn setup_document(&self) -> serde_json::Value {
        serde_json::json!({
            "revision": self.config.setup.as_ref().map_or(0, |doc| doc.revision),
            "document": self.config.setup,
        })
    }

    fn install_setup(&mut self, document: SetupDocument) -> ProgramResult {
        // Replace only declarative setup data. Preserve connectors, trust,
        // profile globals, notifications, sources, and executable definitions.
        let mut value: toml::Value = toml::from_str(&self.saved.config)?;
        value
            .as_table_mut()
            .ok_or("configuration must be a table")?
            .insert("setup".into(), toml::Value::try_from(&document)?);
        let config = toml::to_string(&value)?;
        let parsed = validate(&config)?;
        let preview_candidate = CompiledSetup::compile(&document)?;
        let live_candidate = self
            .active
            .as_ref()
            .map(|_| CompiledSetup::compile(&document))
            .transpose()?;
        let live = if let Some(active) = &mut self.active {
            let candidate = live_candidate.as_ref().expect("compiled live candidate");
            active.session.validate_setup(candidate)?;
            Some(editable_runtime(&mut *active.signal, candidate)?)
        } else {
            None
        };
        // Capture all fallible source work before persistence. Pending old events
        // belong to unchanged rules only; newly added conditions baseline state.
        let cursor = live.as_ref().map_or(0, |runtime| runtime.event_cursor());
        let observation = self.sources.observe(cursor)?;
        let preview_observation = self
            .sources
            .observe(self.preview.automation.event_cursor())?;
        let candidate = Saved {
            config,
            migration_warning: None,
            ..self.saved.clone()
        };
        save(&self.dir, &candidate)?;
        // All remaining work is infallible. Never release/reopen hardware, call
        // initialize, replace a connector supervisor, or alter enabled intent.
        if let Some(runtime) = live {
            runtime.reconfigure(
                live_candidate.expect("compiled live candidate"),
                &observation.snapshot,
                &observation.events,
                observation.now,
            );
        }
        if let Some(runtime) = &mut self.preview.setup {
            runtime.reconfigure(
                preview_candidate,
                &preview_observation.snapshot,
                &preview_observation.events,
                preview_observation.now,
            );
        } else {
            let mut runtime = preview_candidate;
            runtime.baseline(&preview_observation.snapshot, preview_observation.now);
            self.preview.setup = Some(runtime);
        }
        self.saved = candidate;
        self.config = parsed;
        Ok(())
    }

    fn preview(&mut self) -> Result<serde_json::Value, Error> {
        let observation = self
            .sources
            .observe(self.preview.automation.event_cursor())?;
        self.preview.observe(&observation);
        Ok(self.preview.frame(&self.config, observation.now))
    }

    fn diagnostics(&self) -> Result<serde_json::Value, Error> {
        let observation = self.sources.observe(0)?;
        if let Some(active) = &self.active {
            let (rules, layout) = active.session.automation_diagnostics(&*active.signal);
            return Ok(crate::diagnostics::report(
                &observation,
                rules,
                "live",
                true,
                layout,
            ));
        }
        // Paused/retrying/error: explain persistent matches without reviving events.
        let mut runtime =
            underglow::automation::AutomationRuntime::new(self.config.automation.clone())?;
        runtime.baseline(&observation.snapshot, observation.now);
        let info = underglow::device::DeviceInfo::synthetic_80he();
        let layout = underglow::layout::KeyboardLayout::for_device(&info);
        let ctx = underglow::render::RenderContext {
            info: &info,
            layout: &layout,
            brightness: self.config.brightness,
            palette: self.config.palette,
            tick: 0,
            animation_seconds: 0.0,
        };
        let rules = if let Some(document) = &self.config.setup {
            let mut setup = CompiledSetup::compile(document)?;
            setup.baseline(&observation.snapshot, observation.now);
            setup.diagnostics(&ctx)
        } else {
            runtime.diagnostics(&ctx)
        };
        Ok(crate::diagnostics::report(
            &observation,
            rules,
            "potential",
            false,
            layout.name,
        ))
    }

    fn status(&self) -> Status {
        let selected = self.config.signal_config();
        let mode = if self.config.setup.is_some() {
            "setup".to_string()
        } else if selected.kind == crate::signals::SignalKind::StaticEffect {
            selected.effect.unwrap_or(self.config.effect).to_string()
        } else {
            selected
                .kind
                .to_possible_value()
                .expect("named mode")
                .get_name()
                .to_string()
        };
        Status {
            schema_version: 1,
            state: if !self.saved.enabled {
                "paused"
            } else if self.active.is_some() {
                "active"
            } else if self.retry_at.is_some() {
                "retrying"
            } else {
                "error"
            },
            enabled: self.saved.enabled,
            mode,
            brightness: self.config.brightness,
            palette: self.config.palette.to_string(),
            fps: self.config.fps,
            speed: self.config.speed,
            ripple_base_color: selected.ripples.base_color,
            ripple_color: selected.ripples.ripple_color,
            last_error: self.last_error.clone(),
            retry_attempt: self.retry_attempt,
            migration_warning: self.saved.migration_warning.clone(),
        }
    }
    fn release(&mut self) -> ProgramResult {
        if let Some(mut active) = self.active.take() {
            active.close()?;
        }
        Ok(())
    }
    fn failed(&mut self, error: String) {
        let cleanup = self.release().err().map(|e| e.to_string());
        let cleanup_failed = cleanup.is_some();
        self.last_error = Some(match cleanup {
            Some(c) => format!("{error}; cleanup: {c}"),
            None => error,
        });
        self.retry_at = None;
        // Retrying a build/deploy command can repeat external side effects.
        if self.saved.enabled
            && !cleanup_failed
            && !self.config.runs_commands()
            && self.retry_attempt < RETRIES
        {
            self.retry_at = Some(Instant::now() + Duration::from_secs(1 << self.retry_attempt));
            self.retry_attempt += 1;
        }
    }
    fn start(&mut self) {
        match Active::open(&self.config, self.sdk.as_deref(), self.sources.clone()) {
            Ok(active) => {
                self.active = Some(active);
                self.retry_at = None;
                self.last_error = None;
                self.next_frame = Instant::now();
                self.healthy_since = Instant::now();
            }
            Err(error) => self.failed(error.to_string()),
        }
    }
    fn pause(&mut self) -> ProgramResult {
        self.saved.enabled = false;
        self.retry_at = None;
        self.retry_attempt = 0;
        // Release even if persistence fails; never keep writing after a pause request.
        let persisted = save(&self.dir, &self.saved);
        let released = self.release();
        if let Err(error) = persisted {
            self.last_error = Some(format!(
                "paused, but pause could not be saved: {error}; disable login startup until fixed"
            ));
            return Err(self.last_error.clone().unwrap().into());
        }
        if let Err(error) = released {
            self.last_error = Some(error.to_string());
            return Err(error);
        }
        self.last_error = None;
        Ok(())
    }
    fn action(&mut self, action: Action) -> ProgramResult {
        match action {
            Action::Status
            | Action::Diagnostics
            | Action::Preview
            | Action::Connectors
            | Action::ConnectionsShow
            | Action::SetupShow => {}
            Action::ConnectionsReplace {
                expected_revision,
                registry,
            } => {
                self.replace_connections(expected_revision, registry)?;
            }
            Action::SetupEdit {
                expected_revision,
                edits,
            } => {
                let current = self.config.setup.as_ref().ok_or("no canonical setup; use setup replace (legacy configuration is not migrated by show)")?;
                let document = setup_edits::apply_edits(current, expected_revision, &edits)?;
                self.install_setup(document)?;
            }
            Action::SetupReplace {
                expected_revision,
                document,
            } => {
                let document = setup_edits::replace_setup(
                    self.config.setup.as_ref(),
                    expected_revision,
                    &document,
                )?;
                self.install_setup(document)?;
            }
            Action::DevSetup => {
                if std::env::var("WOOTING_DEV_SUPERVISED").as_deref() != Ok("1")
                    || self.saved.enabled
                    || self.active.is_some()
                {
                    return Err("dev-setup requires a supervised, paused development engine".into());
                }
                if self.saved.dev_starter_migrated {
                    return Ok(());
                }
                let mut candidate = self.saved.clone();
                migrate_saved(&mut candidate, true)?;
                let parsed = validate(&candidate.config)?;
                let preview = LivePreview::new(&parsed, &self.sources.observe(0)?)?;
                save(&self.dir, &candidate)?;
                self.saved = candidate;
                self.config = parsed;
                self.preview = preview;
            }
            Action::ConnectorEnable { ref id } | Action::ConnectorDisable { ref id } => {
                let enabled = matches!(action, Action::ConnectorEnable { .. });
                let mut candidate = self.connections.clone();
                let producer = candidate
                    .connections
                    .iter_mut()
                    .filter_map(|c| c.producer.as_mut())
                    .find(|c| c.id == *id)
                    .ok_or("unknown managed connector id")?;
                producer.enabled = enabled;
                self.replace_connections(self.connections.revision, candidate)?;
            }
            Action::Pause | Action::Stop => self.pause()?,
            Action::Resume => {
                if crate::sdk::hardware_disabled() {
                    return Err(crate::sdk::SIMULATION_NOTICE.into());
                }
                if self.active.is_none() {
                    let mut candidate = self.saved.clone();
                    candidate.enabled = true;
                    save(&self.dir, &candidate)?;
                    self.saved = candidate;
                    self.retry_attempt = 0;
                    self.start();
                    if self.active.is_none() {
                        return Err(self.last_error.clone().unwrap_or_default().into());
                    }
                }
            }
            Action::Select { config } => {
                // Selecting visuals never installs/trusts/replaces local producers.
                let config = crate::connection_store::visual_config(&config)?;
                let mut candidate = Saved {
                    config,
                    ..self.saved.clone()
                };
                // Selecting an effect/profile never implicitly reinjects Pi.
                migrate_saved(&mut candidate, false)?;
                let parsed = validate(&candidate.config)?;
                let preview = LivePreview::new(&parsed, &self.sources.observe(0)?)?;
                save(&self.dir, &candidate)?;
                let cleanup = self.release();
                self.saved = candidate;
                self.config = parsed;
                self.preview = preview;
                self.retry_attempt = 0;
                self.retry_at = None;
                if let Err(error) = cleanup {
                    self.last_error = Some(error.to_string());
                    // Do not immediately fight a device that failed handoff.
                    return Err(error);
                }
                self.last_error = None;
                if self.saved.enabled {
                    self.start();
                }
                if self.saved.enabled && self.active.is_none() {
                    return Err(self.last_error.clone().unwrap_or_default().into());
                }
            }
            Action::Settings {
                brightness,
                palette,
                fps,
                speed,
                ripple_base_color,
                ripple_color,
                ripple_palette,
            } => {
                let mut value: toml::Value = toml::from_str(&self.saved.config)?;
                let table = value
                    .as_table_mut()
                    .ok_or("configuration must be a table")?;
                if let Some(v) = brightness {
                    table.insert("brightness".into(), toml::Value::Integer(v.into()));
                }
                if let Some(v) = palette {
                    table.insert("palette".into(), toml::Value::String(v));
                }
                if let Some(v) = fps {
                    table.insert("fps".into(), toml::Value::Integer(v.into()));
                }
                if let Some(v) = speed {
                    table.insert("speed".into(), toml::Value::Integer(v.into()));
                }
                let colors_changed =
                    ripple_base_color.is_some() || ripple_color.is_some() || ripple_palette;
                if colors_changed {
                    update_ripple_config(
                        &mut value,
                        &self.config,
                        ripple_base_color,
                        ripple_color,
                        ripple_palette,
                    )?;
                }
                let config = toml::to_string(&value)?;
                let parsed = validate(&config)?;
                let candidate = Saved {
                    config,
                    ..self.saved.clone()
                };
                save(&self.dir, &candidate)?;
                self.saved = candidate;
                self.config = parsed;
                if let Some(active) = &mut self.active {
                    if colors_changed {
                        let colors = self.config.signal_config().ripples;
                        active
                            .signal
                            .set_ripple_colors(colors.base_color, colors.ripple_color);
                    }
                    active
                        .session
                        .set_visuals(&self.config.signal_run_options());
                }
            }
        }
        Ok(())
    }
    fn advance(&mut self, interrupted: &AtomicBool) {
        if !self.saved.enabled || interrupted.load(Ordering::SeqCst) {
            return;
        }
        if self.retry_at.is_some_and(|t| Instant::now() >= t) {
            self.start();
        }
        if Instant::now() < self.next_frame {
            return;
        }
        if let Some(active) = &mut self.active {
            match active.session.step(&mut *active.signal, interrupted) {
                Ok(true) => {
                    self.next_frame =
                        Instant::now() + Duration::from_secs_f64(1.0 / f64::from(self.config.fps));
                    if self.healthy_since.elapsed() >= Duration::from_secs(30) {
                        self.retry_attempt = 0;
                    }
                }
                Ok(false) => {
                    if !interrupted.load(Ordering::SeqCst)
                        && let Err(e) = self.pause()
                    {
                        self.last_error = Some(e.to_string());
                    }
                }
                Err(error) => self.failed(error.to_string()),
            }
        }
    }
}
struct SocketFile(PathBuf);
impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
pub fn run(
    options: EngineOptions,
    sdk: Option<PathBuf>,
    interrupted: &AtomicBool,
) -> ProgramResult {
    let _engine = Lease::acquire("engine")?;
    let dir = options.state_dir.unwrap_or(ownership::state_dir()?);
    ownership::private_dir(&dir)?;
    let state_path = dir.join("state.json");
    let mut saved: Saved = if state_path.exists() {
        serde_json::from_str(&read_limited(&state_path)?)?
    } else {
        Saved {
            schema_version: 1,
            enabled: false,
            dev_starter_migrated: false,
            migration_warning: None,
            config: options
                .config
                .as_deref()
                .map(read_limited)
                .transpose()?
                .unwrap_or_else(|| underglow::catalog::default_preset().preset_config()),
        }
    };
    if saved.schema_version != 1 {
        return Err("unsupported saved state version".into());
    }
    migrate_saved(
        &mut saved,
        std::env::var("WOOTING_DEV_SUPERVISED").as_deref() == Ok("1"),
    )?;
    let legacy = validate(&saved.config)?;
    let connections = crate::connection_store::load_or_migrate(&dir, &legacy.connectors)?;
    saved.config = crate::connection_store::visual_config(&saved.config)?;
    let config = validate(&saved.config)?;
    // Never automatically rerun a command after service/process restart.
    if options.paused || crate::sdk::hardware_disabled() || config.runs_commands() {
        saved.enabled = false;
    }
    let path = dir.join("control.sock");
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if !metadata.file_type().is_socket() {
            return Err("control.sock exists and is not a socket".into());
        }
        // Engine lease proves our previous process is gone. Never unlink a live endpoint.
        if UnixStream::connect(&path).is_ok() {
            return Err("another engine is listening on control.sock".into());
        }
        fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path).map_err(|e| {
        format!(
            "cannot bind {} (use a shorter --state-dir if needed): {e}",
            path.display()
        )
    })?;
    let _socket = SocketFile(path.clone());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let sources = crate::ingress::SharedSources::default();
    let mut ingress = crate::ingress::Server::bind(&dir, sources.clone())?;
    save(&dir, &saved)?;
    let connectors = Some(crate::managed_connectors::Supervisor::start(
        connections.managed_configs(),
        sources.clone(),
    )?);
    let mut runtime = Runtime {
        preview: LivePreview::new(&config, &sources.observe(0)?)?,
        next_preview: Instant::now(),
        sources,
        connectors,
        connections,
        dir,
        saved,
        config,
        sdk,
        active: None,
        last_error: None,
        retry_attempt: 0,
        retry_at: None,
        next_frame: Instant::now(),
        healthy_since: Instant::now(),
    };
    if runtime.saved.enabled {
        runtime.start();
    }
    eprintln!(
        "engine listening at {}; use control resume to enable paused lighting",
        path.display()
    );
    let mut stop = false;
    while !stop && !interrupted.load(Ordering::SeqCst) {
        // Expiry and ingress stay live while lighting is paused, without touching SDKs.
        runtime.sources.expire()?;
        ingress.poll()?;
        // Bounded work per loop prevents a flood of local requests starving frames.
        for _ in 0..8 {
            let (mut stream, _) = match listener.accept() {
                Ok(client) => client,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            };
            let mut diagnostics = None;
            let mut preview = None;
            let mut setup = None;
            let mut connection_request = false;
            let mut warning = None;
            let result = (|| -> ProgramResult {
                // Darwin can inherit the listener's O_NONBLOCK on accepted sockets.
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_millis(500)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                let mut bytes = Vec::new();
                BufReader::new((&stream).take(LIMIT + 1)).read_until(b'\n', &mut bytes)?;
                if bytes.len() as u64 > LIMIT || !bytes.ends_with(b"\n") {
                    return Err("request too large or missing newline".into());
                }
                let request: Request = serde_json::from_slice(&bytes)?;
                if request.schema_version != 1 {
                    return Err("unsupported protocol version".into());
                }
                connection_request = matches!(
                    request.action,
                    Action::ConnectionsShow
                        | Action::ConnectionsReplace { .. }
                        | Action::ConnectorEnable { .. }
                        | Action::ConnectorDisable { .. }
                );
                stop = matches!(request.action, Action::Stop);
                if let Action::Select { config } = &request.action {
                    warning = crate::connection_store::select_warning(config);
                }
                if matches!(request.action, Action::Diagnostics) {
                    diagnostics = Some(runtime.diagnostics()?);
                }
                if matches!(request.action, Action::Preview) {
                    preview = Some(runtime.preview()?);
                }
                let setup_request = matches!(
                    request.action,
                    Action::SetupShow | Action::SetupEdit { .. } | Action::SetupReplace { .. }
                );
                runtime.action(request.action)?;
                if setup_request {
                    setup = Some(runtime.setup_document());
                }
                Ok(())
            })();
            let response = Response {
                ok: result.is_ok(),
                error: result.err().map(|e| e.to_string()),
                status: runtime.status(),
                connectors: runtime
                    .connectors
                    .as_ref()
                    .map(|c| c.status())
                    .unwrap_or_default(),
                diagnostics,
                preview,
                setup,
                connections: connection_request.then(|| runtime.connection_report()),
                warning,
            };
            // A disconnected UI cannot terminate the engine or undo an applied operation.
            let _ = serde_json::to_writer(&mut stream, &response);
            let _ = stream.write_all(b"\n");
            if stop {
                break;
            }
        }
        // Keep event cursors/transition history current even with no GUI client.
        // Reconnecting does not replay past events, and pausing never stops sources.
        if Instant::now() >= runtime.next_preview {
            let observation = runtime
                .sources
                .observe(runtime.preview.automation.event_cursor())?;
            runtime.preview.observe(&observation);
            runtime.next_preview = Instant::now() + Duration::from_millis(50);
        }
        runtime.advance(interrupted);
        std::thread::sleep(Duration::from_millis(5));
    }
    // SIGTERM preserves enabled intent; an explicit pause/stop persists disabled.
    let released = runtime.release();
    let stopped = runtime
        .connectors
        .as_mut()
        .map(|c| c.shutdown())
        .unwrap_or(Ok(()));
    released?;
    stopped
}
pub fn control(options: ControlOptions) -> ProgramResult {
    let result = (|| -> Result<serde_json::Value, Error> {
        let dir = options.state_dir.unwrap_or(ownership::state_dir()?);
        let action = match options.command {
            ControlCommand::Status => Action::Status,
            ControlCommand::Connections { command } => match command {
                ConnectionCommand::Show => Action::ConnectionsShow,
                ConnectionCommand::Replace { expected_revision } => Action::ConnectionsReplace {
                    expected_revision,
                    registry: crate::connection_store::read_json(std::io::stdin().lock())?,
                },
            },
            ControlCommand::Setup { command } => match command {
                SetupCommand::Show => Action::SetupShow,
                SetupCommand::Edit { expected_revision } => Action::SetupEdit {
                    expected_revision,
                    edits: read_setup_json(std::io::stdin().lock())?,
                },
                SetupCommand::Replace { expected_revision } => Action::SetupReplace {
                    expected_revision,
                    document: read_setup_json(std::io::stdin().lock())?,
                },
            },
            ControlCommand::Diagnostics => Action::Diagnostics,
            ControlCommand::Preview => Action::Preview,
            ControlCommand::DevSetup => Action::DevSetup,
            ControlCommand::Connectors { command } => match command {
                ConnectorCommand::Status => Action::Connectors,
                ConnectorCommand::Enable { id } => Action::ConnectorEnable { id },
                ConnectorCommand::Disable { id } => Action::ConnectorDisable { id },
            },
            ControlCommand::Pause => Action::Pause,
            ControlCommand::Resume => Action::Resume,
            ControlCommand::Stop => Action::Stop,
            ControlCommand::Select {
                config,
                preset: choice,
            } => {
                let config = match config {
                    Some(path) => read_limited(&path)?,
                    None => preset(choice.as_deref().ok_or("config or preset required")?)?,
                };
                validate(&config)?;
                Action::Select { config }
            }
            ControlCommand::Settings {
                brightness,
                palette,
                fps,
                speed,
                ripple_base_color,
                ripple_color,
                ripple_palette,
            } => Action::Settings {
                brightness,
                palette: palette.map(|v| v.to_string()),
                fps,
                speed,
                ripple_base_color,
                ripple_color,
                ripple_palette,
            },
        };
        let mut stream = UnixStream::connect(dir.join("control.sock"))
            .map_err(|e| format!("engine unavailable; start it with 'engine': {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        serde_json::to_writer(
            &mut stream,
            &Request {
                schema_version: 1,
                action,
            },
        )?;
        stream.write_all(b"\n")?;
        let mut bytes = Vec::new();
        BufReader::new(stream.take(LIMIT + 1)).read_until(b'\n', &mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err("response exceeds size limit".into());
        }
        Ok(serde_json::from_slice(&bytes)?)
    })();
    match result {
        Ok(value) => {
            println!("{}", serde_json::to_string(&value)?);
            if value["ok"] == true {
                Ok(())
            } else {
                Err("engine rejected request; see JSON response".into())
            }
        }
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({"ok":false,"error":error.to_string()})
            );
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn setup_json_and_cli_reject_unbounded_malformed_and_unknown_operations() {
        assert!(read_setup_json::<Vec<LayerEdit>>(&b"not json"[..]).is_err());
        assert!(read_setup_json::<Vec<LayerEdit>>(&b"[{\"op\":\"unknown\"}]"[..]).is_err());
        assert!(
            read_setup_json::<Vec<LayerEdit>>(
                &vec![b' '; underglow::setup::MAX_DOCUMENT_BYTES + 1][..]
            )
            .is_err()
        );
        assert!(crate::Cli::try_parse_from(["underglow", "control", "setup", "edit"]).is_err());
        assert!(
            crate::Cli::try_parse_from([
                "underglow",
                "control",
                "setup",
                "edit",
                "--expected-revision",
                "0"
            ])
            .is_ok()
        );
    }

    #[test]
    fn persistence_failure_leaves_saved_config_and_compiled_preview_untouched() {
        let dir = std::env::temp_dir().join(format!("ug-edit-atomic-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut runtime = runtime(
            include_str!("../examples/layers-profile.toml").into(),
            dir.clone(),
        );
        save(&dir, &runtime.saved).unwrap();
        let before_disk = fs::read(dir.join("state.json")).unwrap();
        let before = runtime.saved.config.clone();
        let info = underglow::device::DeviceInfo::synthetic_80he();
        let layout = underglow::layout::KeyboardLayout::for_device(&info);
        let ctx = crate::render::RenderContext {
            info: &info,
            layout: &layout,
            brightness: 255,
            palette: PaletteName::Ocean,
            tick: 0,
            animation_seconds: 0.0,
        };
        let frame = runtime
            .preview
            .setup
            .as_ref()
            .unwrap()
            .render(&ctx, Duration::from_secs(1));
        // create_new fails before rename; the previous durable snapshot stays valid.
        fs::write(
            dir.join(format!(".state-{}.tmp", std::process::id())),
            "occupied",
        )
        .unwrap();
        let err = runtime.action(Action::SetupEdit {
            expected_revision: 0,
            edits: vec![LayerEdit::Remove {
                id: "matrix".into(),
            }],
        });
        assert!(err.is_err());
        assert_eq!(runtime.saved.config, before);
        assert_eq!(fs::read(dir.join("state.json")).unwrap(), before_disk);
        assert_eq!(
            runtime
                .preview
                .setup
                .as_ref()
                .unwrap()
                .render(&ctx, Duration::from_secs(1)),
            frame
        );
        assert_eq!(runtime.config.setup.as_ref().unwrap().revision, 0);
        assert!(runtime.active.is_none());
        assert!(!runtime.saved.enabled);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hot_edit_hook_reaches_wrapped_setup_without_replaying_commands_or_acquiring_sdk() {
        let text = format!(
            "{}\n{}",
            include_str!("../examples/layers-profile.toml"),
            r#"
[[notifications]]
id = 'never-start-command'
duration_seconds = 1
zones = ['function']
[notifications.signal]
kind = 'command-pulse'
command = ['/nonexistent-command-must-not-run']
[[automation.rules]]
id = 'condition'
source = 'pi'
mode = 'while-state'
scene = 'red'
[automation.scenes.red]
color = [255, 0, 0]
zones = ['function']
"#
        );
        let config = validate(&text).unwrap();
        let mut signal = crate::build_config_signal(&config).unwrap();
        let mut document = config.setup.clone().unwrap();
        document.layers[0].renderer = underglow::setup::Renderer::Effect {
            effect: crate::effects::EffectKind::Aurora,
            palette: None,
            speed: 100,
        };
        let candidate = CompiledSetup::compile(&document).unwrap();
        let sources = crate::ingress::SharedSources::default();
        let observation = sources.observe(0).unwrap();
        editable_runtime(&mut *signal, &candidate)
            .unwrap()
            .reconfigure(
                candidate,
                &observation.snapshot,
                &observation.events,
                observation.now,
            );
        let mut analog = document.clone();
        analog.layers[0].renderer = underglow::setup::Renderer::Reactive {
            effect: underglow::reactive::ReactiveKind::Afterimage,
            palette: None,
        };
        assert!(
            editable_runtime(&mut *signal, &CompiledSetup::compile(&analog).unwrap())
                .err()
                .unwrap()
                .to_string()
                .contains("Pause")
        );
        let mut legacy =
            crate::signals::StaticEffectSignal::new(crate::effects::EffectKind::Matrix);
        assert!(
            editable_runtime(&mut legacy, &CompiledSetup::compile(&document).unwrap())
                .err()
                .unwrap()
                .to_string()
                .contains("legacy")
        );
        // Constructor and hooks never initialize the command notification. An
        // accidental initialize would fail immediately on its nonexistent path.
        assert!(!signal.finished());
    }

    #[test]
    fn every_builtin_preset_starts_at_full_brightness() {
        for visualization in underglow::catalog::all() {
            let name = visualization.id;
            let config = validate(&preset(name).unwrap()).unwrap();
            assert_eq!(
                config.brightness, visualization.defaults.brightness,
                "{name}"
            );
            assert_eq!(config.speed, visualization.defaults.speed, "{name}");
            assert_eq!(config.fps, visualization.defaults.fps, "{name}");
            assert_eq!(config.palette, visualization.defaults.palette, "{name}");
        }
        assert!(preset("unknown").is_err());
    }

    #[test]
    fn catalog_presets_preserve_legacy_effective_configuration() {
        for name in [
            "ripples",
            "comet",
            "rainbow",
            "breath",
            "matrix",
            "focus-cockpit",
        ] {
            let old = match name {
                "ripples" => "schema_version=1\npalette='ocean'\nbrightness=255\nfps=30\ncontinuous=true\n[signal]\nkind='ripples'\n".into(),
                "focus-cockpit" => "schema_version=1\ncontinuous=true\n[signal]\nkind='focus-cockpit'\n".into(),
                effect => format!("schema_version=1\ncontinuous=true\n[signal]\nkind='static-effect'\neffect='{effect}'\n"),
            };
            let old = validate(&old).unwrap();
            let new = validate(&preset(name).unwrap()).unwrap();
            assert_eq!((new.brightness, new.fps, new.speed), (255, 30, 100));
            assert_eq!(new.palette, old.palette, "{name}");
            assert_eq!(new.effect, old.effect, "{name}");
            assert_eq!(new.continuous, old.continuous, "{name}");
            assert_eq!(new.seconds, old.seconds, "{name}");
            assert_eq!(new.signal_config().kind, old.signal_config().kind, "{name}");
            assert_eq!(
                new.signal_config().effect,
                old.signal_config().effect,
                "{name}"
            );
            assert_eq!(
                new.signal_config().ripples,
                old.signal_config().ripples,
                "{name}"
            );
        }
    }

    #[test]
    fn catalog_is_the_cli_preset_allowlist() {
        for visualization in underglow::catalog::all() {
            let parsed = crate::Cli::try_parse_from([
                "underglow",
                "control",
                "select",
                "--preset",
                visualization.id,
            ]);
            assert_eq!(
                parsed.is_ok(),
                visualization.selectable,
                "{}",
                visualization.id
            );
        }
        assert!(
            crate::Cli::try_parse_from(["underglow", "control", "select", "--preset", "unknown",])
                .is_err()
        );
    }

    #[test]
    fn strict_hex_and_cli_conflicts() {
        assert_eq!(parse_hex_color("#00aAFF").unwrap(), [0, 170, 255]);
        assert_eq!(parse_hex_color("000000").unwrap(), [0; 3]);
        for value in ["fff", "0x123456", " 123456", "1234567", "gg0000", "é1234"] {
            assert!(parse_hex_color(value).is_err(), "{value}");
        }
        assert!(
            crate::Cli::try_parse_from(["ws", "control", "settings", "--ripple-color", "abcdef"])
                .is_ok()
        );
        assert!(
            crate::Cli::try_parse_from([
                "ws",
                "control",
                "settings",
                "--ripple-color",
                "abcdef",
                "--ripple-palette"
            ])
            .is_err()
        );
        let old: Action = serde_json::from_str(r#"{"action":"settings","brightness":42}"#).unwrap();
        assert!(matches!(
            old,
            Action::Settings {
                ripple_palette: false,
                ripple_color: None,
                ..
            }
        ));
        assert!(
            serde_json::from_str::<Action>(r#"{"action":"settings","ripple_color":[256,0,0]}"#)
                .is_err()
        );
    }

    fn runtime(text: String, dir: PathBuf) -> Runtime {
        let sources = crate::ingress::SharedSources::default();
        let config = validate(&text).unwrap();
        Runtime {
            preview: LivePreview::new(&config, &sources.observe(0).unwrap()).unwrap(),
            next_preview: Instant::now(),
            sources,
            connectors: None,
            connections: ConnectionRegistry::default(),
            config,
            saved: Saved {
                schema_version: 1,
                enabled: false,
                config: text,
                dev_starter_migrated: false,
                migration_warning: None,
            },
            dir,
            sdk: None,
            active: None,
            last_error: None,
            retry_attempt: 0,
            retry_at: None,
            next_frame: Instant::now(),
            healthy_since: Instant::now(),
        }
    }

    #[test]
    fn canonical_control_preview_uses_shared_stack_without_resuming_or_publishing() {
        let text = include_str!("../examples/layers-profile.toml").to_string();
        let mut runtime = runtime(text, PathBuf::from("/unused/offline-preview-test"));
        let before = runtime.sources.observe(0).unwrap().snapshot;
        let result = runtime.preview().unwrap();
        assert_eq!(result["base_effect"], "setup");
        assert_eq!(runtime.status().mode, "setup");
        assert!(!runtime.saved.enabled);
        assert!(runtime.active.is_none());
        assert!(runtime.connectors.is_none());
        assert_eq!(before, runtime.sources.observe(0).unwrap().snapshot);
        let doc = runtime.config.setup.as_ref().unwrap();
        let pure = underglow::layer_runtime::CompiledSetup::compile(doc).unwrap();
        let info = underglow::device::DeviceInfo::synthetic_80he();
        let layout = underglow::layout::KeyboardLayout::for_device(&info);
        // Static effects depend on elapsed, but black top keys are exact.
        for name in ["F1", "F3"] {
            let coord = underglow::key_targets::canonical_coord(name).unwrap();
            let offset = (usize::from(coord.row) * 21 + usize::from(coord.column)) * 3;
            assert_eq!(
                &result["rgb"].as_array().unwrap()[offset..offset + 3],
                &[
                    serde_json::json!(0),
                    serde_json::json!(0),
                    serde_json::json!(0)
                ]
            );
        }
        // At a fixed supplied clock, control preview and pure preview are exact.
        let mut registry = underglow::external::Registry::default();
        let config = &runtime.config;
        let observation = preview_observation(&mut registry, 0, 0);
        let mut preview = LivePreview::new(config, &observation).unwrap();
        let actual = preview.frame(config, Duration::from_secs(2));
        let ctx = underglow::render::RenderContext {
            info: &info,
            layout: &layout,
            brightness: config.brightness,
            palette: config.palette,
            tick: 0,
            animation_seconds: 999.0,
        };
        assert_eq!(
            actual["rgb"],
            serde_json::json!(pure.render(&ctx, Duration::from_secs(2)).as_bytes())
        );
    }

    #[test]
    fn canonical_control_preview_reads_condition_facts_without_replaying_old_events() {
        use underglow::external::{Payload, Registry};
        use underglow::setup::{Activation, Renderer, Target};
        let mut config = validate(include_str!("../examples/layers-profile.toml")).unwrap();
        let doc = config.setup.as_mut().unwrap();
        doc.layers.truncate(2);
        doc.layers[1].target = Target::Whole {};
        doc.layers[1].brightness = 255;
        doc.layers[1].renderer = Renderer::Solid { color: [255, 0, 0] };
        doc.layers[1].activation = Activation::OnEvent {
            source: "pi".into(),
            instance: None,
            event: "done".into(),
            duration_ms: 100,
        };
        let mut registry = Registry::default();
        publish_preview(
            &mut registry,
            "a",
            1,
            Payload::Event {
                event: "done".into(),
                ttl_ms: 1000,
            },
            0,
        );
        let mut preview =
            LivePreview::new(&config, &preview_observation(&mut registry, 0, 0)).unwrap();
        let first = preview.frame(&config, Duration::ZERO);
        assert!(
            preview_colors(&first, underglow::layout::Zone::Alpha)
                .iter()
                .any(|c| *c != [255, 0, 0])
        );
        publish_preview(
            &mut registry,
            "a",
            2,
            Payload::Event {
                event: "done".into(),
                ttl_ms: 1000,
            },
            1,
        );
        preview.observe(&preview_observation(
            &mut registry,
            preview.automation.event_cursor(),
            1,
        ));
        let active = preview.frame(&config, Duration::from_millis(1));
        assert!(
            preview_colors(&active, underglow::layout::Zone::Alpha)
                .iter()
                .all(|c| *c == [255, 0, 0])
        );
        preview.observe(&preview_observation(
            &mut registry,
            preview.automation.event_cursor(),
            101,
        ));
        let expired = preview.frame(&config, Duration::from_millis(101));
        assert!(
            preview_colors(&expired, underglow::layout::Zone::Alpha)
                .iter()
                .any(|c| *c != [255, 0, 0])
        );
    }

    fn preview_observation(
        registry: &mut underglow::external::Registry,
        cursor: u64,
        ms: u64,
    ) -> crate::ingress::Observation {
        let now = Duration::from_millis(ms);
        registry.expire(now).unwrap();
        crate::ingress::Observation {
            now,
            snapshot: registry.snapshot(),
            events: registry.events_since(cursor),
        }
    }

    fn publish_preview(
        registry: &mut underglow::external::Registry,
        instance: &str,
        sequence: u64,
        payload: underglow::external::Payload,
        ms: u64,
    ) {
        registry
            .apply(
                underglow::external::Message {
                    schema_version: 1,
                    source: "pi".into(),
                    instance: instance.into(),
                    sequence,
                    payload,
                },
                Duration::from_millis(ms),
            )
            .unwrap();
    }

    fn preview_colors(frame: &serde_json::Value, zone: underglow::layout::Zone) -> Vec<[u64; 3]> {
        let info = underglow::device::DeviceInfo::synthetic_80he();
        underglow::layout::KeyboardLayout::for_device(&info)
            .keys()
            .iter()
            .filter(|key| key.zone == zone)
            .map(|key| {
                let index = (usize::from(key.coord.row) * 21 + usize::from(key.coord.column)) * 3;
                std::array::from_fn(|channel| frame["rgb"][index + channel].as_u64().unwrap())
            })
            .collect()
    }

    #[test]
    fn live_preview_composes_real_rules_with_independent_instances_and_clock() {
        use underglow::external::{Payload, Registry};
        use underglow::layout::Zone;
        let config = validate(include_str!("../examples/matrix-pi.toml")).unwrap();
        let mut registry = Registry::default();
        let mut preview =
            LivePreview::new(&config, &preview_observation(&mut registry, 0, 0)).unwrap();
        publish_preview(
            &mut registry,
            "a",
            1,
            Payload::State {
                state: "working".into(),
                ttl_ms: 1000,
            },
            0,
        );
        publish_preview(
            &mut registry,
            "b",
            1,
            Payload::State {
                state: "idle".into(),
                ttl_ms: 3000,
            },
            0,
        );
        preview.observe(&preview_observation(
            &mut registry,
            preview.automation.event_cursor(),
            0,
        ));
        let first = preview.frame(&config, Duration::ZERO);
        assert!(
            preview_colors(&first, Zone::Function)
                .iter()
                .any(|c| c[2] > 100 && c[2] > c[1] * 2)
        );
        assert_eq!(first["schema_version"], 1);
        assert_eq!(
            first["rgb"].as_array().unwrap().len(),
            underglow::render::FRAME_BYTES
        );
        preview.observe(&preview_observation(
            &mut registry,
            preview.automation.event_cursor(),
            700,
        ));
        let moving = preview.frame(&config, Duration::from_millis(700));
        assert_ne!(
            preview_colors(&first, Zone::Alpha),
            preview_colors(&moving, Zone::Alpha),
            "Matrix must keep advancing"
        );
        assert_ne!(
            preview_colors(&first, Zone::Function),
            preview_colors(&moving, Zone::Function),
            "working sweep must keep advancing"
        );
        // Expiring the working instance exposes the other session's idle strip.
        preview.observe(&preview_observation(
            &mut registry,
            preview.automation.event_cursor(),
            1000,
        ));
        let idle = preview.frame(&config, Duration::from_millis(1000));
        assert!(
            preview_colors(&idle, Zone::Function)
                .iter()
                .all(|c| *c == [0, 25, 33])
        );
        publish_preview(
            &mut registry,
            "b",
            2,
            Payload::Event {
                event: "finished".into(),
                ttl_ms: 2500,
            },
            1000,
        );
        preview.observe(&preview_observation(
            &mut registry,
            preview.automation.event_cursor(),
            1000,
        ));
        let finished = preview.frame(&config, Duration::from_millis(1000));
        assert!(
            preview_colors(&finished, Zone::Function)
                .iter()
                .any(|c| c[1] > 100 && c[1] > c[2] * 2)
        );
        // Heartbeats do not restart timed alerts; profile changes baseline them away.
        let mut switched =
            LivePreview::new(&config, &preview_observation(&mut registry, 0, 1100)).unwrap();
        let switched_frame = switched.frame(&config, Duration::from_millis(1100));
        assert_eq!(
            preview_colors(&switched_frame, Zone::Function),
            preview_colors(&idle, Zone::Function)
        );
        preview.observe(&preview_observation(
            &mut registry,
            preview.automation.event_cursor(),
            3000,
        ));
        let expired = preview.frame(&config, Duration::from_millis(3000));
        assert!(expired["overlay_keys"].as_array().unwrap().is_empty());
        // A reconnect reads the current state; it does not create a new runtime.
        preview.observe(&preview_observation(
            &mut registry,
            preview.automation.event_cursor(),
            3100,
        ));
        assert!(
            preview.frame(&config, Duration::from_millis(3100))["overlay_keys"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn live_preview_never_builds_configured_command_providers_or_mutates_pause() {
        let config = "effect='matrix'\n[signal]\nkind='command-pulse'\ncommand=['/missing-provider-must-not-run']\n".to_owned();
        let mut runtime = runtime(config, std::env::temp_dir());
        let before = runtime.saved.config.clone();
        assert_eq!(runtime.preview().unwrap()["base_effect"], "matrix");
        assert_eq!(runtime.status().state, "paused");
        assert!(!runtime.saved.enabled);
        assert!(runtime.active.is_none());
        assert!(runtime.last_error.is_none());
        assert_eq!(runtime.saved.config, before);
    }

    fn colors(base: Option<[u8; 3]>, ripple: Option<[u8; 3]>, reset: bool) -> Action {
        Action::Settings {
            brightness: None,
            palette: None,
            fps: None,
            speed: None,
            ripple_base_color: base,
            ripple_color: ripple,
            ripple_palette: reset,
        }
    }

    #[test]
    fn speed_updates_are_persistent_validated_and_atomic() {
        let dir = std::env::temp_dir().join(format!("wooting-speed-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut rt = runtime(preset("matrix").unwrap(), dir.clone());
        let settings = |speed| Action::Settings {
            speed: Some(speed),
            brightness: Some(73),
            palette: None,
            fps: None,
            ripple_base_color: None,
            ripple_color: None,
            ripple_palette: false,
        };
        rt.action(settings(50)).unwrap();
        assert_eq!(rt.status().speed, 50);
        let before = fs::read(dir.join("state.json")).unwrap();
        for invalid in [0, 9, 401, u32::MAX] {
            assert!(rt.action(settings(invalid)).is_err());
            assert_eq!(rt.status().speed, 50);
            assert_eq!(fs::read(dir.join("state.json")).unwrap(), before);
        }
        let saved: Saved = serde_json::from_slice(&before).unwrap();
        let config = validate(&saved.config).unwrap();
        assert_eq!(config.speed, 50);
        assert_eq!(config.brightness, 73);
        assert_eq!(config.fps, 30);
        assert!(!saved.enabled);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn colors_persist_reset_and_preserve_configuration() {
        let dir = std::env::temp_dir().join(format!("wooting-ripple-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for header in [
            "[signal]\nkind='ripples'\n[signal.ripples]",
            "[extension]\nkind='ripples'\n[extension.ripples]",
            "[[sources]]\nid='keys'\ntype='ripples'\n[sources.ripples]",
        ] {
            let text = format!("brightness=73\n{header}\nanalog_sdk_path='/test/analog.dylib'\n");
            let mut rt = runtime(text, dir.clone());
            rt.action(colors(Some([0; 3]), Some([120, 255, 255]), false))
                .unwrap();
            assert_eq!(rt.status().ripple_base_color, Some([0; 3]));
            let saved: Saved =
                serde_json::from_str(&fs::read_to_string(dir.join("state.json")).unwrap()).unwrap();
            let config = validate(&saved.config).unwrap();
            assert_eq!(config.brightness, 73);
            assert_eq!(
                config.signal_config().ripples.ripple_color,
                Some([120, 255, 255])
            );
            assert_eq!(
                config.signal_config().ripples.analog_sdk_path,
                Some(PathBuf::from("/test/analog.dylib"))
            );
            rt.action(colors(None, Some([255, 0, 0]), false)).unwrap();
            assert_eq!(rt.status().ripple_base_color, Some([0; 3]));
            let before = rt.saved.config.clone();
            assert!(rt.action(colors(Some([1; 3]), None, true)).is_err());
            assert_eq!(rt.saved.config, before);
            rt.action(colors(None, None, true)).unwrap();
            assert_eq!(rt.status().ripple_color, None);
            assert_eq!(rt.status().ripple_base_color, None);
        }
        let mut rt = runtime(preset("comet").unwrap(), dir.clone());
        let before = rt.saved.config.clone();
        assert!(rt.action(colors(Some([1; 3]), None, false)).is_err());
        assert_eq!(rt.saved.config, before);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn live_color_hook_reaches_wrapped_programs_without_initialization() {
        let notification = "[[notifications]]\nid='timer'\nduration_seconds=5\nzones=['function']\nstatuses=['break']\n[notifications.signal]\nkind='focus-cockpit'\n";
        for source in [
            "[signal]\nkind='ripples'\n",
            "[[sources]]\nid='keys'\ntype='ripples'\n[scenes.unused]\neffect='comet'\n",
        ] {
            let config = validate(&format!("{source}{notification}")).unwrap();
            let mut signal = crate::build_config_signal(&config).unwrap();
            let info = crate::preview::preview_device();
            let layout = crate::layout::KeyboardLayout::for_device(&info);
            let ctx = crate::render::RenderContext {
                animation_seconds: 0.0,
                info: &info,
                layout: &layout,
                brightness: 255,
                palette: PaletteName::Ocean,
                tick: 0,
            };
            assert_eq!(signal.render(&ctx), crate::render::Frame::black());
            signal.set_ripple_colors(Some([0, 32, 64]), Some([120, 255, 255]));
            assert_eq!(
                signal.render(&ctx).get_coord(layout.keys()[0].coord),
                crate::render::Color::new(0, 32, 64)
            );
            signal.set_ripple_colors(None, None);
            assert_eq!(signal.render(&ctx), crate::render::Frame::black());
        }
    }

    #[test]
    fn ripple_color_config_rejects_invalid_arrays() {
        for color in ["[1,2]", "[1,2,3,4]", "[256,0,0]", "[-1,0,0]", "'ffffff'"] {
            assert!(
                validate(&format!(
                    "[signal]\nkind='ripples'\n[signal.ripples]\nbase_color={color}"
                ))
                .is_err(),
                "accepted {color}"
            );
        }
    }
}
