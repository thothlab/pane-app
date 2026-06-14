/**
 * UI font scale — small / medium / large / xl.
 *
 * Persisted in localStorage (`pane:font-scale`). Side-effect: sets
 * `font-size` on `<html>`, so every Tailwind `rem`-based size
 * (`text-xs`, `text-sm`, padding/gap utilities) scales proportionally
 * with no per-component code changes. Same cross-window sync pattern
 * as `theme.ts` (localStorage + Tauri event) so the logcat WebView
 * tracks the main window.
 *
 * `small` matches browser default (16px root) — the size the app
 * shipped with before this control existed. New users see no change.
 *
 * Importing this module installs the effect — see `main.tsx`.
 */

import { createEffect, createSignal } from "solid-js";
import { emit, listen } from "@tauri-apps/api/event";

export type FontScale = "sm" | "md" | "lg" | "xl";

const STORAGE_KEY = "pane:font-scale";
const SYNC_EVENT = "pane://font-scale-changed";

// Root font-size in pixels per step. Browser default is 16px → `sm`
// reproduces today's look exactly. The +2px steps give a noticeable
// jump on each click without going so large that dense views (logcat
// table, captures list) start wrapping awkwardly.
export const ROOT_PX: Record<FontScale, number> = {
  sm: 16,
  md: 18,
  lg: 20,
  xl: 22,
};

const ORDER: FontScale[] = ["sm", "md", "lg", "xl"];

function loadStored(): FontScale {
  if (typeof localStorage === "undefined") return "sm";
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "sm" || v === "md" || v === "lg" || v === "xl" ? v : "sm";
}

const [scaleSignal, setScaleSignal] = createSignal<FontScale>(loadStored());
export const fontScale = scaleSignal;

export function setFontScale(s: FontScale): void {
  setScaleSignal(s);
  try {
    localStorage.setItem(STORAGE_KEY, s);
  } catch {
    // Private mode / disabled storage — keep in-memory.
  }
  void emit(SYNC_EVENT, s).catch(() => {});
}

export function bumpFontScale(delta: 1 | -1): void {
  const i = ORDER.indexOf(scaleSignal());
  const next = Math.max(0, Math.min(ORDER.length - 1, i + delta));
  setFontScale(ORDER[next]!);
}

export function resetFontScale(): void {
  setFontScale("sm");
}

createEffect(() => {
  if (typeof document === "undefined") return;
  document.documentElement.style.fontSize = `${ROOT_PX[scaleSignal()]}px`;
});

// Cross-window sync. Listen for broadcasts from sibling Tauri windows
// (logcat). Same rationale as theme.ts: storage events alone aren't
// reliable across Tauri's per-window WebViews.
if (typeof window !== "undefined") {
  void listen<FontScale>(SYNC_EVENT, (e) => {
    const v = e.payload;
    if (v === "sm" || v === "md" || v === "lg" || v === "xl") setScaleSignal(v);
  }).catch(() => {});

  window.addEventListener("storage", (e) => {
    if (e.key !== STORAGE_KEY) return;
    const v = e.newValue;
    if (v === "sm" || v === "md" || v === "lg" || v === "xl") setScaleSignal(v);
  });
}

export const FONT_SCALE_OPTIONS: ReadonlyArray<FontScale> = ORDER;
