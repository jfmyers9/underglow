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
    last_error: Option<String>,
    retry_attempt: u32,
}
#[derive(Serialize)]
struct Response {
    ok: bool,
    error: Option<String>,
    status: Status,
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
        self.last_error = Some(match cleanup {
            Some(c) => format!("{error}; cleanup: {c}"),
            None => error,
        });
        self.retry_at = None;
        // Retrying a build/deploy command can repeat external side effects.
        if self.saved.enabled && !self.config.runs_commands() && self.retry_attempt < RETRIES {
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
    if config.runs_commands() {
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
            } => Action::Settings {
                brightness,
                palette: palette.map(|v| v.to_string()),
                fps,
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
