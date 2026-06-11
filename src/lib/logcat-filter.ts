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
//   key   := tag | level | pid | msg | app
//
// Examples:
//   "OkHttp"                       # bareword: substring in tag OR msg
//   "tag:OkHttp,Retrofit"          # tag OR tag
//   "level:E"                      # error only
//   "level:W..F"                   # warn or above
//   "pid:1234"
//   "app:com.foo.bar"              # resolves package → current pid;
//                                  # auto-tracks restarts
//   "~^(?!.*Connection).+"         # regex (negative lookahead)
//   "tag:OkHttp !msg:keep-alive"   # AND + negate
//
// `app:X` is special: it can't be evaluated in this pure-string pass
// because we don't know the PID without a device round-trip. The
// compiler returns its referenced package names alongside the
// predicate so the caller can resolve them out-of-band (via
// `android_pidof`) and intersect the resolved PIDs with the entry
// stream itself. The predicate treats `app:` tokens as always-true.
//
// Returns a `{ predicate, appPackages }` pair, stable for the input —
// call sites usually memoize it in a createMemo over the typed text.

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

export interface CompiledLogcatFilter {
  predicate: Predicate;
  /** Package names referenced via `app:` tokens. May contain
   *  duplicates if the user typed several `app:` terms. The caller
   *  resolves them to PIDs and applies an extra filter step. */
  appPackages: string[];
}

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

interface AtomResult {
  pred: Predicate;
  apps: string[];
}

function buildAtom(token: string): AtomResult {
  let negate = token.startsWith("!");
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
    return { pred: negate ? (e) => !pred(e) : pred, apps: [] };
  }

  const colon = body.indexOf(":");
  if (colon < 0) {
    // Bareword: substring across tag + message, OR.
    const match = makeSubstringMatcher(body);
    const pred: Predicate = (e) => match(e.tag) || match(e.message);
    return { pred: negate ? (e) => !pred(e) : pred, apps: [] };
  }

  const key = body.slice(0, colon).toLowerCase();
  let value = body.slice(colon + 1);
  // Accept `key:!value` as equivalent to `!key:value`. Lograbbit and
  // many other log viewers put the `!` after the colon, so users
  // reach for it that way; rejecting `tag:!Anal` as "tag must equal
  // literal '!Anal'" was a recurring surprise.
  if (value.startsWith("!")) {
    negate = !negate;
    value = value.slice(1);
  }
  const values = splitValues(value);
  if (values.length === 0) {
    return { pred: () => true, apps: [] };
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
    case "app": {
      // Predicate is always-true: the actual PID filtering happens
      // out-of-band, after the caller resolves these package names
      // via `android_pidof`. Negation isn't meaningful here — there's
      // no in-band predicate to invert — so we just ignore `!app:`
      // for now; can revisit if anyone hits that case.
      return { pred: () => true, apps: values };
    }
    default:
      throw new Error(`unknown filter key: ${key}`);
  }
  return { pred: negate ? (e) => !positive(e) : positive, apps: [] };
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

export function compileLogcatFilter(input: string): CompiledLogcatFilter {
  const trimmed = input.trim();
  if (!trimmed) return { predicate: () => true, appPackages: [] };
  const tokens = tokenize(trimmed);
  if (tokens.length === 0) return { predicate: () => true, appPackages: [] };
  const atoms = tokens.map(buildAtom);
  const apps: string[] = [];
  for (const a of atoms) {
    if (a.apps.length > 0) apps.push(...a.apps);
  }
  // Tokens AND together (same as Captures filter — different keys are
  // additive constraints).
  const predicate: Predicate = (e) => atoms.every((a) => a.pred(e));
  return { predicate, appPackages: apps };
}
