//! Why a TLS handshake with the *client* failed.
//!
//! `TlsAcceptor::accept` hands back a plain `io::Error` for every failure mode
//! it has, and they mean opposite things:
//!
//!   - the client sent a TLS alert about our leaf — it looked at the
//!     certificate, didn't like it, and said so. That is a verdict about our
//!     CA and it will be the same verdict next time;
//!   - the socket died — the USB cable came out, `adb reverse` collapsed, the
//!     app was killed mid-handshake. That says nothing about the certificate.
//!
//! Treating the second as the first is what let a single cable-pull silently
//! stop Pane decrypting a host for the rest of the run. So we classify, and
//! only a real alert is taken as proof; transport failures merely accumulate
//! (see `NoMitmSet::note_io_failure`), which still catches the pinning clients
//! that RST without the courtesy of an alert.

use rustls::AlertDescription;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandshakeFailure {
    /// The client sent a TLS alert naming our certificate as the problem.
    CertRejected(&'static str),
    /// The connection died underneath us. No information about trust.
    Transport,
    /// A TLS-level failure that isn't a verdict on the certificate — garbage
    /// on the port, an incompatible peer, a protocol mismatch.
    Other,
}

impl HandshakeFailure {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            HandshakeFailure::CertRejected(_) => "cert_rejected",
            HandshakeFailure::Transport => "transport",
            HandshakeFailure::Other => "other",
        }
    }
}

/// Classify the error `TlsAcceptor::accept` returned.
pub(crate) fn classify(err: &std::io::Error) -> HandshakeFailure {
    if let Some(rustls_err) = find_rustls_error(err) {
        return match rustls_err {
            rustls::Error::AlertReceived(alert) => match cert_alert_name(*alert) {
                Some(name) => HandshakeFailure::CertRejected(name),
                // The client aborted for a reason unrelated to our leaf:
                // `no_application_protocol` (it wanted h2, which we don't
                // offer), a version mismatch, a user cancel.
                None => HandshakeFailure::Other,
            },
            _ => HandshakeFailure::Other,
        };
    }

    use std::io::ErrorKind::*;
    match err.kind() {
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof | NotConnected
        | TimedOut | Interrupted => HandshakeFailure::Transport,
        _ => HandshakeFailure::Other,
    }
}

/// Alerts that mean "I looked at your certificate and refused it".
///
/// `handshake_failure` is included deliberately: pinning stacks on Android
/// commonly send it instead of a certificate-specific alert. Over-including it
/// costs a blind tunnel for a host that might have decrypted — recoverable,
/// visible in Settings, and cleared by a proxy restart — whereas leaving it out
/// costs the user a broken app with no fallback.
fn cert_alert_name(alert: AlertDescription) -> Option<&'static str> {
    Some(match alert {
        AlertDescription::BadCertificate => "bad_certificate",
        AlertDescription::UnsupportedCertificate => "unsupported_certificate",
        AlertDescription::CertificateRevoked => "certificate_revoked",
        AlertDescription::CertificateExpired => "certificate_expired",
        AlertDescription::CertificateUnknown => "certificate_unknown",
        AlertDescription::UnknownCA => "unknown_ca",
        AlertDescription::AccessDenied => "access_denied",
        AlertDescription::CertificateRequired => "certificate_required",
        AlertDescription::BadCertificateStatusResponse => "bad_certificate_status_response",
        AlertDescription::HandshakeFailure => "handshake_failure",
        _ => return None,
    })
}

/// tokio-rustls wraps the rustls error in an `io::Error`; depending on the
/// path it can be the direct inner value or further down the source chain.
fn find_rustls_error(err: &std::io::Error) -> Option<&rustls::Error> {
    let mut source: Option<&(dyn std::error::Error + 'static)> = err.get_ref().map(|e| e as _);
    while let Some(e) = source {
        if let Some(r) = e.downcast_ref::<rustls::Error>() {
            return Some(r);
        }
        source = e.source();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind};

    fn alert(a: AlertDescription) -> Error {
        Error::new(ErrorKind::InvalidData, rustls::Error::AlertReceived(a))
    }

    #[test]
    fn certificate_alerts_are_a_verdict_on_our_leaf() {
        assert_eq!(
            classify(&alert(AlertDescription::CertificateUnknown)),
            HandshakeFailure::CertRejected("certificate_unknown")
        );
        assert_eq!(
            classify(&alert(AlertDescription::UnknownCA)),
            HandshakeFailure::CertRejected("unknown_ca")
        );
        assert_eq!(
            classify(&alert(AlertDescription::BadCertificate)),
            HandshakeFailure::CertRejected("bad_certificate")
        );
    }

    #[test]
    fn pinning_stacks_that_send_handshake_failure_are_believed() {
        assert_eq!(
            classify(&alert(AlertDescription::HandshakeFailure)),
            HandshakeFailure::CertRejected("handshake_failure")
        );
    }

    #[test]
    fn alerts_about_anything_else_are_not_a_cert_verdict() {
        // The client wanted h2 and we only offer http/1.1 — nothing to do with
        // trust, and tunnelling the host over it would hide working traffic.
        assert_eq!(
            classify(&alert(AlertDescription::NoApplicationProtocol)),
            HandshakeFailure::Other
        );
        assert_eq!(
            classify(&alert(AlertDescription::ProtocolVersion)),
            HandshakeFailure::Other
        );
    }

    #[test]
    fn a_yanked_cable_is_transport_not_rejection() {
        // The regression this module exists for: these used to be read as
        // "the client refused our certificate" and poisoned the host.
        for kind in [
            ErrorKind::ConnectionReset,
            ErrorKind::BrokenPipe,
            ErrorKind::UnexpectedEof,
            ErrorKind::ConnectionAborted,
            ErrorKind::TimedOut,
        ] {
            assert_eq!(
                classify(&Error::new(kind, "boom")),
                HandshakeFailure::Transport,
                "{kind:?} must not count as a certificate rejection"
            );
        }
    }

    #[test]
    fn non_tls_garbage_on_the_port_is_not_a_cert_verdict() {
        let err = Error::new(
            ErrorKind::InvalidData,
            rustls::Error::General("not a handshake".into()),
        );
        assert_eq!(classify(&err), HandshakeFailure::Other);
    }

    #[test]
    fn rustls_error_is_found_further_down_the_source_chain() {
        #[derive(Debug)]
        struct Wrapper(rustls::Error);
        impl std::fmt::Display for Wrapper {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "wrapped")
            }
        }
        impl std::error::Error for Wrapper {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let err = Error::new(
            ErrorKind::InvalidData,
            Wrapper(rustls::Error::AlertReceived(AlertDescription::UnknownCA)),
        );
        assert_eq!(classify(&err), HandshakeFailure::CertRejected("unknown_ca"));
    }
}
