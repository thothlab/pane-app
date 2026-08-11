//! Which hosts we decline to decrypt, and why.
//!
//! Pane MITMs every CONNECT it sees. That is correct for the traffic the user
//! is here to inspect, and actively harmful for everything else: because the
//! device's `http_proxy` is global, *all* of its traffic arrives here, not just
//! the app under test. Any client that doesn't trust our CA — a release build
//! (Android 7+ apps trust only the system store unless they opt in), an app
//! that pins its own anchors, a system service — used to have its handshake
//! rejected and its connection closed. From the user's side that reads as
//! "everything on my phone lost internet while Pane was running".
//!
//! The fix is what Charles calls SSL Proxying scoping: for hosts we're not
//! going to successfully decrypt, don't try. Reply `200 Connection
//! Established` and splice the two sockets together, so the client negotiates
//! TLS directly with the real server against the real certificate. We see that
//! a connection happened and to whom, but not its contents — which is exactly
//! the trade the user wants for traffic that isn't theirs to inspect.
//!
//! Two ways a host lands in the set:
//!
//! 1. **Seeded** from the bundled pinned-hints list, but only entries hinted
//!    `app_pin` — see `pane_pinning::is_app_pinned` for why the `system_pin`
//!    and `ct_required` classes are deliberately excluded. Seeded matches are
//!    patterns, not entries: they can't be forgotten, only listed.
//! 2. **Learned** at runtime, and *only* from a TLS alert naming the
//!    certificate (`certificate_unknown`, `bad_certificate`, `unknown_ca`, …).
//!    That is the client stating a verdict, and it will state the same one
//!    next time.
//!
//!    Transport failures teach nothing at all. Two earlier revisions of this
//!    got it wrong in the same direction: first every `accept()` error was
//!    read as "the client rejected our leaf", so one yanked USB cable tunnelled
//!    a host for the rest of the run; then a counter allowed it after three
//!    failures in thirty seconds, which an app retrying through a
//!    re-establishing `adb reverse` clears in about a second. Both made the
//!    host sticky exactly when the link was flaky, which is when the user is
//!    least able to tell a broken tunnel from a broken certificate.
//!
//!    The price of dropping it: a client that pins and closes the socket
//!    without sending an alert never gets tunnelled, so its requests keep
//!    failing. That is a visible, recoverable failure, unlike silently
//!    tunnelling traffic the user came here to read.
//!
//! The learned half carries an unavoidable cost: the ClientHello of the failed
//! handshake is already consumed by the time we know it failed, so that first
//! connection to an unknown pinned host is lost. Clients retry, so in practice
//! the user sees one failed request and then working traffic — but it is a
//! real sacrificial connection, not a seamless fallback.
//!
//! **Scope and recovery.** The set is owned by the app, not by one engine run,
//! so `proxy.start` resets it explicitly (see `commands::proxy`) to keep the
//! historical "restart the proxy and it forgets" behaviour. It is also reset
//! when the ground truth changes — CA rotation, a device paired or re-synced —
//! and can be inspected and cleared by hand from Settings. Before that it was
//! unreachable state: no TTL, no reset, no UI, and the only escape was a proxy
//! restart the user had to guess at.

use std::collections::HashMap;
use std::sync::Arc;

use pane_ipc::{TunneledHostDto, TunneledHostsDto};
use parking_lot::Mutex;
use time::OffsetDateTime;

/// Why a host is being tunnelled. Persisted into the capture row and surfaced
/// in the UI so "why is everything CONNECT" has an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelReason {
    /// The client sent a TLS alert about our certificate. Conclusive, and the
    /// only way a host is ever learned.
    CertRejected,
}

impl TunnelReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            TunnelReason::CertRejected => "cert_rejected",
        }
    }
}

#[derive(Debug, Clone)]
struct LearnedHost {
    learned_at: OffsetDateTime,
    reason: TunnelReason,
    detail: String,
}

/// Keyed by the client's TLS fingerprint as well as the host: trust is a
/// property of the client, and a phone runs several against one API. Keyed by
/// host alone, one SDK rejecting our CA switched decryption off for the app
/// being debugged — observed on a real device as a completed POST and a
/// `certificate_unknown` alert to the same host in the same second.
type Key = (String, String);

#[derive(Default)]
struct Inner {
    learned: HashMap<Key, LearnedHost>,
}

fn key(client: &str, host: &str) -> Key {
    (client.to_string(), norm(host))
}

/// Shared, cheaply cloneable set of hosts to tunnel rather than decrypt.
#[derive(Clone, Default)]
pub struct NoMitmSet {
    inner: Arc<Mutex<Inner>>,
}

impl NoMitmSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether to skip MITM for `host` and splice a blind tunnel instead.
    ///
    /// The seeded half is evaluated per call rather than expanded into the set
    /// up front, because the hint list is pattern-based (`*.facebook.com`) and
    /// matching is cheaper than enumerating.
    pub fn should_tunnel(&self, client: &str, host: &str) -> bool {
        if pane_pinning::is_app_pinned(host) {
            return true;
        }
        self.inner.lock().learned.contains_key(&key(client, host))
    }

    /// Why `host` is being tunnelled, phrased for the capture row's error
    /// detail. `None` when we aren't tunnelling it at all.
    ///
    /// Without this a tunnelled row says only "passed through", and the user
    /// has no way to tell a host we were told to skip from one that rejected
    /// our certificate ten minutes ago.
    pub fn why_tunnel(&self, client: &str, host: &str) -> Option<String> {
        if pane_pinning::is_app_pinned(host) {
            return Some("seeded: host is in the bundled app-pinning list".into());
        }
        self.inner.lock().learned.get(&key(client, host)).map(|l| {
            let detail = if l.detail.is_empty() {
                String::new()
            } else {
                format!(" ({})", l.detail)
            };
            format!("learned: {}{detail}", l.reason.as_str())
        })
    }

    /// Record that `host` sent a TLS alert rejecting our certificate. Returns
    /// `true` if this is new information, so the caller can log the transition
    /// once rather than on every failed connection.
    pub fn learn_rejected(&self, client: &str, host: &str, detail: &str) -> bool {
        let mut inner = self.inner.lock();
        inner
            .learned
            .insert(
                key(client, host),
                LearnedHost {
                    learned_at: OffsetDateTime::now_utc(),
                    reason: TunnelReason::CertRejected,
                    detail: detail.to_string(),
                },
            )
            .is_none()
    }

    /// Forget everything learned this run. Returns how many hosts were
    /// dropped. Seeded patterns are unaffected — they aren't learned state.
    pub fn reset(&self) -> usize {
        let mut inner = self.inner.lock();
        let n = inner.learned.len();
        inner.learned.clear();
        n
    }

    /// Drop a host, for every client that rejected it — the UI lists hosts, and
    /// "try this one again" means all of them.
    pub fn forget(&self, host: &str) -> bool {
        let wanted = norm(host);
        let mut inner = self.inner.lock();
        let before = inner.learned.len();
        inner.learned.retain(|(_, h), _| *h != wanted);
        inner.learned.len() != before
    }

    /// Everything currently being tunnelled, for the Settings panel: what was
    /// learned (and why), plus the seeded patterns that can't be forgotten.
    pub fn list(&self) -> TunneledHostsDto {
        let inner = self.inner.lock();
        // One row per host even though the set is keyed per client: the panel
        // answers "what is not being decrypted", and a hostname is what the
        // user recognises. Where several clients rejected the same host the
        // count says so, because that is exactly the case where some other app
        // may still be decrypting it.
        let mut by_host: HashMap<&str, (&LearnedHost, usize)> = HashMap::new();
        for ((_, host), learned) in inner.learned.iter() {
            by_host
                .entry(host.as_str())
                .and_modify(|(existing, n)| {
                    *n += 1;
                    if learned.learned_at < existing.learned_at {
                        *existing = learned;
                    }
                })
                .or_insert((learned, 1));
        }
        let mut learned: Vec<TunneledHostDto> = by_host
            .into_iter()
            .map(|(host, (l, clients))| TunneledHostDto {
                host: host.to_string(),
                learned_at: l.learned_at.to_string(),
                reason: l.reason.as_str().to_string(),
                detail: if clients > 1 {
                    format!("{} ({clients} clients)", l.detail)
                } else {
                    l.detail.clone()
                },
            })
            .collect();
        learned.sort_by(|a, b| a.host.cmp(&b.host));
        TunneledHostsDto {
            learned,
            seeded: pane_pinning::app_pinned_patterns()
                .iter()
                .map(|p| p.to_string())
                .collect(),
        }
    }
}

/// SNI casing is not normalised by clients; treating `API.Example.com` and
/// `api.example.com` as different hosts would mean paying the sacrificial
/// connection twice for one server.
fn norm(host: &str) -> String {
    host.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two different TLS stacks, as `client_hello` would fingerprint them.
    const A: &str = "aaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbb";

    #[test]
    fn app_pinned_hosts_are_tunnelled_without_being_learned() {
        let s = NoMitmSet::new();
        assert!(s.should_tunnel(A, "graph.facebook.com"));
        // Seeding is evaluated by pattern match, not by pre-inserting hosts:
        // learn reporting `true` proves the host wasn't in the learned set.
        assert!(s.learn_rejected(A, "graph.facebook.com", ""));
    }

    #[test]
    fn ordinary_hosts_are_decrypted_until_the_client_rejects_the_cert() {
        let s = NoMitmSet::new();
        assert!(!s.should_tunnel(A, "api.example.com"));
        assert!(s.learn_rejected(A, "api.example.com", "alert: certificate_unknown"));
        assert!(s.should_tunnel(A, "api.example.com"));
    }

    #[test]
    fn learn_reports_only_the_first_sighting() {
        let s = NoMitmSet::new();
        assert!(s.learn_rejected(A, "api.example.com", ""));
        assert!(!s.learn_rejected(A, "api.example.com", ""));
    }

    #[test]
    fn learned_matching_is_case_insensitive() {
        let s = NoMitmSet::new();
        s.learn_rejected(A, "API.Example.com", "");
        assert!(s.should_tunnel(A, "api.example.com"));
    }

    #[test]
    fn system_pinned_hosts_are_not_seeded() {
        // Regression guard: these decrypt fine for ordinary apps, so seeding
        // them would silently stop showing traffic that works today.
        let s = NoMitmSet::new();
        assert!(!s.should_tunnel(A, "www.googleapis.com"));
        assert!(!s.should_tunnel(A, "fonts.gstatic.com"));
    }

    #[test]
    fn one_client_rejecting_leaves_the_others_decrypting() {
        // The bug this keying exists for: on a real device a debug build
        // completed POST /key-guard/api/v1/publicKey against
        // api.dbo-dengi.online while another client on the same phone answered
        // certificate_unknown for that host in the same second. Keyed by host,
        // the second one silenced the first.
        let s = NoMitmSet::new();
        s.learn_rejected(B, "api.dbo-dengi.online", "alert: certificate_unknown");
        assert!(
            s.should_tunnel(B, "api.dbo-dengi.online"),
            "the rejecting client is tunnelled"
        );
        assert!(
            !s.should_tunnel(A, "api.dbo-dengi.online"),
            "everyone else keeps being decrypted"
        );
    }

    #[test]
    fn forget_clears_a_host_for_every_client() {
        let s = NoMitmSet::new();
        s.learn_rejected(A, "api.example.com", "");
        s.learn_rejected(B, "api.example.com", "");
        assert!(s.forget("api.example.com"));
        assert!(!s.should_tunnel(A, "api.example.com"));
        assert!(!s.should_tunnel(B, "api.example.com"));
        assert!(!s.forget("api.example.com"), "second forget is a no-op");
    }

    #[test]
    fn the_panel_shows_one_row_per_host_and_counts_the_clients() {
        let s = NoMitmSet::new();
        s.learn_rejected(A, "api.example.com", "alert: unknown_ca");
        s.learn_rejected(B, "api.example.com", "alert: certificate_unknown");
        s.learn_rejected(A, "other.example.com", "alert: unknown_ca");
        let listed = s.list();
        assert_eq!(listed.learned.len(), 2);
        assert_eq!(listed.learned[0].host, "api.example.com");
        assert!(listed.learned[0].detail.contains("2 clients"));
        assert!(!listed.learned[1].detail.contains("clients"));
    }

    #[test]
    fn transport_failures_never_learn_a_host() {
        // The bug this replaces: a counter tunnelled a host after three
        // transport failures in thirty seconds. An app retrying while the
        // `adb reverse` is being re-established clears that in about a second,
        // so the host went sticky during every reconnect — and the only way
        // out the user found was deleting the device and pairing it again.
        let s = NoMitmSet::new();
        for _ in 0..50 {
            assert!(
                !s.should_tunnel(A, "api.example.com"),
                "no number of dead sockets is a verdict on the certificate"
            );
        }
        assert_eq!(s.list().learned.len(), 0);
    }

    #[test]
    fn why_tunnel_distinguishes_seeded_from_learned() {
        let s = NoMitmSet::new();
        assert_eq!(s.why_tunnel(A, "api.example.com"), None);
        s.learn_rejected(A, "api.example.com", "alert: unknown_ca");
        assert_eq!(
            s.why_tunnel(A, "api.example.com").as_deref(),
            Some("learned: cert_rejected (alert: unknown_ca)")
        );
        assert!(s
            .why_tunnel(A, "graph.facebook.com")
            .is_some_and(|w| w.starts_with("seeded:")));
    }

    #[test]
    fn reset_forgets_learned_hosts_but_not_seeded_patterns() {
        let s = NoMitmSet::new();
        s.learn_rejected(A, "api.example.com", "");
        assert_eq!(s.reset(), 1);
        assert!(!s.should_tunnel(A, "api.example.com"));
        assert!(
            s.should_tunnel(A, "graph.facebook.com"),
            "seed survives reset"
        );
    }

    #[test]
    fn forget_drops_a_single_host() {
        let s = NoMitmSet::new();
        s.learn_rejected(A, "a.example.com", "");
        s.learn_rejected(A, "b.example.com", "");
        assert!(s.forget("a.example.com"));
        assert!(!s.forget("a.example.com"), "second forget is a no-op");
        assert!(!s.should_tunnel(A, "a.example.com"));
        assert!(s.should_tunnel(A, "b.example.com"));
    }

    #[test]
    fn list_reports_hosts_with_their_reason() {
        let s = NoMitmSet::new();
        s.learn_rejected(A, "b.example.com", "alert: unknown_ca");
        s.learn_rejected(A, "a.example.com", "alert: bad_certificate");
        let listed = s.list();
        assert_eq!(listed.learned.len(), 2);
        assert_eq!(listed.learned[0].host, "a.example.com");
        assert_eq!(listed.learned[1].host, "b.example.com");
        assert_eq!(listed.learned[1].reason, "cert_rejected");
        assert_eq!(listed.learned[1].detail, "alert: unknown_ca");
        assert!(!listed.seeded.is_empty(), "bundled hints are reported");
    }
}
