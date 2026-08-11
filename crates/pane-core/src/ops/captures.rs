use uuid::Uuid;

use pane_ipc::{
    kinds, CaptureBodyDto, CaptureDto, ClearResult, ExportOneResult, ListCapturesArgs, TlsHealthDto,
};

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
        let ListCapturesArgs {
            filter,
            limit,
            before,
        } = args;
        self.db(move |s| s.list_captures(filter.as_deref(), limit, before))
            .await
            .map_err(to_api(kinds::DB))
    }

    pub async fn captures_count(&self) -> CoreResult<i64> {
        self.db(|s| s.captures_count())
            .await
            .map_err(to_api(kinds::DB))
    }

    /// Feeds the "the device doesn't trust our CA" banner. Cheap enough to call
    /// on every capture batch: two indexed counts over the current session.
    pub async fn captures_tls_health(&self) -> CoreResult<TlsHealthDto> {
        self.db(|s| s.tls_health()).await.map_err(to_api(kinds::DB))
    }

    pub async fn capture_get(&self, id: Uuid) -> CoreResult<CaptureDto> {
        self.db(move |s| s.get_capture(id))
            .await
            .map_err(to_api(kinds::NOT_FOUND))
    }

    pub async fn capture_body(
        &self,
        body_id: Uuid,
        max_bytes: Option<u64>,
    ) -> CoreResult<CaptureBodyDto> {
        self.db(move |s| s.get_body(body_id, max_bytes))
            .await
            .map_err(to_api(kinds::NOT_FOUND))
    }

    pub async fn captures_clear(&self, older_than: Option<String>) -> CoreResult<ClearResult> {
        let n = self
            .db(move |s| s.clear_captures(older_than))
            .await
            .map_err(to_api(kinds::DB))?;
        Ok(ClearResult { deleted: n as u64 })
    }

    pub async fn capture_export(&self, id: Uuid, format: &str) -> CoreResult<ExportOneResult> {
        let format = format.to_string();
        self.db(move |s| s.export_one(id, &format))
            .await
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
