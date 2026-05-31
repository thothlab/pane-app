//! Body blob store. Inline ≤ 64 KB in SQLite, larger files spilled to disk
//! addressed by sha256 for dedup. Returned to UI as base64 with truncation.

use std::path::PathBuf;

use anyhow::Result;
use base64::Engine as _;
use pane_ipc::CaptureBodyDto;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

const INLINE_LIMIT: usize = 64 * 1024;

pub struct BodyStore {
    root: PathBuf,
}

impl BodyStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Persist `bytes` and link them to the capture-body table. The connection
    /// is borrowed (not owned) so the caller can keep its transaction.
    pub fn put(
        &self,
        bytes: &[u8],
        encoding: &str,
        mime: Option<&str>,
        conn: &Mutex<Connection>,
    ) -> Result<Uuid> {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let sha = hex::encode(hasher.finalize());

        let storage_kind;
        let mut inline_blob: Option<Vec<u8>> = None;
        let mut file_path: Option<String> = None;

        if bytes.len() <= INLINE_LIMIT {
            storage_kind = "inline";
            inline_blob = Some(bytes.to_vec());
        } else {
            storage_kind = "file";
            let prefix = &sha[..2];
            let dir = self.root.join(prefix);
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(&sha);
            if !path.exists() {
                std::fs::write(&path, bytes)?;
            }
            file_path = Some(format!("{prefix}/{sha}"));
        }

        let id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let conn = conn.lock();
        conn.execute(
            "INSERT INTO capture_body (id, sha256, encoding, mime, size_bytes, storage, inline_blob, file_path, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id.to_string(),
                sha,
                encoding,
                mime,
                bytes.len() as i64,
                storage_kind,
                inline_blob,
                file_path,
                now
            ],
        )?;
        Ok(id)
    }

    pub fn get(
        &self,
        id: Uuid,
        max_bytes: Option<u64>,
        conn: &Mutex<Connection>,
    ) -> Result<CaptureBodyDto> {
        let conn = conn.lock();
        let row = conn.query_row(
            "SELECT encoding, mime, size_bytes, storage, inline_blob, file_path
             FROM capture_body WHERE id=?1",
            params![id.to_string()],
            |r| {
                let encoding: String = r.get(0)?;
                let mime: Option<String> = r.get(1)?;
                let size: i64 = r.get(2)?;
                let storage_kind: String = r.get(3)?;
                let inline: Option<Vec<u8>> = r.get(4)?;
                let file: Option<String> = r.get(5)?;
                Ok((encoding, mime, size, storage_kind, inline, file))
            },
        )?;

        let (encoding, mime, size, storage_kind, inline, file) = row;
        let bytes = match storage_kind.as_str() {
            "inline" => inline.unwrap_or_default(),
            "file" => {
                let p = self.root.join(file.unwrap_or_default());
                std::fs::read(p).unwrap_or_default()
            }
            _ => Vec::new(),
        };

        let (cut, truncated) = match max_bytes {
            Some(m) if (m as usize) < bytes.len() => (&bytes[..m as usize], true),
            _ => (bytes.as_slice(), false),
        };

        Ok(CaptureBodyDto {
            mime,
            encoding,
            bytes_base64: base64::engine::general_purpose::STANDARD.encode(cut),
            truncated,
            total_size: size as u64,
        })
    }

    pub fn exists(&self, id: Uuid, conn: &Mutex<Connection>) -> Result<bool> {
        let conn = conn.lock();
        let v: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM capture_body WHERE id=?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.is_some())
    }
}
