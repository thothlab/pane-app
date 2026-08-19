/**
 * In-app replacement for `window.confirm`.
 *
 * The native one cannot be used here at all. Tauri's webview (wry) sets a
 * `WKUIDelegate` that implements only the file-upload and media-permission
 * panels — there is no `runJavaScriptConfirmPanel`, and WKWebView's documented
 * behaviour without that delegate method is to return **false** immediately.
 * So on macOS every `if (!confirm(…)) return;` in this app was a guaranteed
 * early return: the dialog never appeared and the action never ran. Delete a
 * rule, delete a collection, delete a saved filter, clear captures, rotate the
 * CA — all silently did nothing, and "nothing happened" is indistinguishable
 * from "the button is broken", which is what it was.
 *
 * The shape is deliberately the same as `confirm()` so call sites read the
 * same way, one `await` apart:
 *
 *     if (!(await askConfirm({ message: … }))) return;
 *
 * A single pending request at a time — a confirmation is modal by definition,
 * and the host renders over everything. A second call while one is open
 * resolves the first as cancelled rather than stacking dialogs, so no caller
 * is ever left awaiting a promise that can no longer be answered.
 */

import { createSignal } from "solid-js";

export interface ConfirmRequest {
  /** Sentence the user has to agree with. Required. */
  message: string;
  /** Optional second line: consequences, counts, what is NOT affected. */
  detail?: string;
  /** Label of the confirming button. Defaults to a localized "Delete"/"OK". */
  confirmLabel?: string;
  cancelLabel?: string;
  /** Paints the confirm button as destructive. */
  danger?: boolean;
}

interface PendingConfirm extends ConfirmRequest {
  resolve: (ok: boolean) => void;
}

const [pendingConfirm, setPendingConfirm] = createSignal<PendingConfirm | null>(
  null,
);

export { pendingConfirm };

export function askConfirm(req: ConfirmRequest): Promise<boolean> {
  const previous = pendingConfirm();
  // Never leave an earlier caller awaiting a dialog that is no longer on
  // screen; cancelling is the safe answer for anything gated behind a
  // confirmation.
  if (previous) previous.resolve(false);
  return new Promise<boolean>((resolve) => {
    setPendingConfirm({ ...req, resolve });
  });
}

/** Answer the open dialog. No-op when nothing is pending. */
export function resolveConfirm(ok: boolean): void {
  const current = pendingConfirm();
  if (!current) return;
  setPendingConfirm(null);
  current.resolve(ok);
}
