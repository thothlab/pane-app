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
//!    and `ct_required` classes are deliberately excluded.
//! 2. **Learned** at runtime: a TLS handshake we lost is proof this client
//!    won't accept our leaf, so the host goes in and every subsequent
//!    connection is tunnelled.
//!
//! The learned half carries an unavoidable cost: the ClientHello of the failed
//! handshake is already consumed by the time we know it failed, so that first
//! connection to an unknown pinned host is lost. Clients retry, so in practice
//! the user sees one failed request and then working traffic — but it is a
//! real sacrificial connection, not a seamless fallback.
//!
//! Scope is the process, not the session: the set is not persisted, so the
//! sacrificial connection recurs once per Pane launch per host.

use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::Mutex;

/// Shared, cheaply cloneable set of hosts to tunnel rather than decrypt.
#[derive(Clone, Default)]
pub struct NoMitmSet {
    learned: Arc<Mutex<HashSet<String>>>,
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
        self.learned.lock().contains(&host.to_ascii_lowercase())
    }

    /// Record that `host` rejected our certificate. Returns `true` if this is
    /// new information, so the caller can log the transition once rather than
    /// on every failed connection.
    pub fn learn(&self, host: &str) -> bool {
        self.learned.lock().insert(host.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_pinned_hosts_are_tunnelled_without_being_learned() {
        let s = NoMitmSet::new();
        assert!(s.should_tunnel("graph.facebook.com"));
        // Seeding is evaluated by pattern match, not by pre-inserting hosts:
        // learn() reporting `true` proves the host wasn't in the learned set.
        assert!(s.learn("graph.facebook.com"));
    }

    #[test]
    fn ordinary_hosts_are_decrypted_until_they_fail() {
        let s = NoMitmSet::new();
        assert!(!s.should_tunnel("api.example.com"));
        assert!(s.learn("api.example.com"));
        assert!(s.should_tunnel("api.example.com"));
    }

    #[test]
    fn learn_reports_only_the_first_sighting() {
        let s = NoMitmSet::new();
        assert!(s.learn("api.example.com"));
        assert!(!s.learn("api.example.com"));
    }

    #[test]
    fn learned_matching_is_case_insensitive() {
        // SNI casing is not normalised by clients; treating API.Example.com
        // and api.example.com as different hosts would mean paying the
        // sacrificial connection twice for one server.
        let s = NoMitmSet::new();
        s.learn("API.Example.com");
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
}
