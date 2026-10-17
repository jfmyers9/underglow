//! Real engine/supervisor tests: isolated ownership namespace, simulation, no service/hardware.
#![cfg(unix)]
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_underglow");
static NEXT: AtomicUsize = AtomicUsize::new(0);
const PRODUCER: &str = r#"
import json, os, subprocess, sys, time, uuid
mode, path, source = sys.argv[1:]
if mode == 'env-cwd':
    with open(path + '.context', 'w') as f:
        json.dump(dict(cwd=os.getcwd(), token=os.environ.get('UG_REVIEW_TOKEN')), f)
with open(path + '.starts', 'a') as f: f.write(str(time.monotonic()) + '\n')
descendant = None
if mode in ('descendants', 'exit-descendants'):
    descendant = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
with open(path, 'w') as f: json.dump([os.getpid(), descendant.pid if descendant else None], f)
instance = str(uuid.uuid4())
def msg(seq, payload, name=None):
    print(json.dumps(dict(schema_version=1, source=name or source, instance=instance, sequence=seq, payload=payload)), flush=True)
state = dict(kind='state', state='working', ttl_ms=500)
if mode == 'crash': sys.exit(2)
if mode == 'spoof': msg(1, state, 'not-authorized')
elif mode == 'malformed': print('{"schema_version":1,"payload":{"kind":"remove","extra":true}}', flush=True)
elif mode == 'oversize': print('x' * 4097, flush=True)
elif mode == 'no-newline': sys.stdout.write('{'); sys.stdout.flush()
elif mode == 'flood':
    for i in range(1000): msg(i, state)
elif mode == 'once':
    msg(1, state)
    msg(2, dict(kind='event', event='done', ttl_ms=2000))
    msg(3, dict(kind='remove'))
    sys.exit(0)
elif mode == 'exit-descendants': msg(1, state); sys.exit(0)
else: msg(1, state)
if mode in ('quiet', 'spoof', 'malformed', 'oversize', 'no-newline', 'flood'):
    time.sleep(60)
else:
    i = 2
    while True:
        time.sleep(.1)
        if mode == 'stderr': os.write(2, b'x' * 65536)
        msg(i, state)
        i += 1
"#;

struct Harness {
    dir: PathBuf,
    child: Option<Child>,
}
impl Harness {
    fn new() -> Self {
        let dir = PathBuf::from(format!(
            "/tmp/ug-mc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::create_dir(dir.join("home")).unwrap();
        fs::write(dir.join("producer.py"), PRODUCER).unwrap();
        Self { dir, child: None }
    }
    fn command(&self) -> Command {
        let mut c = Command::new(BIN);
        c.env("HOME", self.dir.join("home"))
            .env("WOOTING_STATE_DIR", self.dir.join("runtime"))
            .env("WOOTING_DEV_SIMULATION", "1")
            .env("WOOTING_RGB_SDK_PATH", self.dir.join("sdk.dylib"))
            .env("WOOTING_ANALOG_SDK_PATH", self.dir.join("sdk.dylib"));
        c
    }
    fn config(
        &self,
        id: &str,
        mode: &str,
        enabled: bool,
        trusted: bool,
        restart_limit: u32,
    ) -> String {
        let argv = vec![
            "python3".to_string(),
            "-u".into(),
            self.dir.join("producer.py").display().to_string(),
            mode.into(),
            self.dir.join(id).display().to_string(),
            id.into(),
        ];
        format!(
            "\n[[connectors]]\nid={id:?}\nsource={id:?}\ncommand={}\nenabled={enabled}\ntrusted={trusted}\nrestart_initial_ms=300\nrestart_max_ms=600\nrestart_limit={restart_limit}\n",
            serde_json::to_string(&argv).unwrap()
        )
    }
    fn profile(&self, config: &str) -> PathBuf {
        let p = self.dir.join("profile.toml");
        fs::write(&p, config).unwrap();
        p
    }
    fn start(&mut self, config: &str) {
        let p = self.profile(config);
        self.child = Some(
            self.command()
                .args(["engine", "--paused", "--config"])
                .arg(p)
                .stdout(Stdio::null())
                .stderr(Stdio::from(
                    File::create(self.dir.join("engine.log")).unwrap(),
                ))
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.dir.join("runtime/state.json").exists()
            || !self.dir.join("runtime/control.sock").exists()
        {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "{}",
                fs::read_to_string(self.dir.join("engine.log")).unwrap()
            );
            assert!(Instant::now() < deadline, "startup timeout");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn cli(&self, args: &[&str]) -> Value {
        let out = self.command().args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn health(&self) -> Value {
        self.cli(&["control", "connectors", "status"])["connectors"].clone()
    }
    fn connections_input(&self, input: &str, revision: &str, ok: bool) -> Value {
        let mut child = self
            .command()
            .args([
                "control",
                "connections",
                "replace",
                "--expected-revision",
                revision,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Oversized input can close its pipe as soon as the bound is detected.
        let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
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
    fn replace_connections(&self, config: &str) -> Value {
        use underglow::connection_registry::{ConnectionRegistry, ConnectorConfig};
        let legacy: toml::Value = toml::from_str(config).unwrap();
        let configs: Vec<ConnectorConfig> = legacy
            .get("connectors")
            .map(|value| value.clone().try_into().unwrap())
            .unwrap_or_default();
        let current = self.cli(&["control", "connections", "show"]);
        let mut registry = ConnectionRegistry::from_legacy(&configs).unwrap();
        registry.revision = current["connections"]["registry"]["revision"]
            .as_u64()
            .unwrap();
        self.request(
            serde_json::json!({"schema_version":1,"action":"connections-replace",
            "expected_revision":registry.revision,"registry":registry}),
        )
    }
    fn request(&self, request: Value) -> Value {
        let mut stream = UnixStream::connect(self.dir.join("runtime/control.sock")).unwrap();
        writeln!(stream, "{request}").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["ok"], true, "{}", response["error"]);
        response
    }
    fn wait_health(&self, id: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let h = self.health();
            if let Some(v) = h.as_array().unwrap().iter().find(|v| v["id"] == id)
                && pred(v)
            {
                return v.clone();
            }
            assert!(Instant::now() < deadline, "health timeout: {h}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn pids(&self, id: &str) -> Vec<Option<u32>> {
        serde_json::from_str(&fs::read_to_string(self.dir.join(id)).unwrap()).unwrap()
    }
    fn inspect(&self) -> Value {
        self.cli(&["external", "inspect"])
    }
    fn stop(&mut self) {
        self.cli(&["control", "stop"]);
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if self.child.as_mut().unwrap().try_wait().unwrap().is_some() {
                self.child = None;
                break;
            }
            assert!(Instant::now() < deadline, "engine failed to stop");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = self
                .command()
                .args(["control", "stop"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}
fn process_status(pid: u32) -> String {
    let out = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
fn assert_gone(pids: &[Option<u32>]) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let alive: Vec<_> = pids
            .iter()
            .flatten()
            .filter(|p| {
                let s = process_status(**p);
                !s.is_empty() && !s.starts_with('Z')
            })
            .collect();
        if alive.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "owned processes still alive: {alive:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // The direct child must actually be reaped, not just killed into a zombie.
    assert!(process_status(pids[0].unwrap()).is_empty());
}

#[test]
fn default_disabled_and_enable_cannot_grant_trust() {
    let mut h = Harness::new();
    let config = h
        .config("focus", "hold", false, false, 0)
        .replace("enabled=false\n", "")
        .replace("trusted=false\n", "");
    h.start(&config);
    h.wait_health("focus", |v| v["state"] == "disabled");
    let out = h
        .command()
        .args(["control", "connectors", "enable", "focus"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("explicitly trusted"));
    assert!(!h.dir.join("focus").exists());
    let bad = h.profile(&h.config("bad", "hold", true, false, 0));
    assert!(
        !h.command()
            .args(["control", "select", "--config"])
            .arg(bad)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(h.cli(&["control", "status"])["status"]["state"], "paused");
}

#[test]
fn validation_dry_run_and_preview_never_spawn_even_when_opted_in() {
    let h = Harness::new();
    let p = h.profile(&h.config("focus", "hold", true, true, 0));
    for flag in ["--dry-run", "--preview"] {
        let out = h
            .command()
            .args(["run", "--config"])
            .arg(&p)
            .arg(flag)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!h.dir.join("focus").exists());
    }
    assert!(!h.dir.join("runtime/hardware.lock").exists());
}

#[test]
fn paused_engine_observes_facts_and_health_is_not_application_state() {
    let mut h = Harness::new();
    // Detect dlopen itself, not merely attempted SDK initialization.
    let sentinel = h.dir.join("sdk.c");
    fs::write(&sentinel, format!("#include <stdio.h>\n__attribute__((constructor)) static void loaded(void) {{ FILE *f=fopen({:?}, \"w\"); if(f) {{ fputs(\"loaded\",f); fclose(f); }} }}\n", h.dir.join("sdk-loaded").display().to_string())).unwrap();
    let output = Command::new("cc")
        .args(if cfg!(target_os = "macos") {
            vec!["-dynamiclib"]
        } else {
            vec!["-shared", "-fPIC"]
        })
        .arg(sentinel)
        .arg("-o")
        .arg(h.dir.join("sdk.dylib"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    h.start(&h.config("focus", "quiet", true, true, 0));
    h.wait_health("focus", |v| v["healthy"] == true);
    let state = h.inspect();
    assert!(state.to_string().contains("working"));
    std::thread::sleep(Duration::from_millis(650));
    assert!(!h.inspect().to_string().contains("working"));
    assert_eq!(h.health()[0]["healthy"], true); // transport is healthy; application's lease expired
    assert_eq!(h.cli(&["control", "status"])["status"]["state"], "paused");
    assert!(!h.dir.join("runtime/hardware.lock").exists());
    assert!(!h.dir.join("sdk-loaded").exists());
}

#[test]
fn malformed_spoofed_oversize_unterminated_and_flooding_output_is_stopped() {
    for (mode, error) in [
        ("spoof", "source"),
        ("malformed", "JSON"),
        ("oversize", "4096"),
        ("no-newline", "newline"),
        ("flood", "rate limit"),
    ] {
        let mut h = Harness::new();
        h.start(&h.config("focus", mode, true, true, 0));
        let health = h.wait_health("focus", |v| v["state"] == "failed" && v["pid"].is_null());
        assert!(
            health["last_error"].as_str().unwrap().contains(error),
            "{mode}: {health}"
        );
        assert_gone(&h.pids("focus"));
        assert!(!h.inspect().to_string().contains("not-authorized"));
        assert_eq!(h.cli(&["control", "status"])["status"]["state"], "paused");
    }
}

#[test]
fn repeated_crashes_use_capped_backoff_and_retry_limit() {
    let mut h = Harness::new();
    h.start(&h.config("focus", "crash", true, true, 2));
    let first = h.wait_health("focus", |v| v["state"] == "backoff");
    assert_eq!(first["healthy"], false);
    assert_eq!(first["running"], false);
    h.wait_health("focus", |v| v["state"] == "failed" && v["pid"].is_null());
    let times: Vec<f64> = fs::read_to_string(h.dir.join("focus.starts"))
        .unwrap()
        .lines()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(times.len(), 3);
    assert!(
        times[1] - times[0] >= 0.25 && times[2] - times[1] >= 0.55,
        "{times:?}"
    );
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        fs::read_to_string(h.dir.join("focus.starts"))
            .unwrap()
            .lines()
            .count(),
        3
    );
}

#[test]
fn disable_enable_and_shutdown_reap_owned_groups_without_touching_other_connectors() {
    let mut h = Harness::new();
    h.start(
        &(h.config("one", "descendants", true, true, 0) + &h.config("two", "hold", true, true, 0)),
    );
    h.wait_health("one", |v| v["healthy"] == true);
    h.wait_health("two", |v| v["healthy"] == true);
    let old = h.pids("one");
    let other = h.pids("two");
    h.cli(&["control", "connectors", "disable", "one"]);
    h.wait_health("one", |v| v["state"] == "disabled");
    assert_gone(&old);
    assert_eq!(h.pids("two"), other);
    assert!(!process_status(other[0].unwrap()).is_empty());
    h.cli(&["control", "connectors", "enable", "one"]);
    h.wait_health("one", |v| v["healthy"] == true);
    let new = h.pids("one");
    assert_ne!(old[0], new[0]);
    h.stop();
    assert_gone(&new);
    assert_gone(&other);
}

#[test]
fn explicit_connection_replacement_stops_old_group_and_leases_expire() {
    let mut h = Harness::new();
    h.start(&h.config("focus", "descendants", true, true, 0));
    h.wait_health("focus", |v| v["healthy"] == true);
    let old = h.pids("focus");
    h.replace_connections("effect='matrix'\n");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !h.health().as_array().unwrap().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_gone(&old);
    std::thread::sleep(Duration::from_millis(600));
    assert!(!h.inspect().to_string().contains("working"));
}

#[test]
fn natural_exit_drains_final_event_and_remove_and_cleans_descendants() {
    let mut h = Harness::new();
    h.start(&h.config("focus", "once", true, true, 0));
    h.wait_health("focus", |v| v["state"] == "failed" && v["pid"].is_null());
    let state = h.inspect().to_string();
    assert!(state.contains("done"));
    assert!(!state.contains("working"));
    h.replace_connections(&h.config("focus", "exit-descendants", true, true, 0));
    // Explicit connection changes pass through stopping; wait for the new invocation.
    let deadline = Instant::now() + Duration::from_secs(4);
    while h.pids("focus")[1].is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    h.wait_health("focus", |v| v["state"] == "failed" && v["pid"].is_null());
    assert_gone(&h.pids("focus"));
}

#[test]
fn stderr_cannot_block_supervisor_and_unknown_control_fields_stay_rejected() {
    let mut h = Harness::new();
    h.start(&h.config("focus", "stderr", true, true, 0));
    h.wait_health("focus", |v| v["healthy"] == true);
    let mut stream = UnixStream::connect(h.dir.join("runtime/control.sock")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(b"{\"schema_version\":1,\"action\":\"connector-enable\",\"id\":\"focus\",\"trusted\":true}\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], false);
    let pids = h.pids("focus");
    h.stop();
    assert_gone(&pids);
}

#[test]
fn sigterm_shutdown_reaps_groups_and_enabled_intent_survives_restart() {
    let mut h = Harness::new();
    h.start(&h.config("focus", "descendants", false, true, 0));
    h.wait_health("focus", |v| v["state"] == "disabled");
    h.cli(&["control", "connectors", "enable", "focus"]);
    h.wait_health("focus", |v| v["healthy"] == true);
    let pids = h.pids("focus");
    assert!(
        Command::new("kill")
            .args(["-TERM", &h.child.as_ref().unwrap().id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(4);
    while h.child.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    h.child = None;
    assert_gone(&pids);
    // Saved state wins over the startup file (which still has enabled=false).
    h.start(&h.config("focus", "descendants", false, true, 0));
    h.wait_health("focus", |v| {
        v["healthy"] == true && v["pid"] != pids[0].unwrap()
    });
    assert_eq!(h.cli(&["control", "status"])["status"]["state"], "paused");
}

#[test]
fn generated_starter_stream_works_without_engine_specific_producer_code() {
    let mut h = Harness::new();
    let output_dir = h.dir.join("starter");
    let output = Command::new("python3")
        .args([
            "scripts/scaffold-connector.py",
            "--language",
            "python",
            "--name",
            "focus",
            "--output",
        ])
        .arg(&output_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let argv = serde_json::json!([
        "python3",
        "-u",
        output_dir.join("connector.py"),
        "--stdout",
        "--duration-seconds",
        "0.1"
    ]);
    h.start(&format!("[[connectors]]\nid='timer'\nsource='focus'\ncommand={argv}\nenabled=true\ntrusted=true\nrestart_limit=0\n"));
    h.wait_health("timer", |v| v["state"] == "failed" && v["pid"].is_null());
    let state = h.inspect().to_string();
    assert!(state.contains("done"), "{state}");
    assert!(!state.contains("focusing"), "{state}");
}

#[test]
fn two_setups_edits_and_legacy_visual_selection_reuse_one_pid_without_machine_secrets() {
    let mut h = Harness::new();
    let config = h.config("focus", "hold", true, true, 0);
    h.start(&config);
    let initial = h.wait_health("focus", |v| v["healthy"] == true);
    let registry_bytes = fs::read(h.dir.join("runtime/connections.json")).unwrap();
    let profile: toml::Value =
        toml::from_str(include_str!("../examples/layers-profile.toml")).unwrap();
    let mut document = serde_json::to_value(&profile["setup"]).unwrap();
    document["layers"][1]["activation"] =
        serde_json::json!({"kind":"while-state","source":"focus"});
    for revision in 0..2 {
        document["name"] = serde_json::json!(format!("Visual {revision}"));
        h.request(
            serde_json::json!({"schema_version":1,"action":"setup-replace",
            "expected_revision":revision,"document":document}),
        );
    }
    h.request(serde_json::json!({"schema_version":1,"action":"setup-edit",
        "expected_revision":2,"edits":[{"op":"remove","id":"matrix"}]}));
    let library = h.cli(&["control", "library", "list"])["library"].clone();
    let revision = library["revision"].as_u64().unwrap();
    let current_id = library["current_id"].as_str().unwrap();
    h.request(serde_json::json!({"schema_version":1,"action":"library-update",
        "expected_revision":revision,"mutation":{"op":"duplicate","id":current_id,"new_id":"second-visual","name":"Second visual"}}));
    h.request(serde_json::json!({"schema_version":1,"action":"library-update",
        "expected_revision":revision+1,"mutation":{"op":"switch","id":"second-visual","expected_setup_revision":3}}));
    h.request(serde_json::json!({"schema_version":1,"action":"library-update",
        "expected_revision":revision+2,"mutation":{"op":"switch","id":current_id,"expected_setup_revision":4}}));
    assert_eq!(
        h.wait_health("focus", |v| v["healthy"] == true)["pid"],
        initial["pid"]
    );
    let p =
        h.profile(&(String::from("effect='aurora'\n") + &h.config("other", "hold", true, true, 0)));
    let selected = h.cli(&["control", "select", "--config", p.to_str().unwrap()]);
    assert!(selected["warning"].as_str().unwrap().contains("ignored"));
    assert!(!h.dir.join("other").exists());
    assert_eq!(
        h.wait_health("focus", |v| v["running"] == true)["pid"],
        initial["pid"]
    );
    assert_eq!(
        fs::read(h.dir.join("runtime/connections.json")).unwrap(),
        registry_bytes
    );
    assert_eq!(
        fs::read_to_string(h.dir.join("focus.starts"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let saved: Value =
        serde_json::from_slice(&fs::read(h.dir.join("runtime/state.json")).unwrap()).unwrap();
    assert!(!saved["config"].as_str().unwrap().contains("producer.py"));
    assert!(!saved["config"].as_str().unwrap().contains("connectors"));
    let report = h.cli(&["control", "connections", "show"]);
    assert_eq!(
        report["connections"]["runtime_index"]["focus"]["producer"]["pid"],
        initial["pid"]
    );
    assert_eq!(report["status"]["state"], "paused");
    assert!(!h.dir.join("runtime/hardware.lock").exists());
}

#[test]
fn explicit_blank_adoption_removes_hidden_legacy_visuals_without_restarting_producers() {
    let mut h = Harness::new();
    let sentinel = h.dir.join("notification-ran");
    let notification_command = serde_json::to_string(&vec![
        "python3".to_string(),
        "-c".into(),
        format!(
            "from pathlib import Path; Path({:?}).write_text('unexpected')",
            sentinel.display().to_string()
        ),
    ])
    .unwrap();
    let config = format!(
        "effect='matrix'\n[signal]\nkind='command-pulse'\ncommand=['/must-not-run-dormant-provider']\n[[notifications]]\nid='legacy-command'\nduration_seconds=1\nzones=['function']\n[notifications.signal]\nkind='command-pulse'\ncommand={notification_command}\n[[automation.rules]]\nid='hidden-red'\nsource='focus'\nmode='while-state'\nstate='working'\nscene='red'\n[automation.scenes.red]\ncolor=[255,0,0]\n{}",
        h.config("focus", "hold", true, true, 0)
    );
    h.start(&config);
    let producer = h.wait_health("focus", |health| health["healthy"] == true);
    let connections = fs::read(h.dir.join("runtime/connections.json")).unwrap();
    let before: Value =
        serde_json::from_slice(&fs::read(h.dir.join("runtime/state.json")).unwrap()).unwrap();
    let retained: toml::Value = toml::from_str(before["config"].as_str().unwrap()).unwrap();
    assert!(retained.get("setup").is_none());
    assert_eq!(retained["notifications"].as_array().unwrap().len(), 1);
    assert_eq!(retained["automation"]["rules"].as_array().unwrap().len(), 1);
    assert_eq!(
        fs::read_to_string(h.dir.join("profile.toml")).unwrap(),
        config
    );
    let blank = underglow::setup_library::starter(
        underglow::setup_library::Starter::Blank,
        "user-owned".into(),
        "My blank".into(),
    )
    .unwrap();
    let reply = h.request(serde_json::json!({"schema_version":1,"action":"setup-replace","expected_revision":0,"document":blank}));
    assert_eq!(reply["status"]["state"], "paused");
    let after: Value =
        serde_json::from_slice(&fs::read(h.dir.join("runtime/state.json")).unwrap()).unwrap();
    let adopted: toml::Value = toml::from_str(after["config"].as_str().unwrap()).unwrap();
    assert!(adopted.get("notifications").is_none());
    assert!(adopted["automation"].get("rules").is_none());
    assert!(adopted["setup"]["layers"].as_array().unwrap().is_empty());
    assert_eq!(
        adopted["signal"], retained["signal"],
        "legacy base command remains dormant, not executed"
    );
    let preview = h.cli(&["control", "preview"]);
    assert!(
        preview["preview"]["rgb"]
            .as_array()
            .unwrap()
            .iter()
            .all(|channel| channel == 0)
    );
    assert!(!sentinel.exists());
    assert_eq!(
        h.wait_health("focus", |health| health["healthy"] == true)["pid"],
        producer["pid"]
    );
    assert_eq!(
        fs::read(h.dir.join("runtime/connections.json")).unwrap(),
        connections
    );
    assert_eq!(
        fs::read_to_string(h.dir.join("profile.toml")).unwrap(),
        config
    );
    assert!(!h.dir.join("runtime/hardware.lock").exists());
    h.stop();
    h.start(&config);
    assert!(
        h.cli(&["control", "setup", "show"])["setup"]["document"]["layers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!sentinel.exists());
}

#[test]
fn registry_cli_is_bounded_atomic_and_does_not_reinstall_removed_legacy_producers() {
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::new();
    h.start(&h.config("focus", "hold", false, true, 0));
    let original = h.cli(&["control", "connections", "show"])["connections"]["registry"].clone();
    let path = h.dir.join("runtime/connections.json");
    let before = fs::read(&path).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    for (input, revision) in [
        ("bad".to_string(), "0"),
        (
            {
                let mut bad = original.clone();
                bad["extra"] = serde_json::json!(true);
                bad.to_string()
            },
            "0",
        ),
        (original.to_string(), "9"),
        (" ".repeat(256 * 1024 + 1), "0"),
    ] {
        h.connections_input(&input, revision, false);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!h.dir.join("focus").exists());
    }
    // Metadata-only replacement is explicit but still must not start a disabled producer.
    let replaced = h.connections_input(&original.to_string(), "0", true);
    assert_eq!(replaced["connections"]["registry"]["revision"], 1);
    h.cli(&["control", "connectors", "enable", "focus"]);
    h.wait_health("focus", |v| v["healthy"] == true);
    let removed = h.replace_connections("effect='matrix'\n");
    assert!(
        removed["connections"]["registry"]["connections"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c.get("producer").is_none())
    );
    h.stop();
    let bytes = fs::read(&path).unwrap();
    h.start(&h.config("focus", "hold", true, true, 0));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(h.health().as_array().unwrap().is_empty());
    assert_eq!(h.cli(&["control", "status"])["status"]["state"], "paused");
}

#[test]
fn descriptor_only_edits_preserve_pid_and_missing_bindings_remain_repairable() {
    let mut h = Harness::new();
    h.start(&h.config("focus", "hold", true, true, 0));
    let original = h.wait_health("focus", |v| v["healthy"] == true);
    let mut registry =
        h.cli(&["control", "connections", "show"])["connections"]["registry"].clone();
    let before = fs::read(h.dir.join("runtime/connections.json")).unwrap();
    let engine_pid = h.child.as_ref().unwrap().id();
    let occupied = h.dir.join(format!("runtime/.connections-{engine_pid}.tmp"));
    fs::write(&occupied, "occupied").unwrap();
    let mut disabled = registry.clone();
    disabled["connections"][1]["producer"]["enabled"] = serde_json::json!(false);
    let rejected = h.connections_input(&disabled.to_string(), "0", false);
    assert_eq!(rejected["connections"]["registry"], registry);
    assert_eq!(
        fs::read(h.dir.join("runtime/connections.json")).unwrap(),
        before
    );
    assert_eq!(
        h.wait_health("focus", |v| v["healthy"] == true)["pid"],
        original["pid"]
    );
    fs::remove_file(occupied).unwrap();
    let local: underglow::connection_registry::ConnectionRegistry =
        serde_json::from_value(registry.clone()).unwrap();
    let mut descriptor = local.connection("focus").unwrap().descriptor.clone();
    descriptor.display_name = "Friendly source".into();
    let descriptor = underglow::capability_manifest::parse(
        &serde_json::to_vec(&descriptor).unwrap(),
        underglow::capability_manifest::ManifestFormat::Json,
    )
    .unwrap();
    assert!(
        underglow::capability_manifest::import_descriptor(&local, 0, descriptor.clone(), false)
            .is_err()
    );
    let candidate =
        underglow::capability_manifest::import_descriptor(&local, 0, descriptor, true).unwrap();
    assert_eq!(candidate.managed_configs(), local.managed_configs());
    registry = serde_json::to_value(&candidate).unwrap();
    h.connections_input(&registry.to_string(), "0", true);
    // A stale metadata import cannot alter either the registry or producer.
    h.connections_input(&registry.to_string(), "0", false);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        h.wait_health("focus", |v| v["healthy"] == true)["pid"],
        original["pid"]
    );
    let config: toml::Value =
        toml::from_str(include_str!("../examples/layers-profile.toml")).unwrap();
    let mut document = serde_json::to_value(&config["setup"]).unwrap();
    document["layers"][1]["activation"] =
        serde_json::json!({"kind":"while-state","source":"missing"});
    h.request(serde_json::json!({"schema_version":1,"action":"setup-replace","expected_revision":0,"document":document}));
    let report = h.cli(&["control", "connections", "show"]);
    assert_eq!(
        report["connections"]["missing_sources"],
        serde_json::json!(["missing"])
    );
    let shown = h.cli(&["control", "setup", "show"]);
    assert_eq!(
        shown["setup"]["document"]["layers"][1]["activation"]["source"],
        "missing"
    );
    let draft: underglow::setup::SetupDocument =
        serde_json::from_value(shown["setup"]["document"].clone()).unwrap();
    let binding = underglow::binding_repair::bindings(&draft)
        .into_iter()
        .find(|b| b.source == "missing")
        .unwrap();
    let repair = underglow::binding_repair::BindingRepair {
        setup_id: draft.id.clone(),
        key: binding.key,
        expected_activation: binding.activation.clone(),
        source: "focus".into(),
        instance: None,
    };
    let (repaired, _) =
        underglow::binding_repair::repair_binding(&draft, &candidate, &repair).unwrap();
    let mut expected = draft.clone();
    expected.layers[1].activation = underglow::setup::Activation::WhileState {
        source: "focus".into(),
        instance: None,
        state: None,
    };
    assert_eq!(
        repaired, expected,
        "binding repair preserves all visual behavior"
    );
    // Explicitly save only now; local repair itself never writes engine state.
    h.request(serde_json::json!({"schema_version":1,"action":"setup-replace","expected_revision":draft.revision,"document":repaired}));
    let report = h.cli(&["control", "connections", "show"]);
    assert_eq!(
        report["connections"]["missing_sources"],
        serde_json::json!([])
    );
    assert_eq!(report["status"]["state"], "paused");
    assert_eq!(
        h.wait_health("focus", |v| v["healthy"] == true)["pid"],
        original["pid"]
    );
    assert!(!h.dir.join("runtime/hardware.lock").exists());
}

#[test]
fn reviewed_argv_environment_and_directory_enable_disable_remain_paused() {
    let mut h = Harness::new();
    h.start(&h.config("focus", "env-cwd", false, false, 0));
    let mut registry =
        h.cli(&["control", "connections", "show"])["connections"]["registry"].clone();
    let mut untrusted = registry.clone();
    untrusted["connections"][1]["producer"]["enabled"] = serde_json::json!(true);
    h.connections_input(&untrusted.to_string(), "0", false);
    assert!(!h.dir.join("focus").exists());
    let cwd = h.dir.join("reviewed directory");
    fs::create_dir(&cwd).unwrap();
    let config = &mut registry["connections"][1]["producer"];
    config["env"] = serde_json::json!({"UG_REVIEW_TOKEN":"private reviewed value"});
    config["working_dir"] = serde_json::json!(cwd);
    config["trusted"] = serde_json::json!(true);
    let saved = h.connections_input(&registry.to_string(), "0", true);
    assert_eq!(saved["status"]["state"], "paused");
    assert!(
        !h.dir.join("focus").exists(),
        "review/save cannot start a disabled producer"
    );
    let mut enabled = saved["connections"]["registry"].clone();
    enabled["connections"][1]["producer"]["enabled"] = serde_json::json!(true);
    h.connections_input(&enabled.to_string(), "1", true);
    let running = h.wait_health("focus", |v| v["healthy"] == true);
    let evidence: Value =
        serde_json::from_slice(&fs::read(h.dir.join("focus.context")).unwrap()).unwrap();
    assert_eq!(
        evidence["cwd"],
        serde_json::json!(fs::canonicalize(cwd).unwrap())
    );
    assert_eq!(evidence["token"], "private reviewed value");
    // A stale disable must not alter the process or registry.
    enabled["connections"][1]["producer"]["enabled"] = serde_json::json!(false);
    h.connections_input(&enabled.to_string(), "1", false);
    assert_eq!(
        h.wait_health("focus", |v| v["healthy"] == true)["pid"],
        running["pid"]
    );
    let mut disabled =
        h.cli(&["control", "connections", "show"])["connections"]["registry"].clone();
    disabled["connections"][1]["producer"]["enabled"] = serde_json::json!(false);
    h.connections_input(&disabled.to_string(), "2", true);
    h.wait_health("focus", |v| {
        v["state"] == "disabled" && v["running"] == false
    });
    assert_eq!(h.cli(&["control", "status"])["status"]["state"], "paused");
    assert!(!h.dir.join("runtime/hardware.lock").exists());
}
