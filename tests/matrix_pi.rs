//! Starter-profile acceptance tests. No real SDK, HID, installed engine, or model run.
//! Requires a C compiler and Node (also required by Pi).
#![cfg(unix)]
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use underglow::device::DeviceInfo;
use underglow::layout::{KeyboardLayout, Zone};
use underglow::render::Color;

const BINARY: &str = env!("CARGO_BIN_EXE_underglow");
const PROFILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/matrix-pi.toml");
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Harness {
    dir: PathBuf,
    engine: Option<Child>,
}
impl Harness {
    fn new() -> Self {
        let dir = PathBuf::from(format!(
            "/tmp/ug-mpi-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::create_dir(dir.join("home")).unwrap();
        let output = Command::new("cc")
            .args(if cfg!(target_os = "macos") {
                vec!["-dynamiclib"]
            } else {
                vec!["-shared", "-fPIC"]
            })
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/matrix_pi_sdk.c"
            ))
            .arg("-o")
            .arg(dir.join("sdk.so"))
            .output()
            .expect("C compiler required");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self { dir, engine: None }
    }
    fn command(&self) -> Command {
        let mut c = Command::new(BINARY);
        c.env("HOME", self.dir.join("home"))
            .env("WOOTING_STATE_DIR", self.dir.join("runtime"))
            .env_remove("WOOTING_DEV_SIMULATION")
            // Authoritative fake library; never fall back to an installed vendor SDK.
            .env("WOOTING_RGB_SDK_PATH", self.dir.join("sdk.so"))
            .env("WOOTING_ANALOG_SDK_PATH", "/missing-analog-sdk")
            .env("MATRIX_PI_LOG", self.dir.join("rgb.log"));
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
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["ok"], true, "{value}");
        value
    }
    fn start(&mut self, profile: &str) {
        self.engine = Some(
            self.command()
                .args(["engine", "--paused", "--config", profile])
                .stdout(Stdio::null())
                .stderr(Stdio::from(
                    File::create(self.dir.join("engine.log")).unwrap(),
                ))
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.dir.join("runtime/events.sock").exists() {
            assert!(
                self.engine.as_mut().unwrap().try_wait().unwrap().is_none(),
                "engine exited: {}",
                fs::read_to_string(self.dir.join("engine.log")).unwrap()
            );
            assert!(Instant::now() < deadline, "engine startup timeout");
            std::thread::sleep(Duration::from_millis(10));
        }
        self.wait_diag(|v| v["status"]["state"] == "paused");
    }
    fn stop(&mut self) {
        self.cli(&["control", "stop"]);
        assert!(self.engine.take().unwrap().wait().unwrap().success());
    }
    fn wait_diag(&self, matches: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let value = self.cli(&["control", "diagnostics"]);
            if matches(&value) {
                return value;
            }
            assert!(Instant::now() < deadline, "diagnostic timeout: {value}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn log(&self) -> String {
        fs::read_to_string(self.dir.join("rgb.log")).unwrap_or_default()
    }
    fn frames(&self) -> Vec<Vec<u8>> {
        self.log()
            .lines()
            .filter_map(|l| l.strip_prefix("frame "))
            .filter(|s| s.len() == 6 * 21 * 3 * 2)
            .map(|s| {
                (0..s.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                    .collect()
            })
            .collect()
    }
    fn wait_frame(&self, after: usize, matches: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(frame) = self.frames().into_iter().skip(after).find(|f| matches(f)) {
                return frame;
            }
            assert!(
                Instant::now() < deadline,
                "frame timeout: {}",
                fs::read_to_string(self.dir.join("engine.log")).unwrap()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(mut c) = self.engine.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct Session(Child);
impl Session {
    fn new(h: &Harness) -> Self {
        Self(
            Command::new("node")
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/matrix_pi_session.mjs"
                ))
                .env("WOOTING_PI_SOCKET", h.dir.join("runtime/events.sock"))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("Node required for Pi integration tests"),
        )
    }
    fn hook(&mut self, name: &str) {
        writeln!(self.0.stdin.as_mut().unwrap(), "{name}").unwrap();
    }
    fn crash(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn rule<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["diagnostics"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["rule_id"] == id)
        .unwrap()
}
fn instances(report: &Value) -> &[Value] {
    report["diagnostics"]["instances"].as_array().unwrap()
}
fn zone_colors(frame: &[u8], zone: Zone) -> Vec<[u8; 3]> {
    KeyboardLayout::for_device(&DeviceInfo::synthetic_80he())
        .keys()
        .iter()
        .filter(|k| k.zone == zone)
        .map(|k| {
            let i = (usize::from(k.coord.row) * 21 + usize::from(k.coord.column)) * 3;
            [frame[i], frame[i + 1], frame[i + 2]]
        })
        .collect()
}
fn blue(frame: &[u8]) -> bool {
    zone_colors(frame, Zone::Function)
        .iter()
        .any(|c| c[2] > 100 && c[2] > c[1] * 2)
}
fn green(frame: &[u8]) -> bool {
    zone_colors(frame, Zone::Function)
        .iter()
        .any(|c| c[1] > 100 && c[2] > 0 && c[1] > c[2] * 2)
}

#[test]
fn paused_control_accepts_a_client_before_its_request_bytes_arrive() {
    let mut h = Harness::new();
    h.start(PROFILE);
    let mut stream = UnixStream::connect(h.dir.join("runtime/control.sock")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    // Darwin accept inherits O_NONBLOCK from the listener. Give the engine time
    // to accept before writing: it must explicitly restore blocking mode and
    // honor its request deadline, not respond immediately with EAGAIN/os35.
    std::thread::sleep(Duration::from_millis(80));
    writeln!(stream, r#"{{"schema_version":1,"action":"diagnostics"}}"#).unwrap();
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response).unwrap();
    let response: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["status"]["state"], "paused");
    assert!(h.log().is_empty());
    h.stop();
}

#[test]
fn starter_preview_preserves_matrix_outside_function_and_restores_idle() {
    let h = Harness::new();
    for elapsed in ["0", "400", "2000"] {
        let preview = |source: &str, state: &str, event: bool| {
            let mut args = vec![
                "diagnostics",
                "preview",
                "--config",
                PROFILE,
                "--source",
                source,
                "--state",
                state,
                "--elapsed-ms",
                elapsed,
            ];
            if event {
                args.extend(["--event", "finished"]);
            }
            h.cli(&args)
        };
        let base = preview("unconfigured", "idle", false);
        let idle = preview("pi", "idle", false);
        let working = preview("pi", "working", false);
        let done = preview("pi", "idle", true);
        let layout = KeyboardLayout::for_device(&DeviceInfo::synthetic_80he());
        for (i, key) in layout.keys().iter().enumerate() {
            if key.zone != Zone::Function {
                for report in [&idle, &working, &done] {
                    assert_eq!(report["preview"]["keys"][i], base["preview"]["keys"][i]);
                }
            }
        }
        assert_eq!(rule(&working, "pi-present")["obscured"], true);
        assert_eq!(
            rule(&working, "pi-working")["visible_keys"],
            rule(&working, "pi-working")["targeted_keys"]
        );
        if elapsed == "2000" {
            assert_eq!(rule(&done, "pi-finished")["active"], false);
            assert_eq!(done["preview"], idle["preview"]);
        } else {
            assert_eq!(rule(&done, "pi-finished")["active"], true);
        }
    }
    assert!(
        !h.dir.join("runtime").exists(),
        "preview must not create/connect to an engine"
    );
    assert!(h.log().is_empty(), "preview must not open the SDK");
}

#[test]
fn pi_hooks_drive_real_engine_frames_multi_session_pause_restart_and_crash_expiry() {
    let mut h = Harness::new();
    // Additional connector rules exercise both disjoint zones and overlapping priority.
    let profile = h.dir.join("profile.toml");
    fs::write(
        &profile,
        format!(
            "{}\n{}",
            fs::read_to_string(PROFILE).unwrap(),
            r#"
[[automation.rules]]
id = 'ci-failed'
source = 'ci'
mode = 'while-state'
state = 'failed'
scene = 'ci-alert'
priority = 100
[[automation.rules]]
id = 'ci-blocking'
source = 'ci'
mode = 'while-state'
state = 'blocked'
scene = 'ci-blocking'
priority = 100
[automation.scenes.ci-alert]
kind = 'solid'
color = [255, 0, 0]
zones = ['navigation']
[automation.scenes.ci-blocking]
kind = 'solid'
color = [255, 0, 0]
zones = ['function']
"#
        ),
    )
    .unwrap();
    h.start(profile.to_str().unwrap());
    let mut a = Session::new(&h);
    a.hook("session_start");
    h.wait_diag(|v| instances(v).len() == 1);
    assert!(h.log().is_empty(), "publishing must never resume lighting");
    h.cli(&["control", "resume"]);
    h.wait_diag(|v| v["status"]["state"] == "active");
    let c = Color::new(0, 36, 48).scale(180);
    let idle = [c.red, c.green, c.blue];
    h.wait_frame(0, |f| {
        zone_colors(f, Zone::Function).iter().all(|c| *c == idle)
    });
    let before = h.frames().len();
    a.hook("agent_start");
    h.wait_frame(before, blue);
    let mut b = Session::new(&h);
    b.hook("session_start");
    let report = h.wait_diag(|v| instances(v).len() == 2);
    assert_ne!(
        instances(&report)[0]["instance"],
        instances(&report)[1]["instance"]
    );
    assert!(instances(&report).iter().any(|s| s["state"] == "working"));
    assert!(instances(&report).iter().any(|s| s["state"] == "idle"));
    b.hook("agent_start");
    h.wait_diag(|v| instances(v).iter().all(|s| s["state"] == "working"));
    let before = h.frames().len();
    a.hook("agent_settled");
    h.wait_frame(before, green);
    h.wait_diag(|v| rule(v, "pi-finished")["active"] == false);
    let before = h.frames().len();
    h.wait_frame(before, blue); // Other session still working, not idle.

    let before = h.frames().len();
    h.cli(&[
        "external",
        "state",
        "--source",
        "ci",
        "--instance",
        "build",
        "--sequence",
        "1",
        "--state",
        "failed",
        "--ttl-ms",
        "10000",
    ]);
    h.wait_diag(|v| {
        rule(v, "ci-failed")["active"] == true && rule(v, "pi-working")["obscured"] == false
    });
    h.wait_frame(before, |f| {
        blue(f)
            && zone_colors(f, Zone::Navigation)
                .iter()
                .all(|c| *c == [180, 0, 0])
    });
    let before = h.frames().len();
    h.cli(&[
        "external",
        "state",
        "--source",
        "ci",
        "--instance",
        "gate",
        "--sequence",
        "1",
        "--state",
        "blocked",
        "--ttl-ms",
        "10000",
    ]);
    h.wait_diag(|v| rule(v, "pi-working")["obscured"] == true);
    h.wait_frame(before, |f| {
        [Zone::Function, Zone::Navigation]
            .iter()
            .all(|z| zone_colors(f, *z).iter().all(|c| *c == [180, 0, 0]))
    });
    h.cli(&[
        "external",
        "remove",
        "--source",
        "ci",
        "--instance",
        "gate",
        "--sequence",
        "2",
    ]);
    let before = h.frames().len();
    h.wait_frame(before, |f| {
        blue(f)
            && zone_colors(f, Zone::Navigation)
                .iter()
                .all(|c| *c == [180, 0, 0])
    });
    h.cli(&[
        "external",
        "remove",
        "--source",
        "ci",
        "--instance",
        "build",
        "--sequence",
        "2",
    ]);
    let before = h.frames().len();
    h.wait_frame(before, blue);
    // Matrix keeps advancing on uncovered keys even while Pi owns the function row.
    let frames = h.frames();
    assert!(
        frames
            .windows(2)
            .any(|f| zone_colors(&f[0], Zone::Alpha) != zone_colors(&f[1], Zone::Alpha))
    );

    h.cli(&["control", "pause"]);
    h.wait_diag(|v| v["status"]["state"] == "paused");
    let paused_log = h.log();
    assert!(paused_log.ends_with("close\n"), "pause must release RGB");
    b.hook("agent_settled");
    h.wait_diag(|v| instances(v).iter().all(|s| s["state"] == "idle"));
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(
        h.log(),
        paused_log,
        "paused state and heartbeats must not paint"
    );
    let before = h.frames().len();
    h.cli(&["control", "resume"]);
    h.wait_frame(before, |f| {
        zone_colors(f, Zone::Function).iter().all(|c| *c == idle)
    });
    assert_eq!(
        rule(&h.cli(&["control", "diagnostics"]), "pi-finished")["active"],
        false
    );

    b.hook("agent_start");
    h.wait_diag(|v| rule(v, "pi-working")["active"] == true);
    let identities: Vec<_> = instances(&h.cli(&["control", "diagnostics"]))
        .iter()
        .map(|s| s["instance"].clone())
        .collect();
    h.stop();
    let stopped_log = h.log();
    h.start(profile.to_str().unwrap());
    let recovered = h.wait_diag(|v| instances(v).len() == 2);
    assert_eq!(
        h.log(),
        stopped_log,
        "restart must stay paused until explicitly resumed"
    );
    assert!(
        instances(&recovered)
            .iter()
            .all(|s| identities.contains(&s["instance"]))
    );
    assert_eq!(rule(&recovered, "pi-working")["matched"], true);
    assert_eq!(rule(&recovered, "pi-finished")["active"], false);
    let before = h.frames().len();
    h.cli(&["control", "resume"]);
    h.wait_frame(before, blue);

    // Kill without shutdown: the production 15-second lease, not PID scanning,
    // removes the working indicator. The other session remains independently idle.
    b.crash();
    let deadline = Instant::now() + Duration::from_secs(18);
    while instances(&h.cli(&["control", "diagnostics"])).len() != 1 {
        assert!(Instant::now() < deadline, "crashed Pi lease never expired");
        std::thread::sleep(Duration::from_millis(100));
    }
    let before = h.frames().len();
    h.wait_frame(before, |f| {
        zone_colors(f, Zone::Function).iter().all(|c| *c == idle)
    });
    a.hook("session_shutdown");
    h.wait_diag(|v| instances(v).is_empty());
    h.stop();
    let log = h.log();
    assert_eq!(
        log.lines().filter(|s| *s == "open").count(),
        log.lines().filter(|s| *s == "close").count()
    );
}
