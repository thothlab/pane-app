/**
 * Captures-view state that should survive route changes.
 *
 * Route components in Pane are lazy-loaded; navigating to /devices
 * unmounts CapturesView and drops all of its local signals. Anything the
 * user expects to "still be there" when they come back belongs here, at
 * module scope, so the signal instance is the same across mount/unmount.
 *
 * Filter string is also persisted to localStorage — survives full restarts.
 * The remaining fields (selectedId, paused) are session-only so a fresh
 * launch starts in a predictable state.
 */

import { createEffect, createSignal } from "solid-js";

const FILTER_STORAGE_KEY = "pane:captures-filter";

function loadFilter(): string {
  try {
    return localStorage.getItem(FILTER_STORAGE_KEY) ?? "";
  } catch {
    return "";
  }
}

const [filter, setFilter] = createSignal<string>(loadFilter());
const [selectedId, setSelectedId] = createSignal<string | null>(null);
const [paused, setPaused] = createSignal(false);

createEffect(() => {
  try {
    localStorage.setItem(FILTER_STORAGE_KEY, filter());
  } catch {
    /* private mode */
  }
});

export {
  filter,
  setFilter,
  selectedId,
  setSelectedId,
  paused,
  setPaused,
};
