// In-memory filter DSL for the Logcat window.
//
// All entries already live in the renderer's signal — we don't push
// down to SQLite. This compiles the DSL string into a predicate that
// runs over the buffer on every render tick. Keep it allocation-light.
//
// Grammar:
//   expr  := term (WS term)*
//   term  := '!'? atom
//   atom  := key ':' valuelist | '~' regex | bareword
//   value := '!'? single                  # per-value negation
//   valuelist := value (',' value)*       # comma → see semantics below
//   key   := tag | level | pid | msg | app
//
// Within a single key list (e.g. `tag:a,b,!c,!d`):
//   - positive values OR together         → (tag~a OR tag~b)
//   - negative values all must NOT match  → (tag!~c AND tag!~d)
//   - the two groups AND together         → ((a OR b)) AND (!c AND !d)
// Outer `!key:...` still flips the whole result.
//
// Examples:
//   "OkHttp"                              # bareword: substring in tag OR msg
//   "tag:OkHttp,Retrofit"                 # tag~OkHttp OR tag~Retrofit
//   "tag:!CatalogParser,!Spam,SSH"        # tag!~CatalogParser AND tag!~Spam AND tag~SSH
//   "level:E"                             # error only
//   "level:W..F"                          # warn or above
//   "pid:1234"
//   "app:com.foo.bar"                     # resolves package → current pid; auto-tracks restarts
//   "app:com.foo,!com.foo.helper"         # com.foo's pids minus com.foo.helper's pids
//   "~^(?!.*Connection).+"                # regex (negative lookahead)
//   "tag:OkHttp !msg:keep-alive"          # AND + outer negate
//
// `app:X` is special: it can't be evaluated in this pure-string pass
// because we don't know the PID without a device round-trip. The
// compiler returns the package names (with per-value negation) so the
// caller can resolve them out-of-band (via `android_pidof`) and
// intersect/subtract the resolved PIDs from the entry stream. The
// predicate treats `app:` tokens as always-true.
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

export interface AppPackageRef {
  pkg: string;
  negate: boolean;
}

export interface CompiledLogcatFilter {
  predicate: Predicate;
  /** Package names referenced via `app:` tokens, with per-value
   *  negation. The caller resolves package names to PIDs and applies
   *  the include/exclude logic. */
  appPackages: AppPackageRef[];
}

interface RawValue {
  value: string;
  negate: boolean;
}

/// Split a value on `,` into trimmed non-empty parts, picking up a
/// leading `!` per part as negation. Single value (no comma) → one-
/// element array. Matches the comma semantics in the file header.
function splitValues(value: string): RawValue[] {
  const parts = value.includes(",") ? value.split(",") : [value];
  const out: RawValue[] = [];
  for (const raw of parts) {
    let s = raw.trim();
    if (s.length === 0) continue;
    let negate = false;
    if (s.startsWith("!")) {
      negate = true;
      s = s.slice(1).trim();
    }
    if (s.length === 0) continue;
    out.push({ value: s, negate });
  }
  return out;
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

/// Combine positive (OR) and negative (all-must-NOT) value lists over
/// a single string-field extractor. Returns a predicate.
function combineStringValues(
  values: RawValue[],
  field: (e: LogEntry) => string,
): Predicate {
  const positives = values.filter((v) => !v.negate).map((v) => makeSubstringMatcher(v.value));
  const negatives = values.filter((v) => v.negate).map((v) => makeSubstringMatcher(v.value));
  return (e) => {
    const s = field(e);
    if (positives.length > 0 && !positives.some((m) => m(s))) return false;
    if (negatives.some((m) => m(s))) return false;
    return true;
  };
}

interface AtomResult {
  pred: Predicate;
  apps: AppPackageRef[];
}

function buildAtom(token: string): AtomResult {
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
  const value = body.slice(colon + 1);
  const values = splitValues(value);
  if (values.length === 0) {
    return { pred: () => true, apps: [] };
  }

  let positive: Predicate;
  switch (key) {
    case "tag": {
      positive = combineStringValues(values, (e) => e.tag);
      break;
    }
    case "msg":
    case "message": {
      positive = combineStringValues(values, (e) => e.message);
      break;
    }
    case "level": {
      const ranges = values.map((v) => {
        const r = parseLevelRange(v.value);
        if (r === null) throw new Error(`bad level: ${v.value}`);
        return { range: r, negate: v.negate };
      });
      const pos = ranges.filter((r) => !r.negate);
      const neg = ranges.filter((r) => r.negate);
      positive = (e) => {
        const n = LEVEL_ORDER[e.level];
        if (pos.length > 0 && !pos.some((r) => n >= r.range.min && n <= r.range.max)) return false;
        if (neg.some((r) => n >= r.range.min && n <= r.range.max)) return false;
        return true;
      };
      break;
    }
    case "pid": {
      const nums = values.map((v) => {
        const n = parseInt(v.value, 10);
        if (!Number.isFinite(n)) throw new Error(`bad pid: ${v.value}`);
        return { n, negate: v.negate };
      });
      const pos = nums.filter((p) => !p.negate).map((p) => p.n);
      const neg = nums.filter((p) => p.negate).map((p) => p.n);
      positive = (e) => {
        if (pos.length > 0 && !pos.includes(e.pid)) return false;
        if (neg.includes(e.pid)) return false;
        return true;
      };
      break;
    }
    case "app": {
      // Predicate is always-true: actual PID filtering happens out-of-
      // band, after the caller resolves these package names via
      // `android_pidof`. Outer `!app:` is not meaningful — there's no
      // in-band predicate to invert — so we drop it. Per-value `!`
      // inside the list IS meaningful and is passed through.
      return {
        pred: () => true,
        apps: values.map((v) => ({ pkg: v.value, negate: v.negate })),
      };
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
  const apps: AppPackageRef[] = [];
  for (const a of atoms) {
    if (a.apps.length > 0) apps.push(...a.apps);
  }
  // Tokens AND together (same as Captures filter — different keys are
  // additive constraints).
  const predicate: Predicate = (e) => atoms.every((a) => a.pred(e));
  return { predicate, appPackages: apps };
}
