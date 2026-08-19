/**
 * The one dialog `askConfirm` renders. Mounted once, in Layout, so every
 * view gets it without wiring modal state through its own component tree.
 *
 * Keyboard and pointer behaviour follows the platform convention for a
 * destructive prompt: Escape and a backdrop click both cancel, and focus
 * lands on **Cancel**, not on the confirming button — so a stray Enter or
 * Space answers "no". There is deliberately no global Enter-confirms
 * shortcut for the same reason.
 */

import { type Component, Show, createEffect, onCleanup } from "solid-js";
import { AlertTriangle } from "lucide-solid";
import { pendingConfirm, resolveConfirm } from "@/stores/confirm";
import { t } from "@/i18n";

export const ConfirmDialog: Component = () => {
  let cancelEl: HTMLButtonElement | undefined;

  createEffect(() => {
    if (!pendingConfirm()) return;
    // Focus after the node is in the document.
    const id = setTimeout(() => cancelEl?.focus(), 0);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        resolveConfirm(false);
      }
    };
    // Capture phase: a view underneath may also listen for Escape (the
    // captures context menu does), and the modal on top owns the key.
    document.addEventListener("keydown", onKey, true);
    onCleanup(() => {
      clearTimeout(id);
      document.removeEventListener("keydown", onKey, true);
    });
  });

  return (
    <Show when={pendingConfirm()}>
      {(req) => (
        <div
          class="fixed inset-0 z-[100] flex items-center justify-center bg-black/40"
          onMouseDown={() => resolveConfirm(false)}
          role="dialog"
          aria-modal="true"
        >
          <div
            class="bg-bg border border-border rounded-lg p-5 shadow-xl max-w-md w-full mx-4 space-y-4"
            onMouseDown={(e) => e.stopPropagation()}
          >
            <div class="flex items-start gap-3">
              <Show when={req().danger}>
                <AlertTriangle size={24} class="text-danger shrink-0 mt-0.5" />
              </Show>
              <div class="space-y-1 min-w-0">
                <div class="font-medium text-sm break-words">{req().message}</div>
                <Show when={req().detail}>
                  <div class="text-xs text-fg-muted break-words">{req().detail}</div>
                </Show>
              </div>
            </div>
            <div class="flex justify-end gap-2">
              <button
                ref={cancelEl}
                type="button"
                class="text-sm px-3 py-1.5 rounded hover:bg-bg-muted text-fg-muted"
                onClick={() => resolveConfirm(false)}
              >
                {req().cancelLabel ?? t()("common.cancel")}
              </button>
              <button
                type="button"
                class={`text-sm px-3 py-1.5 rounded text-white hover:opacity-90 ${
                  req().danger ? "bg-danger" : "bg-accent"
                }`}
                onClick={() => resolveConfirm(true)}
              >
                {req().confirmLabel ?? t()("common.ok")}
              </button>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
};

export default ConfirmDialog;
