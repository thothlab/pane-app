// In-memory filter DSL for the Logcat window.
//
// All entries already live in the renderer's signal — we don't push
// down to SQLite. This compiles the DSL string into a predicate that
// runs over the buffer on every render tick. Keep it allocation-light.
//
// Grammar:
//   expr  := term (WS term)*
//   term  := '!'? atom
//   atom  := key ':' value | '~' regex | bareword
//   value := single (',' single)*    # comma → OR
//   key   := tag | level | pid | msg
//
// Examples:
//   "OkHttp"                       # bareword: substring in tag OR msg
//   "tag:OkHttp,Retrofit"          # tag OR tag
//   "level:E"                      # error only
//   "level:W..F"                   # warn or above
//   "pid:1234"
//   "~^(?!.*Connection).+"         # regex (negative lookahead)
//   "tag:OkHttp !msg:keep-alive"   # AND + negate
//
// Returns a predicate that's stable for the input — call sites usually
// memoize it in a createMemo over the typed text.

import type { LogEntry, LogLevel } from "@/views/LogcatView";

const LEVEL_ORDER: Record<LogLevel, number> = {
  verbose: 0,
  debug: 1,
  info: 2,
  warn: 3,
  error: 4,
  fatal: 5,
  silent: 6,
};

const LEVEL_FROM_TOKEN: Record<string, LogLevel> = {
  v: "verbose",
  verbose: "verbose",
  d: "debug",
  debug: "debug",
  i: "info",
  info: "info",
  w: "warn",
  warn: "warn",
  warning: "warn",
  e: "error",
  error: "error",
  f: "fatal",
  fatal: "fatal",
  s: "silent",
  silent: "silent",
};

type Predicate = (e: LogEntry) => boolean;

/// Split a value on `,` into trimmed non-empty parts. Single value
/// (no comma) → one-element array. Matches the comma-OR semantics we
/// use in the Captures filter for consistency.
function splitValues(value: string): string[] {
  if (!value.includes(",")) return [value];
  return value
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

function parseLevelRange(raw: string): { min: number; max: number } | null {
  // Accept "E", "W..F", "..E", "W..", and named forms.
  const range = raw.split("..");
  if (range.length === 1) {
    const lvl = LEVEL_FROM_TOKEN[range[0]!.toLowerCase()];
    if (!lvl) return null;
    const n = LEVEL_ORDER[lvl];
    return { min: n, max: n };
  }
  if (range.length === 2) {
    const [loRaw, hiRaw] = range;
    const lo = loRaw === "" ? 0 : LEVEL_FROM_TOKEN[loRaw!.toLowerCase()];
    const hi = hiRaw === "" ? LEVEL_ORDER.silent : LEVEL_FROM_TOKEN[hiRaw!.toLowerCase()];
    if (loRaw !== "" && !lo) return null;
    if (hiRaw !== "" && !hi) return null;
    return {
      min: loRaw === "" ? 0 : LEVEL_ORDER[lo as LogLevel],
      max: hiRaw === "" ? LEVEL_ORDER.silent : LEVEL_ORDER[hi as LogLevel],
    };
  }
  return null;
}

/// Build a substring matcher with optional `*` glob. Same flavour as
/// the Captures filter — explicit pattern means anchor-free, `*` maps
/// to "any chars".
function makeSubstringMatcher(value: string): (s: string) => boolean {
  if (value.includes("*")) {
    const re = new RegExp(value.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*"));
    return (s) => re.test(s);
  }
  const lower = value.toLowerCase();
  return (s) => s.toLowerCase().includes(lower);
}

function buildAtom(token: string): Predicate {
  const negate = token.startsWith("!");
  const body = negate ? token.slice(1) : token;

  // Regex form: ~pattern
  if (body.startsWith("~")) {
    const pattern = body.slice(1);
    let re: RegExp;
    try {
      re = new RegExp(pattern);
    } catch (err) {
      throw new Error(`bad regex: ${pattern}: ${(err as Error).message}`);
    }
    const pred: Predicate = (e) => re.test(e.tag) || re.test(e.message);
    return negate ? (e) => !pred(e) : pred;
  }

  const colon = body.indexOf(":");
  if (colon < 0) {
    // Bareword: substring across tag + message, OR.
    const match = makeSubstringMatcher(body);
    const pred: Predicate = (e) => match(e.tag) || match(e.message);
    return negate ? (e) => !pred(e) : pred;
  }

  const key = body.slice(0, colon).toLowerCase();
  const value = body.slice(colon + 1);
  const values = splitValues(value);
  if (values.length === 0) {
    return () => true;
  }

  let positive: Predicate;
  switch (key) {
    case "tag": {
      const matchers = values.map(makeSubstringMatcher);
      positive = (e) => matchers.some((m) => m(e.tag));
      break;
    }
    case "msg":
    case "message": {
      const matchers = values.map(makeSubstringMatcher);
      positive = (e) => matchers.some((m) => m(e.message));
      break;
    }
    case "level": {
      const ranges = values.map(parseLevelRange);
      if (ranges.some((r) => r === null)) {
        throw new Error(`bad level: ${value}`);
      }
      positive = (e) => {
        const n = LEVEL_ORDER[e.level];
        return ranges.some((r) => n >= r!.min && n <= r!.max);
      };
      break;
    }
    case "pid": {
      const nums = values.map((v) => {
        const n = parseInt(v, 10);
        if (!Number.isFinite(n)) throw new Error(`bad pid: ${v}`);
        return n;
      });
      positive = (e) => nums.includes(e.pid);
      break;
    }
    default:
      throw new Error(`unknown filter key: ${key}`);
  }
  return negate ? (e) => !positive(e) : positive;
}

/// Tokenize on whitespace, but keep `"quoted strings"` as single tokens
/// — lets the user filter on tags that contain spaces ("Pane Helper").
function tokenize(input: string): string[] {
  const out: string[] = [];
  let buf = "";
  let inQuote = false;
  for (const c of input) {
    if (c === '"') {
      inQuote = !inQuote;
      continue;
    }
    if ((c === " " || c === "\t") && !inQuote) {
      if (buf.length > 0) {
        out.push(buf);
        buf = "";
      }
      continue;
    }
    buf += c;
  }
  if (buf.length > 0) out.push(buf);
  return out;
}

export function compileLogcatFilter(input: string): Predicate {
  const trimmed = input.trim();
  if (!trimmed) return () => true;
  const tokens = tokenize(trimmed);
  if (tokens.length === 0) return () => true;
  const preds = tokens.map(buildAtom);
  // Tokens AND together (same as Captures filter — different keys are
  // additive constraints).
  return (e) => preds.every((p) => p(e));
}
