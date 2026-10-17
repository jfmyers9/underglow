//! Hardware-independent, bounded local fact publishing. This endpoint cannot control lighting.
use crate::ownership::{self, Error};
use clap::{Args, Subcommand};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use underglow::external::{self, EventBatch, Limits, Message, Payload, Registry, Snapshot};

const MAX_FRAME: usize = 4096;
const MAX_RESPONSE: usize = 256 * 1024;
const MAX_CLIENTS: usize = 32;
const DEADLINE: Duration = Duration::from_secs(1);
pub(crate) const SOCKET_NAME: &str = "events.sock";

struct SourceState {
    origin: Instant,
    registry: Registry,
}

/// All producers and readers share a clock; sample elapsed time only inside the lock.
#[derive(Clone)]
pub(crate) struct SharedSources(Arc<Mutex<SourceState>>);

pub(crate) struct Observation {
    pub now: Duration,
    pub snapshot: Snapshot,
    pub events: EventBatch,
}

impl Default for SharedSources {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(SourceState {
            origin: Instant::now(),
            registry: Registry::default(),
        })))
    }
}

impl SharedSources {
    /// No I/O or hardware access. Consumers keep independent event cursors.
    pub(crate) fn observe(&self, cursor: u64) -> Result<Observation, Error> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "external registry lock poisoned")?;
        let now = state.origin.elapsed();
        state.registry.expire(now)?;
        Ok(Observation {
            now,
            snapshot: state.registry.snapshot(),
            events: state.registry.events_since(cursor),
        })
    }

    pub(crate) fn expire(&self) -> Result<(), Error> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "external registry lock poisoned")?;
        let now = state.origin.elapsed();
        state.registry.expire(now)?;
        Ok(())
    }

    pub(crate) fn apply(&self, message: Message) -> Result<external::ApplyOutcome, Error> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "external registry lock poisoned")?;
        let now = state.origin.elapsed();
        Ok(state.registry.apply(message, now)?)
    }
}

#[derive(Debug, Args)]
pub struct ExternalOptions {
    #[arg(long)]
    state_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: ExternalCommand,
}

#[derive(Debug, Args)]
struct IdentityArgs {
    #[arg(long)]
    source: String,
    #[arg(long)]
    instance: String,
    /// Strictly increasing across state, event and remove for this instance.
    #[arg(long)]
    sequence: u64,
}

#[derive(Debug, Subcommand)]
enum ExternalCommand {
    /// Report or renew a leased state; does not resume paused lighting.
    State {
        #[command(flatten)]
        identity: IdentityArgs,
        #[arg(long)]
        state: String,
        #[arg(long, default_value_t = 15000)]
        ttl_ms: u64,
    },
    /// Report a transient occurrence; does not renew an instance's state.
    Event {
        #[command(flatten)]
        identity: IdentityArgs,
        #[arg(long)]
        event: String,
        #[arg(long, default_value_t = 15000)]
        ttl_ms: u64,
    },
    Remove {
        #[command(flatten)]
        identity: IdentityArgs,
    },
    /// Publish one raw versioned JSON Message from stdin (maximum 4096 bytes).
    Publish,
    /// Inspect current ephemeral state and unexpired events without touching hardware.
    Inspect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectRequest {
    schema_version: u32,
    kind: InspectKind,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum InspectKind {
    Inspect,
}

fn rejection(code: &str, message: impl std::fmt::Display) -> Value {
    json!({"schema_version":1,"ok":false,"error":{"code":code,"message":message.to_string()}})
}

fn external_rejection(error: &external::Error) -> Value {
    let code = match error {
        external::Error::MessageTooLarge => "message_too_large",
        external::Error::InvalidJson(_) => "invalid_json",
        external::Error::UnsupportedVersion(_) => "unsupported_version",
        external::Error::InvalidIdentifier(_) => "invalid_identifier",
        external::Error::InvalidLabel => "invalid_label",
        external::Error::InvalidTtl => "invalid_ttl",
        external::Error::Capacity(_) => "capacity",
        _ => "internal",
    };
    rejection(code, error)
}

fn handle(bytes: &[u8], sources: &SharedSources) -> Value {
    if let Ok(InspectRequest {
        schema_version,
        kind: InspectKind::Inspect,
    }) = serde_json::from_slice(bytes)
    {
        if schema_version != external::SCHEMA_VERSION {
            return external_rejection(&external::Error::UnsupportedVersion(schema_version));
        }
        return match sources.observe(0) {
            Ok(observation) => json!({"schema_version":1,"ok":true,"now":observation.now,
                "snapshot":observation.snapshot,"events":observation.events}),
            Err(error) => rejection("internal", error),
        };
    }
    let message = match Message::from_json(bytes, &Limits::default()) {
        Ok(message) => message,
        Err(error) => return external_rejection(&error),
    };
    match sources.apply(message) {
        Ok(outcome) => json!({"schema_version":1,"ok":true,"outcome":outcome}),
        Err(error) => match error.downcast_ref::<external::Error>() {
            Some(error) => external_rejection(error),
            None => rejection("internal", error),
        },
    }
}

struct Client {
    stream: UnixStream,
    input: Vec<u8>,
    output: Option<Vec<u8>>,
    written: usize,
    deadline: Instant,
}

impl Client {
    fn respond(&mut self, value: Value) {
        let mut bytes = serde_json::to_vec(&value).expect("JSON Value serializes");
        if bytes.len() >= MAX_RESPONSE {
            bytes = serde_json::to_vec(&rejection("response_too_large", "response exceeds limit"))
                .expect("JSON Value serializes");
        }
        bytes.push(b'\n');
        self.output = Some(bytes);
        self.deadline = Instant::now() + DEADLINE;
    }

    /// One bounded read and one bounded write per tick; no blocking socket operations.
    fn poll(&mut self, sources: &SharedSources) -> bool {
        if Instant::now() >= self.deadline {
            if self.output.is_some() {
                return false;
            }
            self.respond(rejection(
                "timeout",
                "request must finish within one second",
            ));
        }
        if self.output.is_none() {
            // One extra byte distinguishes a max-size frame from an oversized one.
            let mut chunk = [0; MAX_FRAME + 1];
            let remaining = MAX_FRAME + 1 - self.input.len();
            match self.stream.read(&mut chunk[..remaining]) {
                Ok(0) => self.respond(rejection("invalid_frame", "request requires a newline")),
                Ok(count) => {
                    self.input.extend_from_slice(&chunk[..count]);
                    if let Some(end) = self.input.iter().position(|byte| *byte == b'\n') {
                        if end != self.input.len() - 1 {
                            self.respond(rejection(
                                "invalid_frame",
                                "one JSON message per connection",
                            ));
                        } else {
                            let response = handle(&self.input[..end], sources);
                            self.respond(response);
                        }
                    } else if self.input.len() > MAX_FRAME {
                        self.respond(rejection("message_too_large", "request exceeds 4096 bytes"));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
        if let Some(output) = &self.output {
            match self.stream.write(&output[self.written..]) {
                Ok(0) => return false,
                Ok(count) => self.written += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
            return self.written < output.len();
        }
        true
    }
}

pub(crate) struct Server {
    listener: UnixListener,
    path: PathBuf,
    file_id: (u64, u64),
    clients: Vec<Client>,
    sources: SharedSources,
}

impl Server {
    /// Caller holds the engine lease, so only demonstrably stale sockets may be replaced.
    pub(crate) fn bind(dir: &Path, sources: SharedSources) -> Result<Self, Error> {
        ownership::private_dir(dir)?;
        let path = dir.join(SOCKET_NAME);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.file_type().is_socket() {
                    return Err("events.sock exists and is not a socket".into());
                }
                match UnixStream::connect(&path) {
                    Ok(_) => {
                        return Err("another publisher endpoint is listening on events.sock".into());
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                        ) => {}
                    Err(error) => return Err(error.into()),
                }
                fs::remove_file(&path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let listener = UnixListener::bind(&path).map_err(|error| {
            format!(
                "cannot bind {} (use a shorter --state-dir if needed): {error}",
                path.display()
            )
        })?;
        let metadata = fs::symlink_metadata(&path)?;
        let server = Self {
            listener,
            path,
            file_id: (metadata.dev(), metadata.ino()),
            clients: Vec::new(),
            sources,
        };
        fs::set_permissions(&server.path, fs::Permissions::from_mode(0o600))?;
        server.listener.set_nonblocking(true)?;
        Ok(server)
    }

    pub(crate) fn poll(&mut self) -> Result<(), Error> {
        for _ in 0..8 {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(true)?;
                    if self.clients.len() >= MAX_CLIENTS {
                        // Best effort only: overload must never wait for a client to read.
                        let mut bytes = serde_json::to_vec(&rejection(
                            "busy",
                            "publisher client capacity reached",
                        ))?;
                        bytes.push(b'\n');
                        let _ = stream.write(&bytes);
                    } else {
                        self.clients.push(Client {
                            stream,
                            input: Vec::new(),
                            output: None,
                            written: 0,
                            deadline: Instant::now() + DEADLINE,
                        });
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => break,
                Err(error) => return Err(error.into()),
            }
        }
        self.clients.retain_mut(|client| client.poll(&self.sources));
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Do not unlink a replacement endpoint if the pathname changed under us.
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|meta| (meta.dev(), meta.ino()) == self.file_id)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn request_bytes(command: ExternalCommand) -> Result<Vec<u8>, Value> {
    let (identity, payload) = match command {
        ExternalCommand::Inspect => return Ok(br#"{"schema_version":1,"kind":"inspect"}"#.to_vec()),
        ExternalCommand::Publish => {
            let mut bytes = Vec::new();
            std::io::stdin()
                .take(MAX_FRAME as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| rejection("stdin", error))?;
            let message = Message::from_json(&bytes, &Limits::default())
                .map_err(|error| external_rejection(&error))?;
            return serde_json::to_vec(&message).map_err(|error| rejection("invalid_json", error));
        }
        ExternalCommand::State {
            identity,
            state,
            ttl_ms,
        } => (identity, Payload::State { state, ttl_ms }),
        ExternalCommand::Event {
            identity,
            event,
            ttl_ms,
        } => (identity, Payload::Event { event, ttl_ms }),
        ExternalCommand::Remove { identity } => (identity, Payload::Remove),
    };
    let message = Message {
        schema_version: 1,
        source: identity.source,
        instance: identity.instance,
        sequence: identity.sequence,
        payload,
    };
    message
        .validate(&Limits::default())
        .map_err(|error| external_rejection(&error))?;
    serde_json::to_vec(&message).map_err(|error| rejection("invalid_json", error))
}

pub fn external(options: ExternalOptions) -> Result<(), Error> {
    let result = (|| -> Result<Value, Value> {
        // Validate input before any connection. Neither branch can acquire a hardware lease.
        let mut bytes = request_bytes(options.command)?;
        let dir = options
            .state_dir
            .map(Ok)
            .unwrap_or_else(ownership::state_dir)
            .map_err(|error| rejection("state_dir", error))?;
        ownership::private_dir(&dir).map_err(|error| rejection("state_dir", error))?;
        let mut stream = UnixStream::connect(dir.join(SOCKET_NAME)).map_err(|error| {
            rejection("unavailable", format!("start the engine first: {error}"))
        })?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| rejection("transport", error))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| rejection("transport", error))?;
        bytes.push(b'\n');
        stream
            .write_all(&bytes)
            .map_err(|error| rejection("transport", error))?;
        let mut response = Vec::new();
        BufReader::new(stream.take(MAX_RESPONSE as u64 + 1))
            .read_until(b'\n', &mut response)
            .map_err(|error| rejection("transport", error))?;
        if response.len() > MAX_RESPONSE || !response.ends_with(b"\n") {
            return Err(rejection(
                "invalid_response",
                "response exceeds limit or lacks newline",
            ));
        }
        serde_json::from_slice(&response).map_err(|error| rejection("invalid_response", error))
    })();
    let value = result.unwrap_or_else(|value| value);
    println!("{}", serde_json::to_string(&value)?);
    if value["ok"] == true {
        Ok(())
    } else {
        Err("external request failed; see JSON response".into())
    }
}
