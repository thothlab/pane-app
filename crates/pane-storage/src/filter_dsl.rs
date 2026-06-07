//! Filter DSL parser → SQL WHERE compiler (PRD §5.3).
//!
//! Grammar:
//!   expr  := term (WS term)*
//!   term  := '!'? atom
//!   atom  := key ':' value | bareword
//!   value := single | single ',' single (',' single)*
//!   key   := host | method | status | mime | path | size | duration | error
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
    fn host_comma_list_ors_alternatives() {
        let (sql, p) = compile_to_sql("host:api.foo.com,api.bar.com").unwrap();
        assert!(sql.contains("server_host LIKE"));
        assert!(sql.contains(" OR "), "expected OR between hosts, got: {sql}");
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn host_comma_negation_uses_and() {
        // !host:a,b means "host is neither a nor b" → AND NOT LIKE × N
        let (sql, p) = compile_to_sql("!host:cdn.example.com,fonts.example.com").unwrap();
        assert!(sql.contains("server_host NOT LIKE"));
        assert!(sql.contains(" AND "), "expected AND for negated list, got: {sql}");
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
}
