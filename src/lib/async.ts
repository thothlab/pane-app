// Bound a promise that talks to the backend.
//
// Tauri's `invoke` has no timeout of its own: if the backend is wedged, the
// promise simply never settles and the UI waits forever with nothing on screen
// to say so. That is indistinguishable from a frozen app. A deadline turns it
// into an error the caller can show.
//
// The underlying work is not cancelled — there is no cancellation channel
// through `invoke` — so this reports rather than aborts. That is the point:
// the user learns something is wrong instead of staring at a dead window.
export function withTimeout<T>(
  promise: Promise<T>,
  ms: number,
  message: string,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout>;
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(message)), ms);
  });
  return Promise.race([promise, deadline]).finally(() =>
    clearTimeout(timer),
  ) as Promise<T>;
}
