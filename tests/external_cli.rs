//! Real publishing CLI/socket tests. Every engine is isolated and hardware-disabled.
#![cfg(unix)]
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const BINARY: &str = env!("CARGO_BIN_EXE_underglow");
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Harness {
    dir: PathBuf,
    child: Option<Child>,
    library: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let dir = PathBuf::from("/tmp").join(format!(
            "ug-ext-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::create_dir(dir.join("home")).unwrap();
        fs::create_dir(dir.join("runtime")).unwrap();
        // Detect even a dlopen attempt, not just a hardware initialization call.
        let source = dir.join("sdk.c");
        fs::write(
            &source,
            r#"#include <stdio.h>
#include <stdlib.h>
__attribute__((constructor)) static void loaded(void) {
  FILE *f = fopen(getenv("MOCK_LOG"), "a");
  if (f) { fputs("SDK LOADED\n", f); fclose(f); }
}
"#,
        )
        .unwrap();
        let library = dir.join(if cfg!(target_os = "macos") {
            "sdk.dylib"
        } else {
            "sdk.so"
        });
        let output = Command::new("cc")
            .args(if cfg!(target_os = "macos") {
                vec!["-dynamiclib"]
            } else {
                vec!["-shared", "-fPIC"]
            })
            .arg(source)
            .arg("-o")
            .arg(&library)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self {
            dir,
            child: None,
            library,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(BINARY);
        command
            .env("HOME", self.dir.join("home"))
            .env("WOOTING_STATE_DIR", self.dir.join("runtime"))
            .env("WOOTING_DEV_SIMULATION", "1")
            .env_remove("WOOTING_DEV_SUPERVISED")
            .env("WOOTING_RGB_SDK_PATH", &self.library)
            .env("WOOTING_ANALOG_SDK_PATH", &self.library)
            .env("MOCK_LOG", self.dir.join("sdk.log"));
        command
    }

    fn start(&mut self) {
        self.start_mode(false);
    }

    fn start_mode(&mut self, supervised: bool) {
        let mut command = self.command();
        if supervised {
            command.env("WOOTING_DEV_SUPERVISED", "1");
        }
        let child = self.start_command(command).spawn().unwrap();
        self.child = Some(child);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if self.dir.join("runtime/events.sock").exists()
                && self.dir.join("runtime/state.json").exists()
            {
                break;
            }
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "engine exited: {}",
                fs::read_to_string(self.dir.join("engine.log")).unwrap()
            );
            assert!(Instant::now() < deadline, "engine startup timeout");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn start_command(&self, mut command: Command) -> Command {
        command
            .args(["engine", "--paused"])
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                File::create(self.dir.join("engine.log")).unwrap(),
            ));
        command
    }

    fn cli(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{:?}: {} {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn state(&self, instance: &str, sequence: &str, ttl_ms: &str) -> Value {
        self.cli(&[
            "external",
            "state",
            "--source",
            "pi",
            "--instance",
            instance,
            "--sequence",
            sequence,
            "--state",
            "working",
            "--ttl-ms",
            ttl_ms,
        ])
    }

    fn connect(&self) -> UnixStream {
        let stream = UnixStream::connect(self.dir.join("runtime/events.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
    }

    fn raw(&self, bytes: &[u8]) -> Value {
        let mut stream = self.connect();
        stream.write_all(bytes).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        read_response(stream)
    }

    fn inspect(&self) -> Value {
        self.cli(&["external", "inspect"])
    }

    fn assert_paused_and_no_sdk(&self) {
        let status = self.cli(&["control", "status"]);
        assert_eq!(status["status"]["state"], "paused");
        assert_eq!(status["status"]["enabled"], false);
        assert!(
            !self.dir.join("runtime/hardware.lock").exists(),
            "hardware lease was acquired"
        );
        assert!(!self.dir.join("sdk.log").exists(), "SDK was loaded");
    }

    fn stop(&mut self) {
        self.cli(&["control", "stop"]);
        assert!(self.child.take().unwrap().wait().unwrap().success());
        assert!(!self.dir.join("runtime/events.sock").exists());
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

fn read_response(stream: UnixStream) -> Value {
    let mut bytes = Vec::new();
    BufReader::new(stream)
        .read_until(b'\n', &mut bytes)
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn preview_zone(value: &Value, zone: underglow::layout::Zone) -> Vec<[u64; 3]> {
    let info = underglow::device::DeviceInfo::synthetic_80he();
    underglow::layout::KeyboardLayout::for_device(&info)
        .keys()
        .iter()
        .filter(|key| key.zone == zone)
        .map(|key| {
            let index = (usize::from(key.coord.row) * 21 + usize::from(key.coord.column)) * 3;
            std::array::from_fn(|channel| {
                value["preview"]["rgb"][index + channel].as_u64().unwrap()
            })
        })
        .collect()
}

#[test]
fn paused_live_preview_reads_real_external_sources_without_sdk_or_providers() {
    use underglow::layout::Zone;
    let mut h = Harness::new();
    h.start();
    let profile = h.dir.join("profile.toml");
    // A side-effecting background exists, but preview must never initialize it.
    let marker = h.dir.join("provider-ran");
    let config = format!(
        "{}\n[signal]\nkind='command-pulse'\ncommand=['touch','{}']\n",
        include_str!("../examples/matrix-pi.toml"),
        marker.display()
    );
    fs::write(&profile, config).unwrap();
    h.cli(&["control", "select", "--config", profile.to_str().unwrap()]);
    h.state("a", "1", "5000");
    h.state("b", "1", "5000");
    let working = h.cli(&["control", "preview"]);
    assert!(
        !working["preview"]["overlay_keys"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        preview_zone(&working, Zone::Function)
            .iter()
            .any(|c| c[2] > 100 && c[2] > c[1] * 2)
    );
    h.cli(&[
        "external",
        "remove",
        "--source",
        "pi",
        "--instance",
        "a",
        "--sequence",
        "2",
    ]);
    let second = h.cli(&["control", "preview"]);
    assert!(
        preview_zone(&second, Zone::Function)
            .iter()
            .any(|c| c[2] > 100 && c[2] > c[1] * 2)
    );
    h.cli(&[
        "external",
        "event",
        "--source",
        "pi",
        "--instance",
        "b",
        "--sequence",
        "2",
        "--event",
        "finished",
        "--ttl-ms",
        "2500",
    ]);
    let finished = h.cli(&["control", "preview"]);
    assert!(
        preview_zone(&finished, Zone::Function)
            .iter()
            .any(|c| c[1] > 100 && c[1] > c[2] * 2)
    );
    // A new control connection sees the same active alert, not a fresh baseline.
    let reconnect = h.cli(&["control", "preview"]);
    assert!(
        preview_zone(&reconnect, Zone::Function)
            .iter()
            .any(|c| c[1] > 100 && c[1] > c[2] * 2)
    );
    // Switching profile baselines events away while retaining current persistent state.
    h.cli(&["control", "select", "--config", profile.to_str().unwrap()]);
    let switched = h.cli(&["control", "preview"]);
    assert!(
        preview_zone(&switched, Zone::Function)
            .iter()
            .any(|c| c[2] > 100 && c[2] > c[1] * 2)
    );
    h.cli(&[
        "external",
        "state",
        "--source",
        "pi",
        "--instance",
        "b",
        "--sequence",
        "3",
        "--state",
        "idle",
        "--ttl-ms",
        "100",
    ]);
    let start = h.cli(&["control", "preview"]);
    let deadline = Instant::now() + Duration::from_secs(3);
    let expired = loop {
        std::thread::sleep(Duration::from_millis(50));
        let frame = h.cli(&["control", "preview"]);
        if frame["preview"]["overlay_keys"]
            .as_array()
            .unwrap()
            .is_empty()
        {
            break frame;
        }
        assert!(Instant::now() < deadline, "state did not expire: {frame}");
    };
    assert_ne!(
        preview_zone(&start, Zone::Alpha),
        preview_zone(&expired, Zone::Alpha)
    );
    assert!(!marker.exists(), "preview initialized configured provider");
    h.assert_paused_and_no_sdk();
    h.stop();
}

#[test]
fn dev_setup_is_guarded_preserves_visuals_and_keeps_pi_when_effect_changes() {
    let mut h = Harness::new();
    h.start();
    let rejected = h.command().args(["control", "dev-setup"]).output().unwrap();
    assert!(!rejected.status.success());
    h.stop();
    h.start_mode(true);
    let before = h.cli(&["control", "status"]);
    h.cli(&["control", "dev-setup"]);
    let after = h.cli(&["control", "status"]);
    assert_eq!(before["status"], after["status"]);
    h.state("a", "1", "5000");
    assert!(
        !h.cli(&["control", "preview"])["preview"]["overlay_keys"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    h.cli(&["control", "select", "--preset", "aurora"]);
    let changed = h.cli(&["control", "preview"]);
    assert_eq!(changed["status"]["mode"], "aurora");
    assert!(
        !changed["preview"]["overlay_keys"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    h.assert_paused_and_no_sdk();
    h.stop();
}

#[test]
fn independent_instances_events_removal_expiry_and_restart_stay_paused() {
    let mut h = Harness::new();
    h.start();
    assert_eq!(h.state("a", "1", "5000")["outcome"], "applied");
    h.state("b", "1", "1000");
    assert_eq!(
        h.inspect()["snapshot"]["states"].as_array().unwrap().len(),
        2
    );
    assert_eq!(h.state("a", "1", "5000")["outcome"], "ignored_stale");
    h.cli(&[
        "external",
        "event",
        "--source",
        "pi",
        "--instance",
        "a",
        "--sequence",
        "2",
        "--event",
        "finished",
        "--ttl-ms",
        "1000",
    ]);
    assert_eq!(h.inspect()["events"]["events"][0]["event"], "finished");
    h.cli(&[
        "external",
        "remove",
        "--source",
        "pi",
        "--instance",
        "a",
        "--sequence",
        "3",
    ]);
    let observation = h.inspect();
    assert_eq!(
        observation["snapshot"]["states"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        observation["snapshot"]["states"][0]["identity"]["instance"],
        "b"
    );
    std::thread::sleep(Duration::from_millis(1100));
    let observation = h.inspect();
    assert_eq!(observation["snapshot"]["states"], json!([]));
    assert_eq!(observation["events"]["events"], json!([]));
    h.assert_paused_and_no_sdk();
    h.state("fresh", "1", "15000");
    let saved = fs::read_to_string(h.dir.join("runtime/state.json")).unwrap();
    assert!(!saved.contains("fresh"), "ephemeral data was persisted");
    h.stop();
    h.start();
    assert_eq!(h.inspect()["snapshot"]["states"], json!([]));
    h.assert_paused_and_no_sdk();
    h.stop();
}

#[test]
fn malformed_oversized_and_control_requests_are_rejected_without_side_effects() {
    let mut h = Harness::new();
    h.start();
    assert_eq!(h.raw(b"not-json\n")["error"]["code"], "invalid_json");
    assert_eq!(
        h.raw(&vec![b'x'; 5000])["error"]["code"],
        "message_too_large"
    );
    assert_eq!(
        h.raw(b"{\"schema_version\":1}")["error"]["code"],
        "invalid_frame"
    );
    for action in ["resume", "select", "execute", "pause", "stop", "settings"] {
        let mut request = serde_json::to_vec(
            &json!({"schema_version":1,"action":action,"config":"bad","command":"touch bad"}),
        )
        .unwrap();
        request.push(b'\n');
        assert_eq!(
            h.raw(&request)["ok"],
            false,
            "accepted control action {action}"
        );
    }
    let mut request = serde_json::to_vec(&json!({"schema_version":2,"source":"pi","instance":"a","sequence":1,"payload":{"kind":"state","state":"working","ttl_ms":15000}})).unwrap();
    request.push(b'\n');
    assert_eq!(h.raw(&request)["error"]["code"], "unsupported_version");
    let output = h
        .command()
        .args([
            "external",
            "state",
            "--source",
            "pi",
            "--instance",
            "a",
            "--sequence",
            "1",
            "--state",
            "working",
            "--ttl-ms",
            "0",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
        "invalid_ttl"
    );
    assert_eq!(h.inspect()["snapshot"]["states"], json!([]));
    h.assert_paused_and_no_sdk();
    h.stop();
}

#[test]
fn slow_clients_cannot_block_control_or_other_publishers_and_have_deadlines() {
    let mut h = Harness::new();
    h.start();
    let mut slow = h.connect();
    slow.write_all(b"{").unwrap();
    std::thread::sleep(Duration::from_millis(20));
    let start = Instant::now();
    h.state("fast", "1", "15000");
    h.assert_paused_and_no_sdk();
    assert!(
        start.elapsed() < Duration::from_millis(700),
        "slow publisher blocked unrelated work"
    );
    assert_eq!(read_response(slow)["error"]["code"], "timeout");

    // Exhaust the bounded client pool; control uses its own endpoint and still responds.
    let mut clients = Vec::new();
    for _ in 0..32 {
        clients.push(h.connect());
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::sleep(Duration::from_millis(25));
    let mut extra = h.connect();
    extra
        .write_all(b"{\"schema_version\":1,\"kind\":\"inspect\"}\n")
        .unwrap();
    assert_eq!(read_response(extra)["error"]["code"], "busy");
    let start = Instant::now();
    h.assert_paused_and_no_sdk();
    assert!(start.elapsed() < Duration::from_millis(700));
    drop(clients);
    std::thread::sleep(Duration::from_millis(30));
    h.state("recovered", "1", "15000");
    h.stop();
}

#[test]
fn raw_stdin_and_explicit_state_directory_work_without_hardware() {
    let mut h = Harness::new();
    h.start();
    let mut child = h
        .command()
        .arg("external")
        .arg("--state-dir")
        .arg(h.dir.join("runtime"))
        .arg("publish")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(br#"{"schema_version":1,"source":"custom","instance":"script","sequence":1,"payload":{"kind":"state","state":"working","ttl_ms":15000}}"#).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        h.inspect()["snapshot"]["states"][0]["identity"]["source"],
        "custom"
    );
    assert_eq!(
        fs::metadata(h.dir.join("runtime"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(h.dir.join("runtime/events.sock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    h.assert_paused_and_no_sdk();
    h.stop();
}

#[test]
fn unsafe_socket_paths_and_live_endpoints_are_preserved_but_stale_socket_recovers() {
    let mut h = Harness::new();
    let path = h.dir.join("runtime/events.sock");
    fs::write(&path, "keep").unwrap();
    let output = h.command().args(["engine", "--paused"]).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep");
    fs::remove_file(&path).unwrap();
    let target = h.dir.join("target");
    fs::write(&target, "keep").unwrap();
    symlink(&target, &path).unwrap();
    let output = h.command().args(["engine", "--paused"]).output().unwrap();
    assert!(!output.status.success());
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::remove_file(&path).unwrap();
    let listener = UnixListener::bind(&path).unwrap();
    let output = h.command().args(["engine", "--paused"]).output().unwrap();
    assert!(!output.status.success());
    assert!(path.exists());
    drop(listener);
    h.start();
    h.state("after-stale", "1", "15000");
    h.assert_paused_and_no_sdk();
    h.stop();
}
