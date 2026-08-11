/**
 * How often the Captures list re-queries the backend on its own.
 *
 * The list is event-driven first: `listenToCaptures` pushes on every completed
 * capture. This timer only exists to recover from a dropped event, so it is a
 * safety net rather than the primary path — which is why it can be slowed or
 * switched off without the list going stale under normal use.
 *
 * It became a setting because one number can't serve both callers. During an
 * automated run (Maestro driving an emulator through the CLI) the UI is
 * unattended and every poll is pure load on a backend that the run itself is
 * using — there, slow or off is right. Someone watching the list while they
 * tap through an app wants it to keep up — there, a second or two is right.
 *
 * Persisted in localStorage with the same cross-window sync as `theme.ts` and
 * `font-scale.ts`: Tauri's per-window WebViews don't reliably see each other's
 * `storage` events, so we also broadcast over the Tauri event bus.
 */

import { createSignal } from "solid-js";
import { emit, listen } from "@tauri-apps/api/event";

const STORAGE_KEY = "pane:captures-poll";
const SYNC_EVENT = "pane://captures-poll-changed";

/** Shipped default — the value the app used before this was configurable. */
export const DEFAULT_POLL_SECONDS = 10;

/** Below a second the timer stops being a safety net and becomes a load
 *  generator; above two minutes a dropped event is visible for long enough
 *  that the user would call it a bug. */
export const MIN_POLL_SECONDS = 1;
export const MAX_POLL_SECONDS = 120;

export interface CapturesPoll {
  enabled: boolean;
  seconds: number;
}

const FALLBACK: CapturesPoll = {
  enabled: true,
  seconds: DEFAULT_POLL_SECONDS,
};

export function clampSeconds(n: number): number {
  if (!Number.isFinite(n)) return DEFAULT_POLL_SECONDS;
  return Math.min(MAX_POLL_SECONDS, Math.max(MIN_POLL_SECONDS, Math.round(n)));
}

function parse(raw: string | null): CapturesPoll {
  if (!raw) return FALLBACK;
  try {
    const v = JSON.parse(raw) as Partial<CapturesPoll>;
    return {
      enabled: v.enabled !== false,
      seconds: clampSeconds(Number(v.seconds)),
    };
  } catch {
    return FALLBACK;
  }
}

function loadStored(): CapturesPoll {
  if (typeof localStorage === "undefined") return FALLBACK;
  return parse(localStorage.getItem(STORAGE_KEY));
}

const [pollSignal, setPollSignal] = createSignal<CapturesPoll>(loadStored());
export const capturesPoll = pollSignal;

export function setCapturesPoll(next: CapturesPoll): void {
  const value: CapturesPoll = {
    enabled: next.enabled,
    seconds: clampSeconds(next.seconds),
  };
  setPollSignal(value);
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
  } catch {
    // Private mode / disabled storage — keep in-memory.
  }
  void emit(SYNC_EVENT, value).catch(() => {});
}

export function setCapturesPollEnabled(enabled: boolean): void {
  setCapturesPoll({ ...pollSignal(), enabled });
}

export function setCapturesPollSeconds(seconds: number): void {
  setCapturesPoll({ ...pollSignal(), seconds });
}

export function resetCapturesPoll(): void {
  setCapturesPoll(FALLBACK);
}

if (typeof window !== "undefined") {
  void listen<CapturesPoll>(SYNC_EVENT, (e) => {
    const v = e.payload;
    if (v && typeof v === "object") {
      setPollSignal({
        enabled: v.enabled !== false,
        seconds: clampSeconds(Number(v.seconds)),
      });
    }
  }).catch(() => {});

  window.addEventListener("storage", (e) => {
    if (e.key !== STORAGE_KEY) return;
    setPollSignal(parse(e.newValue));
  });
}
