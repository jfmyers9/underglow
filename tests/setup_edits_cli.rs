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
    // Startup migrates machine-local producers out of visual configuration.
    outer_before.as_table_mut().unwrap().remove("connectors");
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
fn compatibility_show_is_read_only_and_replacement_is_validated_before_save() {
    let harness = Harness::new(
        "effect = 'matrix'\nbrightness = 99\n[signal]\nkind='command-pulse'\ncommand=['never-run']\n",
    );
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

#[test]
fn named_library_duplicate_dismantle_switch_and_restart_are_atomic_and_private() {
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::new(include_str!("../examples/layers-profile.toml"));
    let original = h.show()["document"].clone();
    let original_id = original["id"].as_str().unwrap();
    let connections_path = h.dir.join("runtime/connections.json");
    let connections = fs::read(&connections_path).unwrap();
    let list = h.call(&["library", "list"], None, true);
    assert_eq!(list["library"]["revision"], 0);
    assert_eq!(list["library"]["setups"].as_array().unwrap().len(), 1);
    assert!(list["library"]["document"].is_null());
    assert_eq!(
        fs::metadata(h.dir.join("runtime/state.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let update = |revision: &str, mutation: Value, ok: bool| {
        h.call(
            &["library", "update", "--expected-revision", revision],
            Some(&mutation.to_string()),
            ok,
        )
    };
    update(
        "0",
        json!({"op":"duplicate","id":original_id,"new_id":"custom/../copy","name":"Mine"}),
        true,
    );
    let copy =
        h.call(&["library", "get", "custom/../copy"], None, true)["library"]["document"].clone();
    assert_eq!(copy["layers"], original["layers"]);
    update(
        "1",
        json!({"op":"switch","id":"custom/../copy","expected_setup_revision":0}),
        true,
    );
    let mut custom = h.show()["document"].clone();
    custom["name"] = json!("Dismantled by user");
    custom["layers"] = json!([]);
    custom["groups"] = json!({});
    let result = h.call(
        &["setup", "replace", "--expected-revision", "1"],
        Some(&custom.to_string()),
        true,
    );
    assert_eq!(result["library"]["revision"], 3);
    custom["revision"] = json!(2);
    assert_eq!(
        h.call(&["library", "get", "custom/../copy"], None, true)["library"]["document"],
        custom
    );
    let before = h.disk();
    for (revision, mutation) in [
        ("0", json!({"op":"delete","id":original_id})),
        ("3", json!({"op":"delete","id":"custom/../copy"})),
        (
            "3",
            json!({"op":"switch","id":original_id,"expected_setup_revision":1}),
        ),
        ("3", json!({"op":"rename","id":"custom/../copy","name":""})),
    ] {
        update(revision, mutation, false);
        assert_eq!(h.disk(), before);
        assert_eq!(h.show()["document"], custom);
    }
    update(
        "3",
        json!({"op":"switch","id":original_id,"expected_setup_revision":2}),
        true,
    );
    let returned = h.show()["document"].clone();
    assert_eq!(returned["layers"], original["layers"]);
    assert_eq!(
        h.call(&["library", "get", original_id], None, true)["library"]["document"],
        original
    );
    update(
        "4",
        json!({"op":"create","starter":"matrix-pi","id":"starter","name":"Reusable"}),
        true,
    );
    let starter = h.call(&["library", "get", "starter"], None, true)["library"]["document"].clone();
    assert_eq!(starter["layers"].as_array().unwrap().len(), 2);
    assert_eq!(
        h.show()["document"],
        returned,
        "creating a saved template cannot switch or activate it"
    );
    update(
        "5",
        json!({"op":"switch","id":"starter","expected_setup_revision":3}),
        true,
    );
    let mut dismantled = h.show()["document"].clone();
    dismantled["layers"] = json!([]);
    dismantled["name"] = json!("User-owned empty setup");
    h.call(
        &["setup", "replace", "--expected-revision", "4"],
        Some(&dismantled.to_string()),
        true,
    );
    update(
        "7",
        json!({"op":"switch","id":original_id,"expected_setup_revision":5}),
        true,
    );
    let returned = h.show()["document"].clone();
    assert_eq!(fs::read(&connections_path).unwrap(), connections);
    assert!(!h.dir.join("runtime/hardware.lock").exists());
    assert!(
        !h.dir.join("custom").exists(),
        "document IDs must not become paths"
    );
    h.stop();
    h.start();
    assert_eq!(
        h.call(&["library", "list"], None, true)["library"]["revision"],
        8
    );
    assert_eq!(
        h.call(&["library", "get", "custom/../copy"], None, true)["library"]["document"],
        custom
    );
    assert_eq!(h.show()["document"], returned);
    let reopened_starter =
        h.call(&["library", "get", "starter"], None, true)["library"]["document"].clone();
    assert_eq!(reopened_starter["layers"], json!([]));
    assert_eq!(reopened_starter["name"], "User-owned empty setup");
    assert_eq!(fs::read(connections_path).unwrap(), connections);
    assert_eq!(h.call(&["status"], None, true)["status"]["state"], "paused");
}
