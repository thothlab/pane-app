/**
 * Rules-view UI state — survives navigation and app restart.
 *
 * The Rules view is unmounted whenever the user navigates to another
 * tab (Captures, Settings, …), so any state held in
 * `createSignal` inside the component is lost. That made the common
 * flow "edit rule → switch to Captures → copy → switch back → paste"
 * painful: every section was collapsed again and the editor was
 * closed.
 *
 * Lifting these two pieces of state into a module-level signal keeps
 * them alive across remounts. Persisting them to localStorage also
 * survives app restart so a half-finished editing session is still
 * there the next day.
 *
 * `editing` is intentionally NOT cleared on save — RulesView keeps
 * pointing at the just-saved rule so it stays in editor view, same
 * as its old in-component behaviour. The caller clears it when
 * a rule is deleted or the user hits Cancel.
 */

import { createEffect, createSignal } from "solid-js";

export type RulesEditing =
  | { kind: "rule"; collectionId: string | null; id: string | "new" }
  | null;

const COLLAPSED_KEY = "pane:rules.collapsed";
const EDITING_KEY = "pane:rules.editing";

function loadCollapsed(): Record<string, boolean> {
  if (typeof localStorage === "undefined") return {};
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      const out: Record<string, boolean> = {};
      for (const [k, v] of Object.entries(parsed)) {
        if (typeof v === "boolean") out[k] = v;
      }
      return out;
    }
  } catch {
    /* corrupted — drop */
  }
  return {};
}

function loadEditing(): RulesEditing {
  if (typeof localStorage === "undefined") return null;
  try {
    const raw = localStorage.getItem(EDITING_KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as unknown;
    if (
      v &&
      typeof v === "object" &&
      (v as { kind?: unknown }).kind === "rule" &&
      ((v as { id?: unknown }).id === "new" ||
        typeof (v as { id?: unknown }).id === "string")
    ) {
      const cid = (v as { collectionId?: unknown }).collectionId;
      return {
        kind: "rule",
        collectionId: cid === null || typeof cid === "string" ? cid : null,
        id: (v as { id: string | "new" }).id,
      };
    }
  } catch {
    /* corrupted — drop */
  }
  return null;
}

const [rulesCollapsed, setRulesCollapsedRaw] = createSignal<Record<string, boolean>>(
  loadCollapsed(),
);
const [rulesEditing, setRulesEditingRaw] = createSignal<RulesEditing>(loadEditing());

export { rulesCollapsed, rulesEditing };

export function setRulesCollapsed(next: Record<string, boolean>): void {
  setRulesCollapsedRaw(next);
}

export function setRulesEditing(next: RulesEditing): void {
  setRulesEditingRaw(next);
}

createEffect(() => {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify(rulesCollapsed()));
  } catch {
    /* storage full / disabled */
  }
});

createEffect(() => {
  if (typeof localStorage === "undefined") return;
  try {
    const v = rulesEditing();
    if (v === null) localStorage.removeItem(EDITING_KEY);
    else localStorage.setItem(EDITING_KEY, JSON.stringify(v));
  } catch {
    /* storage full / disabled */
  }
});

// ── Editor drafts ──────────────────────────────────────────────────
//
// Per-editor draft persistence. Each open editor (existing rule or
// "new" in a given collection) gets its own key, so switching between
// rules — or between "edit rule X" and "new rule in collection C" —
// doesn't merge drafts. Stored as one localStorage entry per key
// (`pane:rules.draft:<id>`) so reading/writing one draft doesn't
// drag in everyone else's.
//
// Lifecycle: editor writes its current draft on every change and
// clears the key on save (the saved state is now authoritative) or
// on cancel (user discarded). Stale keys for rules deleted under us
// stick around — small string per orphan, not worth the cleanup
// machinery.

const DRAFT_PREFIX = "pane:rules.draft:";

export function ruleDraftKey(
  ruleId: string | null,
  defaultCollectionId: string | null,
): string {
  return ruleId ?? `new:${defaultCollectionId ?? ""}`;
}

export function loadRuleDraft<T>(key: string): T | null {
  if (typeof localStorage === "undefined") return null;
  try {
    const raw = localStorage.getItem(DRAFT_PREFIX + key);
    if (!raw) return null;
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

export function saveRuleDraft(key: string, draft: unknown): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(DRAFT_PREFIX + key, JSON.stringify(draft));
  } catch {
    /* storage full / disabled */
  }
}

export function clearRuleDraft(key: string): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.removeItem(DRAFT_PREFIX + key);
  } catch {
    /* ignore */
  }
}
