/**
 * Whether Copy / Export / cURL from Captures mask credentials (see
 * `lib/redact.ts`). On by default: a dump is usually pasted somewhere other
 * people read, and a leaked token is worse than a re-copy with masking off.
 *
 * Persisted in localStorage with the same cross-window sync as `theme.ts`.
 */

import { createSignal } from "solid-js";
import { emit, listen } from "@tauri-apps/api/event";

const STORAGE_KEY = "pane:mask-secrets";
const SYNC_EVENT = "pane://mask-secrets-changed";

// Anything but an explicit "false" means on, so a broken value fails safe.
function parse(raw: string | null): boolean {
  return raw !== "false";
}

function loadStored(): boolean {
  if (typeof localStorage === "undefined") return true;
  return parse(localStorage.getItem(STORAGE_KEY));
}

const [maskSignal, setMaskSignal] = createSignal<boolean>(loadStored());
export const maskSecrets = maskSignal;

export function setMaskSecrets(on: boolean): void {
  setMaskSignal(on);
  try {
    localStorage.setItem(STORAGE_KEY, String(on));
  } catch {
    // Private mode / disabled storage — keep in-memory.
  }
  void emit(SYNC_EVENT, on).catch(() => {});
}

if (typeof window !== "undefined") {
  void listen<boolean>(SYNC_EVENT, (e) => {
    setMaskSignal(e.payload !== false);
  }).catch(() => {});

  window.addEventListener("storage", (e) => {
    if (e.key !== STORAGE_KEY) return;
    setMaskSignal(parse(e.newValue));
  });
}
