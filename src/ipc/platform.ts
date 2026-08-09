// Which shell is this bundle running in.
//
// The same SPA ships two ways: inside the Tauri desktop window, and served over
// HTTP by `pane serve` for a browser. Everything platform-specific — the IPC
// transport, file dialogs, clipboard, the updater — branches on this one
// predicate, so there is exactly one thing to get right and one thing to stub
// in a test.
//
// `__TAURI_INTERNALS__` rather than `__TAURI__`: the latter only exists when
// `withGlobalTauri` is enabled in tauri.conf.json, and it is not. This check
// predates the web build (it lived in lib/updater.ts) and is known to be
// correct in the packaged app.
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** True in the desktop app only. The browser build has no self-update path. */
export const updatesSupported = isTauri();
