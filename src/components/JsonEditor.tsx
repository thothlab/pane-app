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
import { type Component, type JSX, createMemo } from "solid-js";

type Props = {
  value: string;
  onInput: (next: string) => void;
  placeholder?: string;
  disabled?: boolean;
};

// Single regex with five alternatives. Order matters: the "string
// followed by colon" alternative must come before plain strings so
// object keys don't get coloured as values.
const JSON_TOKEN_RE = new RegExp(
  [
    // 1: object key string, 2: trailing colon (incl. whitespace before)
    /("(?:\\.|[^"\\])*")(\s*:)/.source,
    // 3: string value
    /("(?:\\.|[^"\\])*")/.source,
    // 4: number
    /(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/.source,
    // 5: keyword
    /(\btrue\b|\bfalse\b|\bnull\b)/.source,
    // 6: structural punctuation
    /([{}\[\],])/.source,
  ].join("|"),
  "g",
);

function highlight(src: string): JSX.Element[] {
  const out: JSX.Element[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  JSON_TOKEN_RE.lastIndex = 0;
  while ((m = JSON_TOKEN_RE.exec(src)) !== null) {
    if (m.index > last) out.push(src.slice(last, m.index));
    if (m[1] !== undefined) {
      out.push(<span class="text-accent">{m[1]}</span>);
      out.push(<span class="text-fg-muted">{m[2]}</span>);
    } else if (m[3] !== undefined) {
      out.push(<span class="text-success">{m[3]}</span>);
    } else if (m[4] !== undefined) {
      out.push(<span class="text-warn">{m[4]}</span>);
    } else if (m[5] !== undefined) {
      out.push(<span class="text-purple-400">{m[5]}</span>);
    } else if (m[6] !== undefined) {
      out.push(<span class="text-fg-muted">{m[6]}</span>);
    }
    last = m.index + m[0].length;
  }
  if (last < src.length) out.push(src.slice(last));
  // A trailing newline in textarea pushes the caret onto a new visual
  // row but `<pre>` collapses it; appending a space keeps the highlight
  // overlay tall enough that the caret on the empty last row still
  // sits on the painted background instead of falling off the bottom.
  out.push("\n ");
  return out;
}

export const JsonEditor: Component<Props> = (p) => {
  let preRef: HTMLPreElement | undefined;
  let taRef: HTMLTextAreaElement | undefined;
  const tokens = createMemo(() => highlight(p.value));

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
        value={p.value}
        onScroll={syncScroll}
        onInput={(e) => p.onInput(e.currentTarget.value)}
      />
    </div>
  );
};

export default JsonEditor;
