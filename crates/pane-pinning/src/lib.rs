//! Cert pinning heuristic. Lightweight: matches a host against a baked-in
//! list of patterns and returns a `HintKind` describing why inspection failed.
//!
//! Real heuristic combines:
//!  - rapid client RST after our ServerHello,
//!  - TLS Alert (`certificate_unknown` 46 / `bad_certificate` 42),
//!  - host present in the known-pinned list.
//!
//! The engine calls `classify` once a handshake fails fast; the result decides
//! whether to record a `PinningIncident` and emit the UX-facing event.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HintKind {
    AppPin,
    SystemPin,
    CtRequired,
    Unknown,
}

impl HintKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            HintKind::AppPin => "app_pin",
            HintKind::SystemPin => "system_pin",
            HintKind::CtRequired => "ct_required",
            HintKind::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct HintEntry {
    pattern: String,
    hint: String,
}

static HINTS: Lazy<Vec<HintEntry>> = Lazy::new(|| {
    let baked = include_str!("../../../assets/pinned-hints.json");
    serde_json::from_str(baked).unwrap_or_default()
});

pub fn classify(host: &str) -> HintKind {
    for h in HINTS.iter() {
        if pattern_matches(&h.pattern, host) {
            return match h.hint.as_str() {
                "app_pin" => HintKind::AppPin,
                "system_pin" => HintKind::SystemPin,
                "ct_required" => HintKind::CtRequired,
                _ => HintKind::Unknown,
            };
        }
    }
    HintKind::Unknown
}

/// Whether the *application* pins certificates for this host, i.e. MITM is
/// hopeless no matter which client connects or what the device trusts.
///
/// This is the only hint class the proxy pre-seeds into its no-MITM set.
/// Deliberately narrower than "appears in the hint list at all":
///
///   - `app_pin` (Facebook, WhatsApp, banking apps) — the app ships its own
///     trust anchors and rejects ours unconditionally. Attempting MITM can
///     only ever produce a failed handshake, so skipping straight to a blind
///     tunnel costs nothing and saves the user a broken request.
///   - `system_pin` (`*.googleapis.com`, `*.apple.com`) — pinned by *system*
///     components, but an ordinary third-party app calling `googleapis.com`
///     through OkHttp decrypts perfectly well against a user-installed CA.
///     Pre-seeding these would silently stop decrypting traffic that works
///     today, which is a regression for the tool's whole purpose.
///   - `ct_required` (`*.gstatic.com`) — Certificate Transparency enforcement,
///     which in practice only Chrome applies. Same reasoning as above.
///
/// The last two still reach the no-MITM set, just via the learn-on-failure
/// path: one failed handshake and they're tunnelled from then on.
pub fn is_app_pinned(host: &str) -> bool {
    matches!(classify(host), HintKind::AppPin)
}

/// The `app_pin` patterns themselves, for the UI's "what is being tunnelled"
/// panel. These match by pattern rather than by hostname, so unlike learned
/// hosts they can't be forgotten — listing them is the only honest way to
/// explain why a host nobody learned is still going through a blind tunnel.
pub fn app_pinned_patterns() -> Vec<&'static str> {
    HINTS
        .iter()
        .filter(|h| h.hint == "app_pin")
        .map(|h| h.pattern.as_str())
        .collect()
}

fn pattern_matches(pattern: &str, host: &str) -> bool {
    if let Some(stripped) = pattern.strip_prefix("*.") {
        host.ends_with(stripped) && host.len() > stripped.len()
    } else {
        host == pattern
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        // depends on bundled list
        assert!(matches!(classify("api.facebook.com"), _));
    }

    #[test]
    fn unknown_host_returns_unknown() {
        assert!(matches!(classify("nope.invalid"), HintKind::Unknown));
    }

    #[test]
    fn app_pinned_hosts_are_seedable() {
        assert!(is_app_pinned("graph.facebook.com"));
        assert!(is_app_pinned("api.whatsapp.net"));
    }

    #[test]
    fn system_pinned_and_ct_hosts_are_not_seeded() {
        // These decrypt fine for ordinary apps against a user-installed CA;
        // pre-seeding them would hide working traffic. They only reach the
        // no-MITM set after an actual handshake failure.
        assert!(!is_app_pinned("www.googleapis.com"));
        assert!(!is_app_pinned("fonts.gstatic.com"));
        assert!(!is_app_pinned("init.itunes.apple.com"));
    }

    #[test]
    fn unlisted_hosts_are_not_seeded() {
        assert!(!is_app_pinned("api.example.com"));
    }
}
