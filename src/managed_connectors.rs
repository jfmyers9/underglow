//! Explicitly trusted, engine-owned producers. Never constructed by profile validation/rendering.
use crate::ingress::SharedSources;
use crate::ownership::Error;
use serde::Serialize;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
pub use underglow::connection_registry::{ConnectorConfig, validate_producers as validate};
use underglow::external::{ApplyOutcome, Limits, Message};

const MAX_FRAME: usize = 4096;
const MAX_FRAMES_PER_TICK: usize = 32;
const MAX_FRAMES_PER_SECOND: usize = 100;
const TICK: Duration = Duration::from_millis(10);

#[derive(Clone, Debug, Serialize)]
pub struct Health {
    id: String,
    source: String,
    enabled: bool,
    trusted: bool,
    state: &'static str,
    running: bool,
    /// Recent valid protocol traffic, NOT application success or lease freshness.
    healthy: bool,
    pid: Option<u32>,
    restarts: u32,
    retry_in_ms: Option<u64>,
    last_message_age_ms: Option<u64>,
    last_error: Option<String>,
}
impl Health {
    pub fn source(&self) -> &str {
        &self.source
    }
}

struct Desired {
    revision: u64,
    configs: Vec<ConnectorConfig>,
    stop: bool,
}

pub struct Supervisor {
    desired: Arc<Mutex<Desired>>,
    health: Arc<Mutex<Vec<Health>>>,
    done: mpsc::Receiver<()>,
    worker: Option<JoinHandle<()>>,
}

impl Supervisor {
    /// Only the real engine calls this; config parsing and previews are inert.
    pub fn start(configs: Vec<ConnectorConfig>, sources: SharedSources) -> Result<Self, Error> {
        validate(&configs)?;
        let desired = Arc::new(Mutex::new(Desired {
            revision: 1,
            configs,
            stop: false,
        }));
        let health = Arc::new(Mutex::new(Vec::new()));
        let (tx, done) = mpsc::sync_channel(1);
        let d = desired.clone();
        let h = health.clone();
        let worker = thread::Builder::new()
            .name("connectors".into())
            .spawn(move || {
                supervise(d, h, sources);
                let _ = tx.send(());
            })?;
        Ok(Self {
            desired,
            health,
            done,
            worker: Some(worker),
        })
    }

    /// One coalescing slot, never an unbounded work queue. Acknowledges intent;
    /// status remains asynchronous until old children have been reaped.
    pub fn replace(&self, configs: Vec<ConnectorConfig>) {
        let mut desired = self.desired.lock().expect("connector intent lock");
        if desired.configs != configs {
            desired.configs = configs;
            desired.revision = desired.revision.wrapping_add(1);
        }
    }

    pub fn status(&self) -> Vec<Health> {
        self.health.lock().expect("connector health lock").clone()
    }

    pub fn shutdown(&mut self) -> Result<(), Error> {
        if self.worker.is_none() {
            return Ok(());
        }
        self.desired.lock().expect("connector intent lock").stop = true;
        // Cleanup/reaping runs off the render/control thread. Kernel-stuck children
        // cannot be forcibly reaped; report that case rather than hanging shutdown.
        self.done.recv_timeout(Duration::from_secs(2)).map_err(
            |_| "managed connector shutdown timed out; children were signaled for termination",
        )?;
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| "connector supervisor panicked")?;
        }
        Ok(())
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        if let Err(e) = self.shutdown() {
            eprintln!("warning: {e}");
        }
    }
}

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn waitid(idtype: u32, id: u32, info: *mut std::ffi::c_void, options: i32) -> i32;
}

/// Peek without reaping: the owned leader's PID must remain reserved until its
/// group has been signaled, otherwise PID reuse could target an unrelated group.
fn exited_unreaped(pid: u32) -> io::Result<bool> {
    #[cfg(target_os = "macos")]
    const WNOWAIT: i32 = 0x20;
    #[cfg(not(target_os = "macos"))]
    const WNOWAIT: i32 = 0x01000000;
    // siginfo_t is <=128 bytes on supported macOS/Linux, aligned to <=8 bytes.
    // Only si_signo (the first C int) is read; no platform-dependent union layout.
    let mut info = [0u64; 32];
    // SAFETY: P_PID=1, WEXITED=4, WNOHANG=1; storage is writable and oversized.
    // WNOWAIT preserves the child for std::process::Child::try_wait to reap later.
    let result = unsafe { waitid(1, pid, info.as_mut_ptr().cast(), 4 | 1 | WNOWAIT) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the buffer is aligned and initialized; si_signo is the first int.
    Ok(unsafe { *info.as_ptr().cast::<i32>() } != 0)
}

struct Process {
    child: Child,
    stdout: Option<UnixStream>,
    killed: bool,
    group_signaled: bool,
    reaped: bool,
}
impl Process {
    fn spawn(config: &ConnectorConfig) -> io::Result<Self> {
        let (stdout, peer) = UnixStream::pair()?;
        stdout.set_nonblocking(true)?;
        // A socketpair is a bounded stdout transport without blocking reader threads
        // or platform-specific fcntl constants. The child endpoint stays blocking.
        let child = Command::new(&config.command[0])
            .args(&config.command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(peer)))
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        Ok(Self {
            child,
            stdout: Some(stdout),
            killed: false,
            group_signaled: false,
            reaped: false,
        })
    }

    fn terminate(&mut self) {
        if !self.killed {
            self.stdout = None;
            self.signal_group();
            self.killed = true;
        }
    }
    fn signal_group(&mut self) {
        if !self.group_signaled {
            // SAFETY: process_group(0) created an owned group whose ID is this
            // child's positive PID. Never signal zero, the engine group, or a name.
            unsafe {
                kill(-(self.child.id() as i32), 9);
            }
            self.group_signaled = true;
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.terminate();
        // Normal paths already reaped with try_wait. This is a panic safety net,
        // off the render thread; never abandon an owned direct child as a zombie.
        let _ = self.child.wait();
    }
}

struct Entry {
    config: ConnectorConfig,
    process: Option<Process>,
    retiring: bool,
    buffer: Vec<u8>,
    partial_since: Option<Instant>,
    window: Instant,
    frames: usize,
    bytes: usize,
    failures: u32,
    restarts: u32,
    attempted: bool,
    retry_at: Option<Instant>,
    started: Instant,
    last_message: Option<Instant>,
    last_error: Option<String>,
}

impl Entry {
    fn new(config: ConnectorConfig) -> Self {
        Self {
            config,
            process: None,
            retiring: false,
            buffer: Vec::with_capacity(MAX_FRAME),
            partial_since: None,
            window: Instant::now(),
            frames: 0,
            bytes: 0,
            failures: 0,
            restarts: 0,
            attempted: false,
            retry_at: None,
            started: Instant::now(),
            last_message: None,
            last_error: None,
        }
    }
    fn stop(&mut self) {
        self.retiring = true;
        if let Some(p) = &mut self.process {
            p.terminate();
        }
    }
    fn failed(&mut self, error: String) {
        if let Some(p) = &mut self.process {
            p.terminate();
        }
        self.buffer.clear();
        self.partial_since = None;
        self.last_message = None;
        self.last_error = Some(error);
        self.retry_at = if self.failures < self.config.restart_limit {
            let delay = self
                .config
                .restart_initial_ms
                .saturating_mul(1u64 << self.failures)
                .min(self.config.restart_max_ms);
            self.failures += 1;
            Some(Instant::now() + Duration::from_millis(delay))
        } else {
            None
        };
    }
    fn read(&mut self, sources: &SharedSources) -> Result<(), String> {
        let now = Instant::now();
        if self.window.elapsed() >= Duration::from_secs(1) {
            self.window = now;
            self.frames = 0;
            self.bytes = 0;
        }
        let Some(stdout) = self.process.as_mut().and_then(|p| p.stdout.as_mut()) else {
            return Ok(());
        };
        let mut bytes = [0u8; 4096];
        // Bound bytes as well as frame count. A producer cannot starve its siblings.
        if !self.buffer.contains(&b'\n') {
            match stdout.read(&mut bytes) {
                Ok(0) => {
                    return Err(
                        "stdout closed (producer exited or closed its protocol stream)".into(),
                    );
                }
                Ok(n) => {
                    self.bytes += n;
                    if self.bytes > 65536 {
                        return Err("stdout byte rate limit exceeded".into());
                    }
                    self.buffer.extend_from_slice(&bytes[..n]);
                    if self.partial_since.is_none() {
                        self.partial_since = Some(now);
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(format!("stdout read failed: {e}")),
            }
        }
        let mut consumed = 0;
        for _ in 0..MAX_FRAMES_PER_TICK {
            let Some(end) = self.buffer[consumed..].iter().position(|b| *b == b'\n') else {
                break;
            };
            if end > MAX_FRAME {
                return Err("stdout frame exceeds 4096 bytes".into());
            }
            self.frames += 1;
            if self.frames > MAX_FRAMES_PER_SECOND {
                return Err("stdout frame rate limit exceeded".into());
            }
            let message =
                Message::from_json(&self.buffer[consumed..consumed + end], &Limits::default())
                    .map_err(|e| e.to_string())?;
            if message.source != self.config.source {
                return Err("producer source does not match configured source".into());
            }
            let outcome = sources.apply(message).map_err(|e| e.to_string())?;
            if outcome == ApplyOutcome::Applied {
                self.last_message = Some(now);
                self.last_error = None;
            }
            consumed += end + 1;
        }
        if consumed > 0 {
            self.buffer.drain(..consumed);
            self.partial_since = (!self.buffer.is_empty()).then_some(now);
        }
        if self.buffer.len() > MAX_FRAME {
            return Err("stdout frame/buffer limit exceeded".into());
        }
        if self
            .partial_since
            .is_some_and(|t| t.elapsed() > Duration::from_secs(1))
        {
            return Err("stdout frame missing newline within 1 second".into());
        }
        Ok(())
    }
    fn tick(&mut self, sources: &SharedSources) {
        if self.process.as_ref().is_some_and(|p| !p.killed) {
            if let Err(e) = self.read(sources) {
                self.failed(e);
            }
            if self.started.elapsed() >= Duration::from_secs(30)
                && self
                    .last_message
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(30))
            {
                self.failures = 0;
            }
        }
        if let Some(p) = &mut self.process {
            let result = if p.reaped {
                Ok(())
            } else {
                (|| -> io::Result<()> {
                    if exited_unreaped(p.child.id())? {
                        // Signal before reaping, but retain stdout to drain final facts.
                        p.signal_group();
                        p.reaped = p.child.try_wait()?.is_some();
                    }
                    Ok(())
                })()
            };
            if let Err(e) = result
                && !p.killed
            {
                self.failed(format!("cannot inspect producer: {e}"));
            }
        }
        if self.process.as_ref().is_some_and(|p| p.reaped && p.killed) {
            self.process = None;
        }
        if self.retiring || !self.config.enabled || !self.config.trusted || self.process.is_some() {
            return;
        }
        if self.attempted && self.retry_at.is_none_or(|t| Instant::now() < t) {
            return;
        }
        if self.attempted {
            self.restarts = self.restarts.saturating_add(1);
        }
        self.attempted = true;
        self.retry_at = None;
        self.buffer.clear();
        self.partial_since = None;
        self.window = Instant::now();
        self.frames = 0;
        self.bytes = 0;
        match Process::spawn(&self.config) {
            Ok(p) => {
                self.process = Some(p);
                self.started = Instant::now();
            }
            Err(e) => self.failed(format!("cannot start producer: {e}")),
        }
    }
    fn health(&self) -> Health {
        let running = self
            .process
            .as_ref()
            .is_some_and(|p| !p.killed && !p.reaped);
        Health {
            id: self.config.id.clone(),
            source: self.config.source.clone(),
            enabled: self.config.enabled,
            trusted: self.config.trusted,
            state: if self.retiring {
                "stopping"
            } else if !self.config.enabled {
                "disabled"
            } else if running {
                "running"
            } else if self.retry_at.is_some() {
                "backoff"
            } else if self.attempted {
                "failed"
            } else {
                "starting"
            },
            running,
            healthy: running
                && self
                    .last_message
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(30)),
            pid: self.process.as_ref().map(|p| p.child.id()),
            restarts: self.restarts,
            retry_in_ms: self
                .retry_at
                .map(|t| t.saturating_duration_since(Instant::now()).as_millis() as u64),
            last_message_age_ms: self.last_message.map(|t| t.elapsed().as_millis() as u64),
            last_error: self.last_error.clone(),
        }
    }
}

fn supervise(
    desired: Arc<Mutex<Desired>>,
    health: Arc<Mutex<Vec<Health>>>,
    sources: SharedSources,
) {
    let mut revision = 0;
    let mut target = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    loop {
        let stopping;
        {
            let d = desired.lock().expect("connector intent lock");
            stopping = d.stop;
            if d.revision != revision || stopping {
                revision = d.revision;
                target = if stopping {
                    Vec::new()
                } else {
                    d.configs.clone()
                };
                for e in &mut entries {
                    if !target.contains(&e.config) {
                        e.stop();
                    }
                }
            }
        }
        for e in &mut entries {
            e.tick(&sources);
        }
        entries.retain(|e| !e.retiring || e.process.is_some());
        // Never overlap replacement producers/owned groups. Retiring entries remain
        // bounded by the previous profile; the latest target occupies one slot.
        if !entries.iter().any(|e| e.retiring) {
            for c in &target {
                if !entries.iter().any(|e| e.config.id == c.id) {
                    entries.push(Entry::new(c.clone()));
                }
            }
        }
        *health.lock().expect("connector health lock") =
            entries.iter().map(Entry::health).collect();
        if stopping && entries.is_empty() {
            break;
        }
        thread::sleep(TICK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ConnectorConfig {
        ConnectorConfig {
            id: "timer".into(),
            source: "focus".into(),
            command: vec!["/not-an-executable".into()],
            ..Default::default()
        }
    }

    #[test]
    fn configuration_is_strict_bounded_and_inert() {
        let c = config();
        validate(std::slice::from_ref(&c)).unwrap(); // no executable probing/spawning
        let mut bad = c.clone();
        bad.enabled = true;
        assert!(validate(&[bad]).unwrap_err().contains("trusted"));
        let mut other = c.clone();
        other.id = "other".into();
        assert!(validate(&[c.clone(), other]).is_err()); // duplicate source
        assert!(validate(&vec![c.clone(); 17]).is_err());
        for restart_initial_ms in [0, 249, 60001] {
            let mut bad = c.clone();
            bad.restart_initial_ms = restart_initial_ms;
            assert!(validate(&[bad]).is_err());
        }
        for command in [
            vec![],
            vec![String::new()],
            vec!["a\0b".into()],
            vec!["a".repeat(16385)],
            vec!["a".into(); 65],
        ] {
            let mut bad = c.clone();
            bad.command = command;
            assert!(validate(&[bad]).is_err());
        }
        assert!(
            toml::from_str::<ConnectorConfig>("id='x'\nsource='x'\ncommand=['x']\nshell=true\n")
                .is_err()
        );
        let defaults: ConnectorConfig =
            toml::from_str("id='x'\nsource='x'\ncommand=['x']\n").unwrap();
        assert!(!defaults.enabled && !defaults.trusted);
    }
}
