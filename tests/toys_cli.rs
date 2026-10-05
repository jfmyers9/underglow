//! Exercise the real CLI/FFI lifecycle without opening any HID device.
#![cfg(unix)]
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const BINARY: &str = env!("CARGO_BIN_EXE_underglow");
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
    let config = "schema_version = 1\npalette = 'ocean'\n[signal]\nkind = 'ripples'";
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
fn effect_preview_speed_and_fps_sample_the_same_wall_clock_frames() {
    let render = |effect: &str, fps: u32, speed: u32, ticks: u32| {
        let output = Command::new(BINARY)
            .env("WOOTING_DEV_SIMULATION", "1")
            .args([
                "preview",
                "effect",
                effect,
                "--format",
                "json",
                "--fps",
                &fps.to_string(),
                "--speed",
                &speed.to_string(),
                "--ticks",
                &ticks.to_string(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let data: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        data["frames"].as_array().unwrap().last().unwrap()["rows"].clone()
    };
    for effect in ["comet", "matrix", "rainbow", "breath"] {
        let expected = render(effect, 10, 100, 11); // One second at the default speed.
        for fps in [5, 30, 60] {
            assert_eq!(
                render(effect, fps, 100, fps + 1),
                expected,
                "{effect} at {fps} FPS"
            );
        }
        assert_eq!(
            render(effect, 30, 50, 61),
            expected,
            "half speed takes twice as long"
        );
        assert_ne!(
            render(effect, 30, 50, 31),
            expected,
            "speed must affect output"
        );
    }
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

#[test]
fn renamed_binary_reuses_legacy_default_state_and_lock_namespace() {
    use std::io::Read;
    use std::path::Path;
    let mock = Mock::new();
    let home = mock.dir.join("h");
    let xdg = home.join("state");
    // These strings deliberately describe the OLD application's identity, not
    // the current crate name. Changing the name must not create a second owner.
    let state = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/wooting-signals/runtime")
    } else {
        xdg.join("wooting-signals")
    };
    fs::create_dir_all(&state).unwrap();
    fs::write(
        state.join("state.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1, "enabled": false,
            "config": "brightness=73\nfps=9\n[signal]\nkind='static-effect'\neffect='matrix'\n"
        }))
        .unwrap(),
    )
    .unwrap();
    let command = |binary: &Path| {
        let mut command = Command::new(binary);
        command
            .env_remove("WOOTING_STATE_DIR")
            .env("HOME", &home)
            .env("XDG_STATE_HOME", &xdg)
            .env("WOOTING_DEV_SIMULATION", "1")
            .env("WOOTING_RGB_SDK_PATH", &mock.library)
            .env("WOOTING_ANALOG_SDK_PATH", &mock.library)
            .env("MOCK_LOG", &mock.log);
        command
    };
    let binary = Path::new(BINARY);
    let legacy = mock.dir.join("wooting-signals");
    std::os::unix::fs::symlink(binary, &legacy).unwrap();
    let hardware_lock = fs::File::create(state.join("hardware.lock")).unwrap();
    hardware_lock.try_lock().unwrap();
    for executable in [binary, legacy.as_path()] {
        let denied = command(executable).arg("info").output().unwrap();
        assert!(!denied.status.success());
        assert!(String::from_utf8_lossy(&denied.stderr).contains("hardware lock"));
    }
    drop(hardware_lock);
    let engine_lock = fs::File::create(state.join("engine.lock")).unwrap();
    engine_lock.try_lock().unwrap();
    let mut denied = EngineChild(
        command(binary)
            .arg("engine")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    while denied.0.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        denied.0.try_wait().unwrap().is_some(),
        "renamed engine bypassed the old engine lock"
    );
    let mut error = String::new();
    denied
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut error)
        .unwrap();
    assert!(error.contains("engine lock"), "{error}");
    drop(engine_lock);

    let mut engine = EngineChild(
        command(binary)
            .arg("engine")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    let status = loop {
        assert!(engine.0.try_wait().unwrap().is_none());
        let output = command(&legacy)
            .args(["control", "status"])
            .output()
            .unwrap();
        if output.status.success() {
            break serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "legacy state socket was not found"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status["status"]["brightness"], 73);
    assert_eq!(status["status"]["fps"], 9);
    assert_eq!(status["status"]["mode"], "matrix");
    assert_eq!(status["status"]["state"], "paused");
    assert!(
        command(&legacy)
            .args(["control", "stop"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(engine.0.wait().unwrap().success());
    assert!(
        mock.calls().is_empty(),
        "rename compatibility must not initialize SDKs"
    );
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
    let opened = mock.calls().matches("init\n").count();
    let closed = mock.calls().matches("close\n").count();
    let changed = mock.control(&["settings", "--speed", "75", "--fps", "60"]);
    assert_eq!(changed["ok"], true);
    assert_eq!(changed["status"]["speed"], 75);
    assert_eq!(changed["status"]["fps"], 60);
    assert_eq!(changed["status"]["state"], "active");
    assert_eq!(
        mock.calls().matches("init\n").count(),
        opened,
        "speed changes must not reopen SDKs"
    );
    assert_eq!(mock.calls().matches("close\n").count(), closed);
    assert_eq!(mock.control(&["pause"])["ok"], true);
    assert!(
        mock.doctor().output().unwrap().status.success(),
        "paused engine must release device lock"
    );

    assert_eq!(mock.control(&["stop"])["ok"], true);
    assert!(engine.0.wait().unwrap().success());
    let calls = mock.calls();
    let mut restarted = mock.engine();
    let status = mock.wait_state("paused");
    assert_eq!(status["status"]["mode"], "comet");
    assert_eq!(status["status"]["speed"], 75);
    assert_eq!(status["status"]["fps"], 60);
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

#[test]
fn engine_two_tone_cli_ipc_persistence_and_rejected_updates() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let mock = Mock::new();
    let mut engine = mock.engine();
    let initial = mock.wait_state("paused");
    assert!(initial["status"]["ripple_base_color"].is_null());
    assert!(initial["status"]["ripple_color"].is_null());

    let applied = mock.control(&[
        "settings",
        "--ripple-base-color",
        "#000000",
        "--ripple-color",
        "78fFff",
        "--brightness",
        "73",
    ]);
    assert_eq!(applied["ok"], true, "{applied}");
    assert_eq!(
        applied["status"]["ripple_base_color"],
        serde_json::json!([0, 0, 0])
    );
    assert_eq!(
        applied["status"]["ripple_color"],
        serde_json::json!([120, 255, 255])
    );
    assert_eq!(applied["status"]["brightness"], 73);
    let state = mock.dir.join("runtime/state.json");
    let before = fs::read(&state).unwrap();
    let status = mock.control(&["status"])["status"].clone();

    // Malformed CLI arguments must fail before IPC, with the snapshot unchanged.
    for args in [
        vec!["settings", "--ripple-color", "ffffff0"],
        vec!["settings", "--ripple-base-color", "GG0000"],
        vec!["settings", "--ripple-color", "ffffff", "--ripple-palette"],
    ] {
        let output = mock
            .engine_command()
            .arg("control")
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(fs::read(&state).unwrap(), before);
        assert_eq!(mock.control(&["status"])["status"], status);
    }
    // Bypassing clap cannot bypass atomic validation or reset/color conflicts.
    for fields in [
        r#""ripple_color":[256,0,0]"#,
        r#""ripple_color":[0,0,0,0]"#,
        r#""ripple_base_color":[1,2,3],"ripple_palette":true"#,
        r#""ripple_color":[1,2,3],"fps":0"#,
    ] {
        let mut stream = UnixStream::connect(mock.dir.join("runtime/control.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        writeln!(
            stream,
            "{{\"schema_version\":1,\"action\":\"settings\",{fields}}}"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["ok"], false, "{response}");
        assert_eq!(fs::read(&state).unwrap(), before);
        assert_eq!(json["status"], status);
    }

    assert_eq!(mock.control(&["stop"])["ok"], true);
    assert!(engine.0.wait().unwrap().success());
    let mut restarted = mock.engine();
    assert_eq!(mock.wait_state("paused")["status"], status);
    let partial = mock.control(&["settings", "--ripple-color", "ff0000"]);
    assert_eq!(
        partial["status"]["ripple_base_color"],
        serde_json::json!([0, 0, 0])
    );
    assert_eq!(
        partial["status"]["ripple_color"],
        serde_json::json!([255, 0, 0])
    );
    let reset = mock.control(&["settings", "--ripple-palette"]);
    assert_eq!(reset["ok"], true);
    assert!(reset["status"]["ripple_color"].is_null());
    assert!(reset["status"]["ripple_base_color"].is_null());
    assert_eq!(reset["status"]["brightness"], 73);

    assert_eq!(mock.control(&["select", "--preset", "comet"])["ok"], true);
    let before = fs::read(&state).unwrap();
    let rejected = mock.control(&["settings", "--ripple-color", "ffffff", "--brightness", "12"]);
    assert_eq!(rejected["ok"], false);
    assert_eq!(fs::read(&state).unwrap(), before);
    assert_eq!(mock.control(&["stop"])["ok"], true);
    assert!(restarted.0.wait().unwrap().success());
    assert!(
        mock.calls().is_empty(),
        "paused edits/restarts must never open either SDK"
    );
}

#[test]
fn development_start_is_paused_and_simulation_fails_closed() {
    for simulation in [false, true] {
        let mock = Mock::new();
        let state_dir = mock.dir.join("runtime");
        fs::create_dir_all(&state_dir).unwrap();
        let config =
            "brightness=73\n[signal]\nkind='ripples'\n[signal.ripples]\nbase_color=[1,2,3]\n";
        fs::write(
            state_dir.join("state.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version":1, "enabled":true, "config":config
            }))
            .unwrap(),
        )
        .unwrap();
        let mut command = mock.engine_command();
        command
            .arg("engine")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if simulation {
            command.env("WOOTING_DEV_SIMULATION", "1");
        } else {
            command.arg("--paused");
        }
        let mut engine = EngineChild(command.spawn().unwrap());
        let state = mock.wait_state("paused");
        assert_eq!(state["status"]["brightness"], 73);
        assert_eq!(
            state["status"]["ripple_base_color"],
            serde_json::json!([1, 2, 3])
        );
        assert!(mock.calls().is_empty());
        if simulation {
            let reply = mock.control(&["resume"]);
            assert_eq!(reply["ok"], false);
            assert_eq!(reply["status"]["enabled"], false);
            assert!(
                reply["error"]
                    .as_str()
                    .unwrap()
                    .contains("development simulator")
            );
            // Explicit SDK overrides cannot bypass the guard, even outside the engine.
            for args in [vec!["info"], vec!["toy", "ripples", "--seconds", "1"]] {
                let result = mock
                    .engine_command()
                    .env("WOOTING_DEV_SIMULATION", "1")
                    .arg("--sdk-path")
                    .arg(&mock.library)
                    .args(args)
                    .output()
                    .unwrap();
                assert!(!result.status.success());
                assert!(String::from_utf8_lossy(&result.stderr).contains("development simulator"));
            }
        }
        assert!(
            mock.calls().is_empty(),
            "development startup touched an SDK"
        );
        assert_eq!(mock.control(&["stop"])["ok"], true);
        assert!(engine.0.wait().unwrap().success());
    }
}
