//! Filter DSL parser → SQL WHERE compiler (PRD §5.3).
//!
//! Grammar (v1):
//!   expr := term (WS term)*
//!   term := '!'? atom
//!   atom := key ':' value | bareword
//!   key  := host | method | status | mime | path | size | duration | error
//!
//! Bareword without `:` is treated as substring search across host AND path
//! (joined by OR) — this is what users expect when they type a word into
//! the filter box. Status/size/duration values may use `N..M` range form.

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

        let frag = match key {
            "host" => like_clause("server_host", value, negate, &mut params),
            "method" => eq_clause("method", &value.to_uppercase(), negate, &mut params),
            "status" => range_or_eq("status", value, negate, &mut params)?,
            "mime" => {
                params.push(Box::new(format!("%{value}%")));
                let op = if negate { "NOT EXISTS" } else { "EXISTS" };
                format!(
                    "{op} (SELECT 1 FROM header h WHERE h.capture_id = capture.id
                     AND h.direction='response' AND lower(h.name)='content-type'
                     AND lower(h.value) LIKE lower(?))"
                )
            }
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

fn like_clause(col: &str, value: &str, neg: bool, params: &mut Vec<Box<dyn ToSql>>) -> String {
    let pattern = if value.contains('*') {
        value.replace('*', "%")
    } else {
        format!("%{value}%")
    };
    params.push(Box::new(pattern));
    if neg {
        format!("{col} NOT LIKE ?")
    } else {
        format!("{col} LIKE ?")
    }
}

fn eq_clause(col: &str, value: &str, neg: bool, params: &mut Vec<Box<dyn ToSql>>) -> String {
    params.push(Box::new(value.to_string()));
    if neg {
        format!("{col} <> ?")
    } else {
        format!("{col} = ?")
    }
}

fn range_or_eq(
    col: &str,
    value: &str,
    neg: bool,
    params: &mut Vec<Box<dyn ToSql>>,
) -> Result<String> {
    let parse_i = |s: &str| s.parse::<i64>().map_err(|_| anyhow!("bad number: {s}"));

    let frag = if let Some((lo, hi)) = value.split_once("..") {
        match (lo, hi) {
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
        }
    } else {
        params.push(Box::new(parse_i(value)?));
        format!("{col} = ?")
    };

    Ok(if neg { format!("NOT ({frag})") } else { frag })
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
}
