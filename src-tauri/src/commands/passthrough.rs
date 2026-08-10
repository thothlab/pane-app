//! Inspecting and clearing the set of hosts Pane refuses to decrypt.
//!
//! This state used to be invisible and unreachable: a host that once rejected
//! our certificate stayed tunnelled for the rest of the proxy run, with no
//! list, no reason, and no way to clear it short of a proxy restart the user
//! had to guess at. Everything here exists so "why is this host CONNECT" and
//! "try it again" are answerable from the UI.

use super::{to_api, CmdResult};
use crate::state::AppState;
use pane_ipc::{ForgetTunneledHostArgs, TunneledHostsDto};
use tauri::State;

#[tauri::command]
pub async fn tunneled_hosts_list(state: State<'_, AppState>) -> CmdResult<TunneledHostsDto> {
    Ok(state.no_mitm.list())
}

/// Forget every learned host, so the next connection to each is decrypted
/// again. Seeded `app_pin` patterns are unaffected — they aren't learned.
#[tauri::command]
pub async fn tunneled_hosts_reset(state: State<'_, AppState>) -> CmdResult<usize> {
    let n = state.no_mitm.reset();
    tracing::info!(hosts = n, "tunnelled-host set cleared by user");
    Ok(n)
}

#[tauri::command]
pub async fn tunneled_host_forget(
    state: State<'_, AppState>,
    args: ForgetTunneledHostArgs,
) -> CmdResult<bool> {
    if args.host.trim().is_empty() {
        return Err(to_api("bad_host")(anyhow::anyhow!("host is empty")));
    }
    Ok(state.no_mitm.forget(&args.host))
}
