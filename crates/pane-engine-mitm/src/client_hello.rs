//! Reading the client's ClientHello *before* deciding what to do with it.
//!
//! Two problems come from handing the socket straight to `TlsAcceptor`.
//!
//! The first is that trust is a property of the client, not of the host, and by
//! the time we learn a client rejects our certificate we only know which host
//! it was talking to. A phone runs several TLS clients against one API — a
//! debug build that trusts the user CA store, an analytics SDK that doesn't, a
//! security module with its own anchors — so a verdict recorded per host
//! switches decryption off for all of them because one of them said no.
//!
//! The second is that the losing connection is unrecoverable: `accept()` has
//! already consumed the ClientHello, so we cannot fall back to a blind tunnel
//! for it.
//!
//! `TcpStream::peek` copies bytes without removing them from the socket, so we
//! can parse the ClientHello, fingerprint the client, and still hand an
//! untouched stream to either the TLS acceptor or `copy_bidirectional`. The
//! first contact with an unknown client still costs one broken request — we
//! cannot know its verdict before it gives one — but from then on that client
//! is tunnelled while every other client on the device keeps being decrypted.
//!
//! The fingerprint is JA3: TLS version, cipher suites, extension types,
//! supported groups and EC point formats, joined and hashed. It identifies the
//! *stack* (conscrypt vs BoringSSL vs a bundled OpenSSL vs Go), not the app, so
//! two apps built on the same stack share one. That bounds what this can do —
//! it separates an SDK with its own TLS from the host app, not a debug build
//! from a release build of the same code. Where it can't separate, behaviour is
//! exactly what it was before: one key per host.

use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::net::TcpStream;

/// A ClientHello fits comfortably here; anything larger is not one we can use.
const PEEK_LIMIT: usize = 8192;

/// How long to wait for the client to say something after our `200`. Clients
/// that opened a tunnel and then sit idle (connection pools do this) must not
/// hold a task hostage.
const PEEK_TIMEOUT: Duration = Duration::from_secs(5);

/// Pause between peeks while a ClientHello is still arriving.
const POLL_PAUSE: Duration = Duration::from_millis(20);

/// GREASE values (RFC 8701) are deliberately random per connection, so they
/// must be filtered out or every connection gets its own fingerprint.
fn is_grease(v: u16) -> bool {
    v & 0x0f0f == 0x0a0a && (v >> 8) == (v & 0xff)
}

/// What the peek learned about the client: who it is, and what protocols it is
/// willing to speak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeekedClient {
    pub fingerprint: ClientFingerprint,
    /// ALPN protocols the client offered, in its own order. Empty when it
    /// offered none, which means it will accept whatever we pick.
    pub alpn: Vec<String>,
}

impl PeekedClient {
    /// Can we negotiate the only protocol we can decrypt?
    ///
    /// Pane parses HTTP/1.1 and restricts its ALPN to it. A client that offers
    /// only `h2` cannot be answered — rustls fails the handshake with
    /// `no_application_protocol` — so trying is a guaranteed broken request.
    /// Offering nothing at all is not a refusal: such a client takes whatever
    /// we choose.
    pub fn can_negotiate_http11(&self) -> bool {
        self.alpn.is_empty() || self.alpn.iter().any(|p| p == "http/1.1")
    }
}

/// Identifies the client's TLS stack. Empty when the bytes weren't a
/// ClientHello we could read.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientFingerprint(String);

impl ClientFingerprint {
    /// Short hex digest — long enough not to collide, short enough to log.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The stand-in used when the ClientHello couldn't be parsed. Keying under
    /// it means such clients share one bucket, which is the old per-host
    /// behaviour and the right fallback.
    pub fn unknown() -> Self {
        Self("unknown".to_string())
    }
}

/// Peek at what the client sent and fingerprint it, without consuming anything.
///
/// Returns `unknown()` for a client that sends no TLS, sends it in pieces we
/// gave up waiting for, or sends something we can't parse — all of which are
/// reasons to fall back to the previous behaviour rather than to guess.
pub(crate) async fn peek_client(stream: &TcpStream) -> PeekedClient {
    let unknown = || PeekedClient {
        fingerprint: ClientFingerprint::unknown(),
        alpn: Vec::new(),
    };
    let mut buf = vec![0u8; PEEK_LIMIT];
    let deadline = tokio::time::Instant::now() + PEEK_TIMEOUT;

    loop {
        let peeked = match tokio::time::timeout_at(deadline, stream.peek(&mut buf)).await {
            Ok(Ok(0)) => return unknown(), // client hung up
            Ok(Ok(n)) => n,
            Ok(Err(_)) | Err(_) => return unknown(),
        };
        match parse_client_hello(&buf[..peeked]) {
            Ok(hello) => {
                return PeekedClient {
                    fingerprint: fingerprint_of(&hello.ja3),
                    alpn: hello.alpn,
                }
            }
            // Not enough bytes yet: the record is split across packets. Wait
            // for more rather than fingerprinting half a handshake.
            Err(ParseError::Incomplete) => {
                if peeked >= PEEK_LIMIT {
                    return unknown();
                }
                // `peek` is non-destructive, so it returns the same partial
                // record immediately — without a pause this would spin a core
                // until the deadline. Sleep a beat and let the rest arrive.
                if tokio::time::timeout_at(deadline, tokio::time::sleep(POLL_PAUSE))
                    .await
                    .is_err()
                {
                    return unknown();
                }
            }
            Err(ParseError::NotAClientHello) => return unknown(),
        }
    }
}

fn fingerprint_of(ja3: &str) -> ClientFingerprint {
    let digest = Sha256::digest(ja3.as_bytes());
    ClientFingerprint(hex::encode(&digest[..8]))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParseError {
    /// Valid so far, just truncated — worth waiting for more bytes.
    Incomplete,
    /// Not a TLS ClientHello at all.
    NotAClientHello,
}

/// Cursor that reports truncation instead of panicking, since half a
/// ClientHello is the expected case on the first packet.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }
    fn u8(&mut self) -> Result<u8, ParseError> {
        let v = *self.b.get(self.at).ok_or(ParseError::Incomplete)?;
        self.at += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16, ParseError> {
        Ok(((self.u8()? as u16) << 8) | self.u8()? as u16)
    }
    fn skip(&mut self, n: usize) -> Result<(), ParseError> {
        if self.at + n > self.b.len() {
            return Err(ParseError::Incomplete);
        }
        self.at += n;
        Ok(())
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], ParseError> {
        let end = self.at + n;
        let slice = self.b.get(self.at..end).ok_or(ParseError::Incomplete)?;
        self.at = end;
        Ok(slice)
    }
}

fn u16s(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|c| ((c[0] as u16) << 8) | c[1] as u16)
        .filter(|v| !is_grease(*v))
        .collect()
}

fn join(values: &[u16]) -> String {
    values
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("-")
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedHello {
    pub ja3: String,
    pub alpn: Vec<String>,
}

/// Parse the JA3 string (`version,ciphers,extensions,groups,point_formats`)
/// and the ALPN list out of a ClientHello.
pub(crate) fn parse_client_hello(bytes: &[u8]) -> Result<ParsedHello, ParseError> {
    let mut r = Reader::new(bytes);

    // TLS record header. A non-handshake record here means the client is not
    // starting a TLS session — plain HTTP inside CONNECT, or a protocol we
    // don't handle.
    if r.u8()? != 0x16 {
        return Err(ParseError::NotAClientHello);
    }
    r.skip(2)?; // record version — legacy, always 0x0301 in practice
    let record_len = r.u16()? as usize;
    if bytes.len() < 5 + record_len {
        return Err(ParseError::Incomplete);
    }

    if r.u8()? != 0x01 {
        return Err(ParseError::NotAClientHello);
    }
    r.skip(3)?; // handshake length

    // `version` in JA3 is the legacy ClientHello version; TLS 1.3 carries the
    // real one in the supported_versions extension and pins this to 0x0303.
    let version = r.u16()?;
    r.skip(32)?; // random

    let session_id_len = r.u8()? as usize;
    r.skip(session_id_len)?;

    let cipher_len = r.u16()? as usize;
    let ciphers = u16s(r.take(cipher_len)?);

    let compression_len = r.u8()? as usize;
    r.skip(compression_len)?;

    // Extensions are optional in the wire format (SSLv3-era clients omit the
    // block entirely); JA3 treats that as three empty fields.
    let mut extensions: Vec<u16> = Vec::new();
    let mut groups: Vec<u16> = Vec::new();
    let mut formats: Vec<u16> = Vec::new();
    let mut alpn: Vec<String> = Vec::new();
    if r.at < bytes.len() {
        let ext_total = r.u16()? as usize;
        let end = r.at + ext_total;
        while r.at < end {
            let ext_type = r.u16()?;
            let ext_len = r.u16()? as usize;
            let data = r.take(ext_len)?;
            if is_grease(ext_type) {
                continue;
            }
            extensions.push(ext_type);
            match ext_type {
                // supported_groups: 2-byte list length, then the groups.
                10 if data.len() >= 2 => groups = u16s(&data[2..]),
                // ec_point_formats: 1-byte list length, then 1-byte formats.
                11 if !data.is_empty() => {
                    formats = data[1..].iter().map(|b| *b as u16).collect();
                }
                // ALPN: 2-byte list length, then length-prefixed names.
                16 if data.len() >= 2 => alpn = parse_alpn(&data[2..]),
                _ => {}
            }
        }
    }

    Ok(ParsedHello {
        ja3: format!(
            "{version},{},{},{},{}",
            join(&ciphers),
            join(&extensions),
            join(&groups),
            join(&formats)
        ),
        alpn,
    })
}

/// Length-prefixed protocol names. A malformed list yields whatever parsed
/// cleanly before it — ALPN only ever gates a fallback, never a refusal, so
/// being generous here costs nothing.
fn parse_alpn(mut bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    while let Some((&len, rest)) = bytes.split_first() {
        let len = len as usize;
        if rest.len() < len {
            break;
        }
        if let Ok(name) = std::str::from_utf8(&rest[..len]) {
            out.push(name.to_string());
        }
        bytes = &rest[len..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal but wire-accurate ClientHello builder.
    fn client_hello(ciphers: &[u16], exts: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x03, 0x03]); // legacy_version
        body.extend_from_slice(&[0u8; 32]); // random
        body.push(0); // session_id length
        body.extend_from_slice(&((ciphers.len() * 2) as u16).to_be_bytes());
        for c in ciphers {
            body.extend_from_slice(&c.to_be_bytes());
        }
        body.extend_from_slice(&[1, 0]); // compression: 1 method, null

        let mut ext_block = Vec::new();
        for (t, data) in exts {
            ext_block.extend_from_slice(&t.to_be_bytes());
            ext_block.extend_from_slice(&(data.len() as u16).to_be_bytes());
            ext_block.extend_from_slice(data);
        }
        body.extend_from_slice(&(ext_block.len() as u16).to_be_bytes());
        body.extend_from_slice(&ext_block);

        let mut hs = vec![0x01];
        let len = body.len();
        hs.extend_from_slice(&[(len >> 16) as u8, (len >> 8) as u8, len as u8]);
        hs.extend_from_slice(&body);

        let mut rec = vec![0x16, 0x03, 0x01];
        rec.extend_from_slice(&(hs.len() as u16).to_be_bytes());
        rec.extend_from_slice(&hs);
        rec
    }

    #[test]
    fn builds_a_ja3_string() {
        let hello = client_hello(
            &[0x1301, 0x1302],
            &[
                (0, b"\x00\x0e\x00\x00\x0bexample.com".to_vec()), // server_name
                (10, vec![0x00, 0x04, 0x00, 0x1d, 0x00, 0x17]),   // groups
                (11, vec![0x01, 0x00]),                           // point formats
            ],
        );
        assert_eq!(
            parse_client_hello(&hello).unwrap().ja3,
            "771,4865-4866,0-10-11,29-23,0"
        );
    }

    #[test]
    fn two_stacks_fingerprint_differently() {
        // The whole point: an SDK with its own TLS must not share a bucket with
        // the app's OkHttp, or one rejecting the CA silences the other.
        let a = client_hello(&[0x1301, 0x1302], &[(0, vec![]), (10, vec![0, 2, 0, 29])]);
        let b = client_hello(&[0xc02b, 0xc02f], &[(0, vec![]), (23, vec![])]);
        assert_ne!(
            parse_client_hello(&a).unwrap().ja3,
            parse_client_hello(&b).unwrap().ja3
        );
        assert_ne!(
            fingerprint_of(&parse_client_hello(&a).unwrap().ja3),
            fingerprint_of(&parse_client_hello(&b).unwrap().ja3)
        );
    }

    #[test]
    fn the_same_stack_fingerprints_identically_across_connections() {
        // Randomness lives in `random` and the session id, neither of which
        // JA3 reads — otherwise every connection would look like a new client.
        let mut a = client_hello(&[0x1301], &[(0, vec![])]);
        let b = client_hello(&[0x1301], &[(0, vec![])]);
        // record(5) + handshake header(4) + legacy_version(2) = 11
        a[11..43].copy_from_slice(&[0xab; 32]); // different random
        assert_eq!(
            parse_client_hello(&a).unwrap().ja3,
            parse_client_hello(&b).unwrap().ja3
        );
    }

    #[test]
    fn grease_is_ignored() {
        // RFC 8701 values are random per connection by design.
        let plain = client_hello(&[0x1301], &[(10, vec![0, 2, 0, 29])]);
        let greased = client_hello(
            &[0x0a0a, 0x1301],
            &[(0x1a1a, vec![]), (10, vec![0, 2, 0, 29])],
        );
        assert_eq!(
            parse_client_hello(&plain).unwrap().ja3,
            parse_client_hello(&greased).unwrap().ja3
        );
    }

    #[test]
    fn a_truncated_hello_asks_for_more_bytes() {
        // The first packet often carries only part of the record; treating that
        // as "not TLS" would fingerprint every client as unknown.
        let hello = client_hello(&[0x1301, 0x1302], &[(0, vec![]), (10, vec![0, 2, 0, 29])]);
        for cut in [5, 10, 40, hello.len() - 1] {
            assert_eq!(
                parse_client_hello(&hello[..cut]),
                Err(ParseError::Incomplete),
                "cut at {cut}"
            );
        }
        assert!(parse_client_hello(&hello).is_ok());
    }

    #[test]
    fn non_tls_bytes_are_rejected_outright() {
        // A client that speaks plain HTTP inside CONNECT is not worth waiting
        // for — there is no ClientHello coming.
        assert_eq!(
            parse_client_hello(b"GET / HTTP/1.1\r\n"),
            Err(ParseError::NotAClientHello)
        );
        let mut not_hello = client_hello(&[0x1301], &[]);
        not_hello[5] = 0x02; // ServerHello
        assert_eq!(
            parse_client_hello(&not_hello),
            Err(ParseError::NotAClientHello)
        );
    }

    #[test]
    fn a_hello_without_extensions_still_parses() {
        let hello = client_hello(&[0x1301], &[]);
        assert_eq!(parse_client_hello(&hello).unwrap().ja3, "771,4865,,,");
    }

    /// ALPN extension body: 2-byte list length, then length-prefixed names.
    fn alpn_ext(protocols: &[&str]) -> (u16, Vec<u8>) {
        let mut list = Vec::new();
        for p in protocols {
            list.push(p.len() as u8);
            list.extend_from_slice(p.as_bytes());
        }
        let mut data = (list.len() as u16).to_be_bytes().to_vec();
        data.extend_from_slice(&list);
        (16, data)
    }

    fn peeked(protocols: &[&str]) -> PeekedClient {
        let hello = client_hello(&[0x1301], &[alpn_ext(protocols)]);
        let parsed = parse_client_hello(&hello).unwrap();
        PeekedClient {
            fingerprint: fingerprint_of(&parsed.ja3),
            alpn: parsed.alpn,
        }
    }

    #[test]
    fn alpn_names_are_read_in_order() {
        assert_eq!(peeked(&["h2", "http/1.1"]).alpn, vec!["h2", "http/1.1"]);
        assert_eq!(peeked(&[]).alpn, Vec::<String>::new());
    }

    #[test]
    fn an_h2_only_client_cannot_negotiate_with_us() {
        // Pane parses HTTP/1.1 and offers only that, so rustls answers such a
        // client with no_application_protocol. Observed against
        // broker.sistema-capital.com: nine failed handshakes in a row, no
        // decryption and no tunnel — the client simply never worked.
        assert!(!peeked(&["h2"]).can_negotiate_http11());
        assert!(!peeked(&["h3", "h2"]).can_negotiate_http11());
    }

    #[test]
    fn offering_http11_anywhere_in_the_list_is_enough() {
        assert!(peeked(&["h2", "http/1.1"]).can_negotiate_http11());
        assert!(peeked(&["http/1.1"]).can_negotiate_http11());
    }

    #[test]
    fn offering_no_alpn_at_all_is_not_a_refusal() {
        // Such a client accepts whatever we pick, so it must still be decrypted.
        assert!(peeked(&[]).can_negotiate_http11());
    }

    #[test]
    fn fingerprints_are_short_and_stable() {
        let ja3 = "771,4865,0,29,0";
        assert_eq!(fingerprint_of(ja3), fingerprint_of(ja3));
        assert_eq!(fingerprint_of(ja3).as_str().len(), 16);
        assert_ne!(fingerprint_of(ja3), ClientFingerprint::unknown());
    }
}
