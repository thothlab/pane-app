//! Resolving `app:<package>` filter terms to PIDs.
//!
//! `LogcatQueryArgs` takes numeric pids only, so the `app:` sugar has to be
//! expanded client-side — the frontend does the same thing in
//! `src/lib/logcat-filter.ts`.

use anyhow::Result;
use serde_json::json;

use crate::session::Session;

/// Sentinel pid used when a positive `app:` term matches no running process.
///
/// Sending an empty include list would mean "no pid constraint", silently
/// widening the query to the entire firehose — which for an agent means
/// megabytes of unrelated log lines instead of zero rows. A pid that cannot
/// exist keeps the intended "nothing matched" result.
pub const APP_NO_MATCH_PID: u32 = u32::MAX;

/// Extract `app:` terms and turn them into (include, exclude) pid lists.
///
/// Values are comma-separated and each may be individually negated with `!`,
/// matching the DSL the GUI accepts.
pub async fn resolve_app_pids(
    s: &mut Session,
    serial: &str,
    filter: Option<&str>,
) -> Result<(Vec<u32>, Vec<u32>)> {
    let Some(filter) = filter else {
        return Ok((vec![], vec![]));
    };
    let terms = collect_app_terms(filter);
    if terms.is_empty() {
        return Ok((vec![], vec![]));
    }

    let names = s
        .call("logcat.pid_names", json!({ "serial": serial }))
        .await?;
    let table: Vec<(u32, String)> = names
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?.to_lowercase())))
                .collect()
        })
        .unwrap_or_default();

    let mut include = Vec::new();
    let mut exclude = Vec::new();
    let mut had_positive = false;

    for (needle, negated) in terms {
        let needle = needle.to_lowercase();
        let hits: Vec<u32> = table
            .iter()
            .filter(|(_, name)| name.contains(&needle))
            .map(|(pid, _)| *pid)
            .collect();
        if negated {
            exclude.extend(hits);
        } else {
            had_positive = true;
            include.extend(hits);
        }
    }

    if had_positive && include.is_empty() {
        include.push(APP_NO_MATCH_PID);
    }
    include.sort_unstable();
    include.dedup();
    exclude.sort_unstable();
    exclude.dedup();
    Ok((include, exclude))
}

/// Pull `(value, negated)` pairs out of every `app:` term in a filter string.
fn collect_app_terms(filter: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for raw in filter.split_whitespace() {
        let (token, outer_neg) = match raw.strip_prefix('!') {
            Some(rest) => (rest, true),
            None => (raw, false),
        };
        let Some(value) = token.strip_prefix("app:") else {
            continue;
        };
        for part in value.split(',') {
            let (v, inner_neg) = match part.strip_prefix('!') {
                Some(rest) => (rest, true),
                None => (part, false),
            };
            if !v.is_empty() {
                out.push((v.to_string(), outer_neg || inner_neg));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plain_and_negated_terms() {
        assert_eq!(
            collect_app_terms("app:dev.shop.app level:E"),
            vec![("dev.shop.app".to_string(), false)]
        );
        assert_eq!(
            collect_app_terms("!app:dev.shop.helper"),
            vec![("dev.shop.helper".to_string(), true)]
        );
    }

    #[test]
    fn splits_comma_lists_with_per_value_negation() {
        assert_eq!(
            collect_app_terms("app:a,!b,c"),
            vec![
                ("a".to_string(), false),
                ("b".to_string(), true),
                ("c".to_string(), false),
            ]
        );
    }

    #[test]
    fn ignores_filters_without_app_terms() {
        assert!(collect_app_terms("tag:OkHttp level:W..").is_empty());
        assert!(collect_app_terms("").is_empty());
    }
}
