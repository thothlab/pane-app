//! Rule matching for response stubbing.
//!
//! Cheap, regex-free glob matcher: `*` greedily matches any run of chars
//! (including `/` in paths and `.` in hostnames). Anchors are implicit — the
//! pattern is matched against the whole input. Other glob metacharacters
//! (`?`, `[abc]`) are not supported; the input UI offers only `*`.
//!
//! Query matcher: subset semantics. Every (name, value) listed on the rule
//! must appear in the request's query string. Extras in the request are
//! allowed. Duplicate entries on the rule require the value to appear that
//! many times in the request.

use pane_storage::ActiveRule;

pub struct RequestSummary<'a> {
    pub host: &'a str,
    pub method: &'a str,
    pub path: &'a str,
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
    if !rule.query.is_empty() {
        let pairs = parse_query(query_str);
        for required in &rule.query {
            let count_in_rule = rule
                .query
                .iter()
                .filter(|(n, v)| n == &required.0 && v == &required.1)
                .count();
            let count_in_req = pairs
                .iter()
                .filter(|(n, v)| n == &required.0 && v == &required.1)
                .count();
            if count_in_req < count_in_rule {
                return false;
            }
        }
    }
    true
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
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            Some((percent_decode(k), percent_decode(v)))
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
}
