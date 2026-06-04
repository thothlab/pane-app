/**
 * Tauri auto-updater integration.
 *
 * On boot, `checkForUpdatesOnStartup` fetches the manifest at
 * `plugins.updater.endpoints` (see `src-tauri/tauri.conf.json` —
 * currently the `latest.json` asset of the GitHub release marked
 * `releases/latest`). If a newer version is offered, `pendingUpdate`
 * flips to a non-null handle and the sidebar shows an "Update" button.
 *
 * Failures (no network, 404 before manifest is published, signature
 * mismatch) are logged at debug level only — we don't want to nag
 * users about flaky checks every launch.
 */

import { createSignal } from "solid-js";

type UpdateHandle = {
  version: string;
  downloadAndInstall: () => Promise<void>;
};

const [pending, setPending] = createSignal<UpdateHandle | null>(null);

let bootChecked = false;

export const pendingUpdate = pending;

export function pendingUpdateVersion(): string | null {
  return pending()?.version ?? null;
}

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Fire-and-forget check during app boot. Silent on failure. */
export async function checkForUpdatesOnStartup(): Promise<void> {
  if (bootChecked || !isTauri()) return;
  bootChecked = true;
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) {
      setPending(null);
      return;
    }
    setPending({
      version: update.version,
      downloadAndInstall: () => update.downloadAndInstall(),
    });
  } catch (e) {
    console.debug("[updater] check failed", e);
  }
}

/** Download + install the pending bundle, then relaunch the app. */
export async function installPendingUpdate(): Promise<void> {
  const update = pending();
  if (!update) return;
  try {
    await update.downloadAndInstall();
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  } catch (e) {
    console.error("[updater] install failed", e);
    alert(`Update failed to install: ${(e as Error).message ?? e}`);
  }
}
