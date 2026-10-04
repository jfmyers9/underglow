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
    Pause,
    Resume,
    Stop,
    /// Selecting a trusted file can execute its configured commands when enabled.
    Select {
        #[arg(long, required_unless_present = "preset", conflicts_with = "preset")]
        config: Option<PathBuf>,
        #[arg(long, value_parser = ["ripples", "comet", "rainbow", "breath", "matrix", "focus-cockpit"])]
        preset: Option<String>,
    },
    Settings {
        #[arg(long)]
        brightness: Option<u8>,
        #[arg(long, value_enum)]
        palette: Option<PaletteName>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=120))]
        fps: Option<u32>,
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
    ripple_base_color: Option<[u8; 3]>,
    ripple_color: Option<[u8; 3]>,
    last_error: Option<String>,
    retry_attempt: u32,
}
#[derive(Serialize)]
struct Response {
    ok: bool,
    error: Option<String>,
    status: Status,
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
fn validate(text: &str) -> Result<AppConfig, Error> {
    if text.len() as u64 > LIMIT / 4 {
        return Err("configuration exceeds 256 KiB".into());
    }
    let config: AppConfig = toml::from_str(text)?;
    config.validate()?;
    let _ = crate::build_config_signal(&config)?;
    Ok(config)
}
fn preset(name: &str) -> String {
    if name == "ripples" {
        return include_str!("../examples/ripples.toml").to_string();
    }
    if name == "focus-cockpit" {
        return "schema_version = 1\ncontinuous = true\n[signal]\nkind = 'focus-cockpit'\n".into();
    }
    format!(
        "schema_version = 1\ncontinuous = true\n[signal]\nkind = 'static-effect'\neffect = '{name}'\n"
    )
}
fn save(dir: &Path, state: &Saved) -> ProgramResult {
    // Create a new private file, sync contents, atomically rename, then sync the directory.
    // Never truncate the last valid snapshot on a failed edit or process crash.
    let temp = dir.join(format!(".state-{}.tmp", std::process::id()));
    let result = (|| -> ProgramResult {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        serde_json::to_writer(&mut file, state)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temp, dir.join("state.json"))?;
        File::open(dir)?.sync_all()?;
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
impl Active {
    fn open(config: &AppConfig, path: Option<&Path>) -> Result<Self, Error> {
        let lease = Lease::acquire("hardware")?;
        let mut signal = crate::build_config_signal(config)?;
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
struct Runtime {
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
    fn status(&self) -> Status {
        let selected = self.config.signal_config();
        let mode = if selected.kind == crate::signals::SignalKind::StaticEffect {
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
            ripple_base_color: selected.ripples.base_color,
            ripple_color: selected.ripples.ripple_color,
            last_error: self.last_error.clone(),
            retry_attempt: self.retry_attempt,
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
        match Active::open(&self.config, self.sdk.as_deref()) {
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
            Action::Status => {}
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
                let parsed = validate(&config)?;
                let candidate = Saved {
                    config,
                    ..self.saved.clone()
                };
                save(&self.dir, &candidate)?;
                let cleanup = self.release();
                self.saved = candidate;
                self.config = parsed;
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
            config: options
                .config
                .as_deref()
                .map(read_limited)
                .transpose()?
                .unwrap_or_else(|| preset("ripples")),
        }
    };
    if saved.schema_version != 1 {
        return Err("unsupported saved state version".into());
    }
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
    save(&dir, &saved)?;
    let mut runtime = Runtime {
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
        // Bounded work per loop prevents a flood of local requests starving frames.
        for _ in 0..8 {
            let (mut stream, _) = match listener.accept() {
                Ok(client) => client,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            };
            let result = (|| -> ProgramResult {
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
                stop = matches!(request.action, Action::Stop);
                runtime.action(request.action)
            })();
            let response = Response {
                ok: result.is_ok(),
                error: result.err().map(|e| e.to_string()),
                status: runtime.status(),
            };
            // A disconnected UI cannot terminate the engine or undo an applied operation.
            let _ = serde_json::to_writer(&mut stream, &response);
            let _ = stream.write_all(b"\n");
            if stop {
                break;
            }
        }
        runtime.advance(interrupted);
        std::thread::sleep(Duration::from_millis(5));
    }
    // SIGTERM preserves enabled intent; an explicit pause/stop persists disabled.
    runtime.release()
}
pub fn control(options: ControlOptions) -> ProgramResult {
    let result = (|| -> Result<serde_json::Value, Error> {
        let dir = options.state_dir.unwrap_or(ownership::state_dir()?);
        let action = match options.command {
            ControlCommand::Status => Action::Status,
            ControlCommand::Pause => Action::Pause,
            ControlCommand::Resume => Action::Resume,
            ControlCommand::Stop => Action::Stop,
            ControlCommand::Select {
                config,
                preset: choice,
            } => {
                let config = match config {
                    Some(path) => read_limited(&path)?,
                    None => preset(choice.as_deref().ok_or("config or preset required")?),
                };
                validate(&config)?;
                Action::Select { config }
            }
            ControlCommand::Settings {
                brightness,
                palette,
                fps,
                ripple_base_color,
                ripple_color,
                ripple_palette,
            } => Action::Settings {
                brightness,
                palette: palette.map(|v| v.to_string()),
                fps,
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
    fn every_builtin_preset_starts_at_full_brightness() {
        for name in [
            "ripples",
            "focus-cockpit",
            "row-test",
            "rainbow",
            "comet",
            "matrix",
            "breath",
        ] {
            assert_eq!(validate(&preset(name)).unwrap().brightness, 255, "{name}");
        }
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
        Runtime {
            config: validate(&text).unwrap(),
            saved: Saved {
                schema_version: 1,
                enabled: false,
                config: text,
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

    fn colors(base: Option<[u8; 3]>, ripple: Option<[u8; 3]>, reset: bool) -> Action {
        Action::Settings {
            brightness: None,
            palette: None,
            fps: None,
            ripple_base_color: base,
            ripple_color: ripple,
            ripple_palette: reset,
        }
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
        let mut rt = runtime(preset("comet"), dir.clone());
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
