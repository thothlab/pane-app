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

/**
 * Notify on every capture the proxy touches — both when a request starts and
 * when it finishes.
 *
 * `capture.completed` alone is not enough. A tunnelled connection emits it only
 * when `copy_bidirectional` returns, i.e. when one side hangs up, and a
 * keep-alive connection holds that open for minutes. Its row exists in the
 * database from the CONNECT onwards, so subscribing only to completion meant
 * such rows stayed invisible until some *other* request happened to finish —
 * which reads as "the list updates, but with a huge delay". The background
 * poll used to paper over it; making that poll optional exposed it.
 *
 * The caller debounces, so a burst of starts costs one refresh.
 */
export function listenToCaptures(onChanged: (id: string) => void) {
  const offs: UnlistenFn[] = [];
  for (const topic of ["capture://started", "capture://completed"]) {
    listen<{ id: string }>(topic, (e) => onChanged(e.payload.id)).then((fn) =>
      offs.push(fn),
    );
  }
  return () => {
    for (const off of offs) off();
    offs.length = 0;
  };
}
