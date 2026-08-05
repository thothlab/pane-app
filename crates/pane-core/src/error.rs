//! Error adapter shared by every core operation.
//!
//! Moved verbatim from `src-tauri/src/commands/mod.rs` so the CLI and the
//! control server get the same `ApiError` shape the Tauri commands produce,
//! rather than each inventing its own mapping.

use pane_ipc::ApiError;

/// Result type every `Core` operation returns.
pub type CoreResult<T> = Result<T, ApiError>;

/// Adapt any `Display` error into an [`ApiError`] with a fixed `kind`.
///
/// `kind` should come from [`pane_ipc::kinds`] — those strings are public
/// contract now that the CLI maps them onto exit codes.
pub fn to_api<E: std::fmt::Display>(kind: &'static str) -> impl Fn(E) -> ApiError {
    move |e| ApiError {
        kind: kind.to_string(),
        message: e.to_string(),
        details: None,
    }
}

/// Build an [`ApiError`] directly, for the cases that aren't wrapping a
/// lower-level error (precondition failures, mostly).
pub fn api_err(kind: &str, message: impl Into<String>) -> ApiError {
    ApiError {
        kind: kind.to_string(),
        message: message.into(),
        details: None,
    }
}
