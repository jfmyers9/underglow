//! Zero-config development contract across Python, Node, CLI and the real engine.
//! Simulation only: no model calls, real GUI, SDK, global settings or installed services.
#![cfg(unix)]

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const REPOSITORY: &str = env!("CARGO_MANIFEST_DIR");
const BINARY: &str = env!("CARGO_BIN_EXE_underglow");
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Peer {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<String>,
}

impl Peer {
    fn start(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            output,
        }
    }

    fn read(&self) -> Value {
        let line = self
            .output
            .recv_timeout(Duration::from_secs(12))
            .expect("fixture response timeout/exit");
        serde_json::from_str(&line).expect("fixture emitted non-JSON")
    }

    fn action(&mut self, action: &str) -> Value {
        writeln!(
            self.input.as_mut().unwrap(),
            "{}",
            json!({"action": action})
        )
        .unwrap();
        self.read()
    }

    fn finish(&mut self) {
        // EOF lets the fixture clean up its own process groups, including on panic.
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(8);
        while self.child.try_wait().ok().flatten().is_none() {
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.wait();
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.finish();
    }
}

struct Harness {
    supervisor: Peer,
    checkout: PathBuf,
    runtime: PathBuf,
    state: PathBuf,
    descriptor: Value,
}

fn isolated(command: &mut Command, home: &Path) {
    command
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join("state"));
    for key in [
        "WOOTING_PI_SOCKET",
        "WOOTING_STATE_DIR",
        "WOOTING_DEV_SIMULATION",
        "WOOTING_DEV_SUPERVISED",
    ] {
        command.env_remove(key);
    }
    command
        .env("WOOTING_RGB_SDK_PATH", home.join("no-sdk"))
        .env("WOOTING_ANALOG_SDK_PATH", home.join("no-sdk"));
}

impl Harness {
    fn start(existing: bool) -> Self {
        let checkout = PathBuf::from("/tmp").join(format!(
            "ug-workflow-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&checkout).unwrap();
        fs::create_dir(checkout.join("home")).unwrap();
        let mut command = Command::new("python3");
        command
            .arg(Path::new(REPOSITORY).join("tests/fixtures/dev_workflow.py"))
            .args([
                REPOSITORY,
                checkout.to_str().unwrap(),
                BINARY,
                if existing { "existing" } else { "fresh" },
            ]);
        isolated(&mut command, &checkout.join("home"));
        let supervisor = Peer::start(command);
        let ready = supervisor.read();
        let descriptor = ready["descriptor"].clone();
        let runtime = PathBuf::from(ready["runtime"].as_str().unwrap());
        let state = PathBuf::from(descriptor["state_dir"].as_str().unwrap());
        assert_eq!(descriptor["mode"], "simulation");
        assert!(
            state.join("events.sock").exists(),
            "metadata published before ingress ready"
        );
        Self {
            supervisor,
            checkout,
            runtime,
            state,
            descriptor,
        }
    }

    fn pi(&self) -> Peer {
        let mut command = Command::new("node");
        command
            .arg(Path::new(REPOSITORY).join("tests/fixtures/dev_workflow.mjs"))
            .arg(REPOSITORY)
            .arg(&self.checkout);
        isolated(&mut command, &self.checkout.join("home"));
        let peer = Peer::start(command);
        let ready = peer.read();
        assert_eq!(ready["target"]["path"], self.descriptor["socket"]);
        peer
    }

    fn cli(&self, args: &[&str]) -> Value {
        let mut command = Command::new(BINARY);
        isolated(&mut command, &self.checkout.join("home"));
        command
            .env("WOOTING_DEV_SIMULATION", "1")
            .env("WOOTING_STATE_DIR", &self.state)
            .args(args);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn profile(&self) -> toml::Value {
        let saved: Value =
            serde_json::from_slice(&fs::read(self.state.join("state.json")).unwrap()).unwrap();
        assert_eq!(saved["enabled"], false);
        toml::from_str(saved["config"].as_str().unwrap()).unwrap()
    }

    fn preview_until(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let value = self.cli(&["control", "preview"]);
            assert_eq!(value["status"]["state"], "paused");
            assert_eq!(value["status"]["enabled"], false);
            assert_eq!(value["preview"]["schema_version"], 1);
            assert_eq!(value["preview"]["kind"], "simulated-live-sources");
            assert_eq!(value["preview"]["rgb"].as_array().unwrap().len(), 378);
            assert!(!self.state.join("hardware.lock").exists());
            if predicate(&value) {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "preview condition timed out: {value}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.supervisor.finish();
        let _ = fs::remove_dir_all(&self.runtime);
        let _ = fs::remove_dir_all(&self.checkout);
    }
}

fn overlay_colors(value: &Value) -> Vec<[u64; 3]> {
    value["preview"]["overlay_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|key| {
            // Named function-zone targeting, not a full-keyboard replacement.
            assert_eq!(key[0], 0);
            let index = (key[0].as_u64().unwrap() * 21 + key[1].as_u64().unwrap()) as usize * 3;
            std::array::from_fn(|channel| {
                value["preview"]["rgb"][index + channel].as_u64().unwrap()
            })
        })
        .collect()
}

fn blue(value: &Value) -> bool {
    overlay_colors(value)
        .iter()
        .any(|c| c[2] > 50 && c[2] > c[1] * 2)
}
fn green(value: &Value) -> bool {
    overlay_colors(value)
        .iter()
        .any(|c| c[1] > 50 && c[1] > c[2] * 2)
}
fn cyan(value: &Value) -> bool {
    let colors = overlay_colors(value);
    !colors.is_empty()
        && colors
            .iter()
            .all(|c| c[0] == 0 && c[1] > 0 && c[2] > c[1] && c[2] < c[1] * 2)
}

#[test]
fn make_dev_discovers_pi_and_previews_lifecycle_without_resuming_hardware() {
    let mut h = Harness::start(false);
    let profile = h.profile();
    assert_eq!(profile["effect"].as_str(), Some("matrix"));
    assert_eq!(profile["automation"]["rules"].as_array().unwrap().len(), 3);
    let mut pi = h.pi();
    pi.action("session_start");
    h.preview_until(cyan);
    pi.action("agent_start");
    h.preview_until(blue);
    pi.action("agent_settled");
    h.preview_until(green);
    h.preview_until(cyan); // The completion expires; persistent idle survives.

    let before = h.descriptor.clone();
    let gui = h.supervisor.action("gui")["descriptor"].clone();
    assert_eq!(gui, before, "GUI-only rebuild changed engine discovery");
    pi.action("agent_start");
    h.preview_until(blue);
    let restarted = h.supervisor.action("engine")["descriptor"].clone();
    assert_ne!(restarted["nonce"], before["nonce"]);
    assert_ne!(restarted["engine_pid"], before["engine_pid"]);
    h.preview_until(blue); // Same connector rediscovers and republishes current state.
    pi.action("agent_settled");
    h.preview_until(green);
    h.supervisor.action("engine");
    h.preview_until(cyan); // Completion from the old engine must not replay.
    let events = h.cli(&["external", "inspect"]);
    assert!(events["events"]["events"].as_array().unwrap().is_empty());

    let closed = h.supervisor.action("close");
    assert_eq!(closed["discovery_exists"], false);
    assert!(!h.runtime.join("current.json").exists());
    assert_eq!(
        pi.action("target")["target"],
        Value::Null,
        "project connector fell back to installed state"
    );
    pi.action("agent_start"); // Missing engine remains harmless to Pi.
}

#[test]
fn make_dev_keeps_saved_animation_settings_while_adding_pi_rules() {
    let h = Harness::start(true);
    let profile = h.profile();
    assert_eq!(profile["effect"].as_str(), Some("breath"));
    assert_eq!(profile["brightness"].as_integer(), Some(73));
    assert_eq!(profile["speed"].as_integer(), Some(65));
    assert_eq!(profile["automation"]["rules"].as_array().unwrap().len(), 3);
    let mut pi = h.pi();
    pi.action("session_start");
    pi.action("agent_start");
    let preview = h.preview_until(blue);
    assert_eq!(preview["preview"]["base_effect"], "breath");
    pi.action("session_shutdown");
    h.preview_until(|value| overlay_colors(value).is_empty());
    // A producer that disappears without remove must not leave an indicator stuck.
    // Use a short protocol lease rather than waiting for Pi's production 15 seconds.
    h.cli(&[
        "external",
        "state",
        "--source",
        "pi",
        "--instance",
        "crashed-session",
        "--sequence",
        "1",
        "--state",
        "idle",
        "--ttl-ms",
        "150",
    ]);
    h.preview_until(cyan);
    h.preview_until(|value| overlay_colors(value).is_empty());
}
