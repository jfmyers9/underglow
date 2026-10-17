//! Latest-value, read-only development preview. No subprocess or SDK access.
use eframe::egui;
use serde::Deserialize;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use underglow::layout::MatrixCoord;
use underglow::render::{Color, FRAME_BYTES, Frame, MAX_COLUMNS, MAX_ROWS};

const INTERVAL: Duration = Duration::from_millis(67);
const DEADLINE: Duration = Duration::from_millis(250);
const LIMIT: usize = 64 * 1024;

#[derive(Clone)]
pub struct Sample {
    frame: Frame,
    keys: Vec<MatrixCoord>,
    pub mode: String,
    pub brightness: u8,
    pub palette: String,
    pub speed: u32,
}
impl Sample {
    pub fn color(&self, coord: MatrixCoord) -> Option<Color> {
        self.keys
            .contains(&coord)
            .then(|| self.frame.get_coord(coord))
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }
}

#[derive(Deserialize)]
struct Preview {
    schema_version: u32,
    kind: String,
    layout: String,
    rgb: Vec<u8>,
    overlay_keys: Vec<[u8; 2]>,
}
#[derive(Deserialize)]
struct Reply {
    ok: bool,
    error: Option<String>,
    status: Option<super::Status>,
    preview: Option<Preview>,
}

pub(super) fn parse(bytes: &[u8]) -> Result<Sample, String> {
    let reply: Reply = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if !reply.ok {
        return Err(reply.error.unwrap_or_else(|| "Preview rejected".into()));
    }
    let preview = reply
        .preview
        .ok_or("Engine does not support live preview")?;
    let info = underglow::device::DeviceInfo::synthetic_80he();
    let layout = underglow::layout::KeyboardLayout::for_device(&info);
    if preview.schema_version != 1
        || preview.kind != "simulated-live-sources"
        || preview.layout != layout.name
        || preview.rgb.len() != FRAME_BYTES
        || preview.overlay_keys.len() > layout.keys().len()
    {
        return Err("Incompatible live preview frame".into());
    }
    let status = reply
        .status
        .filter(|s| s.schema_version == 1)
        .ok_or("Missing preview status")?;
    let mut frame = Frame::black();
    for row in 0..MAX_ROWS {
        for col in 0..MAX_COLUMNS {
            let i = (row * MAX_COLUMNS + col) * 3;
            frame.set(
                row,
                col,
                Color::new(preview.rgb[i], preview.rgb[i + 1], preview.rgb[i + 2]),
            );
        }
    }
    let mut keys = Vec::new();
    for [row, column] in preview.overlay_keys {
        let coord = MatrixCoord { row, column };
        if !layout.keys().iter().any(|key| key.coord == coord) || keys.contains(&coord) {
            return Err("Invalid preview overlay key".into());
        }
        keys.push(coord);
    }
    Ok(Sample {
        frame,
        keys,
        mode: status.mode,
        brightness: status.brightness,
        palette: status.palette,
        speed: status.speed.unwrap_or(100),
    })
}

fn request(path: &Path) -> Result<Sample, String> {
    let mut stream =
        UnixStream::connect(path).map_err(|e| format!("Waiting for development engine: {e}"))?;
    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
    roundtrip(&mut stream).and_then(|bytes| parse(&bytes))
}

fn roundtrip(stream: &mut UnixStream) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + DEADLINE;
    let request = b"{\"schema_version\":1,\"action\":\"preview\"}\n";
    let mut sent = 0;
    let mut bytes = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err("Live preview timed out".into());
        }
        if sent < request.len() {
            match stream.write(&request[sent..]) {
                Ok(0) => return Err("Preview connection closed".into()),
                Ok(n) => sent += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.to_string()),
            }
        } else {
            let mut chunk = [0; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => return Err("Preview connection closed".into()),
                Ok(n) => {
                    bytes.extend_from_slice(&chunk[..n]);
                    if bytes.len() > LIMIT {
                        return Err("Live preview response too large".into());
                    }
                    if let Some(end) = bytes.iter().position(|b| *b == b'\n') {
                        bytes.truncate(end);
                        return Ok(bytes);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
}

struct Latest {
    at: Instant,
    result: Result<Sample, String>,
}
pub struct Worker {
    latest: Arc<Mutex<Option<Latest>>>,
    stop: Arc<AtomicBool>,
}
impl Worker {
    #[cfg(test)]
    pub(super) fn fixture(sample: Sample) -> Self {
        Self {
            latest: Arc::new(Mutex::new(Some(Latest {
                at: Instant::now(),
                result: Ok(sample),
            }))),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(directory: PathBuf, context: egui::Context) -> Self {
        Self::spawn(
            move || request(&directory.join("control.sock")),
            move || context.request_repaint(),
        )
    }

    fn spawn(
        mut fetch: impl FnMut() -> Result<Sample, String> + Send + 'static,
        repaint: impl Fn() + Send + 'static,
    ) -> Self {
        let latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = Self {
            latest: latest.clone(),
            stop: stop.clone(),
        };
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let started = Instant::now();
                let result = fetch();
                if let Ok(mut slot) = latest.lock() {
                    *slot = Some(Latest {
                        at: Instant::now(),
                        result,
                    });
                }
                repaint();
                thread::sleep(INTERVAL.saturating_sub(started.elapsed()));
            }
        });
        worker
    }

    /// Never waits for socket I/O or even the worker lock; stale alerts disappear.
    pub fn latest(&self) -> Result<Sample, String> {
        let slot = self
            .latest
            .try_lock()
            .map_err(|_| "Waiting for live sources")?;
        let latest = slot
            .as_ref()
            .filter(|v| v.at.elapsed() < Duration::from_millis(600))
            .ok_or("Waiting for live sources")?;
        latest.result.clone()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn reply() -> serde_json::Value {
        let info = underglow::device::DeviceInfo::synthetic_80he();
        let layout = underglow::layout::KeyboardLayout::for_device(&info);
        serde_json::json!({"ok":true,"status":{
            "schema_version":1,"state":"paused","enabled":false,"mode":"matrix",
            "brightness":180,"palette":"terminal","fps":30,"speed":100,
            "last_error":null,"retry_attempt":0},"preview":{
                "schema_version":1,"kind":"simulated-live-sources","layout":layout.name,
                "rgb":vec![120;FRAME_BYTES],"overlay_keys":[[0,1]]}})
    }

    #[test]
    fn versioned_frames_validate_size_layout_and_overlay_coordinates() {
        let mut value = reply();
        let sample = parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            sample.color(MatrixCoord { row: 0, column: 1 }),
            Some(Color::new(120, 120, 120))
        );
        assert_eq!(sample.color(MatrixCoord { row: 1, column: 1 }), None);
        value["preview"]["schema_version"] = 2.into();
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value = reply();
        value["preview"]["rgb"] = serde_json::json!([1, 2, 3]);
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value = reply();
        value["preview"]["overlay_keys"] = serde_json::json!([[99, 99]]);
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value = reply();
        value["preview"]["overlay_keys"] = serde_json::json!([[0, 1], [0, 1]]);
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[test]
    fn slow_fetch_and_contended_slot_never_block_the_gui() {
        let (entered, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let worker = Worker::spawn(
            move || {
                entered.send(()).unwrap();
                let _ = wait.recv();
                Err("disconnected".into())
            },
            || {},
        );
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        let start = Instant::now();
        assert!(worker.latest().is_err());
        {
            let _guard = worker.latest.lock().unwrap();
            assert!(worker.latest().is_err());
        }
        assert!(start.elapsed() < Duration::from_millis(100));
        drop(worker);
        release.send(()).unwrap();
    }

    #[test]
    fn stale_and_failed_samples_do_not_leave_frozen_alerts() {
        let sample = parse(&serde_json::to_vec(&reply()).unwrap()).unwrap();
        let worker = Worker {
            latest: Arc::new(Mutex::new(Some(Latest {
                at: Instant::now(),
                result: Ok(sample),
            }))),
            stop: Arc::new(AtomicBool::new(false)),
        };
        assert!(worker.latest().is_ok());
        worker.latest.lock().unwrap().as_mut().unwrap().at -= Duration::from_secs(1);
        assert!(worker.latest().is_err());
        *worker.latest.lock().unwrap() = Some(Latest {
            at: Instant::now(),
            result: Err("offline".into()),
        });
        assert!(worker.latest().is_err());
    }

    #[test]
    fn socket_reads_have_a_total_deadline_and_size_bound() {
        let (mut client, _server) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        let start = Instant::now();
        assert!(roundtrip(&mut client).unwrap_err().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        let writer = thread::spawn(move || {
            let _ = server.write_all(&vec![b'x'; LIMIT + 1]);
        });
        assert!(roundtrip(&mut client).unwrap_err().contains("too large"));
        drop(client);
        writer.join().unwrap();
    }
}
