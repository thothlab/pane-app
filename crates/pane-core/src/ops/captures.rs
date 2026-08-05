use uuid::Uuid;

use pane_ipc::{kinds, CaptureBodyDto, CaptureDto, ClearResult, ExportOneResult, ListCapturesArgs};

use crate::error::{to_api, CoreResult};
use crate::Core;

impl Core {
    pub async fn captures_list(&self, args: ListCapturesArgs) -> CoreResult<Vec<CaptureDto>> {
        // Validate the filter separately so a malformed DSL string reports as
        // `filter_parse` rather than `db`. The old command mapped both through
        // to_api("db"), which made "your query is wrong" indistinguishable
        // from "the database is unhappy" — and the CLI turns kinds into exit
        // codes, so that distinction is now load-bearing.
        if let Some(q) = args.filter.as_deref() {
            if !q.trim().is_empty() {
                self.storage
                    .validate_capture_filter(q)
                    .map_err(to_api(kinds::FILTER_PARSE))?;
            }
        }
        self.storage
            .list_captures(args.filter.as_deref(), args.limit, args.before)
            .map_err(to_api(kinds::DB))
    }

    pub async fn captures_count(&self) -> CoreResult<i64> {
        self.storage.captures_count().map_err(to_api(kinds::DB))
    }

    pub async fn capture_get(&self, id: Uuid) -> CoreResult<CaptureDto> {
        self.storage
            .get_capture(id)
            .map_err(to_api(kinds::NOT_FOUND))
    }

    pub async fn capture_body(
        &self,
        body_id: Uuid,
        max_bytes: Option<u64>,
    ) -> CoreResult<CaptureBodyDto> {
        self.storage
            .get_body(body_id, max_bytes)
            .map_err(to_api(kinds::NOT_FOUND))
    }

    pub async fn captures_clear(&self, older_than: Option<String>) -> CoreResult<ClearResult> {
        let n = self
            .storage
            .clear_captures(older_than)
            .map_err(to_api(kinds::DB))?;
        Ok(ClearResult { deleted: n as u64 })
    }

    pub async fn capture_export(&self, id: Uuid, format: &str) -> CoreResult<ExportOneResult> {
        self.storage
            .export_one(id, format)
            .map_err(to_api(kinds::EXPORT_FAILED))
    }
}

/// Write a text payload to a path the user picked.
///
/// Routing writes through Rust keeps the renderer out of plugin-fs's
/// per-capability scope rules. Stateless, so it's a free function rather than
/// a `Core` method.
pub fn write_text_file(path: &str, content: &str) -> CoreResult<usize> {
    std::fs::write(path, content).map_err(to_api(kinds::IO))?;
    Ok(content.len())
}

/// Read a text payload back. Paired with [`write_text_file`] for the rules
/// import flow.
pub fn read_text_file(path: &str) -> CoreResult<String> {
    std::fs::read_to_string(path).map_err(to_api(kinds::IO))
}
