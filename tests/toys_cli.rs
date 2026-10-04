//! Exercise the real CLI/FFI lifecycle without opening any HID device.
#![cfg(unix)]
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const BINARY: &str = env!("CARGO_BIN_EXE_wooting-signals");
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Mock {
    dir: PathBuf,
    library: PathBuf,
    log: PathBuf,
}
impl Mock {
    fn new() -> Self {
        // Keep Unix socket paths below macOS's sockaddr_un limit.
        let dir = PathBuf::from("/tmp").join(format!(
            "wooting-toy-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("sdk.c");
        let library = dir.join(if cfg!(target_os = "macos") {
            "sdk.dylib"
        } else {
            "sdk.so"
        });
        fs::write(&source, MOCK_C).unwrap();
        let output = Command::new("cc")
            .args(if cfg!(target_os = "macos") {
                vec!["-dynamiclib"]
            } else {
                vec!["-shared", "-fPIC"]
            })
            .arg(&source)
            .arg("-o")
            .arg(&library)
            .output()
            .expect("C compiler required for SDK ABI tests");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let log = dir.join("calls.log");
        Self { dir, library, log }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(BINARY);
        command
            .args([
                "toy",
                "ripples",
                "--seconds",
                "1",
                "--fps",
                "10",
                "--analog-sdk-path",
            ])
            .arg(&self.library)
            .arg("--sdk-path")
            .arg(&self.library)
            .env("MOCK_LOG", &self.log)
            .env("WOOTING_STATE_DIR", self.dir.join("runtime"));
        command
    }
    fn calls(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn doctor(&self) -> Command {
        let mut command = Command::new(BINARY);
        command
            .args(["doctor", "--json", "--sdk-path"])
            .arg(&self.library)
            .env("MOCK_LOG", &self.log)
            .env("WOOTING_STATE_DIR", self.dir.join("runtime"));
        command
    }

    fn config_command(&self, contents: &str) -> Command {
        let config = self.dir.join("profile.toml");
        fs::write(&config, contents).unwrap();
        let mut command = Command::new(BINARY);
        command
            .arg("run")
            .arg("--config")
            .arg(config)
            .arg("--sdk-path")
            .arg(&self.library)
            .env("WOOTING_ANALOG_SDK_PATH", &self.library)
            .env("MOCK_LOG", &self.log)
            .env("WOOTING_STATE_DIR", self.dir.join("runtime"));
        command
    }
}
impl Drop for Mock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn preview_is_deterministic_and_never_loads_sdks() {
    let run = || {
        Command::new(BINARY)
            .args([
                "toy",
                "ripples",
                "--preview",
                "--ticks",
                "30",
                "--format",
                "json",
                "--sdk-path",
                "/missing-rgb",
                "--analog-sdk-path",
                "/missing-analog",
            ])
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, run().stdout);
    let json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    let frames = json["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 30);
    assert!(
        frames
            .iter()
            .any(|frame| frame["lit_keys"].as_u64().unwrap() > 1)
    );
    assert_eq!(frames.last().unwrap()["lit_keys"], 0);
    for args in [
        ["--fps", "0"],
        ["--fps", "121"],
        ["--ticks", "601"],
        ["--seconds", "0"],
    ] {
        assert!(
            !Command::new(BINARY)
                .args(["toy", "ripples", "--preview"])
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
fn duration_and_failures_restore_rgb_and_uninitialise_analog() {
    let mock = Mock::new();
    for failure in [
        "", "read", "write", "update", "layout", "close", "nan", "overflow",
    ] {
        let _ = fs::remove_file(&mock.log);
        let output = mock
            .command()
            .env("MOCK_FAILURE", failure)
            .output()
            .unwrap();
        let success = failure.is_empty() || failure == "close";
        assert_eq!(
            output.status.success(),
            success,
            "{failure}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let calls = mock.calls();
        assert_eq!(calls.matches("close\n").count(), 1, "{failure}: {calls}");
        assert_eq!(calls.matches("uninit\n").count(), 1, "{failure}: {calls}");
        assert!(calls.find("close\n").unwrap() < calls.find("uninit\n").unwrap());
        if failure.is_empty() {
            assert!(calls.contains("lit\n"));
        }
        if failure == "close" {
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("restoration was not acknowledged")
            );
        }
    }
}

#[test]
fn setup_failures_do_not_open_rgb() {
    let mock = Mock::new();
    for failure in [
        "version",
        "init",
        "mode",
        "devices",
        "multiple",
        "wrong-device",
    ] {
        let _ = fs::remove_file(&mock.log);
        let output = mock
            .command()
            .env("MOCK_FAILURE", failure)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{failure}");
        let calls = mock.calls();
        assert!(!calls.contains("rgb-open"), "{failure}: {calls}");
        assert_eq!(
            calls.contains("uninit"),
            failure != "init" && failure != "version",
            "{failure}: {calls}"
        );
    }
}

#[test]
fn ctrl_c_restores_lighting() {
    let mock = Mock::new();
    // Use a longer deadline as a backstop so a signal regression cannot hang the suite.
    let mut command = Command::new(BINARY);
    command
        .args(["toy", "ripples", "--seconds", "3", "--analog-sdk-path"])
        .arg(&mock.library)
        .arg("--sdk-path")
        .arg(&mock.library)
        .env("MOCK_LOG", &mock.log)
        .env("WOOTING_STATE_DIR", mock.dir.join("runtime"))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().unwrap();
    let start = Instant::now();
    while !mock.calls().contains("update\n") && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let signal = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    let status = child.wait().unwrap();
    assert!(signal.success() && status.success());
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(mock.calls().matches("close\n").count(), 1);
    assert_eq!(mock.calls().matches("uninit\n").count(), 1);
}

fn check_status(report: &serde_json::Value, name: &str) -> String {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == name)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn doctor_without_probe_or_analog_never_paints_or_loads_analog() {
    let mock = Mock::new();
    let output = mock
        .doctor()
        .env("WOOTING_ANALOG_SDK_PATH", "/missing-analog")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(check_status(&report, "rgb-probe"), "skipped");
    assert_eq!(check_status(&report, "rgb-restore"), "passed");
    assert!(!mock.calls().contains("init\n"));
    assert!(!mock.calls().contains("lit\n"));
    assert!(!mock.calls().contains("update\n"));
    assert_eq!(mock.calls().matches("close\n").count(), 1);
    assert_eq!(report["manual_checks"].as_array().unwrap().len(), 3);
}

#[test]
fn doctor_reports_probe_and_analog_results_and_cleanup_failures() {
    let mock = Mock::new();
    for (failure, failed_check) in [
        ("", ""),
        ("write", "rgb-probe"),
        ("read", "analog-read"),
        ("close", "rgb-restore"),
        ("uninit", "analog-close"),
    ] {
        let _ = fs::remove_file(&mock.log);
        let mut command = mock.doctor();
        command
            .args(["--analog", "--analog-sdk-path"])
            .arg(&mock.library)
            .env("MOCK_FAILURE", failure);
        // Only these cases need a frame write; the rest exercise fast setup/read/cleanup.
        if failure.is_empty() || failure == "write" {
            command.args(["--probe-rgb", "--seconds", "1"]);
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.success(),
            failure.is_empty(),
            "{failure}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if !failure.is_empty() {
            assert_eq!(check_status(&report, failed_check), "failed");
        } else {
            assert_eq!(check_status(&report, "rgb-probe"), "passed");
        }
        assert_eq!(mock.calls().matches("close\n").count(), 1, "{failure}");
        assert_eq!(mock.calls().matches("uninit\n").count(), 1, "{failure}");
        // A failure in either subsystem must not prevent the other from releasing.
        assert!(mock.calls().find("close\n").unwrap() < mock.calls().find("uninit\n").unwrap());
    }
}

#[test]
fn doctor_rejects_unusable_metadata_and_cleans_up_partial_rgb_setup() {
    let mock = Mock::new();
    for failure in ["unknown-layout", "metadata-null"] {
        let _ = fs::remove_file(&mock.log);
        let output = mock
            .doctor()
            .arg("--probe-rgb")
            .env("MOCK_FAILURE", failure)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            check_status(
                &report,
                if failure == "metadata-null" {
                    "rgb-open"
                } else {
                    "rgb-metadata"
                }
            ),
            "failed"
        );
        assert!(!mock.calls().contains("lit\n"));
        assert_eq!(mock.calls().matches("close\n").count(), 1);
    }
}

#[test]
fn doctor_missing_library_still_produces_report_and_checks_arguments_before_opening() {
    let mock = Mock::new();
    let output = Command::new(BINARY)
        .args([
            "doctor",
            "--json",
            "--analog",
            "--analog-sdk-path",
            "/missing-analog",
            "--sdk-path",
            "/missing-rgb",
        ])
        .env("WOOTING_RGB_SDK_PATH", &mock.library)
        .env("WOOTING_STATE_DIR", mock.dir.join("runtime"))
        .env("MOCK_LOG", &mock.log)
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(check_status(&report, "analog-open"), "failed");
    assert_eq!(check_status(&report, "rgb-open"), "failed");
    assert!(
        mock.calls().is_empty(),
        "explicit missing SDK must not fall back to the environment override"
    );
    for args in [
        vec!["--seconds", "1"],
        vec!["--probe-rgb", "--seconds", "31"],
        vec!["--analog-sdk-path", "/missing"],
    ] {
        let output = Command::new(BINARY)
            .arg("doctor")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
}

#[test]
fn doctor_interruption_is_incomplete_but_still_reports_cleanup() {
    let mock = Mock::new();
    let child = mock
        .doctor()
        .args([
            "--probe-rgb",
            "--seconds",
            "3",
            "--analog",
            "--analog-sdk-path",
        ])
        .arg(&mock.library)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while !mock.calls().contains("update\n") && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let signal = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    // wait_with_output consumes the child and reaps it even if the assertions fail.
    let output = child.wait_with_output().unwrap();
    assert!(signal.success());
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["interrupted"], true);
    assert_eq!(report["ok"], false);
    assert_eq!(check_status(&report, "rgb-probe"), "skipped");
    assert_eq!(check_status(&report, "rgb-restore"), "passed");
    assert_eq!(check_status(&report, "analog-close"), "passed");
    assert_eq!(mock.calls().matches("close\n").count(), 1);
    assert_eq!(mock.calls().matches("uninit\n").count(), 1);
}

#[test]
fn configured_ripples_preview_matches_toy_and_dry_run_opens_no_sdks() {
    let mock = Mock::new();
    let config =
        "schema_version = 1\npalette = 'ocean'\nbrightness = 180\n[signal]\nkind = 'ripples'";
    let dry = mock
        .config_command(config)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    assert!(String::from_utf8_lossy(&dry.stdout).contains("signal: Ripples"));
    let configured = mock
        .config_command(config)
        .args([
            "--preview",
            "--preview-format",
            "json",
            "--preview-ticks",
            "12",
        ])
        .output()
        .unwrap();
    let toy = Command::new(BINARY)
        .args([
            "toy",
            "ripples",
            "--preview",
            "--format",
            "json",
            "--ticks",
            "12",
        ])
        .output()
        .unwrap();
    assert!(configured.status.success() && toy.status.success());
    let _: serde_json::Value = serde_json::from_slice(&configured.stdout).unwrap();
    assert_eq!(configured.stdout, toy.stdout);
    assert!(
        mock.calls().is_empty(),
        "dry-run/preview must not initialize either SDK"
    );
}

#[test]
fn configured_modes_share_cleanup_and_rgb_modes_do_not_require_analog() {
    let mock = Mock::new();
    for (kind, failure) in [
        ("ripples", ""),
        ("ripples", "read"),
        ("ripples", "write"),
        ("ripples", "layout"),
        ("static-effect", ""),
    ] {
        let _ = fs::remove_file(&mock.log);
        let output = mock
            .config_command(&format!("seconds = 1\nfps = 10\n[signal]\nkind = '{kind}'"))
            .env("MOCK_FAILURE", failure)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            failure.is_empty(),
            "{kind}/{failure}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let calls = mock.calls();
        assert_eq!(calls.matches("close\n").count(), 1);
        if kind == "ripples" {
            assert_eq!(calls.matches("uninit\n").count(), 1);
            assert!(calls.find("close\n").unwrap() < calls.find("uninit\n").unwrap());
        } else {
            assert!(!calls.contains("init\n"));
        }
    }
}

#[test]
fn invalid_config_fails_before_acquiring_hardware_or_starting_a_command() {
    let mock = Mock::new();
    for config in [
        "fps = 0",
        "schema_version = 99",
        "[signal]\nkind = 'missing'",
        "[signal]\nkind = 'command-pulse'",
    ] {
        let output = mock.config_command(config).output().unwrap();
        assert!(!output.status.success(), "{config}");
        assert!(mock.calls().is_empty());
    }
    let output = mock
        .config_command(
            "[signal]\nkind = 'ripples'\n[signal.ripples]\nanalog_sdk_path = '/missing-analog'",
        )
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        mock.calls().is_empty(),
        "missing analog dependency should fail before RGB opens"
    );
    // After a rejected candidate, the normal valid configuration still works.
    let output = mock
        .config_command("[signal]\nkind = 'ripples'")
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(mock.calls().is_empty());
}

#[test]
fn profile_sources_forward_ripple_lifecycle_and_preview_without_input() {
    let mock = Mock::new();
    let config = "seconds = 1\n[[sources]]\nid = 'keys'\ntype = 'ripples'\n[scenes.unused]\neffect = 'comet'";
    let preview = mock
        .config_command(config)
        .args(["--preview", "--preview-format", "json"])
        .output()
        .unwrap();
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert!(json["frames"][0]["lit_keys"].as_u64().unwrap() > 0);
    assert!(mock.calls().is_empty());
    let live = mock
        .config_command(config)
        .env("MOCK_FAILURE", "read")
        .output()
        .unwrap();
    assert!(!live.status.success());
    assert_eq!(mock.calls().matches("uninit\n").count(), 1);
    assert_eq!(mock.calls().matches("close\n").count(), 1);
}

struct EngineChild(std::process::Child);
impl Drop for EngineChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Mock {
    fn engine_command(&self) -> Command {
        let mut command = Command::new(BINARY);
        command
            .env("WOOTING_STATE_DIR", self.dir.join("runtime"))
            .env("WOOTING_RGB_SDK_PATH", &self.library)
            .env("WOOTING_ANALOG_SDK_PATH", &self.library)
            .env("MOCK_LOG", &self.log)
            .env("MOCK_FAILURE_FILE", self.dir.join("failure"));
        command
    }
    fn control(&self, args: &[&str]) -> serde_json::Value {
        let output = self
            .engine_command()
            .arg("control")
            .args(args)
            .output()
            .unwrap();
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "invalid response: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }
    fn engine(&self) -> EngineChild {
        let mut command = self.engine_command();
        command
            .arg("engine")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = EngineChild(command.spawn().unwrap());
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "engine exited before becoming ready"
            );
            if self.control(&["status"])["ok"] == true {
                return child;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("engine did not become ready: {}", self.control(&["status"]));
    }
    fn wait_state(&self, wanted: &str) -> serde_json::Value {
        let start = Instant::now();
        loop {
            let response = self.control(&["status"]);
            if response["status"]["state"] == wanted {
                return response;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "{response}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
#[test]
fn engine_paused_persistence_switching_and_single_writer() {
    let mock = Mock::new();
    let mut engine = mock.engine();
    assert_eq!(mock.wait_state("paused")["status"]["mode"], "ripples");
    assert!(
        mock.calls().is_empty(),
        "starting paused must never open SDKs"
    );
    assert_eq!(
        mock.control(&["settings", "--brightness", "77", "--palette", "terminal"])["ok"],
        true
    );
    assert_eq!(mock.control(&["resume"])["ok"], true);
    mock.wait_state("active");
    let denied = mock.doctor().output().unwrap();
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("hardware lock"));
    let duplicate = mock
        .engine_command()
        .args(["engine", "--state-dir"])
        .arg(mock.dir.join("other"))
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("engine lock"));

    let before = fs::read(mock.dir.join("runtime/state.json")).unwrap();
    let invalid = mock.dir.join("invalid.toml");
    fs::write(&invalid, "fps = 0").unwrap();
    assert_eq!(
        mock.control(&["select", "--config", invalid.to_str().unwrap()])["ok"],
        false
    );
    assert_eq!(
        fs::read(mock.dir.join("runtime/state.json")).unwrap(),
        before
    );
    assert_eq!(mock.control(&["status"])["status"]["brightness"], 77);
    assert!(!mock.calls().contains("close\n"));
    assert_eq!(mock.control(&["select", "--preset", "comet"])["ok"], true);
    assert_eq!(mock.calls().matches("uninit\n").count(), 1);
    assert_eq!(mock.control(&["pause"])["ok"], true);
    assert!(
        mock.doctor().output().unwrap().status.success(),
        "paused engine must release device lock"
    );

    assert_eq!(mock.control(&["stop"])["ok"], true);
    assert!(engine.0.wait().unwrap().success());
    let calls = mock.calls();
    let mut restarted = mock.engine();
    assert_eq!(mock.wait_state("paused")["status"]["mode"], "comet");
    assert_eq!(
        mock.calls(),
        calls,
        "explicit pause survives restart without opening SDKs"
    );
    mock.control(&["stop"]);
    assert!(restarted.0.wait().unwrap().success());
}
#[test]
fn engine_sigterm_restores_and_retry_is_cancelled_by_pause() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let mock = Mock::new();
    let mut engine = mock.engine();
    assert_eq!(mock.control(&["resume"])["ok"], true);
    fs::write(mock.dir.join("failure"), "read").unwrap();
    let status = mock.wait_state("retrying");
    assert_eq!(status["status"]["retry_attempt"], 1);
    assert_eq!(mock.control(&["pause"])["ok"], true);
    let calls = mock.calls();
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(mock.calls(), calls, "pause must cancel scheduled retry");
    fs::remove_file(mock.dir.join("failure")).unwrap();
    assert_eq!(mock.control(&["resume"])["ok"], true);
    // Deliver termination inside request handling, after the outer loop check.
    let mut partial = UnixStream::connect(mock.dir.join("runtime/control.sock")).unwrap();
    partial
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    partial
        .write_all(b"{\"schema_version\":1,\"action\":")
        .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let result = Command::new("kill")
        .args(["-TERM", &engine.0.id().to_string()])
        .status()
        .unwrap();
    assert!(result.success());
    // The signal may interrupt the read and close this partial request first.
    let _ = partial.write_all(b"\"status\"}\n");
    let mut response = String::new();
    let _ = partial.read_to_string(&mut response);
    assert!(engine.0.wait().unwrap().success());
    assert_eq!(
        mock.calls().lines().filter(|line| *line == "init").count(),
        mock.calls().matches("uninit\n").count()
    );
    assert!(!mock.dir.join("runtime/control.sock").exists());
    // SIGTERM preserves enabled intent; explicit pause above did not.
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(mock.dir.join("runtime/state.json")).unwrap()).unwrap();
    assert_eq!(saved["enabled"], true);
}
#[test]
fn engine_invalid_protocol_and_pause_persistence_failure_are_safe() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let mock = Mock::new();
    let _engine = mock.engine();
    for message in [
        "{\"schema_version\":99,\"action\":\"resume\"}\n",
        "{\"schema_version\":1,\"action\":\"settings\",\"fps\":0,\"brightness\":null,\"palette\":null}\n",
    ] {
        let mut stream = UnixStream::connect(mock.dir.join("runtime/control.sock")).unwrap();
        stream.write_all(message.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], false);
    }
    assert!(mock.calls().is_empty());
    assert_eq!(mock.control(&["resume"])["ok"], true);
    let state = mock.dir.join("runtime/state.json");
    fs::remove_file(&state).unwrap();
    fs::create_dir(&state).unwrap(); // force atomic-rename failure
    let response = mock.control(&["pause"]);
    assert_eq!(response["ok"], false);
    assert_eq!(response["status"]["state"], "paused");
    assert_eq!(mock.calls().matches("close\n").count(), 1);
    assert_eq!(mock.calls().matches("uninit\n").count(), 1);
    assert!(mock.doctor().output().unwrap().status.success());
}
#[test]
fn engine_never_replays_command_presets_on_recovery_or_restart() {
    let mock = Mock::new();
    let mut engine = mock.engine();
    let marker = mock.dir.join("ran");
    let config = mock.dir.join("command.toml");
    fs::write(&config, format!("continuous = true\n[signal]\nkind = 'command-pulse'\ncommand = ['sh', '-c', 'echo ran >> {}; exec sleep 10']\noutput = 'quiet'\n", marker.display())).unwrap();
    assert_eq!(
        mock.control(&["select", "--config", config.to_str().unwrap()])["ok"],
        true
    );
    assert!(
        !marker.exists(),
        "importing while paused must not execute a command"
    );
    assert_eq!(mock.control(&["resume"])["ok"], true);
    let start = Instant::now();
    while !marker.exists() {
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
    fs::write(mock.dir.join("failure"), "write").unwrap();
    let error = mock.wait_state("error");
    assert_eq!(error["status"]["retry_attempt"], 0);
    assert_eq!(fs::read_to_string(&marker).unwrap(), "ran\n");
    Command::new("kill")
        .args(["-TERM", &engine.0.id().to_string()])
        .status()
        .unwrap();
    assert!(engine.0.wait().unwrap().success());
    let _restarted = mock.engine();
    mock.wait_state("paused");
    assert_eq!(fs::read_to_string(marker).unwrap(), "ran\n");
}

#[test]
fn engine_does_not_retry_after_unacknowledged_cleanup() {
    let mock = Mock::new();
    let _engine = mock.engine();
    assert_eq!(mock.control(&["resume"])["ok"], true);
    fs::write(mock.dir.join("failure"), "read-close").unwrap();
    let status = mock.wait_state("error");
    assert_eq!(status["status"]["retry_attempt"], 0);
    assert!(
        status["status"]["last_error"]
            .as_str()
            .unwrap()
            .contains("cleanup")
    );
    assert_eq!(mock.calls().matches("close\n").count(), 1);
    assert_eq!(mock.calls().matches("uninit\n").count(), 1);
    mock.control(&["stop"]);
}

const MOCK_C: &str = r#"
#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
static bool fail(const char *name) {
  const char *path = getenv("MOCK_FAILURE_FILE");
  if (path) { FILE *f = fopen(path, "r"); if (f) { char value[64] = {0}; fgets(value, sizeof(value), f); fclose(f); if (!strcmp(value, name) || (!strcmp(value, "read-close") && (!strcmp(name, "read") || !strcmp(name, "close")))) return true; } }
  const char *s = getenv("MOCK_FAILURE"); return s && !strcmp(s, name);
}
static void log_call(const char *name) { FILE *f = fopen(getenv("MOCK_LOG"), "a"); if (f) { fprintf(f, "%s\n", name); fclose(f); } }
typedef struct { uint16_t vid, pid; char *manufacturer, *name; uint64_t id; int type; } AnalogInfo;
static AnalogInfo analog = {0x31e3, 0x1400, "Wooting", "80HE", 42, 1};
const char *wooting_analog_version_semver(void) { return fail("version") ? "0.8.0" : "0.9.1"; }
int wooting_analog_initialise(void) { log_call("init"); return fail("init") ? -1997 : 1; }
int wooting_analog_uninitialise(void) { log_call("uninit"); return fail("uninit") ? -1997 : 1; }
int wooting_analog_set_keycode_mode(unsigned int mode) { return fail("mode") || mode != 0 ? -1996 : 1; }
int wooting_analog_get_connected_devices_info(AnalogInfo **buffer, unsigned int len) {
    if (!len) return -1996;
    buffer[0] = &analog;
    if (fail("wrong-device")) analog.pid = 0x1300;
    return fail("devices") ? 0 : fail("multiple") ? 2 : 1;
}
int wooting_analog_read_full_buffer_device(uint16_t *codes, float *values, unsigned int len, uint64_t id) {
    log_call("read");
    if (fail("read") || id != 42 || len != 256) return -1999;
    codes[0] = 9; values[0] = fail("nan") ? NAN : 0.8f;
    return fail("overflow") ? 257 : 1;
}
typedef struct { bool connected; const char *model; uint8_t rows, columns, led_max; int type; bool v2; int layout; bool small, multi; } RgbInfo;
static RgbInfo rgb = {true, "Wooting 80HE", 6, 17, 117, 5, true, 0, false, false};
bool wooting_rgb_kbd_connected(void) { log_call("rgb-open"); return true; }
bool wooting_rgb_close(void) { log_call("close"); return !fail("close"); }
bool wooting_rgb_direct_set_key(uint8_t r, uint8_t c, uint8_t red, uint8_t green, uint8_t blue) { return true; }
bool wooting_rgb_array_update_keyboard(void) { log_call("update"); return !fail("update"); }
void wooting_rgb_array_auto_update(bool enabled) {}
bool wooting_rgb_array_set_full(const uint8_t *colors) {
    for (unsigned int i = 0; i < 6*21*3; i++) { if (colors[i]) { log_call("lit"); break; } }
    return !fail("write");
}
RgbInfo *wooting_rgb_device_info(void) { return fail("metadata-null") ? NULL : &rgb; }
int wooting_rgb_device_layout(void) { return fail("unknown-layout") ? -1 : fail("layout") ? 1 : 0; }
"#;
