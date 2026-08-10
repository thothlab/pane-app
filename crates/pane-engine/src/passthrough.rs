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
//! 2. **Learned** at runtime, and *only* on evidence that the client rejected
//!    our certificate:
//!    - a TLS alert naming the certificate (`certificate_unknown`,
//!      `bad_certificate`, `unknown_ca`, …) — conclusive, learned at once;
//!    - `IO_STRIKES_TO_LEARN` transport failures inside `STRIKE_WINDOW` — a
//!      pinner that RSTs without bothering to send an alert. Bounded by the
//!      window so three cable-pulls spread over an hour never add up to a
//!      verdict.
//!
//!    A plain I/O error on its own teaches nothing. It used to: every
//!    `accept()` error was read as "the client rejected our leaf", so one
//!    yanked USB cable could silently tunnel a host for the rest of the run.
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
use time::{Duration, OffsetDateTime};

/// Transport failures within `STRIKE_WINDOW` before we conclude the client is
/// rejecting our certificate without saying so. Three is a retry burst; one or
/// two are a flaky cable.
const IO_STRIKES_TO_LEARN: u8 = 3;

/// How long a strike stays relevant. Long enough to cover a client's retry
/// burst, short enough that unrelated failures never accumulate into a verdict.
const STRIKE_WINDOW: Duration = Duration::seconds(30);

/// Why a host is being tunnelled. Persisted into the capture row and surfaced
/// in the UI so "why is everything CONNECT" has an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelReason {
    /// The client sent a TLS alert about our certificate. Conclusive.
    CertRejected,
    /// Repeated transport failures in a short window, with no alert. Inferred.
    RepeatedFailure,
}

impl TunnelReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            TunnelReason::CertRejected => "cert_rejected",
            TunnelReason::RepeatedFailure => "repeated_failure",
        }
    }
}

/// What `note_io_failure` decided about this connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoFailure {
    /// Counted, but not enough to conclude anything yet.
    Noted { strikes: u8 },
    /// This failure crossed the threshold; the host is now tunnelled.
    Learned,
}

#[derive(Debug, Clone)]
struct LearnedHost {
    learned_at: OffsetDateTime,
    reason: TunnelReason,
    detail: String,
}

#[derive(Debug, Clone, Copy)]
struct Strikes {
    count: u8,
    last: OffsetDateTime,
}

#[derive(Default)]
struct Inner {
    learned: HashMap<String, LearnedHost>,
    strikes: HashMap<String, Strikes>,
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
    pub fn should_tunnel(&self, host: &str) -> bool {
        if pane_pinning::is_app_pinned(host) {
            return true;
        }
        self.inner.lock().learned.contains_key(&norm(host))
    }

    /// Record that `host` sent a TLS alert rejecting our certificate. Returns
    /// `true` if this is new information, so the caller can log the transition
    /// once rather than on every failed connection.
    pub fn learn_rejected(&self, host: &str, detail: &str) -> bool {
        let key = norm(host);
        let mut inner = self.inner.lock();
        inner.strikes.remove(&key);
        inner
            .learned
            .insert(
                key,
                LearnedHost {
                    learned_at: OffsetDateTime::now_utc(),
                    reason: TunnelReason::CertRejected,
                    detail: detail.to_string(),
                },
            )
            .is_none()
    }

    /// Record a handshake that died on transport, with no alert to explain it.
    ///
    /// Strikes expire: a failure more than `STRIKE_WINDOW` after the previous
    /// one starts the count over, so only a genuine burst — which is what a
    /// pinning client's retries look like — ever reaches a verdict.
    pub fn note_io_failure(&self, host: &str, detail: &str) -> IoFailure {
        let key = norm(host);
        let now = OffsetDateTime::now_utc();
        let mut inner = self.inner.lock();

        if inner.learned.contains_key(&key) {
            return IoFailure::Learned;
        }

        let count = match inner.strikes.get(&key) {
            Some(prev) if now - prev.last <= STRIKE_WINDOW => prev.count.saturating_add(1),
            _ => 1,
        };

        if count < IO_STRIKES_TO_LEARN {
            inner.strikes.insert(key, Strikes { count, last: now });
            return IoFailure::Noted { strikes: count };
        }

        inner.strikes.remove(&key);
        inner.learned.insert(
            key,
            LearnedHost {
                learned_at: now,
                reason: TunnelReason::RepeatedFailure,
                detail: detail.to_string(),
            },
        );
        IoFailure::Learned
    }

    /// A handshake completed for this host, so whatever the earlier transport
    /// failures were, they weren't the client refusing our leaf.
    pub fn note_handshake_ok(&self, host: &str) {
        self.inner.lock().strikes.remove(&norm(host));
    }

    /// Forget everything learned this run. Returns how many hosts were
    /// dropped. Seeded patterns are unaffected — they aren't learned state.
    pub fn reset(&self) -> usize {
        let mut inner = self.inner.lock();
        inner.strikes.clear();
        let n = inner.learned.len();
        inner.learned.clear();
        n
    }

    /// Drop one learned host, so the next connection to it is decrypted again.
    pub fn forget(&self, host: &str) -> bool {
        let key = norm(host);
        let mut inner = self.inner.lock();
        inner.strikes.remove(&key);
        inner.learned.remove(&key).is_some()
    }

    /// Everything currently being tunnelled, for the Settings panel: what was
    /// learned (and why), plus the seeded patterns that can't be forgotten.
    pub fn list(&self) -> TunneledHostsDto {
        let inner = self.inner.lock();
        let mut learned: Vec<TunneledHostDto> = inner
            .learned
            .iter()
            .map(|(host, l)| TunneledHostDto {
                host: host.clone(),
                learned_at: l.learned_at.to_string(),
                reason: l.reason.as_str().to_string(),
                detail: l.detail.clone(),
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

    #[test]
    fn app_pinned_hosts_are_tunnelled_without_being_learned() {
        let s = NoMitmSet::new();
        assert!(s.should_tunnel("graph.facebook.com"));
        // Seeding is evaluated by pattern match, not by pre-inserting hosts:
        // learn reporting `true` proves the host wasn't in the learned set.
        assert!(s.learn_rejected("graph.facebook.com", ""));
    }

    #[test]
    fn ordinary_hosts_are_decrypted_until_the_client_rejects_the_cert() {
        let s = NoMitmSet::new();
        assert!(!s.should_tunnel("api.example.com"));
        assert!(s.learn_rejected("api.example.com", "alert: certificate_unknown"));
        assert!(s.should_tunnel("api.example.com"));
    }

    #[test]
    fn learn_reports_only_the_first_sighting() {
        let s = NoMitmSet::new();
        assert!(s.learn_rejected("api.example.com", ""));
        assert!(!s.learn_rejected("api.example.com", ""));
    }

    #[test]
    fn learned_matching_is_case_insensitive() {
        let s = NoMitmSet::new();
        s.learn_rejected("API.Example.com", "");
        assert!(s.should_tunnel("api.example.com"));
    }

    #[test]
    fn system_pinned_hosts_are_not_seeded() {
        // Regression guard: these decrypt fine for ordinary apps, so seeding
        // them would silently stop showing traffic that works today.
        let s = NoMitmSet::new();
        assert!(!s.should_tunnel("www.googleapis.com"));
        assert!(!s.should_tunnel("fonts.gstatic.com"));
    }

    #[test]
    fn a_single_io_failure_teaches_nothing() {
        // The whole point of the strike counter: one yanked cable must not
        // tunnel a host that was decrypting perfectly a second ago.
        let s = NoMitmSet::new();
        assert_eq!(
            s.note_io_failure("api.example.com", "connection reset"),
            IoFailure::Noted { strikes: 1 }
        );
        assert!(!s.should_tunnel("api.example.com"));
    }

    #[test]
    fn a_burst_of_io_failures_is_treated_as_rejection() {
        let s = NoMitmSet::new();
        for expected in 1..IO_STRIKES_TO_LEARN {
            assert_eq!(
                s.note_io_failure("api.example.com", "reset"),
                IoFailure::Noted { strikes: expected }
            );
        }
        assert_eq!(
            s.note_io_failure("api.example.com", "reset"),
            IoFailure::Learned
        );
        assert!(s.should_tunnel("api.example.com"));
    }

    #[test]
    fn a_successful_handshake_clears_accumulated_strikes() {
        let s = NoMitmSet::new();
        s.note_io_failure("api.example.com", "reset");
        s.note_io_failure("api.example.com", "reset");
        s.note_handshake_ok("api.example.com");
        // Back to a clean slate: the next failure is strike one, not three.
        assert_eq!(
            s.note_io_failure("api.example.com", "reset"),
            IoFailure::Noted { strikes: 1 }
        );
        assert!(!s.should_tunnel("api.example.com"));
    }

    #[test]
    fn stale_strikes_expire_instead_of_accumulating() {
        // Three cable-pulls an hour apart are three unrelated accidents, not
        // evidence about the certificate.
        let s = NoMitmSet::new();
        let stale = OffsetDateTime::now_utc() - STRIKE_WINDOW - Duration::seconds(1);
        s.inner.lock().strikes.insert(
            "api.example.com".into(),
            Strikes {
                count: IO_STRIKES_TO_LEARN - 1,
                last: stale,
            },
        );
        assert_eq!(
            s.note_io_failure("api.example.com", "reset"),
            IoFailure::Noted { strikes: 1 }
        );
        assert!(!s.should_tunnel("api.example.com"));
    }

    #[test]
    fn reset_forgets_learned_hosts_but_not_seeded_patterns() {
        let s = NoMitmSet::new();
        s.learn_rejected("api.example.com", "");
        assert_eq!(s.reset(), 1);
        assert!(!s.should_tunnel("api.example.com"));
        assert!(s.should_tunnel("graph.facebook.com"), "seed survives reset");
    }

    #[test]
    fn forget_drops_a_single_host() {
        let s = NoMitmSet::new();
        s.learn_rejected("a.example.com", "");
        s.learn_rejected("b.example.com", "");
        assert!(s.forget("a.example.com"));
        assert!(!s.forget("a.example.com"), "second forget is a no-op");
        assert!(!s.should_tunnel("a.example.com"));
        assert!(s.should_tunnel("b.example.com"));
    }

    #[test]
    fn list_reports_hosts_with_their_reason() {
        let s = NoMitmSet::new();
        s.learn_rejected("b.example.com", "alert: unknown_ca");
        for _ in 0..IO_STRIKES_TO_LEARN {
            s.note_io_failure("a.example.com", "reset");
        }
        let listed = s.list();
        assert_eq!(listed.learned.len(), 2);
        assert_eq!(listed.learned[0].host, "a.example.com");
        assert_eq!(listed.learned[0].reason, "repeated_failure");
        assert_eq!(listed.learned[1].host, "b.example.com");
        assert_eq!(listed.learned[1].reason, "cert_rejected");
        assert_eq!(listed.learned[1].detail, "alert: unknown_ca");
        assert!(!listed.seeded.is_empty(), "bundled hints are reported");
    }
}
