/**
 * Tauri auto-updater integration.
 *
 * `checkForUpdatesOnStartup` runs once on boot. `checkForUpdatesNow` is
 * triggered by the user (button) or by a periodic timer/focus event —
 * difference is only how a "no update available" outcome is reported
 * (silent at boot/poll, friendly notice on manual click).
 *
 * Endpoint lives in `src-tauri/tauri.conf.json` under
 * `plugins.updater.endpoints` — currently `latest.json` of the GitHub
 * release marked `releases/latest`. Failures (no network, 404 before
 * manifest is published, signature mismatch) are logged but never
 * surfaced silently — we don't want to nag users about flaky checks.
 */

import { createSignal } from "solid-js";

type UpdateHandle = {
  version: string;
  downloadAndInstall: () => Promise<void>;
};

const [pending, setPending] = createSignal<UpdateHandle | null>(null);
const [lastCheckedAt, setLastCheckedAt] = createSignal<Date | null>(null);
const [checking, setChecking] = createSignal(false);

let bootChecked = false;

export const pendingUpdate = pending;
export const isCheckingForUpdates = checking;
export const updaterLastCheckedAt = lastCheckedAt;

export function pendingUpdateVersion(): string | null {
  return pending()?.version ?? null;
}

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function runCheck(): Promise<{ found: boolean; error?: unknown }> {
  if (!isTauri()) return { found: false };
  setChecking(true);
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    setLastCheckedAt(new Date());
    if (!update) {
      setPending(null);
      return { found: false };
    }
    setPending({
      version: update.version,
      downloadAndInstall: () => update.downloadAndInstall(),
    });
    return { found: true };
  } catch (e) {
    console.debug("[updater] check failed", e);
    return { found: false, error: e };
  } finally {
    setChecking(false);
  }
}

/** Fire-and-forget check during app boot. Silent on failure. */
export async function checkForUpdatesOnStartup(): Promise<void> {
  if (bootChecked || !isTauri()) return;
  bootChecked = true;
  await runCheck();
}

/** User-triggered ("Check for updates" button) or scheduled-tick check.
 *  Returns the outcome so the caller can toast/log; this module stays
 *  UI-agnostic. */
export async function checkForUpdatesNow(): Promise<{
  found: boolean;
  error?: unknown;
}> {
  return runCheck();
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
