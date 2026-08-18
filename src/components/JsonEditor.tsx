// JSON-syntax-highlighted textarea. Implementation: a transparent
// <textarea> stacked on top of a coloured <pre> showing the same text
// tokenised by a small regex-based highlighter. Scroll position is
// synced from the textarea to the pre on every scroll event so they
// stay aligned even with very long bodies.
//
// Both layers MUST share identical font, line-height, padding, border
// and wrap rules — otherwise the visible coloured text drifts from the
// invisible textarea caret. The wrapping parent owns the border so the
// two children can be sized via `inset-0` without worrying about
// border-width math.
import { type Component, createMemo } from "solid-js";
import { highlightJson } from "@/lib/json-highlight";

type Props = {
  value: string;
  onInput: (next: string) => void;
  placeholder?: string;
  disabled?: boolean;
};

export const JsonEditor: Component<Props> = (p) => {
  let preRef: HTMLPreElement | undefined;
  let taRef: HTMLTextAreaElement | undefined;
  // The text, interned. Callers read `p.value` out of a larger object —
  // the rule editor's draft — so the prop's getter re-runs on every
  // keystroke ANYWHERE in that editor, name field included. Solid diffs
  // memos with `===`, so re-reading the same string here stops the change
  // dead: without this, typing a rule name re-tokenised a multi-megabyte
  // body and rebuilt every text node in the overlay on each character.
  const text = createMemo(() => p.value);
  // Trailing "\n " keeps the highlight overlay tall enough that the
  // caret on an empty last row still sits on the painted background
  // instead of falling off the bottom — <pre> otherwise collapses a
  // lone trailing newline.
  const tokens = createMemo(() => [...highlightJson(text()), "\n "]);

  const syncScroll = () => {
    if (preRef && taRef) {
      preRef.scrollTop = taRef.scrollTop;
      preRef.scrollLeft = taRef.scrollLeft;
    }
  };

  return (
    <div class="relative w-full h-full bg-bg border border-border rounded overflow-hidden focus-within:border-accent transition-colors">
      <pre
        ref={preRef}
        aria-hidden="true"
        class="absolute inset-0 m-0 px-2 py-1 text-xs font-mono leading-snug whitespace-pre-wrap break-words overflow-auto pointer-events-none text-fg"
      >
        {tokens()}
      </pre>
      <textarea
        ref={taRef}
        autocomplete="off"
        autocorrect="off"
        autocapitalize="off"
        spellcheck={false}
        class="absolute inset-0 w-full h-full bg-transparent px-2 py-1 text-xs font-mono leading-snug resize-none overflow-auto text-transparent caret-fg placeholder:text-fg-muted outline-none"
        placeholder={p.placeholder}
        disabled={p.disabled}
        // `text()`, not `p.value`: assigning a large string to .value on
        // every unrelated draft change is the same wasted work as the
        // re-tokenise above.
        value={text()}
        onScroll={syncScroll}
        onInput={(e) => p.onInput(e.currentTarget.value)}
      />
    </div>
  );
};

export default JsonEditor;
