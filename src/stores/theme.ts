/**
 * Theme — light / dark / system.
 *
 * Persisted in localStorage (`pane:theme`). System mode follows
 * `prefers-color-scheme` and re-evaluates when the OS appearance flips.
 * Side-effect: toggles `.dark` on `<html>` so Tailwind's `dark:` variants
 * and the CSS-var overrides in `styles/index.css` engage.
 *
 * Importing this module installs the effect — see `main.tsx`.
 */

import { createEffect, createSignal } from "solid-js";
import { emit, listen } from "@tauri-apps/api/event";

export type Theme = "light" | "dark" | "system";

const STORAGE_KEY = "pane:theme";
const SYNC_EVENT = "pane://theme-changed";

function loadStored(): Theme {
  if (typeof localStorage === "undefined") return "system";
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

const [themeSignal, setThemeSignal] = createSignal<Theme>(loadStored());
export const theme = themeSignal;

const [osDark, setOsDark] = createSignal(
  typeof window !== "undefined" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches,
);

if (typeof window !== "undefined") {
  const mql = window.matchMedia("(prefers-color-scheme: dark)");
  mql.addEventListener("change", (e) => setOsDark(e.matches));

  // Cross-window sync. The logcat window is a separate Tauri WebView
  // that imports this module too, but its in-memory signal is frozen
  // at load time — without this listener, flipping the theme in the
  // main window doesn't propagate, so logcat stays light. `storage`
  // fires in OTHER same-origin contexts when localStorage changes, so
  // this is exactly the right hook. (No-op in the originating window.)
  window.addEventListener("storage", (e) => {
    if (e.key !== STORAGE_KEY) return;
    const v = e.newValue;
    if (v === "light" || v === "dark" || v === "system") setThemeSignal(v);
  });
}

export function effectiveTheme(): "light" | "dark" {
  const t = theme();
  if (t !== "system") return t;
  return osDark() ? "dark" : "light";
}

export function setTheme(t: Theme): void {
  setThemeSignal(t);
  try {
    localStorage.setItem(STORAGE_KEY, t);
  } catch {
    // Private mode / disabled storage — keep in-memory.
  }
  // Broadcast to other Tauri windows (logcat). storage events alone
  // aren't reliable across Tauri's per-window WebViews, so a Tauri
  // event is the robust channel. Fire-and-forget — we don't care if
  // there are no listeners yet.
  void emit(SYNC_EVENT, t).catch(() => {});
}

// Subscribe to theme broadcasts from other windows. Each window
// installs its own listener at module load. No unsubscribe — the
// listener lives for the lifetime of the window, same as the signal.
if (typeof window !== "undefined") {
  void listen<Theme>(SYNC_EVENT, (e) => {
    const v = e.payload;
    if (v === "light" || v === "dark" || v === "system") setThemeSignal(v);
  }).catch(() => {});
}

createEffect(() => {
  if (typeof document === "undefined") return;
  document.documentElement.classList.toggle("dark", effectiveTheme() === "dark");
});

export function cycleTheme(): void {
  const order: Theme[] = ["light", "dark", "system"];
  setTheme(order[(order.indexOf(theme()) + 1) % order.length]!);
}
