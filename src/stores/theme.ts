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

export type Theme = "light" | "dark" | "system";

const STORAGE_KEY = "pane:theme";

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
}

createEffect(() => {
  if (typeof document === "undefined") return;
  document.documentElement.classList.toggle("dark", effectiveTheme() === "dark");
});

export function cycleTheme(): void {
  const order: Theme[] = ["light", "dark", "system"];
  setTheme(order[(order.indexOf(theme()) + 1) % order.length]!);
}
