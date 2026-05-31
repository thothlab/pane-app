pub mod ca;
pub mod captures;
pub mod devices;
pub mod filters;
pub mod proxy;
pub mod replay;
pub mod rules;

use pane_ipc::ApiError;

pub type CmdResult<T> = Result<T, ApiError>;

pub(crate) fn to_api<E: std::fmt::Display>(kind: &'static str) -> impl Fn(E) -> ApiError {
    move |e| ApiError {
        kind: kind.to_string(),
        message: e.to_string(),
        details: None,
    }
}
