//! Optional native controller. All engine/service work happens in subprocesses, never SDK calls.
use clap::Parser;
use eframe::egui;
#[path = "gui/preview.rs"]
mod gui_preview;
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
    #[serde(default)]
    ripple_base_color: Option<[u8; 3]>,
    #[serde(default)]
    ripple_color: Option<[u8; 3]>,
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

impl Action {
    fn is_status(&self) -> bool {
        matches!(self, Self::Control(args) if args.len() == 1 && args[0] == "status")
    }
}

#[derive(Clone)]
struct Backend {
    directory: PathBuf,
    state_dir: Option<PathBuf>,
}

impl Backend {
    fn bundled_app(&self) -> bool {
        cfg!(target_os = "macos")
            && self
                .directory
                .file_name()
                .is_some_and(|name| name == "MacOS")
            && self
                .directory
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == "Contents")
    }

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
        if matches!(action, Action::StartEngine)
            && std::env::var_os("WOOTING_DEV_SUPERVISED").is_some()
        {
            return Err(
                "The development watcher owns the engine; check its terminal output".into(),
            );
        }
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
        if matches!(action, Action::Service(_)) {
            // Native service maintenance can require several bounded launchctl calls.
            run_with_timeout(command, Duration::from_secs(30))
        } else {
            run_bounded(command)
        }
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
        let development = std::env::var_os("WOOTING_DEV_SUPERVISED").is_some();
        let menu = Menu::new();
        let show = MenuItem::new("Open controller", true, None);
        let quit = MenuItem::new(
            if development {
                "Quit development session"
            } else {
                "Quit controller (leave engine running)"
            },
            true,
            None,
        );
        menu.append_items(&[&show, &quit])?;
        // Constructed by eframe's app creator on the AppKit main/event-loop thread.
        let icon = tray_icon::TrayIconBuilder::new()
            .with_title(if development { "WS Dev" } else { "WS" })
            .with_tooltip(if development {
                "Wooting Signals — Dev"
            } else {
                "Wooting Signals"
            })
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
    poll_pending: bool,
    queued_action: Option<Action>,
    status: Option<Status>,
    connected: bool,
    message: String,
    diagnostics: String,
    service: String,
    custom_state: bool,
    bundled_app: bool,
    preset: String,
    brightness: u8,
    palette: String,
    fps: u32,
    ripple_colors: RippleColors,
    ripple_colors_dirty: bool,
    settings_dirty: bool,
    config_path: String,
    trust_config: bool,
    last_poll: Instant,
    settings_open: bool,
    dev_supervised: bool,
    dev_simulation: bool,
    ripple_preview: gui_preview::RipplePreview,
}

impl Controller {
    fn new(context: &egui::Context, backend: Backend) -> Self {
        configure_style(context);
        let (requests, incoming) = mpsc::channel::<Action>();
        let (outgoing, replies) = mpsc::channel();
        let repaint = context.clone();
        let custom_state = backend.state_dir.is_some();
        let bundled_app = backend.bundled_app();
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
            poll_pending: false,
            queued_action: None,
            status: None,
            connected: false,
            message: "Checking engine…".into(),
            diagnostics: String::new(),
            service: "Not checked".into(),
            custom_state,
            bundled_app,
            preset: "ripples".into(),
            brightness: 96,
            palette: "wooting".into(),
            fps: 30,
            ripple_colors: RippleColors::default(),
            ripple_colors_dirty: false,
            settings_dirty: false,
            config_path: String::new(),
            trust_config: false,
            last_poll: Instant::now() - Duration::from_secs(5),
            settings_open: false,
            dev_supervised: std::env::var_os("WOOTING_DEV_SUPERVISED").is_some(),
            dev_simulation: std::env::var_os("WOOTING_DEV_SIMULATION").is_some(),
            ripple_preview: gui_preview::RipplePreview::default(),
        }
    }

    fn dispatch(&mut self, action: Action) {
        if self.pending {
            return;
        }
        let poll = action.is_status();
        if self.poll_pending {
            if !poll {
                // Keep polling invisible, but never lose a click during a slow poll.
                self.queued_action = Some(action);
                self.pending = true;
            }
            return;
        }
        match self.requests.send(action) {
            Ok(()) => {
                self.poll_pending = poll;
                self.pending = !poll;
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
            if completion.action.is_status() {
                self.poll_pending = false;
                self.pending = self.queued_action.is_some();
            } else {
                self.pending = false;
            }
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
                            self.ripple_colors_dirty = false;
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
                            self.preset = status.mode.clone();
                            if !self.settings_dirty {
                                self.brightness = status.brightness;
                                self.palette = status.palette.clone();
                                self.fps = status.fps;
                                self.ripple_colors = RippleColors::from_status(&status);
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
        if !self.poll_pending
            && let Some(action) = self.queued_action.take()
        {
            self.pending = false;
            self.dispatch(action);
        }
    }
}

const BG: egui::Color32 = egui::Color32::from_rgb(13, 18, 25);
const SURFACE: egui::Color32 = egui::Color32::from_rgb(21, 28, 37);
const BORDER: egui::Color32 = egui::Color32::from_rgb(42, 53, 66);
const INK: egui::Color32 = egui::Color32::from_rgb(234, 241, 247);
const MUTED: egui::Color32 = egui::Color32::from_rgb(143, 160, 179);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(128, 232, 200);

#[derive(Clone, Copy, Debug, PartialEq)]
struct RippleColors {
    enabled: bool,
    base: [u8; 3],
    ripple: [u8; 3],
}

impl Default for RippleColors {
    fn default() -> Self {
        Self {
            enabled: false,
            base: [0, 32, 64],
            ripple: [120, 255, 255],
        }
    }
}

impl RippleColors {
    fn from_status(status: &Status) -> Self {
        let defaults = Self::default();
        Self {
            enabled: status.ripple_base_color.is_some() || status.ripple_color.is_some(),
            base: status
                .ripple_base_color
                .unwrap_or(if status.ripple_color.is_some() {
                    [0; 3]
                } else {
                    defaults.base
                }),
            ripple: status.ripple_color.unwrap_or(defaults.ripple),
        }
    }
}

fn color_hex(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

fn configure_style(context: &egui::Context) {
    let mut style = (*context.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = SURFACE;
    style.visuals.override_text_color = Some(INK);
    style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.25);
    style.visuals.selection.stroke.color = ACCENT;
    style.visuals.widgets.inactive.bg_fill = SURFACE;
    style.visuals.widgets.inactive.weak_bg_fill = SURFACE;
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(35, 48, 60);
    style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(44, 67, 70);
    style.visuals.slider_trailing_fill = true;
    style.spacing.item_spacing = egui::vec2(12.0, 12.0);
    style.spacing.button_padding = egui::vec2(16.0, 10.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    context.set_style(style);
}

fn effect_description(mode: &str) -> (&str, &str) {
    match mode {
        "ripples" => ("Ripples", "Light that follows your touch"),
        "comet" => ("Comet", "A quiet trail across your keys"),
        "rainbow" => ("Spectrum", "A continuous flow of color"),
        "breath" => ("Breathe", "Slow down. Fade in, fade out."),
        "matrix" => ("Matrix", "A little digital rainfall"),
        "focus-cockpit" => ("Focus", "Keep time, without the noise"),
        _ => ("Custom profile", "Your saved configuration"),
    }
}

fn primary_label(connected: bool, enabled: bool) -> &'static str {
    if !connected {
        "Start engine"
    } else if enabled {
        "Pause lighting"
    } else {
        "Resume lighting"
    }
}

fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(16)
        .inner_margin(20)
}

fn effect_card(ui: &mut egui::Ui, mode: &str, selected: bool, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 76.0), egui::Sense::click());
    let fill = if selected {
        egui::Color32::from_rgb(25, 49, 48)
    } else if response.hovered() {
        egui::Color32::from_rgb(30, 40, 53)
    } else {
        SURFACE
    };
    let outline = if selected || response.has_focus() {
        ACCENT
    } else {
        BORDER
    };
    ui.painter().rect(
        rect,
        12,
        fill,
        egui::Stroke::new(1.0, outline),
        egui::StrokeKind::Inside,
    );
    let (title, detail) = effect_description(mode);
    let text_color = if ui.is_enabled() { INK } else { MUTED };
    ui.painter().text(
        rect.min + egui::vec2(16.0, 22.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(16.0),
        text_color,
    );
    ui.painter().text(
        rect.min + egui::vec2(16.0, 49.0),
        egui::Align2::LEFT_CENTER,
        detail,
        egui::FontId::proportional(11.0),
        MUTED,
    );
    if selected {
        ui.painter()
            .circle_filled(rect.right_top() + egui::vec2(-18.0, 22.0), 3.5, ACCENT);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title)
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl Controller {
    fn ripple_preview_colors(&self) -> gui_preview::RippleColors {
        if !self.ripple_colors_dirty
            && let Some(status) = &self.status
        {
            return (status.ripple_base_color, status.ripple_color);
        }
        if self.ripple_colors.enabled {
            (
                Some(self.ripple_colors.base),
                Some(self.ripple_colors.ripple),
            )
        } else {
            (None, None)
        }
    }

    fn discard_settings(&mut self) {
        if let Some(status) = &self.status {
            self.brightness = status.brightness;
            self.palette = status.palette.clone();
            self.fps = status.fps;
            self.ripple_colors = RippleColors::from_status(status);
            self.settings_dirty = false;
            self.ripple_colors_dirty = false;
        }
    }

    fn apply_settings(&mut self) {
        let mut args = vec![
            "settings".into(),
            "--brightness".into(),
            self.brightness.to_string().into(),
            "--palette".into(),
            self.palette.clone().into(),
            "--fps".into(),
            self.fps.to_string().into(),
        ];
        if self.preset == "ripples" && self.ripple_colors_dirty {
            if self.ripple_colors.enabled {
                args.extend([
                    "--ripple-base-color".into(),
                    color_hex(self.ripple_colors.base).into(),
                    "--ripple-color".into(),
                    color_hex(self.ripple_colors.ripple).into(),
                ]);
            } else {
                args.push("--ripple-palette".into());
            }
        }
        self.dispatch(Action::Control(args));
    }

    fn show_settings(&mut self, context: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Settings").open(&mut open).default_width(480.0)
            .resizable(true).vscroll(true).show(context, |ui| {
            ui.label(egui::RichText::new("App settings").strong());
            ui.small("Effects, brightness, palette and frame rate are on the lighting page.");
            ui.add_enabled_ui(!self.pending, |ui| {
                ui.separator();
                ui.label(egui::RichText::new("Startup").strong());
                ui.small("Closing this window leaves the engine running. Login startup is always opt-in.");
                #[cfg(target_os = "macos")]
                if self.menu_bar.is_some() { ui.small("Use the WS menu-bar item to reopen or quit the controller."); }
                ui.small("Enable login starts the background engine at your next login, not now. Disable login also stops the managed engine.");
                if self.custom_state { ui.small("Service controls are unavailable with a custom state directory."); }
                ui.add_enabled_ui(!self.custom_state, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (label, verb) in [("Check startup status", "status"), ("Enable login", "enable"), ("Disable login", "disable")] {
                            if ui.button(label).clicked() { self.dispatch(Action::Service(verb.into())); }
                        }
                    });
                });
                ui.small(&self.service);
                if self.bundled_app {
                    ui.separator();
                    ui.label(egui::RichText::new("Updates & removal").strong());
                    ui.small("Before replacing this app, stop its engines, then quit the controller. Login preference and saved settings are kept.");
                    ui.add_enabled_ui(!self.custom_state, |ui| {
                        if ui.button("Stop engines for update").clicked() {
                            self.dispatch(Action::Service("prepare-update".into()));
                        }
                        ui.collapsing("Remove this app", |ui| {
                            ui.small("First disable login and stop background engines, then quit and move the app to Trash. Saved profiles, settings, and logs are kept.");
                            if ui.button("Disable login & stop engines").clicked() {
                                self.dispatch(Action::Service("remove".into()));
                            }
                        });
                    });
                }
                ui.separator();
                ui.collapsing("Import a trusted profile", |ui| {
                    ui.colored_label(egui::Color32::from_rgb(245, 192, 127), "Profiles can execute commands as your user.");
                    if ui.add(egui::TextEdit::singleline(&mut self.config_path).hint_text("/path/to/profile.toml").desired_width(f32::INFINITY)).changed() { self.trust_config = false; }
                    ui.checkbox(&mut self.trust_config, "I trust this file and its commands");
                    if ui.add_enabled(self.connected && self.trust_config && !self.config_path.trim().is_empty(), egui::Button::new("Use profile")).clicked() {
                        let path = self.config_path.clone(); self.control(&["select", "--config", &path]);
                    }
                });
            });
            ui.separator();
            ui.collapsing("Advanced engine controls", |ui| {
                ui.small("The engine runs your lighting in the background. For everyday use, pause or resume on the lighting page.");
                ui.label(&self.message);
                if let Some(status) = &self.status {
                    ui.small(format!("{}{} · recovery attempt {}", if self.connected { "" } else { "Last seen: " }, status.state, status.retry_attempt));
                }
                ui.add_enabled_ui(!self.pending, |ui| {
                    if ui.button("Refresh status").clicked() { self.control(&["status"]); }
                    ui.add_enabled_ui(!self.custom_state, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (label, verb) in [("Start managed service", "start"), ("Stop managed service", "stop")] {
                                if ui.button(label).clicked() { self.dispatch(Action::Service(verb.into())); }
                            }
                        });
                    });
                    if ui.add_enabled(self.connected && !self.dev_supervised, egui::Button::new("Stop standalone engine")).clicked() { self.control(&["stop"]); }
                    if self.dev_supervised { ui.small("The development watcher manages process lifetime. Use Ctrl-C in its terminal to stop the session."); }
                });
                ui.small("For a login-managed engine, use Stop managed service; stopping it directly may cause it to restart. Stopping a service keeps login enabled.");
                ui.collapsing("Diagnostics", |ui| {
                    ui.add(egui::TextEdit::multiline(&mut self.diagnostics).code_editor().interactive(false).desired_rows(10).desired_width(f32::INFINITY));
                });
            });
        });
        self.settings_open = open;
    }

    /// Paint only: separated from transport polling for hardware-free UI tests.
    fn show_lighting(&mut self, context: &egui::Context) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(28)).show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("W /").size(23.0).strong().color(ACCENT));
                    ui.label(egui::RichText::new("WOOTING SIGNALS").size(12.0).strong().color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Settings").clicked() { self.settings_open = !self.settings_open; }
                    });
                });
                ui.add_space(16.0);
                let enabled = self.connected && self.status.as_ref().is_some_and(|s| s.enabled);
                let state = if self.dev_simulation { "Development simulation · hardware disabled" } else if !self.connected { "Engine offline" } else {
                    match self.status.as_ref().map(|s| s.state.as_str()) {
                        Some("active") => "Lighting active", Some("retrying") => "Reconnecting",
                        Some("error") => "Needs attention", Some("paused") => "Paused · keyboard in control", _ => "Keyboard lighting",
                    }
                };
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Keyboard lighting").size(30.0).strong());
                        ui.label(egui::RichText::new(state).color(if enabled { ACCENT } else { MUTED }));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.dev_simulation { "Hardware disabled" } else if self.dev_supervised && !self.connected { "Waiting for engine" } else { primary_label(self.connected, enabled) };
                        let button = egui::Button::new(egui::RichText::new(label).strong().color(BG))
                            .fill(ACCENT).corner_radius(10).min_size(egui::vec2(156.0, 44.0));
                        if ui.add_enabled(!self.pending && !self.dev_simulation && (self.connected || !self.dev_supervised), button).clicked() {
                            if !self.connected { self.dispatch(Action::StartEngine); }
                            else { self.control(&[if enabled { "pause" } else { "resume" }]); }
                        }
                        ui.add_visible(self.pending, egui::Spinner::new());
                    });
                });
                ui.add_space(12.0);
                if let Some(error) = self.status.as_ref().and_then(|s| s.last_error.as_ref()) {
                    ui.colored_label(egui::Color32::from_rgb(245, 192, 127), if self.connected { error.clone() } else { format!("Last seen: {error}") });
                }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Lighting controls").size(18.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(self.settings_dirty && self.connected && !self.pending, egui::Button::new("Apply changes")).clicked() { self.apply_settings(); }
                        });
                    });
                    ui.add_enabled_ui(self.connected && !self.pending, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Brightness");
                            let slider_width = (ui.available_width() - 70.0).max(120.0);
                            ui.spacing_mut().slider_width = slider_width;
                            self.settings_dirty |= ui.add(egui::Slider::new(&mut self.brightness, 0..=255).show_value(false)).changed();
                            ui.label(format!("{}%", (u32::from(self.brightness) * 100 + 127) / 255));
                        });
                        if self.preset == "ripples" {
                            let mut changed = ui.checkbox(&mut self.ripple_colors.enabled, "Two-tone ripple").changed();
                            if self.ripple_colors.enabled {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label("Base color");
                                    changed |= ui.color_edit_button_srgb(&mut self.ripple_colors.base).changed();
                                    ui.small(color_hex(self.ripple_colors.base));
                                    ui.label("Ripple color");
                                    changed |= ui.color_edit_button_srgb(&mut self.ripple_colors.ripple).changed();
                                    ui.small(color_hex(self.ripple_colors.ripple));
                                });
                                ui.small("Keys stay lit in the base color; presses send waves of the ripple color. Black base restores a dark idle keyboard.");
                            }
                            self.settings_dirty |= changed;
                            self.ripple_colors_dirty |= changed;
                        }
                        if self.preset != "ripples" || !self.ripple_colors.enabled {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Palette");
                            for (palette, label) in PALETTES.iter().zip(["Wooting", "Neon", "Ocean", "Ember", "Terminal"]) {
                                if ui.selectable_label(self.palette == *palette, label).clicked() {
                                    self.palette = (*palette).into(); self.settings_dirty = true;
                                }
                            }
                        });
                        }
                        ui.horizontal(|ui| {
                            ui.label("Frame rate");
                            ui.spacing_mut().slider_width = (ui.available_width() - 110.0).max(120.0);
                            self.settings_dirty |= ui.add(egui::Slider::new(&mut self.fps, 1..=120).suffix(" FPS")).changed();
                        });
                        ui.small("More FPS can mean smoother or faster motion, with higher CPU usage.");
                        ui.horizontal(|ui| {
                            if ui.add_enabled(self.settings_dirty, egui::Button::new("Discard changes")).clicked() { self.discard_settings(); }
                            ui.label(egui::RichText::new(if self.settings_dirty { "Unapplied changes" } else { "Saved" }).small().color(MUTED));
                        });
                    });
                });
                ui.add_space(8.0);
                card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(effect_description(&self.preset).0).size(18.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(if self.preset == "ripples" { "RIPPLE SIMULATION" } else { "ILLUSTRATIVE PREVIEW" }).size(10.0).color(MUTED));
                        });
                    });
                    let ripple_colors = self.ripple_preview_colors();
                    gui_preview::keyboard(ui, &self.preset, &self.palette, self.brightness, ripple_colors, self.fps, &mut self.ripple_preview);
                    ui.label(egui::RichText::new(if self.preset == "ripples" { "80HE LED matrix · actual ripple math and draft settings · synthetic input, not device feedback" } else { "Simulation · not live input, device status or actual frame rate" }).small().color(MUTED));
                });
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Choose an effect").size(18.0).strong());
                let columns = if ui.available_width() >= 760.0 { 3 } else { 2 };
                let width = (ui.available_width() - 12.0 * (columns - 1) as f32) / columns as f32;
                ui.add_enabled_ui(self.connected && !self.pending, |ui| {
                    for row in PRESETS.chunks(columns) {
                        ui.horizontal(|ui| {
                            for mode in row {
                                if effect_card(ui, mode, self.preset == *mode, width).clicked() { self.control(&["select", "--preset", mode]); }
                            }
                        });
                    }
                });
                ui.add_space(4.0);
                ui.label(egui::RichText::new(if self.dev_supervised { "Development session · save source to rebuild · Ctrl-C stops owned processes" } else { "Pause returns control to your keyboard. Closing this window keeps your lighting running." }).small().color(MUTED));
            });
        });
        if self.settings_open {
            self.show_settings(context);
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
        if !self.pending && !self.poll_pending && self.last_poll.elapsed() >= Duration::from_secs(3)
        {
            self.control(&["status"]);
        }
        self.show_lighting(context);
        context.request_repaint_after(Duration::from_millis(50));
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
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([940.0, 850.0])
            .with_min_inner_size([620.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        if std::env::var_os("WOOTING_DEV_SUPERVISED").is_some() {
            "Wooting Signals — Dev"
        } else {
            "Wooting Signals"
        },
        options,
        Box::new(move |cc| Ok(Box::new(Controller::new(&cc.egui_ctx, backend)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted_text(shape: &egui::Shape, text: &mut String) {
        match shape {
            egui::Shape::Text(shape) => {
                text.push_str(&shape.galley.job.text);
                text.push('\n');
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    painted_text(shape, text);
                }
            }
            _ => {}
        }
    }
    #[test]
    fn primary_control_handles_offline_paused_and_enabled_states() {
        assert_eq!(primary_label(false, false), "Start engine");
        assert_eq!(primary_label(false, true), "Start engine");
        assert_eq!(primary_label(true, false), "Resume lighting");
        assert_eq!(primary_label(true, true), "Pause lighting");
    }

    #[test]
    fn supervised_and_simulated_gui_cannot_start_untracked_engines() {
        for (supervised, simulation, label, can_start) in [
            (false, false, "Start engine", true),
            (true, false, "Waiting for engine", false),
            (true, true, "Hardware disabled", false),
        ] {
            let (mut app, incoming) = controller_fixture();
            app.connected = false;
            app.dev_supervised = supervised;
            app.dev_simulation = simulation;
            let context = egui::Context::default();
            let input = |events| egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(940.0, 850.0),
                )),
                events,
                ..Default::default()
            };
            let output = context.run(input(vec![]), |ctx| app.show_lighting(ctx));
            let position = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .expect("primary button should be visible");
            for pressed in [true, false] {
                let _ = context.run(
                    input(vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]),
                    |ctx| app.show_lighting(ctx),
                );
            }
            assert_eq!(
                matches!(incoming.try_recv(), Ok(Action::StartEngine)),
                can_start
            );
        }
    }

    fn controller_fixture() -> (Controller, Receiver<Action>) {
        let (requests, incoming) = mpsc::channel();
        let (_outgoing, replies) = mpsc::channel();
        let app = Controller {
            #[cfg(target_os = "macos")]
            menu_bar: None,
            requests,
            replies,
            pending: false,
            poll_pending: false,
            queued_action: None,
            status: None,
            connected: true,
            message: "Fixture".into(),
            diagnostics: String::new(),
            service: String::new(),
            custom_state: true,
            bundled_app: true,
            preset: "comet".into(),
            brightness: 128,
            palette: "ocean".into(),
            fps: 30,
            ripple_colors: RippleColors::default(),
            ripple_colors_dirty: false,
            settings_dirty: false,
            config_path: String::new(),
            trust_config: false,
            last_poll: Instant::now(),
            settings_open: false,
            dev_supervised: false,
            dev_simulation: false,
            ripple_preview: gui_preview::RipplePreview::default(),
        };
        (app, incoming)
    }

    #[test]
    fn polling_is_silent_and_serializes_one_user_action_without_dropping_it() {
        let (mut app, incoming) = controller_fixture();
        let (outgoing, replies) = mpsc::channel();
        app.replies = replies;
        app.control(&["status"]);
        assert!(incoming.try_recv().unwrap().is_status());
        assert!(app.poll_pending);
        assert!(
            !app.pending,
            "polls must not disable controls or show a spinner"
        );
        app.control(&["status"]);
        assert!(incoming.try_recv().is_err(), "do not accumulate polls");
        app.control(&["pause"]);
        assert!(app.pending);
        assert!(
            incoming.try_recv().is_err(),
            "serialize behind the current poll"
        );
        app.control(&["resume"]);
        outgoing
            .send(Completion {
                action: Action::Control(vec!["status".into()]),
                result: Ok(format!(r#"{{"ok":true,"status":{STATUS}}}"#)),
            })
            .unwrap();
        app.receive();
        let Action::Control(args) = incoming.try_recv().unwrap() else {
            panic!("expected pause");
        };
        assert_eq!(args, [OsString::from("pause")]);
        assert!(app.pending);
        assert!(!app.poll_pending);
        assert!(app.queued_action.is_none());
        assert!(
            incoming.try_recv().is_err(),
            "only one explicit action may queue"
        );
        outgoing
            .send(Completion {
                action: Action::Control(vec!["pause".into()]),
                result: Err("fixture error".into()),
            })
            .unwrap();
        app.receive();
        assert!(!app.pending);
        assert!(!app.connected);
        assert!(
            incoming.try_recv().is_err(),
            "failed actions are never replayed"
        );
    }

    #[test]
    fn polling_and_dirty_edits_do_not_shift_the_controls() {
        let (mut app, _incoming) = controller_fixture();
        let context = egui::Context::default();
        configure_style(&context);
        let frame = |app: &mut Controller| {
            context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(940.0, 850.0),
                    )),
                    time: Some(1.0),
                    ..Default::default()
                },
                |ctx| app.show_lighting(ctx),
            )
        };
        // Warm up scroll/layout state before comparing frames.
        for _ in 0..3 {
            let _ = frame(&mut app);
        }
        let labels = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t)
                        if [
                            "Brightness",
                            "Palette",
                            "Frame rate",
                            "Apply changes",
                            "Discard changes",
                        ]
                        .contains(&t.galley.job.text.as_str()) =>
                    {
                        Some((t.galley.job.text.clone(), t.pos))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let before = labels(&frame(&mut app));
        assert_eq!(before.len(), 5);
        app.control(&["status"]);
        assert_eq!(labels(&frame(&mut app)), before);
        app.settings_dirty = true;
        assert_eq!(
            labels(&frame(&mut app)),
            before,
            "dragging must not move a slider under the pointer"
        );
        app.settings_dirty = false;
        assert_eq!(labels(&frame(&mut app)), before);
    }

    #[test]
    fn renders_compact_and_desktop_without_dispatching_commands() {
        let (mut app, incoming) = controller_fixture();
        for size in [egui::vec2(620.0, 640.0), egui::vec2(940.0, 850.0)] {
            for settings_open in [false, true] {
                let context = egui::Context::default();
                configure_style(&context);
                app.settings_open = settings_open;
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| app.show_lighting(ctx),
                );
                assert!(!output.shapes.is_empty());
                assert!(incoming.try_recv().is_err());
                if !settings_open {
                    let mut text = String::new();
                    for shape in &output.shapes {
                        painted_text(&shape.shape, &mut text);
                    }
                    for label in ["Lighting controls", "Brightness", "Palette", "Frame rate"] {
                        assert!(text.contains(label), "{label} missing at {size:?}");
                    }
                    assert!(!text.contains("Start managed service"));
                }
            }
        }

        // Settings never duplicates visual sliders or exposes advanced actions by default.
        let context = egui::Context::default();
        app.settings_open = true;
        for _ in 0..2 {
            let _ = context.run(egui::RawInput::default(), |ctx| app.show_settings(ctx));
        }
        let output = context.run(egui::RawInput::default(), |ctx| app.show_settings(ctx));
        let mut text = String::new();
        for shape in &output.shapes {
            painted_text(&shape.shape, &mut text);
        }
        assert!(text.contains("App settings"));
        assert!(text.contains("Advanced engine controls"));
        for label in [
            "Frame rate",
            "Apply changes",
            "Start managed service",
            "Stop standalone engine",
        ] {
            assert!(
                !text.contains(label),
                "{label} leaked into basic app settings"
            );
        }
        assert!(incoming.try_recv().is_err());
    }

    #[test]
    fn visual_edits_apply_together_and_discard_without_changing_engine() {
        let (mut app, incoming) = controller_fixture();
        // A single explicit apply sends all three visual settings together.
        app.brightness = 200;
        app.palette = "ember".into();
        app.fps = 60;
        app.settings_dirty = true;
        app.apply_settings();
        let Action::Control(args) = incoming.try_recv().unwrap() else {
            panic!("expected visual settings");
        };
        assert_eq!(
            args,
            [
                "settings",
                "--brightness",
                "200",
                "--palette",
                "ember",
                "--fps",
                "60"
            ]
            .map(OsString::from)
        );
        assert!(
            app.settings_dirty,
            "wait for confirmation before clearing edits"
        );
        app.pending = false;
        app.status = Some(serde_json::from_str(STATUS).unwrap());
        app.discard_settings();
        assert!(!app.settings_dirty);
        assert_eq!(app.brightness, 96);
        assert_eq!(app.palette, "wooting");
        assert_eq!(app.fps, 30);
        assert!(
            incoming.try_recv().is_err(),
            "discard must not change the engine"
        );
    }

    #[test]
    fn ripple_colors_are_explicit_preserved_and_only_sent_for_ripples() {
        let (mut app, incoming) = controller_fixture();
        app.preset = "ripples".into();
        app.ripple_colors = RippleColors {
            enabled: true,
            base: [0, 32, 64],
            ripple: [120, 255, 255],
        };
        app.ripple_colors_dirty = true;
        app.settings_dirty = true;
        app.apply_settings();
        let Action::Control(args) = incoming.try_recv().unwrap() else {
            panic!("expected settings");
        };
        assert_eq!(
            &args[7..],
            [
                "--ripple-base-color",
                "#002040",
                "--ripple-color",
                "#78ffff"
            ]
            .map(OsString::from)
        );

        app.pending = false;
        app.ripple_colors.enabled = false;
        app.apply_settings();
        let Action::Control(args) = incoming.try_recv().unwrap() else {
            panic!("expected settings");
        };
        assert_eq!(&args[7..], [OsString::from("--ripple-palette")]);

        // Brightness/FPS-only edits must preserve advanced, partially specified colors.
        app.pending = false;
        app.ripple_colors_dirty = false;
        app.apply_settings();
        let Action::Control(args) = incoming.try_recv().unwrap() else {
            panic!("expected settings");
        };
        assert_eq!(args.len(), 7);
        app.pending = false;
        app.preset = "comet".into();
        app.ripple_colors_dirty = true;
        app.apply_settings();
        let Action::Control(args) = incoming.try_recv().unwrap() else {
            panic!("expected settings");
        };
        assert_eq!(args.len(), 7);

        let mut status: Status = serde_json::from_str(STATUS).unwrap();
        assert!(!RippleColors::from_status(&status).enabled);
        status.ripple_base_color = Some([12, 34, 56]);
        status.ripple_color = Some([200, 150, 100]);
        app.status = Some(status);
        app.discard_settings();
        assert_eq!(
            app.ripple_colors,
            RippleColors {
                enabled: true,
                base: [12, 34, 56],
                ripple: [200, 150, 100]
            }
        );
        assert!(!app.ripple_colors_dirty);
        assert!(!app.settings_dirty);
    }

    #[test]
    fn ripple_color_pickers_render_on_the_main_page() {
        let (mut app, incoming) = controller_fixture();
        app.preset = "ripples".into();
        app.ripple_colors.enabled = true;
        let context = egui::Context::default();
        configure_style(&context);
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(620.0, 640.0),
                )),
                ..Default::default()
            },
            |ctx| app.show_lighting(ctx),
        );
        let mut text = String::new();
        for shape in &output.shapes {
            painted_text(&shape.shape, &mut text);
        }
        for label in [
            "Two-tone ripple",
            "Base color",
            "Ripple color",
            "Frame rate",
        ] {
            assert!(text.contains(label), "{label} missing in compact layout");
        }
        assert!(!text.contains("Palette"));
        assert!(incoming.try_recv().is_err());
    }
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
            poll_pending: false,
            queued_action: None,
            status: None,
            connected: false,
            message: String::new(),
            diagnostics: String::new(),
            service: String::new(),
            custom_state: false,
            bundled_app: false,
            preset: "ripples".into(),
            brightness: 7,
            palette: "custom".into(),
            fps: 12,
            ripple_colors: RippleColors::default(),
            ripple_colors_dirty: false,
            settings_dirty: true,
            config_path: String::new(),
            trust_config: false,
            last_poll: Instant::now() - Duration::from_secs(10),
            settings_open: false,
            dev_supervised: false,
            dev_simulation: false,
            ripple_preview: gui_preview::RipplePreview::default(),
        };
        app.ripple_colors = RippleColors {
            enabled: true,
            base: [20, 40, 60],
            ripple: [200, 180, 160],
        };
        app.ripple_colors_dirty = true;
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
        assert_eq!(app.ripple_colors.base, [20, 40, 60]);
        assert!(app.ripple_colors_dirty);
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
        assert!(!app.ripple_colors.enabled);
        assert!(!app.ripple_colors_dirty);
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
        // Under load the fixture may exit after the short startup observation
        // window. Both outcomes must expose diagnostics without retaining a pipe.
        let deadline = Instant::now() + Duration::from_secs(3);
        let log = loop {
            let log = std::fs::read_to_string(directory.join("state/engine.log")).unwrap();
            if log.contains("fixture-start-failure") || Instant::now() >= deadline {
                break log;
            }
            thread::sleep(Duration::from_millis(10));
        };
        std::fs::remove_dir_all(&directory).unwrap();
        match result {
            Err(error) => assert!(error.contains("fixture-start-failure")),
            Ok(message) => assert!(message.contains("engine.log")),
        }
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
