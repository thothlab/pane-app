//! Inspecting and clearing the set of hosts Pane refuses to decrypt.
//!
//! This state used to be invisible and unreachable: a host that once rejected
//! our certificate stayed tunnelled for the rest of the proxy run, with no
//! list, no reason, and no way to clear it short of a proxy restart the user
//! had to guess at. Everything here exists so "why is this host CONNECT" and
//! "try it again" are answerable from the UI — and, now that the set lives on
//! the core rather than in the Tauri layer, from the CLI as well.

use pane_ipc::{kinds, TunneledHostsDto};

use crate::error::{api_err, CoreResult};
use crate::Core;

impl Core {
    pub async fn tunneled_hosts_list(&self) -> CoreResult<TunneledHostsDto> {
        Ok(self.no_mitm.list())
    }

    /// Forget every learned host, so the next connection to each is decrypted
    /// again. Seeded `app_pin` patterns are unaffected — they aren't learned.
    pub async fn tunneled_hosts_reset(&self) -> CoreResult<usize> {
        let n = self.no_mitm.reset();
        tracing::info!(hosts = n, "tunnelled-host set cleared by user");
        Ok(n)
    }

    pub async fn tunneled_host_forget(&self, host: &str) -> CoreResult<bool> {
        if host.trim().is_empty() {
            return Err(api_err(kinds::BAD_HOST, "host is empty"));
        }
        Ok(self.no_mitm.forget(host))
    }
}
