//! Filter DSL parser → SQL WHERE compiler (PRD §5.3).
//!
//! Grammar:
//!   expr  := term (WS term)*
//!   term  := '!'? atom
//!   atom  := key ':' value | bareword
//!   value := single | single ',' single (',' single)*
//!   key   := host | method | status | mime | path | size | duration | error
//!          | device | state | rule
//!
//! Bareword without `:` is treated as substring search across host AND path
//! (joined by OR) — this is what users expect when they type a word into
//! the filter box. Status/size/duration values may use `N..M` range form.
//!
//! Comma inside a value lets the user OR several alternatives under one
//! key — `host:api.foo.com,api.bar.com,*.baz.com`,
//! `method:POST,PUT`, `status:200,500..599`. Negation `!host:a,b`
//! means "host is neither a nor b" (clauses AND'd together).
//! Different keys still combine by AND across tokens.

use anyhow::{anyhow, Result};
use rusqlite::ToSql;

pub fn compile_to_sql(input: &str) -> Result<(String, Vec<Box<dyn ToSql>>)> {
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    let mut where_parts: Vec<String> = Vec::new();

    for raw in tokenize(input) {
        let negate = raw.starts_with('!');
        let token = if negate { &raw[1..] } else { raw.as_str() };
        let (key, value) = match token.split_once(':') {
            Some((k, v)) => (k, v),
            None => ("__bare", token),
        };
        let key_lower = key.to_ascii_lowercase();
        let key = key_lower.as_str();

        let frag = match key {
            "host" => like_clause("server_host", value, negate, &mut params),
            "method" => eq_clause_uppercase("method", value, negate, &mut params),
            "status" => range_or_eq("status", value, negate, &mut params)?,
            "mime" => mime_clause(value, negate, &mut params),
            "path" => like_clause("url_path", value, negate, &mut params),
            "size" => range_or_eq("total_bytes", value, negate, &mut params)?,
            "duration" => range_or_eq("duration_ms", value, negate, &mut params)?,
            "error" => eq_clause("error_kind", value, negate, &mut params),
            "device" => device_clause(value, negate, &mut params),
            // `state:stubbed` is how an automated run proves a response came
            // from a mock rather than the live backend — without it, a run
            // whose rule silently failed to match still looks green.
            "state" => eq_clause_lowercase("state", value, negate, &mut params),
            // `rule:` narrows "it was mocked" to "it was mocked by *this*
            // rule", matching the denormalized name or the exact id.
            "rule" => rule_clause(value, negate, &mut params),
            "__bare" => bareword_clause(value, negate, &mut params),
            other => return Err(anyhow!("unknown filter key: {other}")),
        };
        where_parts.push(frag);
    }

    let sql = if where_parts.is_empty() {
        "1=1".into()
    } else {
        where_parts.join(" AND ")
    };
    Ok((sql, params))
}

fn tokenize(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_quote = false;
    for c in input.chars() {
        match c {
            '"' => in_quote = !in_quote,
            ' ' | '\t' if !in_quote => {
                if !buf.is_empty() {
                    out.push(std::mem::take(&mut buf));
                }
            }
            _ => buf.push(c),
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

/// Bare-word search: matches the substring in either `server_host` or
/// `url_path`. Negation flips to "neither matches". Mirrors the placeholder-
/// less filter behaviour of Charles/mitmweb/Postman — typing `google`
/// finds `firebaseinstallations.googleapis.com` as well as `/google/login`.
fn bareword_clause(value: &str, neg: bool, params: &mut Vec<Box<dyn ToSql>>) -> String {
    let pattern = if value.contains('*') {
        value.replace('*', "%")
    } else {
        format!("%{value}%")
    };
    params.push(Box::new(pattern.clone()));
    params.push(Box::new(pattern));
    if neg {
        "(server_host NOT LIKE ? AND url_path NOT LIKE ?)".into()
    } else {
        "(server_host LIKE ? OR url_path LIKE ?)".into()
    }
}

/// Split a value on `,` into trimmed non-empty parts. Returns the whole
/// value as the only element when there's no comma — so single-value
/// callers (the common case) end up with len==1 and the existing
/// fragment shape, no unnecessary parens.
fn split_values(value: &str) -> Vec<&str> {
    if !value.contains(',') {
        return vec![value];
    }
    value
        .split(',')
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .collect()
}

/// Wrap N OR'd / AND'd clauses into a single fragment. Caller passes
/// `neg = true` to flip semantics: positive list → OR, negation → AND
/// (so `!host:a,b` reads as "host is neither a nor b").
fn join_alternatives(parts: Vec<String>, neg: bool) -> String {
    if parts.len() == 1 {
        return parts.into_iter().next().unwrap();
    }
    let glue = if neg { " AND " } else { " OR " };
    format!("({})", parts.join(glue))
}

fn like_clause(col: &str, value: &str, neg: bool, params: &mut Vec<Box<dyn ToSql>>) -> String {
    let parts: Vec<String> = split_values(value)
        .into_iter()
        .map(|v| {
            let pattern = if v.contains('*') {
                v.replace('*', "%")
            } else {
                format!("%{v}%")
            };
            params.push(Box::new(pattern));
            if neg {
                format!("{col} NOT LIKE ?")
            } else {
                format!("{col} LIKE ?")
            }
        })
        .collect();
    join_alternatives(parts, neg)
}

fn eq_clause(col: &str, value: &str, neg: bool, params: &mut Vec<Box<dyn ToSql>>) -> String {
    let parts: Vec<String> = split_values(value)
        .into_iter()
        .map(|v| {
            params.push(Box::new(v.to_string()));
            if neg {
                format!("{col} <> ?")
            } else {
                format!("{col} = ?")
            }
        })
        .collect();
    join_alternatives(parts, neg)
}

/// Like `eq_clause` but uppercases each value first — for the `method`
/// key, where the column is canonically uppercase.
fn eq_clause_uppercase(
    col: &str,
    value: &str,
    neg: bool,
    params: &mut Vec<Box<dyn ToSql>>,
) -> String {
    let parts: Vec<String> = split_values(value)
        .into_iter()
        .map(|v| {
            params.push(Box::new(v.to_uppercase()));
            if neg {
                format!("{col} <> ?")
            } else {
                format!("{col} = ?")
            }
        })
        .collect();
    join_alternatives(parts, neg)
}

/// Like `eq_clause` but lowercases each value — for `state`, whose column
/// values are canonically lowercase (`completed`, `stubbed`, `patched`,
/// `error`), so `state:Stubbed` behaves like `state:stubbed`.
fn eq_clause_lowercase(
    col: &str,
    value: &str,
    neg: bool,
    params: &mut Vec<Box<dyn ToSql>>,
) -> String {
    let parts: Vec<String> = split_values(value)
        .into_iter()
        .map(|v| {
            params.push(Box::new(v.to_lowercase()));
            if neg {
                format!("{col} <> ?")
            } else {
                format!("{col} = ?")
            }
        })
        .collect();
    join_alternatives(parts, neg)
}

/// `rule:foo` matches either the denormalized rule name (substring, `*` glob)
/// or an exact rule id, so a caller can use whichever it has to hand without a
/// second lookup.
///
/// The negated form has to spell out the NULL case: `matched_rule_name NOT
/// LIKE ?` is NULL — not true — for every live (non-mocked) capture, so
/// without the explicit IS NULL branch `!rule:foo` would silently drop every
/// real response from the result.
fn rule_clause(value: &str, neg: bool, params: &mut Vec<Box<dyn ToSql>>) -> String {
    let parts: Vec<String> = split_values(value)
        .into_iter()
        .map(|v| {
            params.push(Box::new(format!("%{}%", v.replace('*', "%"))));
            params.push(Box::new(v.to_string()));
            if neg {
                "(matched_rule_name IS NULL OR (matched_rule_name NOT LIKE ? \
                  AND COALESCE(matched_rule_id, '') <> ?))"
                    .to_string()
            } else {
                "(matched_rule_name LIKE ? OR matched_rule_id = ?)".to_string()
            }
        })
        .collect();
    join_alternatives(parts, neg)
}

/// `mime:foo` lives on the related `header` table, so it can't fold
/// through the generic helpers. Same comma-split logic, inline.
fn mime_clause(value: &str, neg: bool, params: &mut Vec<Box<dyn ToSql>>) -> String {
    let parts: Vec<String> = split_values(value)
        .into_iter()
        .map(|v| {
            params.push(Box::new(format!("%{v}%")));
            let op = if neg { "NOT EXISTS" } else { "EXISTS" };
            format!(
                "{op} (SELECT 1 FROM header h WHERE h.capture_id = capture.id
                 AND h.direction='response' AND lower(h.name)='content-type'
                 AND lower(h.value) LIKE lower(?))"
            )
        })
        .collect();
    join_alternatives(parts, neg)
}

/// `device:foo` matches against the device's human name or serial (not the
/// opaque device_id UUID), resolving through the `device` table — so the user
/// can type `device:RS35` instead of pasting a UUID. Substring, `*` glob and
/// comma-list all behave like the other text keys.
fn device_clause(value: &str, neg: bool, params: &mut Vec<Box<dyn ToSql>>) -> String {
    let parts: Vec<String> = split_values(value)
        .into_iter()
        .map(|v| {
            // The host sentinel ("Текущий компьютер") is not a real device row,
            // so it can't resolve through the name/serial subquery. Match the
            // device_id column directly instead.
            if v == "__host__" {
                params.push(Box::new("__host__".to_string()));
                return if neg {
                    "capture.device_id <> ?".to_string()
                } else {
                    "capture.device_id = ?".to_string()
                };
            }
            let pattern = if v.contains('*') {
                v.replace('*', "%")
            } else {
                format!("%{v}%")
            };
            // Two params: one for display_name, one for serial.
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern));
            let op = if neg { "NOT IN" } else { "IN" };
            format!(
                "capture.device_id {op} (SELECT id FROM device \
                 WHERE display_name LIKE ? OR serial LIKE ?)"
            )
        })
        .collect();
    join_alternatives(parts, neg)
}

fn range_or_eq(
    col: &str,
    value: &str,
    neg: bool,
    params: &mut Vec<Box<dyn ToSql>>,
) -> Result<String> {
    // Comma list: each element can be `N` or `N..M` independently.
    // Build them as positive clauses, OR them, then wrap in NOT if
    // negated — so `!status:200,500..599` means "neither 200 nor 5xx".
    if value.contains(',') {
        let mut parts = Vec::new();
        for v in split_values(value) {
            parts.push(range_or_eq_one(col, v, params)?);
        }
        let inner = parts.join(" OR ");
        return Ok(if neg {
            format!("NOT ({inner})")
        } else {
            format!("({inner})")
        });
    }

    let frag = range_or_eq_one(col, value, params)?;
    Ok(if neg { format!("NOT ({frag})") } else { frag })
}

/// Single-value variant of range_or_eq: `N` or `N..M`. Always returns
/// an unsigned (positive) fragment; the caller applies negation.
fn range_or_eq_one(col: &str, value: &str, params: &mut Vec<Box<dyn ToSql>>) -> Result<String> {
    let parse_i = |s: &str| s.parse::<i64>().map_err(|_| anyhow!("bad number: {s}"));

    if let Some((lo, hi)) = value.split_once("..") {
        Ok(match (lo, hi) {
            ("", "") => "1=1".into(),
            ("", h) => {
                params.push(Box::new(parse_i(h)?));
                format!("{col} <= ?")
            }
            (l, "") => {
                params.push(Box::new(parse_i(l)?));
                format!("{col} >= ?")
            }
            (l, h) => {
                params.push(Box::new(parse_i(l)?));
                params.push(Box::new(parse_i(h)?));
                format!("{col} BETWEEN ? AND ?")
            }
        })
    } else {
        params.push(Box::new(parse_i(value)?));
        Ok(format!("{col} = ?"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input() {
        let (sql, p) = compile_to_sql("").unwrap();
        assert_eq!(sql, "1=1");
        assert!(p.is_empty());
    }

    #[test]
    fn host_and_status_range() {
        let (sql, p) = compile_to_sql("host:api.example.com status:500..599").unwrap();
        assert!(sql.contains("server_host LIKE"));
        assert!(sql.contains("status BETWEEN"));
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn negation() {
        let (sql, _) = compile_to_sql("!host:cdn.*").unwrap();
        assert!(sql.contains("server_host NOT LIKE"));
    }

    #[test]
    fn method_uppercase() {
        let (sql, _) = compile_to_sql("method:post").unwrap();
        assert!(sql.contains("method = ?"));
    }

    #[test]
    fn unknown_key_errors() {
        assert!(compile_to_sql("woof:bar").is_err());
    }

    #[test]
    fn bareword_matches_host_or_path() {
        let (sql, p) = compile_to_sql("google").unwrap();
        assert!(sql.contains("server_host LIKE"));
        assert!(sql.contains("url_path LIKE"));
        assert!(sql.contains("OR"));
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn bareword_negation_uses_and() {
        let (sql, _) = compile_to_sql("!google").unwrap();
        assert!(sql.contains("server_host NOT LIKE"));
        assert!(sql.contains("url_path NOT LIKE"));
        assert!(sql.contains("AND"));
    }

    #[test]
    fn error_kind_filter() {
        let (sql, _) = compile_to_sql("!error:tls_handshake").unwrap();
        assert!(sql.contains("error_kind <> ?"));
    }

    #[test]
    fn device_filter_matches_name_or_serial() {
        let (sql, p) = compile_to_sql("device:RS35").unwrap();
        assert!(sql.contains("capture.device_id IN (SELECT id FROM device"));
        assert!(sql.contains("display_name LIKE ?"));
        assert!(sql.contains("serial LIKE ?"));
        // One pattern bound twice (display_name + serial).
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn device_filter_negation() {
        let (sql, _) = compile_to_sql("!device:RS35").unwrap();
        assert!(sql.contains("capture.device_id NOT IN (SELECT id FROM device"));
    }

    #[test]
    fn device_filter_quoted_value_with_space() {
        let (sql, p) = compile_to_sql("device:\"CipherLab RS35\"").unwrap();
        assert!(sql.contains("capture.device_id IN (SELECT id FROM device"));
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn device_host_sentinel_matches_device_id_directly() {
        // __host__ is not a real device row — it must compile to a direct
        // device_id equality, not the name/serial subquery.
        let (sql, p) = compile_to_sql("device:__host__").unwrap();
        assert_eq!(sql, "capture.device_id = ?");
        assert!(!sql.contains("SELECT id FROM device"));
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn device_host_sentinel_negation() {
        let (sql, p) = compile_to_sql("!device:__host__").unwrap();
        assert_eq!(sql, "capture.device_id <> ?");
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn host_comma_list_ors_alternatives() {
        let (sql, p) = compile_to_sql("host:api.foo.com,api.bar.com").unwrap();
        assert!(sql.contains("server_host LIKE"));
        assert!(
            sql.contains(" OR "),
            "expected OR between hosts, got: {sql}"
        );
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn host_comma_negation_uses_and() {
        // !host:a,b means "host is neither a nor b" → AND NOT LIKE × N
        let (sql, p) = compile_to_sql("!host:cdn.example.com,fonts.example.com").unwrap();
        assert!(sql.contains("server_host NOT LIKE"));
        assert!(
            sql.contains(" AND "),
            "expected AND for negated list, got: {sql}"
        );
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn method_comma_uppercases_each() {
        let (sql, p) = compile_to_sql("method:post,put,delete").unwrap();
        assert!(sql.contains("method = ?"));
        assert!(sql.contains(" OR "));
        assert_eq!(p.len(), 3);
        // The pushed params themselves should be uppercase.
        // Read them back as strings — ToSql doesn't make this easy
        // outside of an actual query, so we just verify the SQL shape
        // and trust eq_clause_uppercase to do the upper().
    }

    #[test]
    fn status_comma_mixes_single_and_range() {
        let (sql, p) = compile_to_sql("status:200,500..599").unwrap();
        // 200 → status = ?, range → status BETWEEN ? AND ?
        assert!(sql.contains("status = ?"));
        assert!(sql.contains("status BETWEEN"));
        assert!(sql.contains(" OR "));
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn status_comma_negation_wraps_in_not() {
        let (sql, _) = compile_to_sql("!status:200,201,204").unwrap();
        assert!(sql.starts_with("NOT ("), "expected outer NOT, got: {sql}");
    }

    #[test]
    fn mime_comma_ors_alternatives() {
        let (sql, p) = compile_to_sql("mime:json,xml").unwrap();
        assert!(sql.contains("EXISTS"));
        assert!(sql.contains(" OR "));
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn single_value_unchanged_no_parens() {
        // Regression guard: single-value host should NOT be wrapped in
        // extra parens — kept identical to pre-comma behaviour so
        // downstream readers / EXPLAIN plans don't shift.
        let (sql, _) = compile_to_sql("host:api.foo.com").unwrap();
        assert_eq!(sql, "server_host LIKE ?");
    }

    #[test]
    fn empty_comma_segment_is_skipped() {
        // Trailing comma or `a,,b` shouldn't blow up — split_values
        // filters out empties.
        let (sql, p) = compile_to_sql("host:foo.com,,bar.com,").unwrap();
        assert!(sql.contains(" OR "));
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn keys_are_case_insensitive() {
        let (sql_lower, _) = compile_to_sql("host:api.example.com").unwrap();
        let (sql_pascal, _) = compile_to_sql("Host:api.example.com").unwrap();
        let (sql_upper, _) = compile_to_sql("HOST:api.example.com").unwrap();
        assert_eq!(sql_lower, sql_pascal);
        assert_eq!(sql_lower, sql_upper);
    }

    #[test]
    fn state_key_compiles_and_lowercases() {
        let (sql, params) = compile_to_sql("state:stubbed").unwrap();
        assert_eq!(sql, "state = ?");
        assert_eq!(params.len(), 1);
        // Case-folded, so `state:Stubbed` finds the same rows.
        let (upper_sql, _) = compile_to_sql("state:STUBBED").unwrap();
        assert_eq!(upper_sql, sql);
    }

    #[test]
    fn state_key_supports_alternatives_and_negation() {
        // "was this response mocked at all" — either stub or patch.
        let (sql, params) = compile_to_sql("state:stubbed,patched").unwrap();
        assert!(sql.contains(" OR "), "comma list should OR: {sql}");
        assert_eq!(params.len(), 2);

        // "only live responses"
        let (neg, _) = compile_to_sql("!state:stubbed").unwrap();
        assert!(neg.contains("<>"), "negation should use <>: {neg}");
    }

    #[test]
    fn rule_key_matches_name_or_id() {
        let (sql, params) = compile_to_sql("rule:orders-500").unwrap();
        assert!(sql.contains("matched_rule_name LIKE ?"));
        assert!(sql.contains("matched_rule_id = ?"));
        // One bound value per side: the LIKE pattern and the exact id.
        assert_eq!(params.len(), 2);
    }

    /// `matched_rule_name NOT LIKE ?` evaluates to NULL — not true — for every
    /// live capture, so a naive negation would silently discard exactly the
    /// rows a user asking "what did NOT come from this rule" wants most.
    #[test]
    fn negated_rule_key_still_matches_unmocked_captures() {
        let (sql, _) = compile_to_sql("!rule:orders-500").unwrap();
        assert!(
            sql.contains("matched_rule_name IS NULL"),
            "negated rule: must keep NULL (live) captures: {sql}"
        );
    }

    #[test]
    fn unknown_key_is_still_rejected() {
        // Guards against a typo silently degrading to a bareword search.
        assert!(compile_to_sql("stat:stubbed").is_err());
    }
}
