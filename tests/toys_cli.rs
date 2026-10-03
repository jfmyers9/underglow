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
        let dir = std::env::temp_dir().join(format!(
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
            .env("MOCK_LOG", &self.log);
        command
    }
    fn calls(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
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

const MOCK_C: &str = r#"
#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
static bool fail(const char *name) { const char *s = getenv("MOCK_FAILURE"); return s && !strcmp(s, name); }
static void log_call(const char *name) { FILE *f = fopen(getenv("MOCK_LOG"), "a"); if (f) { fprintf(f, "%s\n", name); fclose(f); } }
typedef struct { uint16_t vid, pid; char *manufacturer, *name; uint64_t id; int type; } AnalogInfo;
static AnalogInfo analog = {0x31e3, 0x1400, "Wooting", "80HE", 42, 1};
const char *wooting_analog_version_semver(void) { return fail("version") ? "0.8.0" : "0.9.1"; }
int wooting_analog_initialise(void) { log_call("init"); return fail("init") ? -1997 : 1; }
int wooting_analog_uninitialise(void) { log_call("uninit"); return 1; }
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
RgbInfo *wooting_rgb_device_info(void) { return &rgb; }
int wooting_rgb_device_layout(void) { return fail("layout") ? 1 : 0; }
"#;
