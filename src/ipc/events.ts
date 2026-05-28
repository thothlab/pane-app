// Reactive Solid signals fed by the Tauri event bus.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { createSignal, onCleanup } from "solid-js";

export function useEvent<T>(topic: string, initial: T | null = null) {
  const [value, setValue] = createSignal<T | null>(initial);
  let unlisten: UnlistenFn | undefined;
  listen<T>(topic, (e) => setValue(() => e.payload)).then((fn) => (unlisten = fn));
  onCleanup(() => unlisten?.());
  return value;
}

export function listenToCaptures(onCompleted: (id: string) => void) {
  let off: UnlistenFn | undefined;
  listen<{ id: string }>("capture.completed", (e) => onCompleted(e.payload.id)).then(
    (fn) => (off = fn),
  );
  return () => off?.();
}
