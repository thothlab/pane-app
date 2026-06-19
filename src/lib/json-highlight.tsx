// Regex-based JSON tokenizer used for read-only highlighting in
// BodyViewer (Pretty mode) and as the overlay layer behind the
// editable JsonEditor textarea. Colours come from project tokens
// (text-accent / text-success / text-warn / text-fg-muted) so the
// highlight follows the app theme.
import type { JSX } from "solid-js";

// Single regex with five alternatives. Order matters: the "string
// followed by colon" alternative must come before plain strings so
// object keys don't get coloured as values.
const JSON_TOKEN_RE = new RegExp(
  [
    /("(?:\\.|[^"\\])*")(\s*:)/.source,
    /("(?:\\.|[^"\\])*")/.source,
    /(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/.source,
    /(\btrue\b|\bfalse\b|\bnull\b)/.source,
    /([{}\[\],])/.source,
  ].join("|"),
  "g",
);

export function highlightJson(src: string): JSX.Element[] {
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
  return out;
}
