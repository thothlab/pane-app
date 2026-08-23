/**
 * Tag chips and the little editor that produces them.
 *
 * Tags label rules and collections. They carry no behaviour — the engine
 * never reads them — so the only thing they have to be good at is being
 * found again: a chip is clickable and drops `tag:<name>` into the rules
 * filter, which is the whole point of putting a label on something.
 *
 * A tag is a single token. The input splits on whitespace and commas, so
 * "smoke ios" becomes two tags rather than one two-word tag. That is not
 * a cosmetic rule: the filter grammar is whitespace-separated terms with
 * no quoting, so a tag containing a space could never be addressed by
 * `tag:` — it would be unfindable by the one mechanism it exists for.
 */

import { type Component, For, Show, createSignal } from "solid-js";
import { Tag as TagIcon, X } from "lucide-solid";
import { t } from "@/i18n";

/** Split raw input into tokens and drop what can't be a tag. */
export function splitTagInput(raw: string): string[] {
  return raw
    .split(/[\s,]+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/**
 * Append tags to a list, skipping blanks and case-insensitive duplicates —
 * the same normalisation the backend applies on write, done here so the UI
 * never shows a state the next read would contradict.
 */
export function addTags(existing: string[], incoming: string[]): string[] {
  const out = existing.slice();
  const seen = new Set(existing.map((tg) => tg.toLowerCase()));
  for (const tag of incoming) {
    const key = tag.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(tag);
  }
  return out;
}

/** Read-only chips. `onPick` makes them a filter shortcut.
 *
 *  Without `onPick` a chip is a `<span>`, not a dead button — the captures
 *  context menu renders these INSIDE its row button, and a button nested in a
 *  button is invalid markup whose click handling is browser's choice. */
export const TagChips: Component<{
  tags: string[];
  onPick?: (tag: string) => void;
  /** Muted variant for a row that is switched off. */
  dim?: boolean;
}> = (p) => {
  const chipClass = () =>
    `shrink-0 max-w-[160px] truncate rounded-full border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] leading-4 ${
      p.dim ? "opacity-60" : ""
    } ${p.onPick ? "hover:bg-accent/20 cursor-pointer" : "cursor-default"}`;

  return (
    <Show when={p.tags.length > 0}>
      <div class="flex items-center gap-1 flex-wrap min-w-0">
        <For each={p.tags}>
          {(tag) =>
            p.onPick ? (
              <button
                type="button"
                class={chipClass()}
                title={t()("rules.tag_filter_title", { tag })}
                onClick={(e) => {
                  // Rows are drag sources and section headers toggle on click.
                  e.stopPropagation();
                  p.onPick?.(tag);
                }}
                onDblClick={(e) => e.stopPropagation()}
              >
                {tag}
              </button>
            ) : (
              <span class={chipClass()} title={tag}>
                {tag}
              </span>
            )
          }
        </For>
      </div>
    </Show>
  );
};

/**
 * Chips with a remove button plus a free-text input. Commits on Enter, on
 * comma/space (via the split above) and on blur; Backspace in an empty
 * input removes the last chip, the convention every tag field uses.
 *
 * `suggestions` are the tags already in use elsewhere in the library, minus
 * the ones on this entity. Offered as one-click chips because the failure
 * mode of a free-text label is not typing effort, it is "smoke", "Smoke "
 * and "smoke-test" accumulating until no single query finds them all.
 */
export const TagEditor: Component<{
  tags: string[];
  onChange: (next: string[]) => void;
  suggestions?: string[];
  placeholder?: string;
  /**
   * The uncommitted text in the input, reported on every keystroke.
   *
   * A dialog whose Save button is gated on `tags.length > 0` needs it: a user
   * who typed "smoke" and reached straight for Save has no committed tag yet,
   * and a *disabled* button never takes the mousedown that would blur the
   * input and commit it — so the button stays dead exactly when it is needed.
   * Callers enable on `tags.length > 0 || draft !== ""`; the blur-commit then
   * lands before the click handler reads the tags.
   */
  onDraftChange?: (raw: string) => void;
  /** Focus the input on mount. For a dialog that exists only to take tags:
   *  it saves a click, and it puts the keyboard somewhere Escape can be
   *  heard from. */
  autoFocus?: boolean;
}> = (p) => {
  const [draft, setDraft] = createSignal("");

  const setDraftText = (raw: string) => {
    setDraft(raw);
    p.onDraftChange?.(raw);
  };

  const commit = () => {
    const parts = splitTagInput(draft());
    setDraftText("");
    if (parts.length === 0) return;
    p.onChange(addTags(p.tags, parts));
  };

  const remove = (tag: string) =>
    p.onChange(p.tags.filter((x) => x !== tag));

  const unusedSuggestions = () => {
    const mine = new Set(p.tags.map((tg) => tg.toLowerCase()));
    return (p.suggestions ?? []).filter((s) => !mine.has(s.toLowerCase()));
  };

  return (
    <div class="flex-1 space-y-1 min-w-0">
      <div class="flex items-center gap-1 flex-wrap">
        <For each={p.tags}>
          {(tag) => (
            <span class="inline-flex items-center gap-1 rounded-full border border-accent/40 bg-accent/10 px-2 py-0.5 text-xs">
              <span class="max-w-[200px] truncate">{tag}</span>
              <button
                type="button"
                class="text-fg-muted hover:text-danger"
                aria-label={t()("rules.tag_remove")}
                title={t()("rules.tag_remove")}
                onClick={() => remove(tag)}
              >
                <X size={10} />
              </button>
            </span>
          )}
        </For>
        <div class="inline-flex items-center gap-1 min-w-[140px] flex-1">
          <TagIcon size={12} class="text-fg-muted shrink-0" />
          <input
            ref={(el) => {
              if (p.autoFocus) setTimeout(() => el?.focus(), 0);
            }}
            class="flex-1 min-w-0 bg-bg border border-border rounded px-2 py-0.5 text-xs"
            placeholder={p.placeholder ?? t()("rules.tag_placeholder")}
            value={draft()}
            autocomplete="off"
            autocorrect="off"
            autocapitalize="off"
            spellcheck={false}
            onInput={(e) => setDraftText(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === "," || e.key === " ") {
                e.preventDefault();
                commit();
                return;
              }
              if (e.key === "Backspace" && draft() === "" && p.tags.length > 0) {
                e.preventDefault();
                p.onChange(p.tags.slice(0, -1));
              }
            }}
            onBlur={commit}
          />
        </div>
      </div>
      <Show when={unusedSuggestions().length > 0}>
        <div class="flex items-center gap-1 flex-wrap">
          <span class="text-[10px] text-fg-muted">{t()("rules.tag_suggestions")}</span>
          <For each={unusedSuggestions()}>
            {(tag) => (
              <button
                type="button"
                class="shrink-0 max-w-[160px] truncate rounded-full border border-border px-1.5 py-px text-[10px] leading-4 text-fg-muted hover:border-accent/40 hover:text-fg"
                onClick={() => p.onChange(addTags(p.tags, [tag]))}
              >
                {tag}
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};
