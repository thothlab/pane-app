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

use pane_storage::{ActiveRule, ConditionOp, RuleCondition};

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
    // Body-based matchers: the nested subset template and the predicate
    // conditions. Both need the parsed JSON body, so parse it once. A request
    // whose body isn't JSON (or fails to parse) fails closed when either
    // matcher is active. (NULL/empty already collapsed upstream.)
    if rule.req_body_match.is_some() || !rule.conditions.is_empty() {
        let is_json = req
            .content_type
            .map(|ct| ct.contains("json"))
            .unwrap_or(false);
        if !is_json {
            return false;
        }
        let actual: serde_json::Value = match serde_json::from_slice(req.body) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if let Some(template) = &rule.req_body_match {
            if !json_subset(template, &actual) {
                return false;
            }
        }
        for cond in &rule.conditions {
            if !eval_condition(cond, &actual) {
                return false;
            }
        }
    }
    true
}

/// Evaluate one predicate condition against the parsed request body. The
/// field is resolved by dot/bracket path; a missing field, a type that can't
/// satisfy the operator, or a non-numeric side for a numeric op all fail
/// closed (the rule won't match) — consistent with the rest of the matcher.
fn eval_condition(cond: &RuleCondition, body: &serde_json::Value) -> bool {
    let actual = match lookup_body_path(body, &cond.path) {
        Some(v) => v,
        None => return false,
    };
    match cond.op {
        ConditionOp::Gt | ConditionOp::Gte | ConditionOp::Lt | ConditionOp::Lte => {
            let (a, b) = match (json_as_f64(actual), cond.value.trim().parse::<f64>().ok()) {
                (Some(a), Some(b)) => (a, b),
                _ => return false,
            };
            match cond.op {
                ConditionOp::Gt => a > b,
                ConditionOp::Gte => a >= b,
                ConditionOp::Lt => a < b,
                ConditionOp::Lte => a <= b,
                _ => unreachable!(),
            }
        }
        ConditionOp::Eq | ConditionOp::Ne => {
            // Numeric equality when both sides look numeric, else exact string.
            // (Float `==` is fragile by nature; ranges via gte/lte are safer.)
            let eq = match (json_as_f64(actual), cond.value.trim().parse::<f64>().ok()) {
                (Some(a), Some(b)) => a == b,
                _ => json_as_string(actual).as_deref() == Some(cond.value.as_str()),
            };
            if cond.op == ConditionOp::Eq {
                eq
            } else {
                !eq
            }
        }
        // Case-insensitive substring. Containers have no scalar form → no match.
        ConditionOp::Contains => match json_as_string(actual) {
            Some(s) => s.to_lowercase().contains(&cond.value.to_lowercase()),
            None => false,
        },
    }
}

/// Resolve a dot/bracket path against the request body JSON. A leading
/// `body.` is optional (mirrors how patch paths are written). Each segment is
/// an optional object key followed by zero+ `[index]` suffixes
/// (`items[0].sum`). Missing keys / out-of-range indices → `None`.
fn lookup_body_path<'a>(body: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let path = path.strip_prefix("body.").unwrap_or(path);
    let mut cur = body;
    for raw_seg in path.split('.') {
        let mut chars = raw_seg.chars().peekable();
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '[' {
                break;
            }
            key.push(c);
            chars.next();
        }
        if !key.is_empty() {
            cur = cur.get(&key)?;
        }
        while chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            let mut idx = String::new();
            for c in chars.by_ref() {
                if c == ']' {
                    break;
                }
                idx.push(c);
            }
            let n: usize = idx.parse().ok()?;
            cur = cur.get(n)?;
        }
    }
    Some(cur)
}

/// Numeric view of a JSON value: a Number yields its f64; a String is parsed
/// (money often arrives as `"1000.00"`); anything else → `None`.
fn json_as_f64(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Scalar string view for `eq`/`ne` fallback and `contains`. Containers have
/// no meaningful scalar form.
fn json_as_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => Some("null".to_string()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => None,
    }
}

/// Deep "is `template` a subset of `actual`?" used by the request-body
/// matcher. Objects: every key in `template` must exist in `actual` and
/// recurse (extra keys in `actual` are fine). Arrays: positional prefix —
/// `actual` must be at least as long and each `template[i]` subset-matches
/// `actual[i]`. Scalars (and type mismatches): exact equality.
///
/// Note: `serde_json` treats integer `0` and float `0.0` as unequal
/// Numbers. Template and request are serialized by the same client, so
/// the numeric form is consistent in practice; this is a known edge, not
/// a bug to paper over here.
fn json_subset(template: &serde_json::Value, actual: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (template, actual) {
        (Value::Object(t), Value::Object(a)) => t
            .iter()
            .all(|(k, tv)| a.get(k).is_some_and(|av| json_subset(tv, av))),
        (Value::Array(t), Value::Array(a)) => {
            t.len() <= a.len() && t.iter().zip(a.iter()).all(|(tv, av)| json_subset(tv, av))
        }
        (t, a) => t == a,
    }
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
            req_body_match: None,
            conditions: vec![],
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

    fn rule_body(template: &str) -> ActiveRule {
        let mut r = rule(vec![]);
        r.req_body_match = Some(serde_json::from_str(template).unwrap());
        r
    }

    fn json_post<'a>(body: &'a [u8]) -> RequestSummary<'a> {
        RequestSummary {
            host: "x",
            method: "POST",
            path: "/x",
            body,
            content_type: Some("application/json"),
        }
    }

    #[test]
    fn body_subset_matches_nested_with_extras() {
        let r = rule_body(r#"{"a":{"b":1}}"#);
        assert!(rule_matches(
            &r,
            &json_post(br#"{"a":{"b":1,"c":2},"d":3}"#)
        ));
    }

    #[test]
    fn body_subset_missing_nested_key_fails() {
        let r = rule_body(r#"{"a":{"b":1}}"#);
        assert!(!rule_matches(&r, &json_post(br#"{"a":{"c":2}}"#)));
    }

    #[test]
    fn body_array_positional_prefix() {
        let r = rule_body(r#"{"ids":[1,2]}"#);
        assert!(rule_matches(&r, &json_post(br#"{"ids":[1,2,3]}"#)));
        // Template longer than actual → no match.
        let r2 = rule_body(r#"{"ids":[1,2,3]}"#);
        assert!(!rule_matches(&r2, &json_post(br#"{"ids":[1,2]}"#)));
    }

    #[test]
    fn body_empty_object_matches_any_object() {
        let r = rule_body("{}");
        assert!(rule_matches(&r, &json_post(br#"{"anything":true}"#)));
    }

    #[test]
    fn body_match_fails_closed_on_non_json() {
        let r = rule_body(r#"{"a":1}"#);
        let req = RequestSummary {
            host: "x",
            method: "POST",
            path: "/x",
            body: b"not json",
            content_type: Some("text/plain"),
        };
        assert!(!rule_matches(&r, &req));
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

    fn cond(path: &str, op: ConditionOp, value: &str) -> RuleCondition {
        RuleCondition {
            path: path.into(),
            op,
            value: value.into(),
        }
    }

    fn rule_conds(conditions: Vec<RuleCondition>) -> ActiveRule {
        let mut r = rule(vec![]);
        r.conditions = conditions;
        r
    }

    #[test]
    fn cond_numeric_range_two_rows() {
        // amount between 1000 and 1500 (inclusive) = two AND-ed conditions.
        let r = rule_conds(vec![
            cond("amount", ConditionOp::Gte, "1000"),
            cond("amount", ConditionOp::Lte, "1500"),
        ]);
        assert!(rule_matches(&r, &json_post(br#"{"amount":1200}"#)));
        assert!(rule_matches(&r, &json_post(br#"{"amount":1000}"#))); // boundary
        assert!(rule_matches(&r, &json_post(br#"{"amount":1500}"#))); // boundary
        assert!(!rule_matches(&r, &json_post(br#"{"amount":999}"#)));
        assert!(!rule_matches(&r, &json_post(br#"{"amount":1501}"#)));
    }

    #[test]
    fn cond_numeric_coerces_string_money() {
        // amount arrives as a quoted decimal string.
        let r = rule_conds(vec![cond("amount", ConditionOp::Gt, "1000")]);
        assert!(rule_matches(&r, &json_post(br#"{"amount":"1200.50"}"#)));
        assert!(!rule_matches(&r, &json_post(br#"{"amount":"900.00"}"#)));
    }

    #[test]
    fn cond_nested_path_and_array_index() {
        let r = rule_conds(vec![cond("payment.items[0].sum", ConditionOp::Gte, "50")]);
        assert!(rule_matches(
            &r,
            &json_post(br#"{"payment":{"items":[{"sum":75}]}}"#)
        ));
        assert!(!rule_matches(
            &r,
            &json_post(br#"{"payment":{"items":[{"sum":10}]}}"#)
        ));
    }

    #[test]
    fn cond_missing_field_fails_closed() {
        let r = rule_conds(vec![cond("amount", ConditionOp::Gte, "1000")]);
        assert!(!rule_matches(&r, &json_post(br#"{"other":1}"#)));
    }

    #[test]
    fn cond_string_contains_is_case_insensitive() {
        let r = rule_conds(vec![cond("user.status", ConditionOp::Contains, "active")]);
        assert!(rule_matches(
            &r,
            &json_post(br#"{"user":{"status":"ACTIVE_NOW"}}"#)
        ));
        assert!(!rule_matches(
            &r,
            &json_post(br#"{"user":{"status":"blocked"}}"#)
        ));
    }

    #[test]
    fn cond_eq_and_ne() {
        let eq = rule_conds(vec![cond("type", ConditionOp::Eq, "card")]);
        assert!(rule_matches(&eq, &json_post(br#"{"type":"card"}"#)));
        assert!(!rule_matches(&eq, &json_post(br#"{"type":"cash"}"#)));
        let ne = rule_conds(vec![cond("type", ConditionOp::Ne, "card")]);
        assert!(rule_matches(&ne, &json_post(br#"{"type":"cash"}"#)));
        assert!(!rule_matches(&ne, &json_post(br#"{"type":"card"}"#)));
    }

    #[test]
    fn cond_fails_closed_on_non_json() {
        let r = rule_conds(vec![cond("amount", ConditionOp::Gte, "1000")]);
        let req = RequestSummary {
            host: "x",
            method: "POST",
            path: "/x",
            body: b"amount=1200",
            content_type: Some("text/plain"),
        };
        assert!(!rule_matches(&r, &req));
    }
}
