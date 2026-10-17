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
fn profile_replacement_stops_old_group_and_leases_expire() {
    let mut h = Harness::new();
    h.start(&h.config("focus", "descendants", true, true, 0));
    h.wait_health("focus", |v| v["healthy"] == true);
    let old = h.pids("focus");
    let p = h.profile("effect='matrix'\n");
    h.cli(&["control", "select", "--config", p.to_str().unwrap()]);
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
    let p = h.profile(&h.config("focus", "exit-descendants", true, true, 0));
    h.cli(&["control", "select", "--config", p.to_str().unwrap()]);
    // A changed profile passes through stopping; wait for the new invocation.
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
