//! Diagnostics and synthetic previews must work without a keyboard or live SDK.
#![cfg(unix)]
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const BINARY: &str = env!("CARGO_BIN_EXE_underglow");
const CONFIG: &str = r#"
effect = 'matrix'
brightness = 255
[[automation.rules]]
id = 'present'
source = 'pi'
mode = 'while-state'
scene = 'idle'
[[automation.rules]]
id = 'working'
source = 'pi'
mode = 'while-state'
state = 'working'
scene = 'working'
priority = 10
[[automation.rules]]
id = 'finished'
source = 'pi'
mode = 'on-event'
event = 'finished'
scene = 'done'
priority = 20
duration_ms = 1000
[automation.scenes.idle]
zones = ['function']
color = [0, 100, 100]
[automation.scenes.working]
zones = ['function']
color = [0, 0, 255]
[automation.scenes.done]
zones = ['function']
color = [0, 255, 0]
"#;

struct Harness {
    dir: PathBuf,
    child: Option<Child>,
}
impl Harness {
    fn new() -> Self {
        let dir = PathBuf::from("/tmp").join(format!(
            "ug-diag-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::create_dir(dir.join("home")).unwrap();
        fs::write(dir.join("profile.toml"), CONFIG).unwrap();
        Self { dir, child: None }
    }
    fn command(&self) -> Command {
        let mut c = Command::new(BINARY);
        c.env("HOME", self.dir.join("home"))
            .env("WOOTING_STATE_DIR", self.dir.join("runtime"))
            .env("WOOTING_DEV_SIMULATION", "1")
            .env("WOOTING_RGB_SDK_PATH", "/nonexistent-sdk")
            .env("WOOTING_ANALOG_SDK_PATH", "/nonexistent-sdk");
        c
    }
    fn cli(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn preview(&self, extra: &[&str]) -> Value {
        let mut args = vec!["diagnostics", "preview", "--config"];
        let path = self.dir.join("profile.toml");
        args.push(path.to_str().unwrap());
        args.extend_from_slice(extra);
        self.cli(&args)
    }
    fn start(&mut self) {
        self.child = Some(
            self.command()
                .args(["engine", "--paused", "--config"])
                .arg(self.dir.join("profile.toml"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.dir.join("runtime/events.sock").exists() {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "engine exited"
            );
            assert!(Instant::now() < deadline, "startup timeout");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn preview_uses_real_rules_and_compositor_without_ipc_or_hardware() {
    let h = Harness::new();
    let report = h.preview(&["--event", "finished"]);
    let diag = &report["diagnostics"];
    assert_eq!(diag["evaluation"], "synthetic");
    assert_eq!(diag["lighting_applied"], false);
    assert_eq!(diag["instances"][0]["instance"], "preview");
    let rules = diag["rules"].as_array().unwrap();
    assert_eq!(rules[0]["obscured"], true);
    assert_eq!(rules[1]["obscured"], true);
    assert_eq!(rules[2]["active"], true);
    assert_eq!(rules[2]["visible_keys"], rules[2]["targeted_keys"]);
    assert!(
        report["preview"]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k["rgb"] == serde_json::json!([0, 255, 0]))
    );
    let expired = h.preview(&["--event", "finished", "--elapsed-ms", "1000"]);
    assert_eq!(expired["diagnostics"]["rules"][2]["active"], false);
    assert_eq!(expired["diagnostics"]["rules"][1]["obscured"], false);
    assert!(
        !h.dir.join("runtime").exists(),
        "offline preview must not create state or connect to engine"
    );
}

#[test]
fn preview_never_initializes_configured_command_provider() {
    let h = Harness::new();
    let marker = h.dir.join("executed");
    fs::write(h.dir.join("profile.toml"), format!("{CONFIG}\n[signal]\nkind='command-pulse'\n[signal.command_pulse]\ncommand=['touch','{}']\n", marker.display())).unwrap();
    h.preview(&[]);
    assert!(!marker.exists());
}

#[test]
fn paused_diagnostics_report_leases_and_potential_rules_without_replaying_events() {
    let mut h = Harness::new();
    h.start();
    h.cli(&[
        "external",
        "state",
        "--source",
        "pi",
        "--instance",
        "real",
        "--sequence",
        "1",
        "--state",
        "working",
        "--ttl-ms",
        "10000",
    ]);
    h.cli(&[
        "external",
        "event",
        "--source",
        "pi",
        "--instance",
        "real",
        "--sequence",
        "2",
        "--event",
        "finished",
    ]);
    let saved = fs::read(h.dir.join("runtime/state.json")).unwrap();
    let value = h.cli(&["control", "diagnostics"]);
    assert_eq!(value["status"]["state"], "paused");
    let diag = &value["diagnostics"];
    assert_eq!(diag["evaluation"], "potential");
    assert_eq!(diag["lighting_applied"], false);
    assert_eq!(diag["instances"][0]["instance"], "real");
    assert_eq!(diag["instances"][0]["available"], true);
    assert!(diag["instances"][0]["lease_remaining_ms"].as_u64().unwrap() > 0);
    assert_eq!(diag["rules"][1]["matched"], true);
    assert_eq!(
        diag["rules"][2]["active"], false,
        "paused diagnostics must not replay old events"
    );
    h.preview(&[]);
    assert_eq!(
        h.cli(&["external", "inspect"])["snapshot"]["states"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "preview must not publish synthetic instance"
    );
    assert_eq!(fs::read(h.dir.join("runtime/state.json")).unwrap(), saved);
    h.cli(&[
        "external",
        "remove",
        "--source",
        "pi",
        "--instance",
        "real",
        "--sequence",
        "3",
    ]);
    let removed = h.cli(&["control", "diagnostics"]);
    assert!(
        removed["diagnostics"]["instances"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(removed["diagnostics"]["rules"][1]["matched"], false);
    h.cli(&["control", "stop"]);
    assert!(h.child.take().unwrap().wait().unwrap().success());
}
