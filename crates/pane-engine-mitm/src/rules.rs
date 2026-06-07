//! Rule matching for response stubbing.
//!
//! Cheap, regex-free glob matcher: `*` greedily matches any run of chars
//! (including `/` in paths and `.` in hostnames). Anchors are implicit — the
//! pattern is matched against the whole input. Other glob metacharacters
//! (`?`, `[abc]`) are not supported; the input UI offers only `*`.
//!
//! Params matcher: every (name, value) listed on the rule must be present in
//! either the request's query string OR (for JSON bodies) the top-level
//! fields of the parsed body. Extras in the request are allowed. The matcher
//! stringifies JSON numbers/booleans before comparing.

use pane_storage::ActiveRule;

pub struct RequestSummary<'a> {
    pub host: &'a str,
    pub method: &'a str,
    pub path: &'a str,
    /// Raw request body bytes (may be empty).
    pub body: &'a [u8],
    /// Value of the Content-Type header (lowercased), if any. Used to decide
    /// whether to attempt JSON parsing of `body` for param matching.
    pub content_type: Option<&'a str>,
}

/// Walk active rules in priority order; return the first match. Rules are
/// already sorted (priority ASC, created_at ASC) by storage.
pub fn first_match<'a>(rules: &'a [ActiveRule], req: RequestSummary<'_>) -> Option<&'a ActiveRule> {
    rules.iter().find(|r| rule_matches(r, &req))
}

fn rule_matches(rule: &ActiveRule, req: &RequestSummary<'_>) -> bool {
    if let Some(host) = &rule.host_glob {
        if !glob_matches(host, req.host) {
            return false;
        }
    }
    if let Some(method) = &rule.method {
        if !method.eq_ignore_ascii_case(req.method) {
            return false;
        }
    }
    let (path_only, query_str) = split_path_query(req.path);
    if let Some(path) = &rule.path_glob {
        if !glob_matches(path, path_only) {
            return false;
        }
    }
    if !rule.params.is_empty() {
        let query_pairs = parse_query(query_str);
        let body_pairs = parse_json_top_level(req);
        for (n, v) in &rule.params {
            let in_query = query_pairs.iter().any(|(qn, qv)| qn == n && qv == v);
            let in_body = body_pairs.iter().any(|(bn, bv)| bn == n && bv == v);
            if !in_query && !in_body {
                return false;
            }
        }
    }
    true
}

/// Parse the request body as JSON and flatten its top-level object into
/// `(name, stringified_value)` pairs. Returns an empty vec when:
/// - the body is empty,
/// - Content-Type is not JSON-ish,
/// - the body fails to parse as JSON,
/// - or the JSON root is not an object.
fn parse_json_top_level(req: &RequestSummary<'_>) -> Vec<(String, String)> {
    if req.body.is_empty() {
        return Vec::new();
    }
    let is_json = req
        .content_type
        .map(|ct| ct.contains("json"))
        .unwrap_or(false);
    if !is_json {
        return Vec::new();
    }
    let v: serde_json::Value = match serde_json::from_slice(req.body) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let obj = match v.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };
    obj.iter()
        .filter_map(|(k, val)| {
            let s = match val {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Null => "null".to_string(),
                // Arrays/objects don't have a meaningful name=value form.
                serde_json::Value::Array(_) | serde_json::Value::Object(_) => return None,
            };
            Some((k.clone(), s))
        })
        .collect()
}

fn split_path_query(p: &str) -> (&str, &str) {
    match p.find('?') {
        Some(i) => (&p[..i], &p[i + 1..]),
        None => (p, ""),
    }
}

fn parse_query(q: &str) -> Vec<(String, String)> {
    if q.is_empty() {
        return Vec::new();
    }
    q.split('&')
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    // Minimal decoder for common cases — full URL-decoding is not needed
    // because the matcher compares pair-by-pair; both sides go through the
    // same function so even partial decoding is symmetric.
    let mut out = String::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        match b {
            b'+' => out.push(' '),
            b'%' => {
                let h = bytes.next();
                let l = bytes.next();
                if let (Some(h), Some(l)) = (h, l) {
                    let hi = hex_val(h);
                    let lo = hex_val(l);
                    if let (Some(hi), Some(lo)) = (hi, lo) {
                        out.push((hi * 16 + lo) as char);
                        continue;
                    }
                }
                out.push('%');
            }
            _ => out.push(b as char),
        }
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Greedy glob with `*` ⇒ `.*`. Anchored on both ends. Used for host and
/// path matching. ~20 lines, no dep.
pub fn glob_matches(pattern: &str, input: &str) -> bool {
    let pat: Vec<&str> = pattern.split('*').collect();
    if pat.len() == 1 {
        return pattern == input;
    }
    let mut cursor = 0usize;
    let first = pat[0];
    if !input[cursor..].starts_with(first) {
        return false;
    }
    cursor += first.len();
    let last = pat[pat.len() - 1];
    for mid in &pat[1..pat.len() - 1] {
        if mid.is_empty() {
            continue;
        }
        match input[cursor..].find(mid) {
            Some(off) => cursor += off + mid.len(),
            None => return false,
        }
    }
    if last.is_empty() {
        return true;
    }
    if input.len() < cursor + last.len() {
        return false;
    }
    let tail_start = input.len() - last.len();
    tail_start >= cursor && &input[tail_start..] == last
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn glob_basics() {
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("rc1.test.dev-og.com", "rc1.test.dev-og.com"));
        assert!(glob_matches("*.dev-og.com", "rc1.test.dev-og.com"));
        assert!(glob_matches("/api/v1/*", "/api/v1/document"));
        assert!(!glob_matches("/api/v1/*", "/api/v2/document"));
        assert!(glob_matches("/api/*/document", "/api/v1/document"));
        assert!(glob_matches("*document*", "/api/v1/document?x=1"));
    }

    fn rule(params: Vec<(&str, &str)>) -> ActiveRule {
        ActiveRule {
            id: Uuid::new_v4(),
            name: "test".into(),
            priority: 0,
            mode: pane_storage::RuleMode::Stub,
            patches: vec![],
            host_glob: None,
            method: None,
            path_glob: None,
            params: params
                .into_iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect(),
            status: 200,
            headers: vec![],
            body: vec![],
            body_mime: None,
            delay_ms: 0,
        }
    }

    #[test]
    fn params_match_query() {
        let r = rule(vec![("login", "root")]);
        let req = RequestSummary {
            host: "x",
            method: "GET",
            path: "/api/auth?login=root&extra=1",
            body: b"",
            content_type: None,
        };
        assert!(rule_matches(&r, &req));
    }

    #[test]
    fn params_match_json_body_string() {
        let r = rule(vec![("login", "root")]);
        let req = RequestSummary {
            host: "x",
            method: "POST",
            path: "/api/auth",
            body: br#"{"login":"root","password":"x"}"#,
            content_type: Some("application/json; charset=utf-8"),
        };
        assert!(rule_matches(&r, &req));
    }

    #[test]
    fn params_match_json_body_bool_and_number() {
        let r = rule(vec![("force", "true"), ("count", "42")]);
        let req = RequestSummary {
            host: "x",
            method: "POST",
            path: "/x",
            body: br#"{"force":true,"count":42}"#,
            content_type: Some("application/json"),
        };
        assert!(rule_matches(&r, &req));
    }

    #[test]
    fn params_miss_when_value_differs() {
        let r = rule(vec![("login", "root")]);
        let req = RequestSummary {
            host: "x",
            method: "POST",
            path: "/x",
            body: br#"{"login":"guest"}"#,
            content_type: Some("application/json"),
        };
        assert!(!rule_matches(&r, &req));
    }

    #[test]
    fn params_skipped_when_not_json_body() {
        let r = rule(vec![("login", "root")]);
        let req = RequestSummary {
            host: "x",
            method: "POST",
            path: "/x",
            body: b"login=root&extra=1",
            content_type: Some("application/x-www-form-urlencoded"),
        };
        // Form-urlencoded bodies aren't parsed; param must come from query.
        assert!(!rule_matches(&r, &req));
    }
}
