//! Thin adapters over `Core`'s tunnelled-host operations.
//!
//! The set itself, and the reasoning about when it may be cleared, lives in
//! `pane_core::ops::passthrough` so the CLI and the control server reach the
//! same state as the window does.

use super::{CmdResult, CoreState};
use pane_ipc::{ForgetTunneledHostArgs, TunneledHostsDto};

#[tauri::command]
pub async fn tunneled_hosts_list(state: CoreState<'_>) -> CmdResult<TunneledHostsDto> {
    state.tunneled_hosts_list().await
}

#[tauri::command]
pub async fn tunneled_hosts_reset(state: CoreState<'_>) -> CmdResult<usize> {
    state.tunneled_hosts_reset().await
}

#[tauri::command]
pub async fn tunneled_host_forget(
    state: CoreState<'_>,
    args: ForgetTunneledHostArgs,
) -> CmdResult<bool> {
    state.tunneled_host_forget(&args.host).await
}
