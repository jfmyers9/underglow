//! Native, opt-in macOS login integration. Never initializes keyboard SDKs.
use serde_json::{Value, json};

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = if args.len() != 1
        || ![
            "status",
            "enable",
            "disable",
            "start",
            "stop",
            "remove",
            "prepare-update",
        ]
        .contains(&args[0].as_str())
    {
        Err("usage: wooting-service status|enable|disable|start|stop|remove|prepare-update".into())
    } else {
        dispatch(&args[0])
    };
    let response = result.unwrap_or_else(|error: String| json!({"ok":false,"supported":cfg!(target_os="macos"),"enabled":false,"running":false,"error":error}));
    println!("{response}");
    std::process::exit(if response["ok"] == true { 0 } else { 1 });
}

fn dispatch(verb: &str) -> Result<Value, String> {
    #[cfg(target_os = "macos")]
    {
        mac::dispatch(verb)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(
            json!({"ok":verb == "status","supported":false,"enabled":false,"running":false,"error":"native service helper supports macOS only"}),
        )
    }
}

#[cfg(any(target_os = "macos", all(test, unix)))]
mod mac {
    use super::*;
    use std::{
        fs::{self, OpenOptions},
        io::{Read, Write},
        os::unix::{
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::UnixStream,
        },
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };
    const LABEL: &str = "io.github.jfmyers9.wooting-signals";
    const LIMIT: u64 = 256 * 1024;
    type Result<T> = std::result::Result<T, String>;
    fn err(e: impl std::fmt::Display) -> String {
        e.to_string()
    }
    #[derive(Debug)]
    struct Output {
        ok: bool,
        text: String,
    }
    trait Runner {
        fn run(&mut self, program: &Path, args: &[&str], state: Option<&Path>) -> Result<Output>;
    }
    struct Native {
        deadline: Instant,
    }
    impl Native {
        fn new() -> Self {
            Self {
                deadline: Instant::now() + Duration::from_secs(15),
            }
        }
    }
    impl Runner for Native {
        fn run(&mut self, program: &Path, args: &[&str], state: Option<&Path>) -> Result<Output> {
            if Instant::now() >= self.deadline {
                return Err("service operation timed out".into());
            }
            let mut cmd = Command::new(program);
            cmd.args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(state) = state {
                cmd.env("WOOTING_STATE_DIR", state);
            }
            let mut child = cmd.spawn().map_err(err)?;
            let (tx, rx) = mpsc::channel();
            let stdout = child.stdout.take().unwrap();
            let stderr = child.stderr.take().unwrap();
            fn collect(
                reader: impl Read + Send + 'static,
                tx: mpsc::Sender<std::io::Result<Vec<u8>>>,
            ) {
                thread::spawn(move || {
                    let mut bytes = Vec::new();
                    let result = reader
                        .take(LIMIT + 1)
                        .read_to_end(&mut bytes)
                        .map(|_| bytes);
                    let _ = tx.send(result);
                });
            }
            collect(stdout, tx.clone());
            collect(stderr, tx);
            let deadline = self.deadline.min(Instant::now() + Duration::from_secs(5));
            let result = (|| {
                let mut chunks = Vec::new();
                let mut status = None;
                loop {
                    while let Ok(bytes) = rx.try_recv() {
                        let bytes = bytes.map_err(err)?;
                        if bytes.len() as u64 > LIMIT {
                            return Err("subprocess output exceeded limit".into());
                        }
                        chunks.push(bytes);
                    }
                    if status.is_none() {
                        status = child.try_wait().map_err(err)?;
                    }
                    if let Some(status) = status
                        && chunks.len() == 2
                    {
                        return Ok(Output {
                            ok: status.success(),
                            text: chunks
                                .into_iter()
                                .map(|v| String::from_utf8_lossy(&v).into_owned())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        });
                    }
                    if Instant::now() >= deadline {
                        return Err("subprocess timed out after 5 seconds".into());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            })();
            if result.is_err() {
                let _ = child.kill();
                let _ = child.wait();
            }
            result
        }
    }
    struct Paths {
        binary: PathBuf,
        state: PathBuf,
        registration: PathBuf,
        domain: String,
        uid: u32,
    }
    impl Paths {
        fn new(home: &Path, binary: PathBuf, uid: u32) -> Result<Self> {
            if !home.is_absolute() || !binary.is_absolute() {
                return Err("home and executable paths must be absolute".into());
            }
            Ok(Self {
                binary,
                state: home.join("Library/Application Support/wooting-signals/runtime"),
                registration: home
                    .join("Library/LaunchAgents")
                    .join(format!("{LABEL}.plist")),
                domain: format!("gui/{uid}"),
                uid,
            })
        }
        fn target(&self) -> String {
            format!("{}/{LABEL}", self.domain)
        }
    }
    fn path(p: &Path) -> Result<&str> {
        p.to_str().ok_or_else(|| "paths must be UTF-8".into())
    }
    fn xml(value: &str) -> Result<String> {
        if value.chars().any(|c| {
            (c < ' ' && !['\t', '\n', '\r'].contains(&c)) || ['\u{fffe}', '\u{ffff}'].contains(&c)
        }) {
            return Err("path contains invalid XML characters".into());
        }
        Ok(value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;"))
    }
    fn plist(p: &Paths, enabled: bool) -> Result<String> {
        let binary = xml(path(&p.binary)?)?;
        let state = xml(path(&p.state)?)?;
        let log = xml(path(&p.state.join("engine.log"))?)?;
        Ok(format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>{binary}</string><string>engine</string><string>--state-dir</string><string>{state}</string></array>
<key>RunAtLoad</key><{enabled}/>
<key>EnvironmentVariables</key><dict><key>WOOTING_STATE_DIR</key><string>{state}</string></dict>
<key>StandardOutPath</key><string>{log}</string><key>StandardErrorPath</key><string>{log}</string>
<key>Umask</key><integer>63</integer>
</dict></plist>
"#,
            enabled = if enabled { "true" } else { "false" }
        ))
    }
    fn checked(out: Output) -> Result<String> {
        if out.ok {
            Ok(out.text)
        } else {
            Err(format!("service command failed: {}", out.text.trim()))
        }
    }
    fn manager(r: &mut impl Runner, args: &[&str]) -> Result<Output> {
        r.run(Path::new("/bin/launchctl"), args, None)
    }
    #[derive(Default)]
    struct Status {
        enabled: bool,
        running: bool,
        loaded: bool,
        stale: bool,
    }
    fn status(p: &Paths, r: &mut impl Runner) -> Result<Status> {
        let mut s = Status::default();
        if p.registration.try_exists().map_err(err)? {
            let meta = fs::symlink_metadata(&p.registration).map_err(err)?;
            if !meta.is_file() || meta.len() > LIMIT {
                return Err("registration must be a bounded regular plist".into());
            }
            let raw = checked(r.run(
                Path::new("/usr/bin/plutil"),
                &["-convert", "json", "-o", "-", path(&p.registration)?],
                None,
            )?)?;
            let v: Value = serde_json::from_str(raw.trim()).map_err(err)?;
            if v["Label"] != LABEL {
                return Err("refusing unrelated LaunchAgent".into());
            }
            s.enabled = v["RunAtLoad"].as_bool().unwrap_or(false);
            s.stale = v["ProgramArguments"][0] != path(&p.binary)?
                || v["EnvironmentVariables"]["WOOTING_STATE_DIR"] != path(&p.state)?;
        }
        // Verify the user domain first so permission/domain failures are not treated as 'stopped'.
        checked(manager(r, &["print-disabled", &p.domain])?)?;
        let out = manager(r, &["print", &p.target()])?;
        s.loaded = out.ok;
        if !out.ok
            && !out.text.contains("Could not find service")
            && !out.text.contains("Could not find specified service")
        {
            return Err(format!("cannot query LaunchAgent: {}", out.text.trim()));
        }
        s.running = out.ok && out.text.lines().any(|l| l.trim() == "state = running");
        if out.ok {
            // launchctl prints the loaded program, which can differ from a rewritten plist.
            if let Some(program) = out
                .text
                .lines()
                .find_map(|l| l.trim().strip_prefix("program = "))
            {
                s.stale |= program != path(&p.binary)?;
            }
        }
        Ok(s)
    }
    fn response(s: Status) -> Value {
        json!({"ok":true,"supported":true,"enabled":s.enabled,"running":s.running,"error":if s.stale {Some("LaunchAgent points to an old app/state path. Stop it, then explicitly re-enable from this app before starting.")} else {None}})
    }
    fn real_dir(p: &Path, uid: u32, private: bool) -> Result<()> {
        // Reject symlink ancestors before creating anything; never chmod a redirected home path.
        for ancestor in p.ancestors() {
            if let Ok(m) = fs::symlink_metadata(ancestor)
                && (!m.is_dir() || m.file_type().is_symlink())
            {
                return Err("service directories must not traverse symlinks".into());
            }
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(p)
            .map_err(err)?;
        if fs::metadata(p).map_err(err)?.uid() != uid {
            return Err("service directory belongs to another user".into());
        }
        if private {
            fs::set_permissions(p, fs::Permissions::from_mode(0o700)).map_err(err)?;
        }
        Ok(())
    }
    fn registration(p: &Paths, enabled: bool) -> Result<()> {
        let content = plist(p, enabled)?;
        real_dir(&p.state, p.uid, true)?;
        let log = p.state.join("engine.log");
        if let Ok(m) = fs::symlink_metadata(&log)
            && (!m.is_file() || m.uid() != p.uid || m.nlink() != 1)
        {
            return Err("engine log must be a private regular file".into());
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(log)
            .map_err(err)?;
        log.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(err)?;
        real_dir(p.registration.parent().unwrap(), p.uid, false)?;
        if let Ok(m) = fs::symlink_metadata(&p.registration)
            && (!m.is_file() || m.uid() != p.uid)
        {
            return Err("registration must be an owned regular file".into());
        }
        let tmp = p
            .registration
            .with_extension(format!("plist.{}.tmp", std::process::id()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(err)?;
        let result = (|| {
            file.write_all(content.as_bytes()).map_err(err)?;
            file.sync_all().map_err(err)?;
            fs::rename(&tmp, &p.registration).map_err(err)
        })();
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result
    }
    fn action(verb: &str, p: &Paths, r: &mut impl Runner, custom: bool) -> Result<Value> {
        if verb != "status" && custom {
            return Err(
                "service control manages default state only; unset WOOTING_STATE_DIR first".into(),
            );
        }
        let s = status(p, r)?;
        match verb {
            "status" => return Ok(response(s)),
            "enable" => {
                if !p.binary.is_file() {
                    return Err("sibling wooting-signals executable is missing".into());
                }
                // Never bootout/kickstart here, even if an old engine is running.
                registration(p, true)?;
            }
            "start" => {
                if s.stale {
                    return Err(
                        "stale app path: stop and explicitly re-enable login startup first".into(),
                    );
                }
                if s.running {
                    return Ok(response(s));
                }
                if !p.binary.is_file() {
                    return Err("sibling wooting-signals executable is missing".into());
                }
                if !s.loaded {
                    registration(p, s.enabled)?;
                    checked(manager(
                        r,
                        &["bootstrap", &p.domain, path(&p.registration)?],
                    )?)?;
                }
                checked(manager(r, &["kickstart", &p.target()])?)?;
            }
            "stop" | "disable" | "remove" | "prepare-update" => {
                if s.loaded {
                    checked(manager(r, &["bootout", &p.target()])?)?;
                }
                if verb == "disable" || verb == "remove" {
                    match fs::remove_file(&p.registration) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(err(e)),
                    }
                }
                if verb == "remove" || verb == "prepare-update" {
                    remove_engine(p, r)?;
                }
            }
            _ => return Err("unknown service action".into()),
        }
        Ok(response(status(p, r)?))
    }
    fn remove_engine(p: &Paths, r: &mut impl Runner) -> Result<()> {
        // Probe only the default socket. Absence/refusal means no standalone engine;
        // permission and protocol failures are real errors, never successful removal.
        let socket = p.state.join("control.sock");
        if !socket.try_exists().map_err(err)? {
            return Ok(());
        }
        match UnixStream::connect(socket) {
            Ok(stream) => drop(stream),
            Err(e)
                if [
                    std::io::ErrorKind::NotFound,
                    std::io::ErrorKind::ConnectionRefused,
                ]
                .contains(&e.kind()) =>
            {
                return Ok(());
            }
            Err(e) => return Err(err(e)),
        }
        let out = checked(r.run(
            &p.binary,
            &["control", "--state-dir", path(&p.state)?, "stop"],
            Some(&p.state),
        )?)?;
        let value: Value = serde_json::from_str(out.trim()).map_err(err)?;
        if value["ok"] != true {
            return Err(format!("engine rejected stop: {out}"));
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    pub fn dispatch(verb: &str) -> Result<Value> {
        let mut runner = Native::new();
        let uid = checked(runner.run(Path::new("/usr/bin/id"), &["-u"], None)?)?
            .trim()
            .parse()
            .map_err(err)?;
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?);
        let binary = std::env::current_exe()
            .map_err(err)?
            .with_file_name("wooting-signals");
        let p = Paths::new(&home, binary, uid)?;
        action(
            verb,
            &p,
            &mut runner,
            std::env::var_os("WOOTING_STATE_DIR").is_some(),
        )
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            os::unix::net::UnixListener,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        struct Fixture {
            root: PathBuf,
            p: Paths,
        }
        impl Fixture {
            fn new() -> Self {
                let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
                    "ws-svc-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                fs::create_dir(&root).unwrap();
                let uid = fs::metadata(&root).unwrap().uid();
                let binary = root.join("Wooting & Signals.app/Contents/MacOS/wooting-signals");
                fs::create_dir_all(binary.parent().unwrap()).unwrap();
                fs::write(&binary, b"fake").unwrap();
                let p = Paths::new(&root, binary, uid).unwrap();
                Self { root, p }
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.root).unwrap();
            }
        }
        struct Fake {
            loaded: bool,
            running: bool,
            old_program: bool,
            calls: Vec<Vec<String>>,
            fail: Option<String>,
            stop_ok: bool,
            expected_state: PathBuf,
            binary: PathBuf,
        }
        impl Fake {
            fn new(p: &Paths) -> Self {
                Self {
                    loaded: false,
                    running: false,
                    old_program: false,
                    calls: Vec::new(),
                    fail: None,
                    stop_ok: true,
                    expected_state: p.state.clone(),
                    binary: p.binary.clone(),
                }
            }
            fn mutations(&self) -> Vec<&str> {
                self.calls
                    .iter()
                    .map(|a| a[0].as_str())
                    .filter(|s| !["print", "print-disabled", "-convert"].contains(s))
                    .collect()
            }
        }
        impl Runner for Fake {
            fn run(
                &mut self,
                program: &Path,
                args: &[&str],
                state: Option<&Path>,
            ) -> Result<Output> {
                self.calls
                    .push(args.iter().map(|s| s.to_string()).collect());
                if let Some(fail) = &self.fail {
                    return Err(fail.clone());
                }
                let text = match args[0] {
                    "-convert" => {
                        let content = fs::read_to_string(args[4]).unwrap();
                        json!({"Label":LABEL,"RunAtLoad":content.contains("<key>RunAtLoad</key><true/>"),"ProgramArguments":[if content.contains("OLD") {"OLD".to_string()} else {self.binary.to_str().unwrap().to_string()}],"EnvironmentVariables":{"WOOTING_STATE_DIR":self.expected_state}}).to_string()
                    }
                    "print-disabled" => String::new(),
                    "print" if !self.loaded => {
                        return Ok(Output {
                            ok: false,
                            text: "Could not find service".into(),
                        });
                    }
                    "print" => format!(
                        "state = {}\nprogram = {}",
                        if self.running { "running" } else { "waiting" },
                        if self.old_program {
                            "OLD"
                        } else {
                            self.binary.to_str().unwrap()
                        }
                    ),
                    "bootstrap" => {
                        self.loaded = true;
                        String::new()
                    }
                    "kickstart" => {
                        self.running = true;
                        String::new()
                    }
                    "bootout" => {
                        self.loaded = false;
                        self.running = false;
                        self.old_program = false;
                        String::new()
                    }
                    "control" => {
                        assert_eq!(program, self.binary);
                        assert_eq!(state, Some(self.expected_state.as_path()));
                        assert_eq!(
                            args,
                            [
                                "control",
                                "--state-dir",
                                self.expected_state.to_str().unwrap(),
                                "stop"
                            ]
                        );
                        return Ok(Output {
                            ok: self.stop_ok,
                            text: json!({"ok":self.stop_ok}).to_string(),
                        });
                    }
                    _ => panic!("unexpected fake command"),
                };
                Ok(Output { ok: true, text })
            }
        }
        #[test]
        fn enable_is_next_login_only_and_permissions_are_private() {
            let f = Fixture::new();
            let mut r = Fake::new(&f.p);
            let v = action("enable", &f.p, &mut r, false).unwrap();
            assert_eq!(v["enabled"], true);
            assert_eq!(v["running"], false);
            assert!(r.mutations().is_empty());
            let content = fs::read_to_string(&f.p.registration).unwrap();
            assert!(content.contains("Wooting &amp; Signals.app"));
            assert!(content.contains("<key>WOOTING_STATE_DIR</key>"));
            assert_eq!(
                fs::metadata(&f.p.state).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for p in [&f.p.registration, &f.p.state.join("engine.log")] {
                assert_eq!(fs::metadata(p).unwrap().permissions().mode() & 0o777, 0o600);
            }
        }
        #[test]
        fn start_is_explicit_and_idempotent_not_restart() {
            let f = Fixture::new();
            let mut r = Fake::new(&f.p);
            let v = action("start", &f.p, &mut r, false).unwrap();
            assert_eq!(v["enabled"], false);
            assert_eq!(v["running"], true);
            assert_eq!(r.mutations(), ["bootstrap", "kickstart"]);
            r.calls.clear();
            action("start", &f.p, &mut r, false).unwrap();
            assert!(r.mutations().is_empty());
        }
        #[test]
        fn relocation_requires_explicit_enable_and_does_not_restart_old_engine() {
            let f = Fixture::new();
            let mut r = Fake::new(&f.p);
            registration(&f.p, true).unwrap();
            let old = fs::read_to_string(&f.p.registration)
                .unwrap()
                .replace("Wooting &amp; Signals.app", "OLD");
            fs::write(&f.p.registration, old).unwrap();
            r.loaded = true;
            r.running = true;
            r.old_program = true;
            assert!(action("status", &f.p, &mut r, false).unwrap()["error"].is_string());
            assert!(action("start", &f.p, &mut r, false).is_err());
            r.calls.clear();
            action("enable", &f.p, &mut r, false).unwrap();
            assert!(r.mutations().is_empty());
            assert!(r.running);
            action("stop", &f.p, &mut r, false).unwrap();
            action("start", &f.p, &mut r, false).unwrap();
        }
        #[test]
        fn disable_stops_service_and_preserves_saved_state() {
            let f = Fixture::new();
            let mut r = Fake::new(&f.p);
            registration(&f.p, true).unwrap();
            fs::write(f.p.state.join("saved.json"), "settings").unwrap();
            r.loaded = true;
            r.running = true;
            let v = action("disable", &f.p, &mut r, false).unwrap();
            assert_eq!(v["enabled"], false);
            assert_eq!(v["running"], false);
            assert_eq!(r.mutations(), ["bootout"]);
            assert!(!f.p.registration.exists());
            assert_eq!(
                fs::read_to_string(f.p.state.join("saved.json")).unwrap(),
                "settings"
            );
        }
        #[test]
        fn remove_stops_reachable_standalone_and_prepare_update_keeps_opt_in() {
            for verb in ["remove", "prepare-update"] {
                let f = Fixture::new();
                let mut r = Fake::new(&f.p);
                registration(&f.p, true).unwrap();
                let mut p = Paths::new(&f.root, f.p.binary.clone(), f.p.uid).unwrap();
                p.state = f.root.join("s");
                fs::create_dir(&p.state).unwrap();
                r.expected_state = p.state.clone();
                let _listener = UnixListener::bind(p.state.join("control.sock")).unwrap();
                let v = action(verb, &p, &mut r, false).unwrap();
                assert_eq!(r.mutations(), ["control"]);
                assert_eq!(v["enabled"], verb == "prepare-update");
                assert_eq!(p.registration.exists(), verb == "prepare-update");
            }
        }
        #[test]
        fn remove_reports_stop_failure_and_absence_is_success() {
            let f = Fixture::new();
            let mut r = Fake::new(&f.p);
            assert!(action("remove", &f.p, &mut r, false).is_ok());
            assert!(r.mutations().is_empty());
            let mut p = Paths::new(&f.root, f.p.binary.clone(), f.p.uid).unwrap();
            p.state = f.root.join("s");
            fs::create_dir(&p.state).unwrap();
            r.expected_state = p.state.clone();
            let _listener = UnixListener::bind(p.state.join("control.sock")).unwrap();
            r.stop_ok = false;
            assert!(action("remove", &p, &mut r, false).is_err());
        }
        #[test]
        fn custom_state_mutations_and_manager_errors_fail_closed() {
            let f = Fixture::new();
            let mut r = Fake::new(&f.p);
            for verb in [
                "enable",
                "disable",
                "start",
                "stop",
                "remove",
                "prepare-update",
            ] {
                assert!(action(verb, &f.p, &mut r, true).is_err());
            }
            assert!(r.calls.is_empty());
            r.fail = Some("permission denied".into());
            assert!(
                action("status", &f.p, &mut r, false)
                    .unwrap_err()
                    .contains("permission denied")
            );
        }
        #[test]
        fn symlink_log_and_invalid_xml_are_rejected() {
            let f = Fixture::new();
            real_dir(&f.p.state, f.p.uid, true).unwrap();
            let elsewhere = f.root.join("elsewhere");
            fs::write(&elsewhere, "keep").unwrap();
            std::os::unix::fs::symlink(&elsewhere, f.p.state.join("engine.log")).unwrap();
            assert!(registration(&f.p, true).is_err());
            assert_eq!(fs::read_to_string(elsewhere).unwrap(), "keep");
            assert!(xml("bad\u{0}").is_err());
            assert_eq!(xml("<&>\"'").unwrap(), "&lt;&amp;&gt;&quot;&apos;");
        }

        #[test]
        #[cfg(target_os = "macos")]
        fn generated_plist_roundtrips_with_system_parser() {
            let f = Fixture::new();
            registration(&f.p, true).unwrap();
            let text = checked(
                Native::new()
                    .run(
                        Path::new("/usr/bin/plutil"),
                        &[
                            "-convert",
                            "json",
                            "-o",
                            "-",
                            path(&f.p.registration).unwrap(),
                        ],
                        None,
                    )
                    .unwrap(),
            )
            .unwrap();
            let v: Value = serde_json::from_str(text.trim()).unwrap();
            assert_eq!(v["ProgramArguments"][0], path(&f.p.binary).unwrap());
            assert_eq!(v["ProgramArguments"][3], path(&f.p.state).unwrap());
            assert_eq!(
                v["EnvironmentVariables"]["WOOTING_STATE_DIR"],
                path(&f.p.state).unwrap()
            );
            assert_eq!(v["RunAtLoad"], true);
            assert_eq!(v["Umask"], 63);
        }
        #[test]
        fn native_subprocess_deadline_and_output_cap() {
            let mut r = Native::new();
            let out = r
                .run(Path::new("/usr/bin/printf"), &["hello"], None)
                .unwrap();
            assert!(out.ok);
            assert_eq!(out.text.trim(), "hello");
            r.deadline = Instant::now();
            assert!(
                r.run(Path::new("/usr/bin/printf"), &["hello"], None)
                    .is_err()
            );
            let out = Native::new().run(Path::new("/usr/bin/yes"), &[], None);
            assert!(out.unwrap_err().contains("output exceeded"));
        }
    }
}
