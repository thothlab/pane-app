//! Import/export of rule bundles in the GUI's `pane-rules` format.
//!
//! Byte-compatible with `src/lib/rules-portfile.ts` in both directions, so a
//! bundle exported here opens in Rules → Import and vice versa. That
//! compatibility is the point: it replaces hand-rolled workarounds for getting
//! a large rule library into Pane without clicking through the GUI.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::output::{exit, note, print_json, Format};
use crate::session::Session;

const FORMAT_ID: &str = "pane-rules";
const FORMAT_VERSION: u64 = 1;

pub async fn import(s: &mut Session, file: &Path, dry_run: bool, format: Format) -> Result<i32> {
    let raw =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let doc: Value = serde_json::from_str(&raw)
        .with_context(|| format!("{} is not valid JSON", file.display()))?;

    validate(&doc)?;

    let collections = doc["collections"].as_array().cloned().unwrap_or_default();
    let rules = doc["rules"].as_array().cloned().unwrap_or_default();

    if dry_run {
        let summary = json!({
            "dry_run": true,
            "collections": collections.len(),
            "rules": rules.len(),
            "rule_names": rules.iter().map(|r| r["name"].clone()).collect::<Vec<_>>(),
        });
        print_json(&summary);
        return Ok(exit::OK);
    }

    // Every entity gets a fresh id, matching the GUI's conflict policy: an
    // import lands beside anything with the same name rather than overwriting
    // it. Map old collection ref → new id so rules land in the right group.
    let mut ref_to_id = std::collections::HashMap::new();
    for c in &collections {
        let created = s
            .call(
                "collections.upsert",
                json!({
                    "id": null,
                    "name": c["name"],
                    "enabled": c["enabled"].as_bool().unwrap_or(true),
                    "priority": c["priority"].as_i64().unwrap_or(0),
                }),
            )
            .await?;
        if let (Some(old), Some(new)) = (c["ref"].as_str(), created["id"].as_str()) {
            ref_to_id.insert(old.to_string(), new.to_string());
        }
    }

    let mut created_rules = 0usize;
    for r in &rules {
        let collection_id = r["collection_ref"]
            .as_str()
            .and_then(|k| ref_to_id.get(k))
            .map(|v| json!(v))
            .unwrap_or(Value::Null);

        s.call(
            "rules.upsert",
            json!({
                "id": null,
                "name": r["name"],
                "enabled": r["enabled"].as_bool().unwrap_or(true),
                "priority": r["priority"].as_i64().unwrap_or(0),
                "collection_id": collection_id,
                "mode": r["mode"].as_str().unwrap_or("stub"),
                "patches": r["patches"].as_array().cloned().unwrap_or_default(),
                "match_host_glob": r["match_host_glob"],
                "match_method": r["match_method"],
                "match_path_glob": r["match_path_glob"],
                "match_params": r["match_params"].as_array().cloned().unwrap_or_default(),
                "match_req_body": r["match_req_body"],
                "match_conditions": r["match_conditions"].as_array().cloned().unwrap_or_default(),
                "res_status": r["res_status"].as_u64().unwrap_or(200),
                "res_headers": r["res_headers"].as_array().cloned().unwrap_or_default(),
                "res_body_id": null,
                "res_body_base64": r["res_body_base64"],
                "res_body_mime": r["res_body_mime"],
                "res_delay_ms": r["res_delay_ms"].as_u64().unwrap_or(0),
            }),
        )
        .await?;
        created_rules += 1;
    }

    let summary = json!({ "collections": collections.len(), "rules": created_rules });
    match format {
        Format::Json => print_json(&summary),
        Format::Human => note(format!(
            "imported {} rule(s) in {} collection(s)",
            created_rules,
            collections.len()
        )),
    }
    Ok(exit::OK)
}

pub async fn export(s: &mut Session, out: Option<&Path>) -> Result<i32> {
    let rules = s.call("rules.list", Value::Null).await?;
    let collections = s.call("collections.list", Value::Null).await?;

    let mut exported_rules = Vec::new();
    for r in rules.as_array().cloned().unwrap_or_default() {
        // Bodies travel inline as base64 so the file is self-contained — no
        // companion blobs to ship alongside it.
        let body_b64 = match r["res_body_id"].as_str() {
            Some(id) => s
                .call("captures.body", json!({ "body_id": id, "max_bytes": null }))
                .await
                .ok()
                .and_then(|b| b["bytes_base64"].as_str().map(String::from)),
            None => None,
        };
        exported_rules.push(json!({
            "collection_ref": r["collection_id"],
            "name": r["name"],
            "enabled": r["enabled"],
            "priority": r["priority"],
            "mode": r["mode"],
            "patches": r["patches"],
            "match_host_glob": r["match_host_glob"],
            "match_method": r["match_method"],
            "match_path_glob": r["match_path_glob"],
            "match_params": r["match_params"],
            "match_req_body": r["match_req_body"],
            "match_conditions": r["match_conditions"],
            "res_status": r["res_status"],
            "res_headers": r["res_headers"],
            "res_body_mime": r["res_body_mime"],
            "res_body_base64": body_b64,
            "res_delay_ms": r["res_delay_ms"],
        }));
    }

    let doc = json!({
        "format": FORMAT_ID,
        "version": FORMAT_VERSION,
        "exported_at": time::OffsetDateTime::now_utc().to_string(),
        "kind": "library",
        "collections": collections.as_array().cloned().unwrap_or_default()
            .iter()
            .map(|c| json!({
                "ref": c["id"], "name": c["name"],
                "enabled": c["enabled"], "priority": c["priority"],
            }))
            .collect::<Vec<_>>(),
        "rules": exported_rules,
    });

    match out {
        Some(p) => {
            std::fs::write(p, serde_json::to_vec_pretty(&doc)?)?;
            note(format!(
                "{} rule(s) → {}",
                doc["rules"].as_array().map(|a| a.len()).unwrap_or(0),
                p.display()
            ));
        }
        None => print_json(&doc),
    }
    Ok(exit::OK)
}

/// Same gate as the GUI's `validatePortFile`: check the envelope, not every
/// field. `rules.upsert` rejects bad values at insert time.
fn validate(doc: &Value) -> Result<()> {
    if doc["format"].as_str() != Some(FORMAT_ID) {
        anyhow::bail!(
            "unexpected format `{}` — expected `{FORMAT_ID}`",
            doc["format"].as_str().unwrap_or("(none)")
        );
    }
    let version = doc["version"].as_u64().context("missing `version`")?;
    if version > FORMAT_VERSION {
        anyhow::bail!("file version {version} is newer than supported ({FORMAT_VERSION})");
    }
    if !doc["rules"].is_array() {
        anyhow::bail!("missing `rules` array");
    }
    match doc["kind"].as_str() {
        Some("rule") | Some("collection") | Some("library") => {}
        other => anyhow::bail!("unknown kind: {}", other.unwrap_or("(none)")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(kind: &str, version: u64) -> Value {
        json!({ "format": FORMAT_ID, "version": version, "kind": kind, "rules": [] })
    }

    #[test]
    fn accepts_every_kind_the_gui_writes() {
        for k in ["rule", "collection", "library"] {
            validate(&doc(k, 1)).unwrap();
        }
    }

    #[test]
    fn rejects_foreign_or_future_files() {
        assert!(validate(
            &json!({"format": "something-else", "version": 1, "kind": "library", "rules": []})
        )
        .is_err());
        assert!(validate(&doc("library", FORMAT_VERSION + 1)).is_err());
        assert!(validate(&json!({"format": FORMAT_ID, "version": 1, "kind": "library"})).is_err());
        assert!(validate(&doc("nonsense", 1)).is_err());
    }
}
