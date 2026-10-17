//! Ephemeral, hardware-independent external facts. No persistence, I/O, or clock reads.
//!
//! A producer uses a unique instance ID per session and a strictly increasing sequence
//! across *all* message kinds. Renewing state means resending it with a higher sequence.
//! Events neither create nor renew state. An event means an occurrence, not success.
//! Call [`Registry::expire`] before reading if no messages have arrived recently.
//!
//! Version 1 wire example:
//! ```json
//! {"schema_version":1,"source":"pi","instance":"session-123","sequence":1,
//!  "payload":{"kind":"state","state":"working","ttl_ms":15000}}
//! ```

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub schema_version: u32,
    pub source: String,
    pub instance: String,
    pub sequence: u64,
    pub payload: Payload,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Payload {
    State { state: String, ttl_ms: u64 },
    Event { event: String, ttl_ms: u64 },
    Remove,
}

impl<'de> Deserialize<'de> for Payload {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Serde's internally tagged *unit* variants ignore extra fields, even
        // with deny_unknown_fields. An empty struct variant closes that hole.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum WirePayload {
            State { state: String, ttl_ms: u64 },
            Event { event: String, ttl_ms: u64 },
            Remove {},
        }
        Ok(match WirePayload::deserialize(deserializer)? {
            WirePayload::State { state, ttl_ms } => Self::State { state, ttl_ms },
            WirePayload::Event { event, ttl_ms } => Self::Event { event, ttl_ms },
            WirePayload::Remove {} => Self::Remove,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Identity {
    pub source: String,
    pub instance: String,
}

/// Bounds include inactive sequence tombstones, so identity churn cannot grow memory.
/// Full identity/state capacity rejects new entries; a full event queue drops oldest.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_message_bytes: usize,
    pub max_identifier_bytes: usize,
    pub max_label_bytes: usize,
    pub max_sources: usize,
    pub max_active_instances: usize,
    pub max_tracked_instances: usize,
    pub max_events: usize,
    pub max_ttl: Duration,
    /// Retain sequence high-water marks after removal/expiry. After this bounded
    /// window, old messages can be accepted; producers must not reuse session IDs.
    pub tombstone_ttl: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_message_bytes: 4096,
            max_identifier_bytes: 128,
            max_label_bytes: 128,
            max_sources: 32,
            max_active_instances: 128,
            max_tracked_instances: 256,
            max_events: 256,
            max_ttl: Duration::from_secs(300),
            tombstone_ttl: Duration::from_secs(300),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidLimits,
    MessageTooLarge,
    InvalidJson(String),
    UnsupportedVersion(u32),
    InvalidIdentifier(&'static str),
    InvalidLabel,
    InvalidTtl,
    Capacity(&'static str),
    ClockWentBackwards,
    ClockOverflow,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => f.write_str("invalid external source limits"),
            Self::MessageTooLarge => f.write_str("external message exceeds byte limit"),
            Self::InvalidJson(error) => write!(f, "invalid external JSON: {error}"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported schema version {version}"),
            Self::InvalidIdentifier(field) => write!(f, "invalid {field} identifier"),
            Self::InvalidLabel => f.write_str("invalid state/event label"),
            Self::InvalidTtl => f.write_str("TTL must be positive and within the configured limit"),
            Self::Capacity(kind) => write!(f, "external {kind} capacity reached"),
            Self::ClockWentBackwards => f.write_str("external registry clock went backwards"),
            Self::ClockOverflow => f.write_str("external registry deadline overflow"),
        }
    }
}

impl std::error::Error for Error {}

impl Message {
    /// Bound input before deserialization. Ingress must additionally bound its own
    /// framing buffer; calling this after an unbounded read is not sufficient.
    pub fn from_json(bytes: &[u8], limits: &Limits) -> Result<Self, Error> {
        if bytes.len() > limits.max_message_bytes {
            return Err(Error::MessageTooLarge);
        }
        let message: Self =
            serde_json::from_slice(bytes).map_err(|error| Error::InvalidJson(error.to_string()))?;
        message.validate(limits)?;
        Ok(message)
    }

    pub fn validate(&self, limits: &Limits) -> Result<(), Error> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(Error::UnsupportedVersion(self.schema_version));
        }
        for (field, value) in [("source", &self.source), ("instance", &self.instance)] {
            if !valid_token(value, limits.max_identifier_bytes) {
                return Err(Error::InvalidIdentifier(field));
            }
        }
        let label_ttl = match &self.payload {
            Payload::State { state, ttl_ms } => Some((state, ttl_ms)),
            Payload::Event { event, ttl_ms } => Some((event, ttl_ms)),
            Payload::Remove => None,
        };
        if let Some((label, ttl_ms)) = label_ttl {
            if !valid_token(label, limits.max_label_bytes) {
                return Err(Error::InvalidLabel);
            }
            if *ttl_ms == 0 || Duration::from_millis(*ttl_ms) > limits.max_ttl {
                return Err(Error::InvalidTtl);
            }
        }
        // Also constrain messages constructed directly in Rust, not only JSON ingress.
        if serde_json::to_vec(self)
            .map_err(|error| Error::InvalidJson(error.to_string()))?
            .len()
            > limits.max_message_bytes
        {
            return Err(Error::MessageTooLarge);
        }
        Ok(())
    }
}

fn valid_token(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyOutcome {
    Applied,
    /// Equal and lower sequences have no effect, including no lease renewal.
    IgnoredStale,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub identity: Identity,
    /// Sequence of the state update, not of a subsequent event or removal.
    pub sequence: u64,
    pub state: String,
    pub updated_at: Duration,
    pub expires_at: Duration,
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventSnapshot {
    pub cursor: u64,
    pub identity: Identity,
    pub sequence: u64,
    pub event: String,
    pub received_at: Duration,
    pub expires_at: Duration,
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub revision: u64,
    pub event_cursor: u64,
    /// Stable source/instance ordering; only currently leased states appear.
    pub states: Vec<StateSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventBatch {
    /// Save this cursor after processing, even when no events remain.
    pub cursor: u64,
    /// At least one event after the requested cursor has expired or been evicted.
    pub gap: bool,
    pub events: Vec<EventSnapshot>,
}

struct Entry {
    sequence: u64,
    state: Option<StateSnapshot>,
    forget_at: Duration,
}

/// Owned by the engine; consumers read snapshots and use independent event cursors.
/// Time is monotonic elapsed time from an arbitrary origin, injected by the caller.
/// A fresh registry is empty: none of this state is persisted across restarts.
pub struct Registry {
    limits: Limits,
    entries: BTreeMap<Identity, Entry>,
    events: VecDeque<EventSnapshot>,
    now: Duration,
    revision: u64,
    event_cursor: u64,
    dropped_through: u64,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new(Limits::default()).expect("default registry limits are valid")
    }
}

impl Registry {
    pub fn new(limits: Limits) -> Result<Self, Error> {
        if limits.max_message_bytes == 0
            || limits.max_identifier_bytes == 0
            || limits.max_label_bytes == 0
            || limits.max_sources == 0
            || limits.max_active_instances == 0
            || limits.max_tracked_instances < limits.max_active_instances
            || limits.max_events == 0
            || limits.max_ttl.is_zero()
            || limits.tombstone_ttl.is_zero()
        {
            return Err(Error::InvalidLimits);
        }
        Ok(Self {
            limits,
            entries: BTreeMap::new(),
            events: VecDeque::new(),
            now: Duration::ZERO,
            revision: 0,
            event_cursor: 0,
            dropped_through: 0,
        })
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn apply(&mut self, message: Message, now: Duration) -> Result<ApplyOutcome, Error> {
        message.validate(&self.limits)?;
        self.expire(now)?;
        let identity = Identity {
            source: message.source,
            instance: message.instance,
        };
        if self
            .entries
            .get(&identity)
            .is_some_and(|entry| message.sequence <= entry.sequence)
        {
            return Ok(ApplyOutcome::IgnoredStale);
        }
        if !self.entries.contains_key(&identity) {
            if self.entries.len() >= self.limits.max_tracked_instances {
                return Err(Error::Capacity("tracked instance"));
            }
            let sources: BTreeSet<_> = self.entries.keys().map(|id| &id.source).collect();
            if !sources.contains(&identity.source) && sources.len() >= self.limits.max_sources {
                return Err(Error::Capacity("source"));
            }
        }
        if matches!(message.payload, Payload::State { .. })
            && self
                .entries
                .get(&identity)
                .is_none_or(|entry| entry.state.is_none())
            && self
                .entries
                .values()
                .filter(|entry| entry.state.is_some())
                .count()
                >= self.limits.max_active_instances
        {
            return Err(Error::Capacity("active instance"));
        }
        let ttl = match &message.payload {
            Payload::State { ttl_ms, .. } | Payload::Event { ttl_ms, .. } => {
                Duration::from_millis(*ttl_ms)
            }
            Payload::Remove => Duration::ZERO,
        };
        let expires_at = now.checked_add(ttl).ok_or(Error::ClockOverflow)?;
        let forget_at = expires_at
            .checked_add(self.limits.tombstone_ttl)
            .ok_or(Error::ClockOverflow)?;
        self.revision = self.revision.saturating_add(1);
        let entry = self.entries.entry(identity.clone()).or_insert(Entry {
            sequence: message.sequence,
            state: None,
            forget_at,
        });
        entry.sequence = message.sequence;
        // An event must not shorten an existing state's sequence retention.
        entry.forget_at = entry.forget_at.max(forget_at);
        match message.payload {
            Payload::State { state, .. } => {
                entry.state = Some(StateSnapshot {
                    identity,
                    sequence: message.sequence,
                    state,
                    updated_at: now,
                    expires_at,
                    revision: self.revision,
                });
            }
            Payload::Event { event, .. } => {
                self.event_cursor = self.event_cursor.saturating_add(1);
                if self.events.len() == self.limits.max_events {
                    let removed = self.events.pop_front().expect("full nonempty queue");
                    self.dropped_through = self.dropped_through.max(removed.cursor);
                }
                self.events.push_back(EventSnapshot {
                    cursor: self.event_cursor,
                    identity,
                    sequence: message.sequence,
                    event,
                    received_at: now,
                    expires_at,
                    revision: self.revision,
                });
            }
            Payload::Remove => entry.state = None,
        }
        Ok(ApplyOutcome::Applied)
    }

    /// Expire at the deadline (not one tick later). Returns whether visible state
    /// or event availability changed. Tombstone cleanup alone is not a revision.
    pub fn expire(&mut self, now: Duration) -> Result<bool, Error> {
        if now < self.now {
            return Err(Error::ClockWentBackwards);
        }
        self.now = now;
        let mut changed = false;
        for entry in self.entries.values_mut() {
            if entry
                .state
                .as_ref()
                .is_some_and(|state| state.expires_at <= now)
            {
                entry.state = None;
                changed = true;
            }
        }
        self.entries.retain(|_, entry| entry.forget_at > now);
        // TTLs may differ, so expired events are not necessarily at the front.
        self.events.retain(|event| {
            if event.expires_at <= now {
                self.dropped_through = self.dropped_through.max(event.cursor);
                changed = true;
                false
            } else {
                true
            }
        });
        if changed {
            self.revision = self.revision.saturating_add(1);
        }
        Ok(changed)
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            revision: self.revision,
            event_cursor: self.event_cursor,
            states: self
                .entries
                .values()
                .filter_map(|entry| entry.state.clone())
                .collect(),
        }
    }

    /// Non-draining broadcast observation. New consumers can start at the current
    /// snapshot's event_cursor to avoid replaying earlier notifications.
    pub fn events_since(&self, cursor: u64) -> EventBatch {
        EventBatch {
            cursor: self.event_cursor,
            gap: cursor < self.dropped_through,
            events: self
                .events
                .iter()
                .filter(|event| event.cursor > cursor)
                .cloned()
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn message(instance: &str, sequence: u64, payload: Payload) -> Message {
        Message {
            schema_version: SCHEMA_VERSION,
            source: "pi".into(),
            instance: instance.into(),
            sequence,
            payload,
        }
    }

    fn state(instance: &str, sequence: u64, label: &str, ttl_ms: u64) -> Message {
        message(
            instance,
            sequence,
            Payload::State {
                state: label.into(),
                ttl_ms,
            },
        )
    }

    fn event(instance: &str, sequence: u64, ttl_ms: u64) -> Message {
        message(
            instance,
            sequence,
            Payload::Event {
                event: "finished".into(),
                ttl_ms,
            },
        )
    }

    #[test]
    fn wire_round_trips_all_kinds_and_full_width_sequences() {
        for message in [
            state("session-a", 0, "working", 15000),
            event("session-a", u64::MAX, 2000),
            message("session-a", 42, Payload::Remove),
        ] {
            let bytes = serde_json::to_vec(&message).unwrap();
            assert_eq!(
                Message::from_json(&bytes, &Limits::default()).unwrap(),
                message
            );
        }
    }

    #[test]
    fn wire_rejects_unknown_fields_bad_kinds_and_noninteger_sequences() {
        for json in [
            r#"{"schema_version":1,"source":"pi","instance":"a","sequence":1,"extra":1,"payload":{"kind":"remove"}}"#,
            r#"{"schema_version":1,"source":"pi","instance":"a","sequence":1,"payload":{"kind":"remove","ttl_ms":1}}"#,
            r#"{"schema_version":1,"source":"pi","instance":"a","sequence":1,"payload":{"kind":"state","state":"idle","ttl_ms":1,"extra":1}}"#,
            r#"{"schema_version":1,"source":"pi","instance":"a","sequence":1,"payload":{"kind":"resume"}}"#,
            r#"{"schema_version":1,"source":"pi","instance":"a","sequence":-1,"payload":{"kind":"remove"}}"#,
            r#"{"schema_version":1,"source":"pi","instance":"a","sequence":1.5,"payload":{"kind":"remove"}}"#,
        ] {
            assert!(
                matches!(
                    Message::from_json(json.as_bytes(), &Limits::default()),
                    Err(Error::InvalidJson(_))
                ),
                "{json}"
            );
        }
    }

    #[test]
    fn validates_versions_identifiers_labels_ttl_and_size() {
        let limits = Limits::default();
        let mut input = state("a", 1, "working", 1);
        input.schema_version = 2;
        assert_eq!(input.validate(&limits), Err(Error::UnsupportedVersion(2)));
        input.schema_version = 1;
        for invalid in ["", "has spaces", "\n", "é"] {
            input.source = invalid.into();
            assert_eq!(
                input.validate(&limits),
                Err(Error::InvalidIdentifier("source"))
            );
        }
        input.source = "pi".into();
        input.instance = "a".repeat(limits.max_identifier_bytes + 1);
        assert_eq!(
            input.validate(&limits),
            Err(Error::InvalidIdentifier("instance"))
        );
        assert_eq!(
            state("a", 1, "", 1).validate(&limits),
            Err(Error::InvalidLabel)
        );
        for ttl in [0, 300001, u64::MAX] {
            assert_eq!(
                state("a", 1, "working", ttl).validate(&limits),
                Err(Error::InvalidTtl)
            );
        }
        assert!(state("a", 1, "working", 300000).validate(&limits).is_ok());
        let small = Limits {
            max_message_bytes: 8,
            ..limits
        };
        assert_eq!(
            state("a", 1, "working", 1).validate(&small),
            Err(Error::MessageTooLarge)
        );
        assert_eq!(
            Message::from_json(b"not json but too long", &small),
            Err(Error::MessageTooLarge)
        );
    }

    #[test]
    fn sessions_update_renew_and_expire_independently_at_exact_deadline() {
        let mut registry = Registry::default();
        registry
            .apply(state("b", 1, "working", 100), ms(0))
            .unwrap();
        registry.apply(state("a", 1, "idle", 200), ms(0)).unwrap();
        assert_eq!(registry.snapshot().states[0].identity.instance, "a");
        registry
            .apply(state("b", 2, "working", 100), ms(90))
            .unwrap();
        assert!(!registry.expire(ms(100)).unwrap());
        assert_eq!(registry.snapshot().states.len(), 2);
        assert!(registry.expire(ms(190)).unwrap());
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.states.len(), 1);
        assert_eq!(snapshot.states[0].identity.instance, "a");
        assert!(registry.expire(ms(200)).unwrap());
        assert!(registry.snapshot().states.is_empty());
        assert!(!registry.expire(ms(201)).unwrap());
        assert_eq!(registry.snapshot().revision, 5);
    }

    #[test]
    fn duplicates_and_out_of_order_updates_do_not_renew_or_emit() {
        let mut registry = Registry::default();
        registry
            .apply(state("a", 5, "working", 100), ms(0))
            .unwrap();
        for sequence in [5, 4, 0] {
            assert_eq!(
                registry
                    .apply(state("a", sequence, "idle", 100), ms(50))
                    .unwrap(),
                ApplyOutcome::IgnoredStale
            );
        }
        assert_eq!(registry.snapshot().revision, 1);
        registry.expire(ms(100)).unwrap();
        assert!(registry.snapshot().states.is_empty());
        assert_eq!(
            registry.apply(event("a", 5, 100), ms(100)).unwrap(),
            ApplyOutcome::IgnoredStale
        );
        assert!(registry.events_since(0).events.is_empty());
        assert_eq!(
            registry.apply(state("a", 6, "idle", 100), ms(100)).unwrap(),
            ApplyOutcome::Applied
        );
    }

    #[test]
    fn events_do_not_create_or_renew_state_and_are_broadcast_not_drained() {
        let mut registry = Registry::default();
        registry
            .apply(state("a", 1, "working", 100), ms(0))
            .unwrap();
        registry.apply(event("a", 2, 200), ms(90)).unwrap();
        registry.apply(event("b", 1, 200), ms(90)).unwrap();
        assert_eq!(registry.snapshot().states.len(), 1);
        registry.expire(ms(100)).unwrap();
        assert!(registry.snapshot().states.is_empty());
        let first = registry.events_since(0);
        assert_eq!(first.events.len(), 2);
        assert_eq!(registry.events_since(0), first);
        assert!(registry.events_since(first.cursor).events.is_empty());
        let before = registry.snapshot();
        assert_eq!(
            registry.apply(event("a", 2, 200), ms(100)).unwrap(),
            ApplyOutcome::IgnoredStale
        );
        assert_eq!(registry.snapshot(), before);
    }

    #[test]
    fn remove_and_expiry_keep_bounded_sequence_tombstones() {
        let limits = Limits {
            tombstone_ttl: ms(10),
            ..Limits::default()
        };
        let mut registry = Registry::new(limits).unwrap();
        registry.apply(state("a", 9, "working", 5), ms(0)).unwrap();
        registry
            .apply(message("a", 10, Payload::Remove), ms(1))
            .unwrap();
        assert!(registry.snapshot().states.is_empty());
        assert_eq!(
            registry.apply(state("a", 9, "working", 5), ms(14)).unwrap(),
            ApplyOutcome::IgnoredStale
        );
        // Old state deadline + retention is the conservative retention boundary.
        registry.expire(ms(15)).unwrap();
        assert_eq!(registry.entries.len(), 0);
        assert_eq!(
            registry.apply(state("a", 0, "idle", 5), ms(15)).unwrap(),
            ApplyOutcome::Applied
        );
        registry.expire(ms(20)).unwrap();
        assert_eq!(
            registry.apply(state("a", 0, "idle", 5), ms(20)).unwrap(),
            ApplyOutcome::IgnoredStale
        );
    }

    #[test]
    fn event_queue_eviction_and_variable_ttl_expiry_report_gaps() {
        let mut registry = Registry::new(Limits {
            max_events: 2,
            ..Limits::default()
        })
        .unwrap();
        registry.apply(event("a", 1, 100), ms(0)).unwrap();
        registry.apply(event("a", 2, 10), ms(0)).unwrap();
        registry.expire(ms(10)).unwrap();
        let batch = registry.events_since(0);
        assert!(batch.gap);
        assert_eq!(
            batch
                .events
                .iter()
                .map(|event| event.cursor)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert!(!registry.events_since(2).gap);
        registry.apply(event("a", 3, 100), ms(10)).unwrap();
        registry.apply(event("a", 4, 100), ms(10)).unwrap();
        assert_eq!(
            registry
                .events_since(0)
                .events
                .iter()
                .map(|event| event.cursor)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert!(registry.events_since(0).gap);
        let resume_cursor = registry.snapshot().event_cursor;
        assert!(registry.events_since(resume_cursor).events.is_empty());
        registry.expire(ms(110)).unwrap();
        assert!(registry.events_since(resume_cursor).events.is_empty());
        assert!(!registry.events_since(resume_cursor).gap);
    }

    #[test]
    fn capacities_bound_active_sources_and_all_tracked_identities() {
        let limits = Limits {
            max_sources: 1,
            max_active_instances: 1,
            max_tracked_instances: 2,
            tombstone_ttl: ms(10),
            ..Limits::default()
        };
        let mut registry = Registry::new(limits).unwrap();
        registry.apply(state("a", 1, "idle", 5), ms(0)).unwrap();
        assert_eq!(
            registry.apply(state("b", 1, "idle", 5), ms(0)),
            Err(Error::Capacity("active instance"))
        );
        let mut other_source = event("a", 1, 5);
        other_source.source = "codex".into();
        assert_eq!(
            registry.apply(other_source, ms(0)),
            Err(Error::Capacity("source"))
        );
        registry
            .apply(message("a", 2, Payload::Remove), ms(0))
            .unwrap();
        registry.apply(state("b", 1, "idle", 5), ms(0)).unwrap();
        assert_eq!(
            registry.apply(message("c", 1, Payload::Remove), ms(0)),
            Err(Error::Capacity("tracked instance"))
        );
        assert_eq!(registry.entries.len(), 2);
        registry.expire(ms(15)).unwrap();
        registry.apply(state("c", 1, "idle", 5), ms(15)).unwrap();
    }

    #[test]
    fn rejected_messages_do_not_consume_sequence_and_max_sequence_never_wraps() {
        let mut registry = Registry::default();
        assert_eq!(
            registry.apply(state("a", 5, "idle", 0), ms(0)),
            Err(Error::InvalidTtl)
        );
        registry.apply(state("a", 1, "idle", 100), ms(0)).unwrap();
        registry.apply(event("a", u64::MAX, 100), ms(0)).unwrap();
        assert_eq!(
            registry.apply(event("a", u64::MAX, 100), ms(0)).unwrap(),
            ApplyOutcome::IgnoredStale
        );
        assert_eq!(
            registry
                .apply(state("a", 0, "working", 100), ms(0))
                .unwrap(),
            ApplyOutcome::IgnoredStale
        );
        assert_eq!(registry.events_since(0).events.len(), 1);
    }

    #[test]
    fn clock_errors_and_restart_are_explicit() {
        let mut registry = Registry::default();
        registry.apply(state("a", 1, "idle", 10), ms(5)).unwrap();
        let before = registry.snapshot();
        assert_eq!(registry.expire(ms(4)), Err(Error::ClockWentBackwards));
        assert_eq!(
            registry.apply(state("a", 2, "idle", 10), ms(4)),
            Err(Error::ClockWentBackwards)
        );
        assert_eq!(registry.snapshot(), before);
        assert_eq!(
            registry.apply(state("b", 1, "idle", 10), Duration::MAX),
            Err(Error::ClockOverflow)
        );
        let fresh = Registry::default();
        assert_eq!(fresh.snapshot().revision, 0);
        assert!(fresh.snapshot().states.is_empty());
        assert!(fresh.events_since(0).events.is_empty());
    }

    #[test]
    fn invalid_limits_are_rejected() {
        assert!(matches!(
            Registry::new(Limits {
                max_events: 0,
                ..Limits::default()
            }),
            Err(Error::InvalidLimits)
        ));
        assert!(matches!(
            Registry::new(Limits {
                max_active_instances: 257,
                ..Limits::default()
            }),
            Err(Error::InvalidLimits)
        ));
        assert!(matches!(
            Registry::new(Limits {
                tombstone_ttl: Duration::ZERO,
                ..Limits::default()
            }),
            Err(Error::InvalidLimits)
        ));
    }
}
