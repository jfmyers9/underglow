//! Isolated external-v1 facts for composition previews. No I/O, clock reads, providers,
//! live registry handles, or hardware intent are available to this context.
use std::time::Duration;

use crate::external::{
    ApplyOutcome, Error, EventBatch, Limits, Message, Payload, Registry, Snapshot,
};

/// A private registry, monotonic injected clock, and independent sequence space.
/// Multiple source/session identities can coexist; consumers own event cursors.
pub struct Scenario {
    registry: Registry,
    now: Duration,
    sequence: u64,
    generation: u64,
}

impl Default for Scenario {
    fn default() -> Self {
        Self::new(Limits::default()).expect("valid default limits")
    }
}

impl Scenario {
    pub fn new(limits: Limits) -> Result<Self, Error> {
        Ok(Self {
            registry: Registry::new(limits)?,
            now: Duration::ZERO,
            sequence: 0,
            generation: 0,
        })
    }

    pub fn now(&self) -> Duration {
        self.now
    }
    /// Changes on reset so consumers baseline even if time/cursors coincide.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn limits(&self) -> &Limits {
        self.registry.limits()
    }
    pub fn snapshot(&self) -> Snapshot {
        self.registry.snapshot()
    }
    pub fn events_since(&self, cursor: u64) -> EventBatch {
        self.registry.events_since(cursor)
    }

    /// Drop every synthetic fact, cursor, sequence and deadline. Keep limits.
    pub fn reset(&mut self) {
        let generation = self.generation.saturating_add(1);
        *self = Self::new(self.limits().clone()).expect("previously validated limits");
        self.generation = generation;
    }

    pub fn advance_to(&mut self, now: Duration) -> Result<(), Error> {
        self.registry.expire(now)?;
        self.now = now;
        Ok(())
    }

    pub fn advance_by(&mut self, delta: Duration) -> Result<(), Error> {
        self.advance_to(self.now.checked_add(delta).ok_or(Error::ClockOverflow)?)
    }

    /// Apply a bounded typed protocol message at the current scenario time.
    /// Errors leave facts, clock, and generated sequence unchanged. Manual high
    /// sequences advance the generator, so subsequent UI messages cannot go stale.
    pub fn apply(&mut self, message: Message) -> Result<ApplyOutcome, Error> {
        message.validate(self.limits())?;
        let ttl = match &message.payload {
            Payload::State { ttl_ms, .. } | Payload::Event { ttl_ms, .. } => {
                Duration::from_millis(*ttl_ms)
            }
            Payload::Remove => Duration::ZERO,
        };
        // Registry::apply expires before capacity/deadline validation. This
        // registry is already expired to `now`; preflight overflow as well.
        self.now
            .checked_add(ttl)
            .and_then(|t| t.checked_add(self.limits().tombstone_ttl))
            .ok_or(Error::ClockOverflow)?;
        let sequence = message.sequence;
        let result = self.registry.apply(message, self.now)?;
        if result == ApplyOutcome::Applied {
            self.sequence = self.sequence.max(sequence);
        }
        Ok(result)
    }

    fn publish(
        &mut self,
        source: &str,
        instance: &str,
        payload: Payload,
    ) -> Result<ApplyOutcome, Error> {
        self.apply(Message {
            schema_version: 1,
            source: source.into(),
            instance: instance.into(),
            sequence: self
                .sequence
                .checked_add(1)
                .ok_or(Error::Capacity("scenario sequence"))?,
            payload,
        })
    }

    pub fn publish_state(
        &mut self,
        source: &str,
        instance: &str,
        state: &str,
        ttl_ms: u64,
    ) -> Result<ApplyOutcome, Error> {
        self.publish(
            source,
            instance,
            Payload::State {
                state: state.into(),
                ttl_ms,
            },
        )
    }

    pub fn publish_event(
        &mut self,
        source: &str,
        instance: &str,
        event: &str,
        ttl_ms: u64,
    ) -> Result<ApplyOutcome, Error> {
        self.publish(
            source,
            instance,
            Payload::Event {
                event: event.into(),
                ttl_ms,
            },
        )
    }

    pub fn remove(&mut self, source: &str, instance: &str) -> Result<ApplyOutcome, Error> {
        self.publish(source, instance, Payload::Remove)
    }
}
