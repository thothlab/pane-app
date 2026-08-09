//! What belongs on an event subscription, decided independently of how it is
//! sent.
//!
//! Two front ends now read the same bus: the Unix-socket server in `server.rs`
//! (CLI, MCP) and the HTTP/SSE server in `pane-serve` (browser). Topic
//! filtering, capture-filter evaluation and enrichment have to be identical
//! across them, or the same `pane captures tail` and the same Captures view
//! would disagree about which requests happened. So the decision lives here and
//! each transport only owns its own plumbing.

use std::sync::Arc;

use pane_core::{Core, CoreEvent};

use crate::protocol::{EventFrame, SubscribeArgs};

/// Decide whether `ev` belongs on a subscription described by `args`, and
/// produce the frame to emit. `None` means drop it.
///
/// Note the asymmetry around `capture.completed` with an unparseable id: it is
/// dropped when a filter is set and passed through when there is none. A filter
/// is a promise that everything delivered matches it, and an id we cannot
/// resolve is an id we cannot check that promise against — so the only honest
/// answer is to withhold it. With no filter there is nothing to verify and the
/// event goes out unenriched.
pub async fn shape_event(
    core: &Arc<Core>,
    args: &SubscribeArgs,
    ev: &CoreEvent,
) -> Option<EventFrame> {
    if !args.topics.is_empty() && !args.topics.contains(&ev.topic) {
        return None;
    }

    let mut payload = ev.payload.clone();

    // `capture.completed` carries only {id, status, duration_ms, total_bytes};
    // host/method/path lived on `capture.started`, a different event. Re-read
    // the row so one frame is one whole capture.
    //
    // Only this topic is filterable — the DSL is written against capture rows,
    // so applying it to a logcat ping would drop every one of them.
    if ev.topic == "capture.completed" {
        let cap_id = ev
            .payload
            .get("id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok());

        match cap_id {
            Some(cap_id) => {
                if let Some(filter) = args.filter.as_deref() {
                    match core.storage.capture_matches(cap_id, filter) {
                        Ok(true) => {}
                        Ok(false) => return None,
                        Err(e) => {
                            tracing::debug!(error = %e, "tail filter evaluation failed");
                            return None;
                        }
                    }
                }
                if args.enrich == "summary" {
                    if let Ok(cap) = core.capture_get(cap_id).await {
                        if let Ok(v) = serde_json::to_value(&cap) {
                            payload = v;
                        }
                    }
                }
            }
            None if args.filter.is_some() => return None,
            None => {}
        }
    }

    Some(EventFrame {
        topic: ev.topic.clone(),
        payload,
    })
}

/// The frame emitted in place of tearing a stream down when a subscriber lags.
///
/// A consumer would rather know it missed `skipped` events than have the
/// connection die mid-run, so lag is data, not an error.
pub fn lagged_frame(skipped: u64) -> EventFrame {
    EventFrame {
        topic: "stream.lagged".into(),
        payload: serde_json::json!({ "skipped": skipped }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pane_core::{Core, CoreConfig};
    use serde_json::json;

    /// A Core over a throwaway data dir.
    ///
    /// `bootstrap`, not `attach_unowned`: the latter refuses a directory with
    /// no database in it, and a tempdir has none. `take_instance_lock: false`
    /// so these run in parallel with each other and with a real instance on the
    /// developer's machine.
    fn core(dir: &tempfile::TempDir) -> Arc<Core> {
        Arc::new(
            Core::bootstrap(CoreConfig {
                data_dir: Some(dir.path().to_path_buf()),
                take_instance_lock: false,
            })
            .expect("core"),
        )
    }

    /// Insert a minimal capture row and return its id.
    ///
    /// Raw SQL because there is no public insert API — the proxy loop and the
    /// replay path both write this table directly. `ca_certificate` already has
    /// a row (bootstrap generated the CA), so the session FK is satisfiable
    /// without inventing one.
    fn seed_capture(core: &Arc<Core>, host: &str) -> uuid::Uuid {
        let cap = uuid::Uuid::new_v4();
        let session = uuid::Uuid::new_v4();
        let conn = core.storage.conn().lock();
        let ca_id: String = conn
            .query_row("SELECT id FROM ca_certificate LIMIT 1", [], |r| r.get(0))
            .expect("bootstrap generated a CA");
        conn.execute(
            "INSERT INTO session (id, started_at, listen, ca_id, status)
             VALUES (?1, 0, '127.0.0.1:8888', ?2, 'stopped')",
            rusqlite::params![session.to_string(), ca_id],
        )
        .expect("session");
        conn.execute(
            "INSERT INTO capture (id, session_id, started_at, client_addr, server_host,
                                  server_port, scheme, http_version, method, url_path,
                                  status, total_bytes, state)
             VALUES (?1, ?2, 0, '127.0.0.1:1', ?3, 443, 'https', 'HTTP/1.1', 'GET', '/x',
                     200, 0, 'completed')",
            rusqlite::params![cap.to_string(), session.to_string(), host],
        )
        .expect("capture");
        cap
    }

    fn args(topics: &[&str], filter: Option<&str>, enrich: &str) -> SubscribeArgs {
        SubscribeArgs {
            topics: topics.iter().map(|s| s.to_string()).collect(),
            filter: filter.map(str::to_string),
            enrich: enrich.to_string(),
        }
    }

    #[tokio::test]
    async fn empty_topic_list_means_everything() {
        let d = tempfile::tempdir().unwrap();
        let c = core(&d);
        let ev = CoreEvent::new("logcat.appended", json!({"serial": "X", "inserted": 3}));
        let f = shape_event(&c, &args(&[], None, "none"), &ev).await;
        assert_eq!(f.expect("passed").topic, "logcat.appended");
    }

    #[tokio::test]
    async fn a_topic_outside_the_list_is_dropped() {
        let d = tempfile::tempdir().unwrap();
        let c = core(&d);
        let ev = CoreEvent::new("logcat.appended", json!({}));
        let f = shape_event(&c, &args(&["capture.completed"], None, "none"), &ev).await;
        assert!(f.is_none());
    }

    /// The DSL is written against capture rows. Applying it to any other topic
    /// would silently drop every logcat ping on a filtered tail.
    #[tokio::test]
    async fn a_filter_never_applies_to_non_capture_topics() {
        let d = tempfile::tempdir().unwrap();
        let c = core(&d);
        let ev = CoreEvent::new("logcat.appended", json!({"serial": "X"}));
        let f = shape_event(&c, &args(&[], Some("host:nope.example.com"), "none"), &ev).await;
        assert!(f.is_some(), "logcat pings must survive a captures filter");
    }

    #[tokio::test]
    async fn enrich_none_passes_the_raw_payload_through() {
        let d = tempfile::tempdir().unwrap();
        let c = core(&d);
        let raw = json!({"id": uuid::Uuid::new_v4().to_string(), "status": 200});
        let ev = CoreEvent::new("capture.completed", raw.clone());
        let f = shape_event(&c, &args(&[], None, "none"), &ev)
            .await
            .expect("passed");
        assert_eq!(f.payload, raw);
    }

    /// An id that is not a UUID cannot be checked against the filter, and a
    /// filter promises that everything delivered matches. Withhold it.
    #[tokio::test]
    async fn a_non_uuid_id_is_dropped_when_a_filter_is_set() {
        let d = tempfile::tempdir().unwrap();
        let c = core(&d);
        let ev = CoreEvent::new("capture.completed", json!({"id": "not-a-uuid"}));
        let f = shape_event(&c, &args(&[], Some("host:api.example.com"), "none"), &ev).await;
        assert!(f.is_none());
    }

    /// …but with no filter there is no promise to keep, so it goes out as-is.
    #[tokio::test]
    async fn a_non_uuid_id_passes_when_no_filter_is_set() {
        let d = tempfile::tempdir().unwrap();
        let c = core(&d);
        let ev = CoreEvent::new("capture.completed", json!({"id": "not-a-uuid"}));
        let f = shape_event(&c, &args(&[], None, "summary"), &ev).await;
        assert!(f.is_some());
    }

    /// A row that does not exist cannot match, so a filtered subscription drops
    /// it rather than guessing.
    #[tokio::test]
    async fn an_unknown_capture_is_dropped_when_a_filter_is_set() {
        let d = tempfile::tempdir().unwrap();
        let c = core(&d);
        let ev = CoreEvent::new(
            "capture.completed",
            json!({"id": uuid::Uuid::new_v4().to_string()}),
        );
        let f = shape_event(&c, &args(&[], Some("host:api.example.com"), "none"), &ev).await;
        assert!(f.is_none());
    }

    /// The reason `enrich` exists: `capture.completed` carries four fields, and
    /// a consumer printing the stream needs host/method/path off the row.
    #[tokio::test]
    async fn enrich_summary_replaces_the_payload_with_the_row() {
        let d = tempfile::tempdir().unwrap();
        let c = core(&d);
        let id = seed_capture(&c, "api.example.com");
        let ev = CoreEvent::new("capture.completed", json!({"id": id.to_string()}));

        let f = shape_event(&c, &args(&[], None, "summary"), &ev)
            .await
            .expect("passed");
        assert_eq!(f.payload["server_host"], "api.example.com");
        assert_eq!(f.payload["method"], "GET");
        assert_eq!(f.payload["url_path"], "/x");
    }

    #[tokio::test]
    async fn a_matching_filter_lets_the_capture_through() {
        let d = tempfile::tempdir().unwrap();
        let c = core(&d);
        let id = seed_capture(&c, "api.example.com");
        let ev = CoreEvent::new("capture.completed", json!({"id": id.to_string()}));

        let f = shape_event(&c, &args(&[], Some("host:api.example.com"), "none"), &ev).await;
        assert!(f.is_some(), "the row matches the filter");
    }

    #[tokio::test]
    async fn a_non_matching_filter_drops_the_capture() {
        let d = tempfile::tempdir().unwrap();
        let c = core(&d);
        let id = seed_capture(&c, "api.example.com");
        let ev = CoreEvent::new("capture.completed", json!({"id": id.to_string()}));

        let f = shape_event(&c, &args(&[], Some("host:other.example.com"), "none"), &ev).await;
        assert!(f.is_none());
    }

    #[test]
    fn lagged_frame_reports_the_gap() {
        let f = lagged_frame(17);
        assert_eq!(f.topic, "stream.lagged");
        assert_eq!(f.payload["skipped"], 17);
    }
}
