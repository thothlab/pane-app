//! The core event bus.
//!
//! # Why this exists
//!
//! `commands/proxy.rs` used to build the `Arc<dyn ProxyEngine>` as a local,
//! call `engine.events()` exactly once to feed a forwarder task, and then drop
//! the `Arc` when `start` returned. The `broadcast::Sender` lived inside
//! `MitmEngine`, kept alive only by clones held in the accept loops — so once
//! `start` had returned, **no new subscriber could ever be created**.
//!
//! That was fine when the webview was the only consumer. It makes `pane
//! captures tail` impossible: a second consumer, attaching at an arbitrary
//! time, is the entire feature.
//!
//! So the bus is created once at bootstrap, outlives any number of proxy
//! start/stop cycles, and anyone can subscribe whenever they like. [`Core`]
//! also retains the `Arc<dyn ProxyEngine>` (see `core.rs`) so the engine's own
//! sender stays alive while the proxy runs.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Matches `MitmEngine`'s own channel capacity. Slow subscribers get
/// `Lagged` rather than stalling the producer; consumers are expected to
/// surface the gap rather than treat it as fatal.
const BUS_CAPACITY: usize = 4096;

/// One event on the bus.
///
/// `topic` deliberately reuses the strings `EngineEvent::topic()` already
/// returns (`capture.started`, `capture.completed`, …) because the webview
/// listens on exactly those names. Keeping them identical means the frontend
/// contract is untouched by this refactor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreEvent {
    pub topic: String,
    pub payload: serde_json::Value,
}

impl CoreEvent {
    pub fn new(topic: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            topic: topic.into(),
            payload,
        }
    }
}

/// Topics emitted by the core itself rather than forwarded from the engine.
pub mod topics {
    /// Proxy started or stopped. Payload: `SessionDto` on start, `{running:false}` on stop.
    pub const PROXY_STATUS_CHANGED: &str = "proxy.status_changed";
    /// A logcat batch was persisted. Payload: `{serial, inserted}`.
    ///
    /// Deliberately a count, not the rows: the firehose never crosses IPC.
    /// Consumers re-query the database. This preserves the behaviour
    /// documented in the old `commands/logcat.rs`.
    pub const LOGCAT_APPENDED: &str = "logcat.appended";
    /// The logcat stream reported an error. Payload: `{serial, message}`.
    pub const LOGCAT_ERROR: &str = "logcat.error";
}

/// Owns the sender so subscribers can come and go independently of the engine.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<CoreEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BUS_CAPACITY);
        Self { tx }
    }

    /// Publish. Errors only when there are no subscribers, which is normal
    /// (the GUI may not have attached yet, and headless runs may have nobody
    /// listening at all) — hence the discard.
    pub fn publish(&self, event: CoreEvent) {
        let _ = self.tx.send(event);
    }

    pub fn publish_topic(&self, topic: impl Into<String>, payload: serde_json::Value) {
        self.publish(CoreEvent::new(topic, payload));
    }

    /// Attach a new consumer. Safe to call at any point in the process
    /// lifetime, any number of times — that is the whole point of this type.
    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_attaching_late_still_receive() {
        let bus = EventBus::new();
        // Nobody listening: publish must not panic or block.
        bus.publish_topic("capture.completed", serde_json::json!({"id": 1}));

        let mut rx = bus.subscribe();
        bus.publish_topic("capture.completed", serde_json::json!({"id": 2}));

        let ev = rx.recv().await.expect("late subscriber gets later events");
        assert_eq!(ev.topic, "capture.completed");
        assert_eq!(ev.payload["id"], 2);
    }

    #[tokio::test]
    async fn multiple_subscribers_each_get_every_event() {
        let bus = EventBus::new();
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        bus.publish_topic("capture.started", serde_json::json!({"id": 7}));

        assert_eq!(a.recv().await.unwrap().payload["id"], 7);
        assert_eq!(b.recv().await.unwrap().payload["id"], 7);
    }
}
