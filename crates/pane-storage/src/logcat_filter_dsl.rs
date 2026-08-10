//! Logcat filter DSL → SQL WHERE compiler.
//!
//! A faithful Rust port of `src/lib/logcat-filter.ts` (which compiles the same
//! grammar to an in-memory predicate). Keep the two in lock-step — the tests
//! below mirror the TS examples.
//!
//! Grammar (all tokens AND together):
//!   term  := '!'? atom
//!   atom  := key ':' valuelist | '~' regex | bareword
//!   value := '!'? single                 # per-value negation
//!   valuelist := value (',' value)*
//!   key   := tag | msg/message | level | pid | app
//!
//! Within one key list `tag:a,b,!c,!d`: positives OR together, negatives all
//! must NOT match, the two groups AND. Outer `!key:…` flips the whole result.
//!
//! `app:` is a no-op here: package→PID resolution needs a live device, so the
//! frontend resolves it (via `android_pid_names`) and passes explicit
//! include/exclude PID lists to the query alongside this WHERE clause.
//!
//! `~regex` compiles to `col REGEXP ?`, backed by the scalar function
//! registered on the read connection (see `logcat::register_regexp`).

use anyhow::{anyhow, Result};
use regex::Regex;
use rusqlite::ToSql;

struct RawValue {
    value: String,
    negate: bool,
}

/// Compile a filter string to a SQL WHERE fragment + ordered bind params.
/// Empty / all-`app:` input yields `"1=1"`.
pub fn compile_to_sql(input: &str) -> Result<(String, Vec<Box<dyn ToSql>>)> {
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    let mut where_parts: Vec<String> = Vec::new();

    for raw in tokenize(input) {
        let negate = raw.starts_with('!');
        let body = if negate { &raw[1..] } else { raw.as_str() };

        // Regex form `~pattern` — checked before `:` (matches the TS order).
        if let Some(pattern) = body.strip_prefix('~') {
            // Validate early so a bad pattern surfaces as a filter error rather
            // than failing per-row inside the REGEXP function.
            Regex::new(pattern).map_err(|e| anyhow!("bad regex: {pattern}: {e}"))?;
            params.push(Box::new(pattern.to_string()));
            params.push(Box::new(pattern.to_string()));
            let frag = "tag REGEXP ? OR message REGEXP ?";
            where_parts.push(if negate {
                format!("NOT ({frag})")
            } else {
                format!("({frag})")
            });
            continue;
        }

        match body.find(':') {
            None => {
                // Bareword: substring across tag OR message.
                let pat = regex_pattern(body);
                params.push(Box::new(pat.clone()));
                params.push(Box::new(pat));
                where_parts.push(if negate {
                    "(NOT (tag REGEXP ?) AND NOT (message REGEXP ?))".into()
                } else {
                    "(tag REGEXP ? OR message REGEXP ?)".into()
                });
            }
            Some(idx) => {
                let key = body[..idx].to_ascii_lowercase();
                let value = &body[idx + 1..];
                // `app:` is resolved to PIDs on the frontend — ignore here.
                if key == "app" {
                    continue;
                }
                let values = split_values(value);
                if values.is_empty() {
                    where_parts.push("1=1".into());
                    continue;
                }
                let frag = match key.as_str() {
                    "tag" => text_group("tag", &values, &mut params),
                    "msg" | "message" => text_group("message", &values, &mut params),
                    "level" => level_group(&values, &mut params)?,
                    "pid" => pid_group(&values, &mut params)?,
                    other => return Err(anyhow!("unknown filter key: {other}")),
                };
                where_parts.push(if negate {
                    format!("NOT ({frag})")
                } else {
                    frag
                });
            }
        }
    }

    let sql = if where_parts.is_empty() {
        "1=1".into()
    } else {
        where_parts.join(" AND ")
    };
    Ok((sql, params))
}

/// Split on `,` into trimmed non-empty parts, each with an optional leading `!`.
fn split_values(value: &str) -> Vec<RawValue> {
    let parts: Vec<&str> = if value.contains(',') {
        value.split(',').collect()
    } else {
        vec![value]
    };
    let mut out = Vec::new();
    for raw in parts {
        let mut s = raw.trim();
        if s.is_empty() {
            continue;
        }
        let mut negate = false;
        if let Some(rest) = s.strip_prefix('!') {
            negate = true;
            s = rest.trim();
        }
        if s.is_empty() {
            continue;
        }
        out.push(RawValue {
            value: s.to_string(),
            negate,
        });
    }
    out
}

/// Case-insensitive (Unicode) substring pattern faithful to the TS
/// `makeSubstringMatcher`: `*` is the only glob, matching is anchor-free. We
/// use REGEXP rather than LIKE so non-ASCII case folding works — SQLite `LIKE`
/// is case-sensitive for anything outside A–Z, which would silently
/// under-match Cyrillic tags/messages (the JS path used `.toLowerCase()`).
/// `(?i)` gives Rust regex's Unicode-aware fold; regex metachars are escaped,
/// `*`→`.*`. Paired with `col REGEXP ?`.
fn regex_pattern(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 4);
    out.push_str("(?i)");
    for c in value.chars() {
        match c {
            '*' => out.push_str(".*"),
            '.' | '^' | '$' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Positives OR, negatives (NOT REGEXP) all-must-not, groups AND — over one
/// text column. Params are pushed in the same order the `?`s appear.
fn text_group(col: &str, values: &[RawValue], params: &mut Vec<Box<dyn ToSql>>) -> String {
    let mut parts: Vec<String> = Vec::new();
    let pos: Vec<&RawValue> = values.iter().filter(|v| !v.negate).collect();
    let neg: Vec<&RawValue> = values.iter().filter(|v| v.negate).collect();

    if !pos.is_empty() {
        let ors: Vec<String> = pos
            .iter()
            .map(|v| {
                params.push(Box::new(regex_pattern(&v.value)));
                format!("{col} REGEXP ?")
            })
            .collect();
        parts.push(if ors.len() > 1 {
            format!("({})", ors.join(" OR "))
        } else {
            ors.into_iter().next().unwrap()
        });
    }
    for v in neg {
        params.push(Box::new(regex_pattern(&v.value)));
        parts.push(format!("NOT ({col} REGEXP ?)"));
    }
    join_and(parts)
}

fn level_group(values: &[RawValue], params: &mut Vec<Box<dyn ToSql>>) -> Result<String> {
    let mut ranges: Vec<(i64, i64, bool)> = Vec::new();
    for v in values {
        let (min, max) =
            parse_level_range(&v.value).ok_or_else(|| anyhow!("bad level: {}", v.value))?;
        ranges.push((min, max, v.negate));
    }
    let mut parts: Vec<String> = Vec::new();
    let pos: Vec<&(i64, i64, bool)> = ranges.iter().filter(|r| !r.2).collect();
    let neg: Vec<&(i64, i64, bool)> = ranges.iter().filter(|r| r.2).collect();

    if !pos.is_empty() {
        let ors: Vec<String> = pos
            .iter()
            .map(|(mn, mx, _)| {
                params.push(Box::new(*mn));
                params.push(Box::new(*mx));
                "level BETWEEN ? AND ?".to_string()
            })
            .collect();
        parts.push(if ors.len() > 1 {
            format!("({})", ors.join(" OR "))
        } else {
            ors.into_iter().next().unwrap()
        });
    }
    for (mn, mx, _) in neg {
        params.push(Box::new(*mn));
        params.push(Box::new(*mx));
        parts.push("level NOT BETWEEN ? AND ?".to_string());
    }
    Ok(join_and(parts))
}

fn pid_group(values: &[RawValue], params: &mut Vec<Box<dyn ToSql>>) -> Result<String> {
    let mut pos: Vec<i64> = Vec::new();
    let mut neg: Vec<i64> = Vec::new();
    for v in values {
        let n: i64 = v
            .value
            .parse()
            .map_err(|_| anyhow!("bad pid: {}", v.value))?;
        if v.negate {
            neg.push(n);
        } else {
            pos.push(n);
        }
    }
    let mut parts: Vec<String> = Vec::new();
    if !pos.is_empty() {
        let qs = vec!["?"; pos.len()].join(", ");
        for n in &pos {
            params.push(Box::new(*n));
        }
        parts.push(format!("pid IN ({qs})"));
    }
    if !neg.is_empty() {
        let qs = vec!["?"; neg.len()].join(", ");
        for n in &neg {
            params.push(Box::new(*n));
        }
        parts.push(format!("pid NOT IN ({qs})"));
    }
    Ok(join_and(parts))
}

fn join_and(parts: Vec<String>) -> String {
    match parts.len() {
        0 => "1=1".into(),
        1 => parts.into_iter().next().unwrap(),
        _ => format!("({})", parts.join(" AND ")),
    }
}

/// Token → LogLevel discriminant (0=verbose .. 6=silent). Mirrors the TS
/// LEVEL_FROM_TOKEN + LEVEL_ORDER maps.
fn level_num(tok: &str) -> Option<i64> {
    Some(match tok.to_ascii_lowercase().as_str() {
        "v" | "verbose" => 0,
        "d" | "debug" => 1,
        "i" | "info" => 2,
        "w" | "warn" | "warning" => 3,
        "e" | "error" => 4,
        "f" | "fatal" => 5,
        "s" | "silent" => 6,
        _ => return None,
    })
}

/// Accept "E", "W..F", "..E", "W.." and named forms → inclusive (min,max).
fn parse_level_range(raw: &str) -> Option<(i64, i64)> {
    let parts: Vec<&str> = raw.split("..").collect();
    match parts.as_slice() {
        [single] => {
            let n = level_num(single)?;
            Some((n, n))
        }
        [lo, hi] => {
            let min = if lo.is_empty() { 0 } else { level_num(lo)? };
            let max = if hi.is_empty() { 6 } else { level_num(hi)? };
            Some((min, max))
        }
        _ => None,
    }
}

/// Whitespace tokenizer that keeps `"quoted strings"` as single tokens.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sql(input: &str) -> String {
        compile_to_sql(input).unwrap().0
    }
    fn nparams(input: &str) -> usize {
        compile_to_sql(input).unwrap().1.len()
    }

    #[test]
    fn empty_is_true() {
        assert_eq!(sql(""), "1=1");
        assert_eq!(nparams(""), 0);
    }

    #[test]
    fn bareword_matches_tag_or_message() {
        let s = sql("OkHttp");
        assert!(s.contains("tag REGEXP ?"));
        assert!(s.contains("message REGEXP ?"));
        assert!(s.contains(" OR "));
        assert_eq!(nparams("OkHttp"), 2);
    }

    #[test]
    fn bareword_negation_uses_and() {
        let s = sql("!OkHttp");
        assert!(s.contains("NOT (tag REGEXP ?)"));
        assert!(s.contains("NOT (message REGEXP ?)"));
        assert!(s.contains(" AND "));
    }

    #[test]
    fn tag_pos_neg_split() {
        // positives OR, negatives all-must-not, AND together
        let (s, p) = compile_to_sql("tag:!CatalogParser,!Spam,SSH").unwrap();
        assert!(s.contains("tag REGEXP ?"));
        assert_eq!(s.matches("NOT (tag REGEXP ?)").count(), 2);
        assert!(s.contains(" AND "));
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn tag_comma_positives_or() {
        let s = sql("tag:OkHttp,Retrofit");
        assert!(s.contains(" OR "));
        assert_eq!(nparams("tag:OkHttp,Retrofit"), 2);
    }

    #[test]
    fn level_single_and_range() {
        assert!(sql("level:E").contains("level BETWEEN ? AND ?"));
        assert_eq!(nparams("level:E"), 2);
        let (s, p) = compile_to_sql("level:W..F").unwrap();
        assert!(s.contains("level BETWEEN ? AND ?"));
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn level_open_ended() {
        // "..E" → 0..4 ; "W.." → 3..6
        compile_to_sql("level:..E").unwrap();
        compile_to_sql("level:W..").unwrap();
    }

    #[test]
    fn level_bad_errors() {
        assert!(compile_to_sql("level:ZZ").is_err());
    }

    #[test]
    fn pid_in_and_not_in() {
        let s = sql("pid:1234,!5678");
        assert!(s.contains("pid IN (?)"));
        assert!(s.contains("pid NOT IN (?)"));
        assert_eq!(nparams("pid:1234,!5678"), 2);
    }

    #[test]
    fn pid_bad_errors() {
        assert!(compile_to_sql("pid:notanumber").is_err());
    }

    #[test]
    fn regex_both_columns() {
        let s = sql("~^Foo.*bar$");
        assert!(s.contains("tag REGEXP ?"));
        assert!(s.contains("message REGEXP ?"));
        assert_eq!(nparams("~^Foo.*bar$"), 2);
    }

    #[test]
    fn regex_negation_wraps_not() {
        assert!(sql("!~keepalive").starts_with("NOT ("));
    }

    #[test]
    fn regex_bad_errors() {
        assert!(compile_to_sql("~[unclosed").is_err());
    }

    #[test]
    fn app_is_noop() {
        // Resolved to PIDs on the frontend — contributes no WHERE fragment.
        assert_eq!(sql("app:com.foo.bar"), "1=1");
        assert_eq!(nparams("app:com.foo,!com.foo.helper"), 0);
    }

    #[test]
    fn app_combined_with_real_key() {
        // Only the tag token contributes; app is dropped.
        let s = sql("app:com.foo tag:OkHttp");
        assert!(s.contains("tag REGEXP ?"));
        assert!(!s.contains("app"));
    }

    #[test]
    fn tokens_and_together_with_outer_negate() {
        let s = sql("tag:OkHttp !msg:keep-alive");
        assert!(s.contains("tag REGEXP ?"));
        assert!(s.contains("NOT ("));
        assert!(s.contains("message REGEXP ?"));
        assert!(s.contains(" AND "));
    }

    #[test]
    fn unknown_key_errors() {
        assert!(compile_to_sql("woof:bar").is_err());
    }

    #[test]
    fn quoted_token_kept_whole() {
        // "Pane Helper" is one tag value, space preserved.
        let (s, p) = compile_to_sql("tag:\"Pane Helper\"").unwrap();
        assert!(s.contains("tag REGEXP ?"));
        assert_eq!(p.len(), 1);
    }

    fn param_text(p: &dyn ToSql) -> String {
        use rusqlite::types::{ToSqlOutput, Value, ValueRef};
        match p.to_sql().unwrap() {
            ToSqlOutput::Borrowed(ValueRef::Text(b)) => String::from_utf8_lossy(b).into_owned(),
            ToSqlOutput::Owned(Value::Text(t)) => t,
            other => panic!("expected text param, got {other:?}"),
        }
    }

    #[test]
    fn pattern_is_case_insensitive_and_literal_underscore() {
        let (_s, p) = compile_to_sql("msg:foo_bar").unwrap();
        let text = param_text(p[0].as_ref());
        // (?i) Unicode fold; `_` is a regex literal (not a wildcard, no escape).
        assert!(
            text.starts_with("(?i)"),
            "expected case-insensitive flag, got {text}"
        );
        assert!(
            text.contains("foo_bar"),
            "expected literal underscore, got {text}"
        );
    }

    #[test]
    fn regex_metachars_escaped_in_substring() {
        // A literal dot/paren in a value must not become a regex wildcard.
        let (_s, p) = compile_to_sql("tag:a.b(c)").unwrap();
        let text = param_text(p[0].as_ref());
        assert!(
            text.contains(r"a\.b\(c\)"),
            "metachars must be escaped, got {text}"
        );
    }

    #[test]
    fn cyrillic_case_insensitive_matches() {
        // The regression that motivated REGEXP over LIKE: lowercase Cyrillic
        // query must match a capitalized occurrence.
        let re = regex::Regex::new(&regex_pattern("сбор")).unwrap();
        assert!(re.is_match("СборСредств_КарточкаСбора"));
    }
}
