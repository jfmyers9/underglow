//! Optional native controller. All engine/service work happens in subprocesses, never SDK calls.
use clap::Parser;
use eframe::egui;
use serde::Deserialize;
use std::{
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

const PALETTES: &[&str] = &["wooting", "cyberpunk", "ocean", "heat", "terminal"];
const PRESETS: &[&str] = &[
    "ripples",
    "comet",
    "rainbow",
    "breath",
    "matrix",
    "focus-cockpit",
];

#[derive(Parser)]
#[command(about = "Native controller for the sibling wooting-signals engine")]
struct Args {
    /// Use a separate engine state directory (disables system service controls).
    #[arg(long)]
    state_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
struct Status {
    schema_version: u32,
    state: String,
    enabled: bool,
    mode: String,
    brightness: u8,
    palette: String,
    fps: u32,
    last_error: Option<String>,
    retry_attempt: u32,
}

#[derive(Debug, Deserialize)]
struct Reply {
    ok: bool,
    error: Option<String>,
    status: Option<Status>,
}

fn parse_reply(bytes: &[u8]) -> Result<Reply, String> {
    let reply: Reply =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid engine response: {e}"))?;
    if reply.status.as_ref().is_some_and(|s| s.schema_version != 1) {
        return Err(
            "Unsupported engine status schema; install matching CLI and GUI versions".into(),
        );
    }
    Ok(reply)
}

#[derive(Clone, Debug)]
enum Action {
    Control(Vec<OsString>),
    StartEngine,
    Service(String),
}

#[derive(Clone)]
struct Backend {
    directory: PathBuf,
    state_dir: Option<PathBuf>,
}

impl Backend {
    fn command(&self, action: &Action) -> Command {
        let service = matches!(action, Action::Service(_));
        let name = if service {
            "wooting-service"
        } else {
            "wooting-signals"
        };
        let mut command = Command::new(
            self.directory
                .join(format!("{name}{}", std::env::consts::EXE_SUFFIX)),
        );
        match action {
            Action::Service(verb) => {
                command.arg(verb);
            }
            Action::StartEngine | Action::Control(_) => {
                command.arg(if matches!(action, Action::StartEngine) {
                    "engine"
                } else {
                    "control"
                });
                if let Some(directory) = &self.state_dir {
                    command.arg("--state-dir").arg(directory);
                }
                if let Action::Control(args) = action {
                    command.args(args);
                }
            }
        }
        command.stdin(Stdio::null());
        command
    }

    fn engine_log(&self) -> Result<(PathBuf, std::fs::File), String> {
        let directory = if let Some(path) = &self.state_dir {
            path.clone()
        } else {
            let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
            if cfg!(target_os = "macos") {
                PathBuf::from(home).join("Library/Application Support/wooting-signals/runtime")
            } else {
                std::env::var_os("XDG_STATE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(home).join(".local/state"))
                    .join("wooting-signals")
            }
        };
        let result = (|| -> std::io::Result<_> {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&directory)?;
            let metadata = std::fs::symlink_metadata(&directory)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(std::io::Error::other(
                    "log directory must be a real private directory",
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
            }
            let path = directory.join("engine.log");
            if std::fs::symlink_metadata(&path)
                .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
            {
                return Err(std::io::Error::other("engine.log must be a regular file"));
            }
            let mut options = std::fs::OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = options.open(&path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            Ok((path, file))
        })();
        result.map_err(|e| format!("Cannot prepare private engine log: {e}"))
    }

    fn execute(&self, action: &Action) -> Result<String, String> {
        let mut command = self.command(action);
        if matches!(action, Action::StartEngine) {
            // The window owns neither the engine lifetime nor its console streams.
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x00000008 | 0x00000200);
            }
            let (log_path, log) = self.engine_log()?;
            let offset = log.metadata().map_err(|e| e.to_string())?.len();
            let mut child = command
                .stdout(Stdio::null())
                .stderr(Stdio::from(log))
                .spawn()
                .map_err(|e| format!("Cannot start sibling engine: {e}"))?;
            let deadline = Instant::now() + Duration::from_millis(250);
            while Instant::now() < deadline {
                if let Some(exit) = child.try_wait().map_err(|e| e.to_string())? {
                    use std::io::{Seek, SeekFrom};
                    let detail = (|| -> std::io::Result<String> {
                        let mut log = std::fs::File::open(&log_path)?;
                        log.seek(SeekFrom::Start(offset))?;
                        let mut text = String::new();
                        log.take(16 * 1024).read_to_string(&mut text)?;
                        Ok(text)
                    })()
                    .unwrap_or_default();
                    return Err(format!(
                        "Engine exited {exit}: {} (log: {})",
                        detail.trim(),
                        log_path.display()
                    ));
                }
                thread::sleep(Duration::from_millis(25));
            }
            // Reap eventual exit without tying engine lifetime or logging to the UI.
            thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(format!(
                "Engine start requested. Fresh state starts paused. Log: {}",
                log_path.display()
            ));
        }
        run_bounded(command)
    }
}

fn read_pipe(mut pipe: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = (&mut pipe).take(64 * 1024).read_to_end(&mut bytes);
    bytes
}

fn run_bounded(command: Command) -> Result<String, String> {
    run_with_timeout(command, Duration::from_secs(15))
}

fn stop_helper(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        // SAFETY: every bounded helper starts in a private process group whose
        // ID is its positive child PID. Negative PID addresses only that group.
        unsafe {
            kill(-(child.id() as i32), 9);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn run_with_timeout(mut command: Command, timeout: Duration) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Cannot run sibling helper: {e}"))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (out_send, out) = mpsc::channel();
    let (err_send, err) = mpsc::channel();
    thread::spawn(move || {
        let _ = out_send.send(read_pipe(stdout));
    });
    thread::spawn(move || {
        let _ = err_send.send(read_pipe(stderr));
    });
    let deadline = Instant::now() + timeout;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(exit)) => break exit,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            result => {
                stop_helper(&mut child);
                return Err(match result {
                    Err(e) => e.to_string(),
                    _ => format!("Helper timed out after {} ms", timeout.as_millis()),
                });
            }
        }
    };
    // A completed helper must not leave grandchildren holding its output pipes.
    stop_helper(&mut child);
    let stdout = out
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| "Timed out reading helper output".to_string())?;
    let stderr = err
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| "Timed out reading helper diagnostics".to_string())?;
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    // Errors from the protocol are JSON even on a nonzero exit; preserve that body.
    if !stdout.trim().is_empty() {
        return Ok(stdout);
    }
    Err(format!("Helper exited {exit}: {}", stderr.trim()))
}

#[derive(Deserialize)]
struct ServiceReply {
    ok: bool,
    supported: bool,
    enabled: bool,
    running: bool,
    error: Option<String>,
}

fn service_summary(body: &str) -> Result<String, String> {
    let reply: ServiceReply =
        serde_json::from_str(body).map_err(|e| format!("Invalid service response: {e}"))?;
    Ok(format!(
        "{} · login {} · service {}{}",
        if reply.supported {
            "Supported"
        } else {
            "Unsupported platform"
        },
        if reply.enabled { "enabled" } else { "disabled" },
        if reply.running { "running" } else { "stopped" },
        reply
            .error
            .map(|e| format!(" · {e}"))
            .unwrap_or_else(|| if reply.ok {
                String::new()
            } else {
                " · request failed".into()
            })
    ))
}

struct Completion {
    action: Action,
    result: Result<String, String>,
}

// Resolve at startup so service isolation follows the same override as engine/control.
fn effective_state_dir(
    explicit: Option<PathBuf>,
    environment: Option<OsString>,
) -> Option<PathBuf> {
    explicit.or_else(|| environment.map(PathBuf::from))
}

#[cfg(target_os = "macos")]
struct MenuBar {
    _icon: tray_icon::TrayIcon,
    quitting: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(target_os = "macos")]
impl MenuBar {
    fn new(context: &egui::Context) -> Result<Self, Box<dyn std::error::Error>> {
        use tray_icon::menu::{Menu, MenuEvent, MenuItem};
        let menu = Menu::new();
        let show = MenuItem::new("Open controller", true, None);
        let quit = MenuItem::new("Quit controller (leave engine running)", true, None);
        menu.append_items(&[&show, &quit])?;
        // Constructed by eframe's app creator on the AppKit main/event-loop thread.
        let icon = tray_icon::TrayIconBuilder::new()
            .with_title("WS")
            .with_tooltip("Wooting Signals")
            .with_menu(Box::new(menu))
            .build()?;
        let show_id = show.id().clone();
        let quit_id = quit.id().clone();
        let quitting = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let quit_flag = quitting.clone();
        let context = context.clone();
        // Wake egui directly: a hidden window cannot be relied on to poll menu events.
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show_id {
                context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                context.send_viewport_cmd(egui::ViewportCommand::Focus);
            } else if event.id == quit_id {
                quit_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            context.request_repaint();
        }));
        Ok(Self {
            _icon: icon,
            quitting,
        })
    }

    fn hide_on_close(&self, context: &egui::Context) {
        if context.input(|i| i.viewport().close_requested())
            && !self.quitting.load(std::sync::atomic::Ordering::SeqCst)
        {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }
}

struct Controller {
    #[cfg(target_os = "macos")]
    menu_bar: Option<MenuBar>,
    requests: Sender<Action>,
    replies: Receiver<Completion>,
    pending: bool,
    status: Option<Status>,
    connected: bool,
    message: String,
    diagnostics: String,
    service: String,
    custom_state: bool,
    preset: String,
    brightness: u8,
    palette: String,
    fps: u32,
    settings_dirty: bool,
    config_path: String,
    trust_config: bool,
    last_poll: Instant,
}

impl Controller {
    fn new(context: &egui::Context, backend: Backend) -> Self {
        let (requests, incoming) = mpsc::channel::<Action>();
        let (outgoing, replies) = mpsc::channel();
        let repaint = context.clone();
        let custom_state = backend.state_dir.is_some();
        thread::spawn(move || {
            while let Ok(action) = incoming.recv() {
                let result = backend.execute(&action);
                if outgoing.send(Completion { action, result }).is_err() {
                    break;
                }
                repaint.request_repaint();
            }
        });
        #[cfg(target_os = "macos")]
        let menu_bar = MenuBar::new(context)
            .map_err(|error| {
                eprintln!("Menu bar unavailable; use platform launcher: {error}");
            })
            .ok();
        Self {
            #[cfg(target_os = "macos")]
            menu_bar,
            requests,
            replies,
            pending: false,
            status: None,
            connected: false,
            message: "Checking engine…".into(),
            diagnostics: String::new(),
            service: "Not checked".into(),
            custom_state,
            preset: "ripples".into(),
            brightness: 96,
            palette: "wooting".into(),
            fps: 30,
            settings_dirty: false,
            config_path: String::new(),
            trust_config: false,
            last_poll: Instant::now() - Duration::from_secs(5),
        }
    }

    fn dispatch(&mut self, action: Action) {
        if self.pending {
            return;
        }
        match self.requests.send(action) {
            Ok(()) => {
                self.pending = true;
                self.last_poll = Instant::now();
            }
            Err(_) => self.message = "Background worker stopped; reopen this window".into(),
        }
    }

    fn control(&mut self, args: &[&str]) {
        self.dispatch(Action::Control(args.iter().map(OsString::from).collect()));
    }

    fn receive(&mut self) {
        while let Ok(completion) = self.replies.try_recv() {
            self.pending = false;
            // Leave an interactive interval even when the previous poll was slow.
            self.last_poll = Instant::now();
            match completion.action {
                Action::Service(_) => {
                    self.service = completion
                        .result
                        .and_then(|body| service_summary(&body))
                        .unwrap_or_else(|e| e);
                }
                Action::StartEngine => {
                    self.message = completion.result.unwrap_or_else(|e| e);
                }
                Action::Control(args) => match completion.result.and_then(|body| {
                    self.diagnostics = body.clone();
                    parse_reply(body.as_bytes())
                }) {
                    Ok(reply) => {
                        if reply.ok && args.first().is_some_and(|arg| arg == "settings") {
                            self.settings_dirty = false;
                        }
                        self.connected = reply.status.is_some();
                        self.message = reply.error.unwrap_or_else(|| {
                            if reply.ok {
                                "Engine connected".into()
                            } else {
                                "Engine request failed".into()
                            }
                        });
                        if let Some(status) = reply.status {
                            if !self.settings_dirty {
                                self.brightness = status.brightness;
                                self.palette = status.palette.clone();
                                self.fps = status.fps;
                            }
                            self.status = Some(status);
                        }
                    }
                    Err(error) => {
                        self.connected = false;
                        self.message = error;
                    }
                },
            }
        }
    }
}

impl eframe::App for Controller {
    fn update(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        if let Some(menu_bar) = &self.menu_bar {
            menu_bar.hide_on_close(context);
        }
        self.receive();
        if !self.pending && self.last_poll.elapsed() >= Duration::from_secs(3) {
            self.control(&["status"]);
        }
        context.request_repaint_after(Duration::from_millis(200));
        egui::CentralPanel::default().show(context, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Wooting Signals");
            ui.label("Native controller • closing this window leaves the engine running");
            ui.separator();
            ui.label(&self.message);
            if let Some(status) = &self.status {
                ui.label(format!("{}{} · {} · enabled={} · retry {}", if self.connected { "" } else { "Last seen: " }, status.state, status.mode, status.enabled, status.retry_attempt));
                if let Some(error) = &status.last_error { ui.colored_label(egui::Color32::LIGHT_RED, error); }
            }
            if self.pending { ui.spinner(); }
            ui.add_enabled_ui(!self.pending, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Refresh").clicked() { self.control(&["status"]); }
                    if !self.connected && ui.button("Start engine").clicked() { self.dispatch(Action::StartEngine); }
                    if ui.add_enabled(self.connected, egui::Button::new("Resume effects")).clicked() { self.control(&["resume"]); }
                    if ui.add_enabled(self.connected, egui::Button::new("Pause / return lighting")).clicked() { self.control(&["pause"]); }
                });
                ui.small("Pause releases SDK lighting back to the keyboard/Wootility; it does not change saved keyboard profiles.");
                ui.separator();
                ui.add_enabled_ui(self.connected, |ui| {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_label("Preset").selected_text(&self.preset).show_ui(ui, |ui| {
                            for preset in PRESETS { ui.selectable_value(&mut self.preset, (*preset).into(), *preset); }
                        });
                        if ui.button("Select").clicked() { let preset = self.preset.clone(); self.control(&["select", "--preset", &preset]); }
                    });
                    self.settings_dirty |= ui.add(egui::Slider::new(&mut self.brightness, 0..=255).text("Brightness")).changed();
                    self.settings_dirty |= ui.add(egui::Slider::new(&mut self.fps, 1..=120).text("FPS")).changed();
                    egui::ComboBox::from_label("Palette").selected_text(&self.palette).show_ui(ui, |ui| {
                        for palette in PALETTES {
                            self.settings_dirty |= ui.selectable_value(&mut self.palette, (*palette).into(), *palette).changed();
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Apply settings").clicked() {
                            let args = vec!["settings".into(), "--brightness".into(), self.brightness.to_string().into(), "--palette".into(), self.palette.clone().into(), "--fps".into(), self.fps.to_string().into()];
                            self.dispatch(Action::Control(args));
                        }
                        if ui.button("Reload settings").clicked() { self.settings_dirty = false; self.control(&["status"]); }
                    });
                    ui.collapsing("Trusted configuration", |ui| {
                        ui.label("Warning: configurations can execute commands as your user. Select only files you trust.");
                        if ui.text_edit_singleline(&mut self.config_path).changed() { self.trust_config = false; }
                        ui.checkbox(&mut self.trust_config, "I trust this file and its commands");
                        if ui.add_enabled(self.trust_config && !self.config_path.trim().is_empty(), egui::Button::new("Select config")).clicked() {
                            let path = self.config_path.clone(); self.control(&["select", "--config", &path]);
                        }
                    });
                });
                ui.separator();
                ui.collapsing("Startup and service settings", |ui| {
                    #[cfg(target_os = "macos")]
                    if self.menu_bar.is_some() {
                        ui.label("Closing hides this window. Use the WS menu bar item to reopen it or quit the controller; the engine keeps running.");
                    } else {
                        ui.label("Menu bar unavailable. Reopen this window from the platform launcher.");
                    }
                    #[cfg(not(target_os = "macos"))]
                    ui.label("No tray icon on this platform. Reopen this window from the platform launcher to control the engine.");
                    ui.label("Enable login only registers startup. Start service launches now. Disable login also stops the managed engine.");
                    if self.custom_state { ui.label("Service controls unavailable with --state-dir or WOOTING_STATE_DIR (service uses default state)."); }
                    ui.add_enabled_ui(!self.custom_state, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (label, verb) in [("Service status", "status"), ("Enable login", "enable"), ("Disable login", "disable"), ("Start service", "start"), ("Stop service", "stop")] {
                                if ui.button(label).clicked() { self.dispatch(Action::Service(verb.into())); }
                            }
                        });
                    });
                    ui.label(&self.service);
                    if ui.add_enabled(self.connected, egui::Button::new("Stop standalone engine")).clicked() { self.control(&["stop"]); }
                    ui.small("For a managed engine use Stop service, not Stop standalone engine.");
                });
            });
            ui.collapsing("Diagnostics (last engine JSON)", |ui| { ui.monospace(&self.diagnostics); });
            });
        });
    }
}

fn main() -> eframe::Result {
    let args = Args::parse();
    let directory = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .expect("Cannot locate this executable's sibling helpers");
    let backend = Backend {
        directory,
        state_dir: effective_state_dir(args.state_dir, std::env::var_os("WOOTING_STATE_DIR")),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([650.0, 740.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Wooting Signals",
        options,
        Box::new(move |cc| Ok(Box::new(Controller::new(&cc.egui_ctx, backend)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    const STATUS: &str = r#"{"schema_version":1,"state":"retrying","enabled":true,"mode":"ripples","brightness":96,"palette":"wooting","fps":30,"last_error":"unplugged","retry_attempt":2}"#;

    #[test]
    fn parses_status_and_nonzero_error_protocol() {
        let reply = parse_reply(
            format!(r#"{{"ok":false,"error":"unplugged","status":{STATUS}}}"#).as_bytes(),
        )
        .unwrap();
        assert!(!reply.ok);
        assert_eq!(reply.error.as_deref(), Some("unplugged"));
        assert_eq!(reply.status.unwrap().retry_attempt, 2);
        assert!(
            parse_reply(br#"{"ok":false,"error":"engine absent"}"#)
                .unwrap()
                .status
                .is_none()
        );
    }

    #[test]
    fn rejects_malformed_or_incompatible_protocol() {
        assert!(parse_reply(b"not JSON").is_err());
        assert!(
            parse_reply(
                format!(
                    r#"{{"ok":true,"status":{}}}"#,
                    STATUS.replace("\"schema_version\":1", "\"schema_version\":2")
                )
                .as_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn subprocess_arguments_preserve_paths_without_shell() {
        let backend = Backend {
            directory: PathBuf::from("/installed tools"),
            state_dir: Some("/state dir;not a command".into()),
        };
        let command = backend.command(&Action::Control(vec![
            "select".into(),
            "--config".into(),
            "/trusted file.toml".into(),
        ]));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "control",
                "--state-dir",
                "/state dir;not a command",
                "select",
                "--config",
                "/trusted file.toml"
            ]
        );
        assert!(
            command
                .get_program()
                .to_string_lossy()
                .contains("/installed tools/wooting-signals")
        );
    }

    #[test]
    fn start_does_not_resume_or_override_saved_configuration() {
        let backend = Backend {
            directory: "/tools".into(),
            state_dir: None,
        };
        assert_eq!(
            backend
                .command(&Action::StartEngine)
                .get_args()
                .collect::<Vec<_>>(),
            ["engine"]
        );
        assert_eq!(
            backend
                .command(&Action::Service("enable".into()))
                .get_args()
                .collect::<Vec<_>>(),
            ["enable"]
        );
    }

    #[test]
    fn polling_does_not_clobber_unsaved_edits() {
        let (requests, _incoming) = mpsc::channel();
        let (outgoing, replies) = mpsc::channel();
        let mut app = Controller {
            #[cfg(target_os = "macos")]
            menu_bar: None,
            requests,
            replies,
            pending: true,
            status: None,
            connected: false,
            message: String::new(),
            diagnostics: String::new(),
            service: String::new(),
            custom_state: false,
            preset: "ripples".into(),
            brightness: 7,
            palette: "custom".into(),
            fps: 12,
            settings_dirty: true,
            config_path: String::new(),
            trust_config: false,
            last_poll: Instant::now() - Duration::from_secs(10),
        };
        outgoing
            .send(Completion {
                action: Action::Control(vec!["status".into()]),
                result: Ok(format!(r#"{{"ok":true,"status":{STATUS}}}"#)),
            })
            .unwrap();
        app.receive();
        assert!(app.connected);
        assert!(!app.pending);
        assert!(app.last_poll.elapsed() < Duration::from_secs(3));
        assert_eq!(app.brightness, 7);
        assert_eq!(app.fps, 12);
        assert_eq!(app.palette, "custom");
        outgoing
            .send(Completion {
                action: Action::Control(vec!["settings".into()]),
                result: Ok(format!(
                    r#"{{"ok":false,"error":"disk full","status":{STATUS}}}"#
                )),
            })
            .unwrap();
        app.receive();
        assert!(app.settings_dirty);
        assert_eq!(app.brightness, 7);
        outgoing
            .send(Completion {
                action: Action::Control(vec!["settings".into()]),
                result: Ok(format!(r#"{{"ok":true,"status":{STATUS}}}"#)),
            })
            .unwrap();
        app.receive();
        assert!(!app.settings_dirty);
        assert_eq!(app.brightness, app.status.as_ref().unwrap().brightness);
        outgoing
            .send(Completion {
                action: Action::StartEngine,
                result: Err("fixture-start-failure".into()),
            })
            .unwrap();
        app.receive();
        assert!(app.message.contains("fixture-start-failure"));
        assert!(
            app.last_poll.elapsed() < Duration::from_secs(3),
            "startup diagnostics need an interactive interval before polling"
        );
    }
    #[test]
    fn service_status_and_errors_are_human_readable() {
        let message =
            service_summary(r#"{"ok":true,"supported":true,"enabled":true,"running":false}"#)
                .unwrap();
        assert!(message.contains("login enabled"));
        assert!(message.contains("service stopped"));
        let message = service_summary(r#"{"ok":false,"supported":false,"enabled":false,"running":false,"error":"no manager"}"#).unwrap();
        assert!(message.contains("Unsupported platform"));
        assert!(message.contains("no manager"));
        assert!(service_summary("not JSON").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn helper_nonzero_json_is_not_discarded() {
        // A shell fixture only; production commands never use a shell.
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "printf '%s' '{\"ok\":false,\"error\":\"fixture failure\"}'; exit 7",
        ]);
        let body = run_bounded(command).unwrap();
        assert_eq!(
            parse_reply(body.as_bytes()).unwrap().error.as_deref(),
            Some("fixture failure")
        );
    }

    #[test]
    #[cfg(unix)]
    fn unresponsive_helper_is_bounded() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec sleep 2"]);
        let started = Instant::now();
        assert!(
            run_with_timeout(command, Duration::from_millis(30))
                .unwrap_err()
                .contains("timed out")
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    #[cfg(unix)]
    fn inherited_pipes_do_not_stall_worker() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 1 & exit 0"]);
        let started = Instant::now();
        assert!(run_with_timeout(command, Duration::from_millis(30)).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
    #[test]
    #[cfg(unix)]
    fn timeout_stops_helper_descendants() {
        let marker = std::env::temp_dir().join(format!(
            "wooting-gui-descendant-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "(sleep 0.2; echo unexpected > \"$1\") & wait",
                "fixture",
            ])
            .arg(&marker);
        assert!(run_with_timeout(command, Duration::from_millis(30)).is_err());
        thread::sleep(Duration::from_millis(300));
        let leaked = marker.exists();
        let _ = std::fs::remove_file(&marker);
        assert!(!leaked, "a timed-out helper left a running descendant");
    }

    #[test]
    #[cfg(unix)]
    fn startup_errors_are_reported_and_logged_without_a_pipe_to_the_window() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("wooting-gui-start-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let binary = directory.join("wooting-signals");
        std::fs::write(
            &binary,
            "#!/bin/sh\necho fixture-start-failure >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let backend = Backend {
            directory: directory.clone(),
            state_dir: Some(directory.join("state")),
        };
        let result = backend.execute(&Action::StartEngine);
        let log = std::fs::read_to_string(directory.join("state/engine.log")).unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        assert!(result.unwrap_err().contains("fixture-start-failure"));
        assert!(log.contains("fixture-start-failure"));
    }

    #[test]
    fn state_override_includes_environment_and_preserves_explicit_precedence() {
        assert_eq!(effective_state_dir(None, None), None);
        assert_eq!(
            effective_state_dir(None, Some("/environment state".into())),
            Some("/environment state".into())
        );
        assert_eq!(
            effective_state_dir(Some("/explicit".into()), Some("/environment".into())),
            Some("/explicit".into())
        );
        // Even an empty env override must not silently enable default-instance controls.
        assert!(effective_state_dir(None, Some(OsString::new())).is_some());
    }
}
