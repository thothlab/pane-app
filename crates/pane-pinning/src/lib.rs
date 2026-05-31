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
}
