//! Real IPC/CAS/persistence acceptance; no SDK, HID, GUI, or producer is started.
#![cfg(unix)]
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Harness {
    dir: PathBuf,
    child: Option<Child>,
}
impl Harness {
    fn new(config: &str) -> Self {
        let dir = PathBuf::from(format!(
            "/tmp/ug-edits-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::create_dir(dir.join("home")).unwrap();
        fs::write(dir.join("profile.toml"), config).unwrap();
        let mut harness = Self { dir, child: None };
        harness.start();
        harness
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_underglow"));
        command
            .env("HOME", self.dir.join("home"))
            .env("WOOTING_STATE_DIR", self.dir.join("runtime"))
            .env("WOOTING_DEV_SIMULATION", "1")
            .env("WOOTING_RGB_SDK_PATH", "/must-not-open-rgb-sdk")
            .env("WOOTING_ANALOG_SDK_PATH", "/must-not-open-analog-sdk");
        command
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
        while !self.dir.join("runtime/control.sock").exists()
            || !self.dir.join("runtime/state.json").exists()
        {
            assert!(self.child.as_mut().unwrap().try_wait().unwrap().is_none());
            assert!(Instant::now() < deadline, "engine startup timeout");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn call(&self, args: &[&str], input: Option<&str>, ok: bool) -> Value {
        let mut child = self
            .command()
            .arg("control")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(input) = input {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
        }
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.success(),
            ok,
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn show(&self) -> Value {
        self.call(&["setup", "show"], None, true)["setup"].clone()
    }
    fn stop(&mut self) {
        self.call(&["stop"], None, true);
        assert!(self.child.take().unwrap().wait().unwrap().success());
    }
    fn disk(&self) -> Vec<u8> {
        fs::read(self.dir.join("runtime/state.json")).unwrap()
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
fn atomic_renderer_edit_preserves_outer_config_and_survives_restart_without_hardware() {
    let config = format!(
        "{}\n{}",
        include_str!("../examples/layers-profile.toml"),
        r#"
[[connectors]]
id = 'retained'
source = 'pi'
command = ['/must-not-run-provider']
enabled = false
trusted = true
"#
    );
    let mut harness = Harness::new(&config);
    let original = harness.show();
    let before_disk = harness.disk();
    harness.call(&["setup", "show"], None, true);
    assert_eq!(harness.disk(), before_disk, "show must be read-only");
    let edit = json!([{"op":"update", "id":"matrix", "patch":{"renderer":{"kind":"effect", "effect":"aurora", "palette":"ocean", "speed":170}}}]);
    let result = harness.call(
        &["setup", "edit", "--expected-revision", "0"],
        Some(&edit.to_string()),
        true,
    );
    let mut expected = original["document"].clone();
    expected["layers"][0]["renderer"] = edit[0]["patch"]["renderer"].clone();
    expected["revision"] = json!(1);
    assert_eq!(result["setup"]["document"], expected);
    assert_eq!(result["status"]["enabled"], false);
    let saved: Value = serde_json::from_slice(&harness.disk()).unwrap();
    let mut outer_before: toml::Value = toml::from_str(&config).unwrap();
    let mut outer_after: toml::Value = toml::from_str(saved["config"].as_str().unwrap()).unwrap();
    outer_before.as_table_mut().unwrap().remove("setup");
    outer_after.as_table_mut().unwrap().remove("setup");
    assert_eq!(outer_after, outer_before);
    for (revision, input) in [
        ("0", edit.to_string()), // stale
        ("1", json!([{"op":"remove","id":"missing"}]).to_string()),
        ("1", json!([{"op":"update","id":"matrix","patch":{"renderer":{"kind":"effect","effect":"aurora","speed":0}}}]).to_string()),
        ("1", "[{\"op\":\"execute\",\"command\":[\"sh\"]}]".into()),
        ("1", "malformed".into()),
    ] {
        let before = harness.disk();
        harness.call(&["setup", "edit", "--expected-revision", revision], Some(&input), false);
        assert_eq!(harness.disk(), before);
        assert_eq!(harness.show()["document"], expected);
    }
    harness.call(&["resume"], None, false);
    harness.call(&["preview"], None, true);
    assert!(
        !harness.dir.join("runtime/hardware.lock").exists(),
        "edit/show/preview or rejected resume acquired an SDK lease"
    );
    harness.stop();
    harness.start();
    assert_eq!(harness.show()["document"], expected);
    assert_eq!(
        harness.call(&["status"], None, true)["status"]["state"],
        "paused"
    );
    // Imported revision and setup identity must not reset the local CAS epoch.
    expected["id"] = json!("different-setup");
    expected["revision"] = json!(0);
    let replaced = harness.call(
        &["setup", "replace", "--expected-revision", "1"],
        Some(&expected.to_string()),
        true,
    );
    assert_eq!(replaced["setup"]["revision"], 2);
    assert_eq!(replaced["setup"]["document"]["id"], "different-setup");
}

#[test]
fn legacy_show_does_not_migrate_and_replacement_is_validated_before_save() {
    let harness = Harness::new("effect = 'matrix'\nbrightness = 99\n");
    let before = harness.disk();
    assert_eq!(harness.show(), json!({"revision":0,"document":null}));
    assert_eq!(harness.disk(), before);
    harness.call(
        &["setup", "edit", "--expected-revision", "0"],
        Some("[{\"op\":\"remove\",\"id\":\"x\"}]"),
        false,
    );
    let mut draft =
        underglow::setup::SetupDocument::from_toml(include_str!("../examples/layer-setup.toml"))
            .unwrap();
    // Pure model validity is insufficient: competing analog consumers fail compile.
    for layer in &mut draft.layers {
        layer.renderer = underglow::setup::Renderer::Reactive {
            effect: underglow::reactive::ReactiveKind::Afterimage,
            palette: None,
        };
    }
    draft.validate().unwrap();
    harness.call(
        &["setup", "replace", "--expected-revision", "0"],
        Some(&serde_json::to_string(&draft).unwrap()),
        false,
    );
    assert_eq!(harness.disk(), before);
    let draft =
        underglow::setup::SetupDocument::from_toml(include_str!("../examples/layer-setup.toml"))
            .unwrap();
    let result = harness.call(
        &["setup", "replace", "--expected-revision", "0"],
        Some(&serde_json::to_string(&draft).unwrap()),
        true,
    );
    assert_eq!(result["setup"]["revision"], 1);
    assert_eq!(result["status"]["brightness"], 99);
    assert_eq!(result["status"]["enabled"], false);
    assert!(!harness.dir.join("runtime/hardware.lock").exists());
}
