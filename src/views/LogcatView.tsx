import {
  type Component,
  createEffect,
  createMemo,
  createSignal,
  For,
  type JSX,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { createVirtualizer } from "@tanstack/solid-virtual";
import {
  ArrowDown,
  Braces,
  Copy,
  Download,
  Filter as FilterIcon,
  Pause,
  Pencil,
  Pin,
  Play,
  Search as SearchIcon,
  Star,
  Trash2,
  Type,
  X,
} from "lucide-solid";
import { t, tr } from "@/i18n";
import { compileLogcatFilter } from "@/lib/logcat-filter";
import { savedFiltersFor } from "@/stores/saved-filters";
import { fontScale, ROOT_PX } from "@/stores/font-scale";
import { writeClipboard } from "@/lib/clipboard";
import { VerticalResizer } from "@/components/VerticalResizer";
import JsonEditor from "@/components/JsonEditor";

// Filters sidebar width, drag-resizable + persisted (px, independent of
// the font scale — same as the main window's sidebar).
const SIDEBAR_MIN = 120;
const SIDEBAR_MAX = 480;
const SIDEBAR_DEFAULT = 192;
const SIDEBAR_STORAGE_KEY = "pane.logcat.sidebar-width";

// Same palette as CapturesView's save popover so the colour dots in the
// two scopes look identical. Kept local rather than shared because there's
// no other consumer and inter-view coupling for six hex strings isn't
// worth a new module.
const FILTER_PALETTE = [
  "#60a5fa", // blue
  "#f87171", // red
  "#facc15", // yellow
  "#34d399", // green
  "#a78bfa", // purple
  "#fb923c", // orange
];

// Mirror of crates/pane-android/src/logcat.rs::LogEntry. Serde-renamed
// lowercase enum on the wire — keep this in sync.
export type LogLevel = "verbose" | "debug" | "info" | "warn" | "error" | "fatal" | "silent";

export interface LogEntry {
  timestamp: string;
  pid: number;
  tid: number;
  level: LogLevel;
  tag: string;
  message: string;
}

// Hard cap to bound memory on a chatty device. 100k entries × ~200B
// average = ~20 MB resident — fits comfortably and gives ~5 min of
// history even on a verbose firehose, so filtered-by-app views don't
// lose context as the unfiltered firehose churns. Older entries shift
// out FIFO.
const MAX_ENTRIES = 100_000;

const LEVEL_COLOR: Record<LogLevel, string> = {
  verbose: "text-fg-muted",
  debug: "text-accent",
  info: "text-success",
  warn: "text-warn",
  error: "text-danger",
  fatal: "text-danger font-bold",
  silent: "text-fg-muted",
};

// Whole-row tint by level. All cells inherit from the row — no cell
// hardcodes a colour any more, so time/pid/app/tag/message all tint
// together. The level cell still uses LEVEL_COLOR explicitly because
// it carries `font-bold` and matches what Android Studio's logcat
// does (one-letter level indicator stands out). Fatal also gets a
// soft red background so it's impossible to miss in a firehose.
const LEVEL_ROW_COLOR: Record<LogLevel, string> = {
  verbose: "text-fg-muted",
  debug: "text-accent",
  info: "text-success",
  warn: "text-warn",
  error: "text-danger",
  fatal: "text-danger font-bold bg-danger/10",
  silent: "text-fg-muted",
};

// Platform-aware shortcut labels for the toolbar inputs. macOS users
// expect the symbol form (⌘F / ⌘⇧F); Windows/Linux users expect the
// verbal form (Ctrl+F / Ctrl+Shift+F). Picked once at module load —
// the host OS doesn't change mid-session, no point making these
// reactive.
const IS_MAC_PLATFORM = /Mac|iPhone|iPad/.test(
  navigator.platform || navigator.userAgent,
);
const FILTER_HOTKEY_LABEL = IS_MAC_PLATFORM ? "⌘F" : "Ctrl+F";
const SEARCH_HOTKEY_LABEL = IS_MAC_PLATFORM ? "⌘⇧F" : "Ctrl+Shift+F";

const LEVEL_CHAR: Record<LogLevel, string> = {
  verbose: "V",
  debug: "D",
  info: "I",
  warn: "W",
  error: "E",
  fatal: "F",
  silent: "S",
};

// One log entry as a `threadtime`-format line — the same shape
// `adb logcat -v threadtime` emits and what Export writes. Shared by
// Export, the "Copy selected" command (⌘C) and the plain-text view so
// all three produce identical, paste-anywhere output.
function formatEntryLine(e: LogEntry): string {
  const ts = e.timestamp || "";
  const pid = String(e.pid).padStart(5);
  const tid = String(e.tid).padStart(5);
  const lvl = LEVEL_CHAR[e.level];
  return `${ts} ${pid} ${tid} ${lvl} ${e.tag}: ${e.message}`;
}

// Best-effort JSON pretty-printer that doesn't require valid input — it
// re-indents purely by structure (braces/brackets/commas), tracking
// strings so punctuation inside them is left alone. Used when JSON.parse
// fails (broken / truncated logs) so the readable part still formats,
// the way LogRabbit does. Strings keep their escapes; structural
// whitespace is collapsed and re-added by indent level.
function formatJsonLoose(src: string): string {
  let out = "";
  let indent = 0;
  let inStr = false;
  let esc = false;
  const pad = () => "  ".repeat(Math.max(0, indent));
  for (const c of src) {
    if (inStr) {
      out += c;
      if (esc) esc = false;
      else if (c === "\\") esc = true;
      else if (c === '"') inStr = false;
      continue;
    }
    switch (c) {
      case '"':
        inStr = true;
        out += c;
        break;
      case "{":
      case "[":
        indent++;
        out += `${c}\n${pad()}`;
        break;
      case "}":
      case "]":
        indent--;
        out += `\n${pad()}${c}`;
        break;
      case ",":
        out += `,\n${pad()}`;
        break;
      case ":":
        out += ": ";
        break;
      case " ":
      case "\t":
      case "\n":
      case "\r":
        break; // collapse — indentation is recomputed
      default:
        out += c;
    }
  }
  return out;
}

// Cap the plain-text snapshot so a 100k-line firehose doesn't build a
// pathologically large string / textarea value. The tail is what
// matters; narrow with a filter to see less.
const TEXT_VIEW_CAP = 5000;

/// Column header with a thin draggable right-edge handle. The handle
/// is a 1-px vertical line at the cell's right edge — it works as
/// both the visual column divider and the resize affordance.
/// Picks up the accent colour on hover/active. Double-click resets
/// that column to its default width.
const HeaderCell: Component<{
  label: string;
  onResize: (e: MouseEvent) => void;
  onReset?: () => void;
}> = (p) => (
  <span class="relative px-2">
    <span class="truncate">{p.label}</span>
    <span
      class="absolute top-0 right-0 h-full w-px bg-border cursor-col-resize hover:w-1 hover:bg-accent active:bg-accent"
      onMouseDown={p.onResize}
      onDblClick={() => p.onReset?.()}
      title="Drag to resize · double-click to reset"
    />
  </span>
);

const LogcatView: Component = () => {
  // ?serial=... + ?app_label=... come from the WebviewWindow URL set
  // by the Rust `logcat_open` command. serial is mandatory; we trust
  // it (the window won't have been created without one).
  const params = new URLSearchParams(window.location.search);
  const serial = params.get("serial") ?? "";
  const appLabel = params.get("app_label") ?? undefined;

  const [entries, setEntries] = createSignal<LogEntry[]>([]);
  const [paused, setPaused] = createSignal(false);
  const [autoScroll, setAutoScroll] = createSignal(true);
  const [filter, setFilter] = createSignal("");
  // Plain-substring search that stacks on top of the DSL filter. Narrows
  // the visible rows to those containing the term AND highlights every
  // match in-cell with <mark>. Separate from `filter` because the DSL
  // (`tag:X level:E`) is for shaping the firehose; this is the
  // "I know what I'm looking for, get me there" pass over the result.
  const [search, setSearch] = createSignal("");
  const [errorMsg, setErrorMsg] = createSignal<string | null>(null);
  // Rows open in the detail overlay (the only way to read a long,
  // single-line `truncate`d message in full — wrap mode fought the
  // virtualizer). One entry → metadata header + its message; multiple
  // (View message over a selection) → all the selected rows as text.
  const [detail, setDetail] = createSignal<LogEntry[] | null>(null);
  // Editable scratch text shown in the overlay's highlighted JSON editor:
  // a single row's message, or the selected rows as threadtime lines.
  // Format pretty-prints it when it parses as JSON.
  const [detailText, setDetailText] = createSignal("");
  const [detailFormatErr, setDetailFormatErr] = createSignal<string | null>(null);
  const openDetail = (rows: LogEntry[]) => {
    if (rows.length === 0) return;
    setDetailFormatErr(null);
    // Just the Message column of the selected rows — when a big JSON was
    // logged line-by-line, joining the messages reconstructs it (and
    // Format can then pretty-print it).
    setDetailText(rows.map((e) => e.message).join("\n"));
    setDetail(rows);
  };
  const formatDetail = () => {
    const raw = detailText();
    const trimmed = raw.trim();
    // Strictly valid → canonical pretty-print (and stays highlighted).
    try {
      const pretty = JSON.stringify(JSON.parse(raw), null, 2);
      if (pretty !== raw) setDetailText(pretty);
      setDetailFormatErr(null);
      return;
    } catch {
      /* fall through to best-effort */
    }
    // JSON-ish but broken/truncated → re-indent structurally anyway
    // (LogRabbit-style); it still highlights (the tokenizer is tolerant).
    if (trimmed[0] === "{" || trimmed[0] === "[") {
      const loose = formatJsonLoose(trimmed);
      if (loose !== raw) setDetailText(loose);
      setDetailFormatErr(null);
      return;
    }
    // Not JSON at all (plain log lines) — nothing to format.
    setDetailFormatErr(t()("logcat.detail_not_json"));
    setTimeout(() => setDetailFormatErr(null), 2000);
  };
  // Highlight when the content LOOKS like JSON (starts with { or [),
  // valid or not — the highlighter is a tolerant regex tokenizer, so
  // broken/truncated JSON still colours correctly. We only gate out
  // plain log lines (which start with a date), where the tokenizer would
  // paint stray numbers/keywords as noise.
  const detailLooksJson = createMemo(() => {
    const s = detailText().trim();
    return s.length > 0 && (s[0] === "{" || s[0] === "[");
  });

  // ── Row selection (LogRabbit-style) ───────────────────────────────
  // Selected entries, keyed by object identity (LogEntry has no id, but
  // the same object stays in the buffer until it FIFO-drops, so the Set
  // survives Tail churn — far safer than indices, which shift under
  // saturation). Click selects a row, Shift-click extends a range from
  // the anchor, ⌘/Ctrl-click toggles one. ⌘C copies the selected rows as
  // full threadtime lines; ⇧⌘C copies just their messages.
  const [selected, setSelected] = createSignal<Set<LogEntry>>(new Set());
  // Anchor index (into visible()) for Shift-range selection.
  let selectAnchor = -1;
  // Right-click row menu (Copy / Copy message / View message). Copy
  // actions operate on the whole selection; View targets the row the
  // menu was opened on.
  const [rowMenu, setRowMenu] = createSignal<{ x: number; y: number } | null>(null);
  let rowMenuRef: HTMLDivElement | undefined;
  let menuViewEntry: LogEntry | null = null;
  // Brief "copied N" confirmation shown in the status bar.
  const [copyHint, setCopyHint] = createSignal<string | null>(null);

  // ── Plain-text view (free-form select / scroll / copy) ────────────
  // A read-only textarea is the right tool for "grab arbitrary text":
  // native substring + multi-line selection, horizontal scroll to the
  // end of any long line, and copy — none of which a virtualized grid
  // can offer. Snapshotted on toggle (not live) so the content doesn't
  // shift under a selection while the firehose runs.
  const [textMode, setTextMode] = createSignal(false);
  const [textSnapshot, setTextSnapshot] = createSignal("");
  // PID → process name snapshot. Polled every 10s via
  // `android_pid_names` so the App column in the table can label
  // each entry with the package it came from. Accumulates across
  // ticks so historical entries from a process that's since exited
  // still display its name (PID reuse on Android is rare and gets
  // overwritten on the next tick when it happens).
  //
  // Also drives `app:<query>` filtering — `appPids` below is a
  // derived memo over this map. One source of truth means there's
  // no race between two separate poll cycles, and no chance for an
  // adb hiccup on one of them to flicker the filtered view back to
  // empty.
  // PID → set of every process-name ever seen on that PID. Set rather
  // than a single string because Android reuses PIDs across process
  // restarts: a tester's app dies and respawns with the same number
  // but Linux still cycles through PIDs eventually, so `ps -A` polled
  // 10s later can return that PID under a different package. The old
  // "Map<pid, string>" overwrote the previous binding on every poll —
  // and ALL of that PID's already-buffered entries (from when it was
  // ru.lewis.dbo, say) would silently drop out of an `app:ru.lewis.dbo`
  // filter the moment the next poll landed, manifesting as "Pause
  // empties the list" because pause coincided with the next poll
  // tick. Accumulating names per PID means once an entry matched the
  // filter, it keeps matching even after that PID has been recycled.
  const [pidNames, setPidNames] = createSignal<Map<number, Set<string>>>(new Map());

  // Resizable column widths. Persisted in localStorage so a user's
  // preferred layout survives close/reopen of the logcat window.
  // `level` is fixed-1-char; `message` takes whatever space is left
  // (`1fr`). Drag handles live only on Time/PID/Tag.
  type ColKey = "time" | "pid" | "app" | "tag";
  const COL_DEFAULTS: Record<ColKey, number> = {
    time: 90,
    pid: 60,
    app: 200,
    tag: 180,
  };
  const COL_MIN = 40;
  const COL_STORAGE_KEY = "pane.logcat.col-widths";

  // Per-column show/hide, with `level` and `message` included so the
  // header context-menu can hide them too. Persisted alongside widths
  // (separate key so an older build that doesn't know about visibility
  // still finds its widths).
  type AllCol = "time" | "pid" | "app" | "level" | "tag" | "message";
  const ALL_COLS: AllCol[] = ["time", "pid", "app", "level", "tag", "message"];
  const VISIBLE_DEFAULTS: Record<AllCol, boolean> = {
    time: true,
    pid: true,
    app: true,
    level: true,
    tag: true,
    message: true,
  };
  const VISIBLE_STORAGE_KEY = "pane.logcat.col-visible";
  const loadColVisible = (): Record<AllCol, boolean> => {
    try {
      const raw = localStorage.getItem(VISIBLE_STORAGE_KEY);
      if (!raw) return { ...VISIBLE_DEFAULTS };
      const parsed = JSON.parse(raw) as Partial<Record<AllCol, boolean>>;
      const out = { ...VISIBLE_DEFAULTS };
      for (const k of ALL_COLS) {
        if (typeof parsed[k] === "boolean") out[k] = parsed[k] as boolean;
      }
      // Refuse all-hidden — at least one column must stay visible so
      // there's still a place to right-click for the menu.
      if (!ALL_COLS.some((k) => out[k])) return { ...VISIBLE_DEFAULTS };
      return out;
    } catch {
      return { ...VISIBLE_DEFAULTS };
    }
  };
  const [colVisible, setColVisibleRaw] = createSignal(loadColVisible());
  const setColVisible = (next: Record<AllCol, boolean>) => {
    if (!ALL_COLS.some((k) => next[k])) return; // keep at least one
    setColVisibleRaw(next);
    try {
      localStorage.setItem(VISIBLE_STORAGE_KEY, JSON.stringify(next));
    } catch {
      /* storage unavailable */
    }
  };

  // Header context-menu state. Right-click anywhere on the header row
  // opens the column toggle list at the mouse position; outside click
  // closes it.
  const [headerMenuPos, setHeaderMenuPos] = createSignal<
    { x: number; y: number } | null
  >(null);
  let headerMenuRef: HTMLDivElement | undefined;
  const openHeaderMenu = (e: MouseEvent) => {
    e.preventDefault();
    setHeaderMenuPos({ x: e.clientX, y: e.clientY });
  };

  const loadColWidths = (): Record<ColKey, number> => {
    try {
      const raw = localStorage.getItem(COL_STORAGE_KEY);
      if (!raw) return { ...COL_DEFAULTS };
      const parsed = JSON.parse(raw) as Partial<Record<ColKey, number>>;
      return {
        time: clampWidth(parsed.time ?? COL_DEFAULTS.time),
        pid: clampWidth(parsed.pid ?? COL_DEFAULTS.pid),
        app: clampWidth(parsed.app ?? COL_DEFAULTS.app),
        tag: clampWidth(parsed.tag ?? COL_DEFAULTS.tag),
      };
    } catch {
      return { ...COL_DEFAULTS };
    }
  };
  function clampWidth(n: number): number {
    return Math.max(COL_MIN, Math.round(n));
  }
  const [colWidths, setColWidthsRaw] = createSignal(loadColWidths());
  const setColWidths = (next: Record<ColKey, number>) => {
    setColWidthsRaw(next);
    try {
      localStorage.setItem(COL_STORAGE_KEY, JSON.stringify(next));
    } catch {
      /* storage unavailable */
    }
  };
  const gridTemplate = () => {
    const w = colWidths();
    const v = colVisible();
    const parts: string[] = [];
    if (v.time) parts.push(`${w.time}px`);
    if (v.pid) parts.push(`${w.pid}px`);
    if (v.app) parts.push(`${w.app}px`);
    if (v.level) parts.push("14px");
    if (v.tag) parts.push(`${w.tag}px`);
    if (v.message) parts.push("1fr");
    return parts.join(" ");
  };

  // Initiate a drag-resize for one of the resizable columns. Single
  // window listeners during the drag; cursor + selection-block
  // applied on body so the user gets visual feedback and doesn't
  // accidentally select log text mid-drag.
  const startColResize = (col: ColKey, e: MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = colWidths()[col];
    const onMove = (ev: MouseEvent) => {
      const next = clampWidth(startW + (ev.clientX - startX));
      setColWidths({ ...colWidths(), [col]: next });
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  // Track whether the user is currently scrolled to the bottom. We
  // only auto-stick if they are — if they've scrolled up to read
  // something, we leave them there.
  //
  // The classic "ignore the scroll event we just generated" flag
  // doesn't work at firehose rate: auto-scroll fires ~10× per
  // second, so when the user actually does scroll up, the flag is
  // very likely set right at that moment and the user's intent
  // gets eaten — they're yanked back to the bottom and Follow stays
  // on no matter what they do. Instead we track scrollTop deltas:
  // programmatic scroll-to-bottom only ever moves scrollTop forward
  // (or keeps it the same once the buffer caps), so any decrease is
  // unambiguously user-driven.
  let scrollEl: HTMLDivElement | undefined;
  let lastScrollTop = 0;

  // Compile the filter once per typed input — cheap regex/parse, then
  // we apply the predicate over the buffer on every render tick.
  // The compiler also returns the list of `app:<pkg>` package names
  // it saw; we resolve those to PIDs out-of-band (see effect below).
  const matcher = createMemo(() => {
    try {
      setErrorMsg(null);
      return compileLogcatFilter(filter());
    } catch (e: unknown) {
      setErrorMsg((e as { message?: string })?.message ?? String(e));
      return {
        predicate: () => true,
        appPackages: [] as { pkg: string; negate: boolean }[],
      };
    }
  });

  // PIDs whose process name matches any `app:<query>` token in the
  // current filter, derived from the pidNames snapshot rather than
  // a separate poll. The accumulative nature of pidNames (we never
  // forget PIDs we've seen) means historical entries from a process
  // that's since exited still show up — their PID is still in the
  // map. New processes / restarts are picked up at the next
  // pidNames tick (10s).
  //
  // Declared before `visible` so the eager evaluation of the visible
  // memo's body doesn't hit a TDZ ReferenceError — Solid runs the
  // memo body once on creation to establish the initial value.
  // Two PID sets: `include` from positive `app:` values (e.g. `app:foo`)
  // and `exclude` from negated values (e.g. `app:!bar`). An entry passes
  // when its pid is in `include` (or include is empty) AND not in
  // `exclude`. Both sets derive from the same pidNames snapshot so they
  // can't disagree.
  const appPids = createMemo(() => {
    const apps = matcher().appPackages;
    const empty = { include: new Set<number>(), exclude: new Set<number>(), hasPositive: false };
    if (apps.length === 0) return empty;
    const pos = apps.filter((a) => !a.negate).map((a) => a.pkg.trim().toLowerCase()).filter(Boolean);
    const neg = apps.filter((a) => a.negate).map((a) => a.pkg.trim().toLowerCase()).filter(Boolean);
    const include = new Set<number>();
    const exclude = new Set<number>();
    for (const [pid, names] of pidNames()) {
      for (const name of names) {
        const lower = name.toLowerCase();
        if (pos.some((n) => lower.includes(n))) include.add(pid);
        if (neg.some((n) => lower.includes(n))) exclude.add(pid);
      }
    }
    return { include, exclude, hasPositive: pos.length > 0 };
  });

  // Substring search applied AFTER the DSL filter. Lowercased once per
  // typed input and checked against tag / message / app-name; the
  // numeric pid is also matched as a string so `search:1234` finds
  // entries from that pid even when the App column is empty.
  const searchLower = createMemo(() => search().trim().toLowerCase());
  const matchesSearch = (e: LogEntry, term: string, names: Map<number, Set<string>>): boolean => {
    if (!term) return true;
    if (e.tag.toLowerCase().includes(term)) return true;
    if (e.message.toLowerCase().includes(term)) return true;
    if (String(e.pid).includes(term)) return true;
    if (e.timestamp.toLowerCase().includes(term)) return true;
    const set = names.get(e.pid);
    if (set) {
      for (const name of set) {
        if (name.toLowerCase().includes(term)) return true;
      }
    }
    return false;
  };

  // The single source of truth for "does this entry pass the current
  // filter?" — DSL predicate + app:<pkg> PID gate + substring search,
  // combined. Shared by visible() (the rendered table) and the "+N new"
  // badge's pending count, so the badge reflects how many *filtered*
  // rows will appear on Tail rather than the raw firehose total. A
  // positive `app:X` whose package isn't running leaves `include` empty
  // → every entry fails the `!include.has(e.pid)` gate → empty result,
  // which surfaces the "app not running" state (same as the old
  // explicit early-return).
  const filterPredicate = createMemo(() => {
    const { predicate, appPackages } = matcher();
    const { include, exclude, hasPositive } = appPids();
    const term = searchLower();
    const names = pidNames();
    return (e: LogEntry): boolean => {
      if (appPackages.length > 0) {
        if (hasPositive && !include.has(e.pid)) return false;
        if (exclude.has(e.pid)) return false;
      }
      if (!predicate(e)) return false;
      if (term && !matchesSearch(e, term, names)) return false;
      return true;
    };
  });

  const visible = createMemo(() => entries().filter(filterPredicate()));

  // Poll PID → process-name snapshot. 10s cadence is enough — process
  // launches/exits are infrequent on a Logcat-watch timescale, and
  // `ps -A` is ~50ms over USB so cost is negligible.
  onMount(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const raw = await invoke<Record<string, string>>("android_pid_names", {
          serial,
        });
        if (cancelled) return;
        setPidNames((prev) => {
          const next = new Map(prev);
          for (const [k, v] of Object.entries(raw)) {
            const pid = Number(k);
            const existing = next.get(pid);
            if (existing) {
              if (!existing.has(v)) {
                // Clone before mutating so any consumer holding the
                // previous set reference still sees the value it was
                // handed (memo identity invariant).
                const merged = new Set(existing);
                merged.add(v);
                next.set(pid, merged);
              }
            } else {
              next.set(pid, new Set([v]));
            }
          }
          return next;
        });
      } catch {
        // adb hiccup — keep the previous snapshot, try again next tick.
      }
    };
    void tick();
    const handle = setInterval(tick, 10000);
    onCleanup(() => {
      cancelled = true;
      clearInterval(handle);
    });
  });

  // Incoming batches are held here between rAF ticks and (when Follow
  // is OFF) for as long as the user stays scrolled away from the
  // bottom. Lifted to component scope so toggleAutoScroll() can
  // drain them when the user re-engages Follow.
  let pending: LogEntry[][] = [];
  let pendingTotal = 0;
  let flushScheduled = false;
  let rafHandle: number | undefined;
  // Surfaced in the status bar as a "+N new" badge so the user can
  // see the stream is still alive while their view is frozen. Counts
  // only entries that pass the active filter — i.e. the number of rows
  // that will actually appear in the table when Tail re-engages, not
  // the raw firehose total. Only updated while !autoScroll(); under
  // Follow ON, pending drains within one rAF tick and a transient
  // setter would flash the badge on every batch.
  const [pendingCount, setPendingCount] = createSignal(0);

  // Re-derive the filtered pending count from scratch over the whole
  // `pending` buffer. Used for the cases where the running incremental
  // total can't be patched cheaply: a FIFO drop from the front, the
  // filter/search changing, or Tail being switched off (establishes the
  // baseline). No-op under Follow ON — the badge is hidden then.
  const recountPending = () => {
    if (autoScroll()) return;
    const pred = filterPredicate();
    let n = 0;
    for (const batch of pending) {
      for (const e of batch) if (pred(e)) n++;
    }
    setPendingCount(n);
  };

  // When the filter/search changes (or Tail flips off) while frozen,
  // recompute the badge so it tracks the active filter over everything
  // already held in `pending`.
  createEffect(() => {
    filterPredicate();
    if (!autoScroll()) recountPending();
  });

  // Coalesce incoming batches through requestAnimationFrame: when
  // adb logcat is first attached, the ring buffer dumps thousands
  // of entries in 1–2 seconds (50–100+ IPC events/sec). Each event
  // would trigger setEntries → visible() recompute → virtualizer
  // recompute, monopolising the main thread and starving the OS
  // resize-event queue. With rAF coalescing we collapse N batches
  // arriving between two frames into a single setEntries call;
  // the user sees the window resize react instantly while the
  // logs still load smoothly behind it. Steady-state firehose
  // (post-init) still benefits — 60Hz UI updates regardless of
  // backend event rate.
  //
  // When !autoScroll() flush() early-returns without touching
  // entries. This is the fix for the "constant flicker with Follow
  // off" bug: committing FIFO-shifted batches rotates the buffer
  // under the user's anchor, and the virtualizer's scroll-offset
  // catches up async via the scroll event (WebKit queues those),
  // so for one frame scrollTop and the rendered range disagree.
  // Holding batches until the user re-engages Follow eliminates the
  // churn entirely — same UX as IntelliJ logcat / Console.app.
  const flush = () => {
    flushScheduled = false;
    rafHandle = undefined;
    if (pending.length === 0) return;
    if (!autoScroll()) return;
    const merged: LogEntry[] =
      pending.length === 1 ? pending[0]! : pending.flat();
    pending = [];
    pendingTotal = 0;
    setPendingCount(0);
    setEntries((prev) => {
      const next = prev.length === 0 ? merged : prev.concat(merged);
      return next.length > MAX_ENTRIES
        ? next.slice(next.length - MAX_ENTRIES)
        : next;
    });
  };

  const scheduleFlush = () => {
    if (flushScheduled) return;
    flushScheduled = true;
    rafHandle = requestAnimationFrame(flush);
  };

  // Subscribe to the per-window batched stream. Backend emits
  // `logcat://batch` with payload Vec<LogEntry> every 100ms / 50
  // entries (whichever first) on this WebviewWindow only — so the
  // main window never sees the firehose.
  onMount(() => {
    let unlistenBatch: UnlistenFn | undefined;
    let unlistenError: UnlistenFn | undefined;

    listen<LogEntry[]>("logcat://batch", (e) => {
      if (paused()) return;
      pending.push(e.payload);
      pendingTotal += e.payload.length;
      // Cap pending at MAX_ENTRIES so a user who scrolls up and
      // walks away doesn't blow out memory. FIFO drop from the
      // front — same as the entries() ring buffer.
      let didDrop = false;
      while (pendingTotal > MAX_ENTRIES && pending.length > 0) {
        const dropped = pending.shift()!;
        pendingTotal -= dropped.length;
        didDrop = true;
      }
      // Update the filtered "+N new" badge while frozen. A drop
      // invalidates the running total (we don't track how many of the
      // dropped batch matched), so recount fully on that rare path;
      // otherwise just add this batch's matches.
      if (!autoScroll()) {
        if (didDrop) {
          recountPending();
        } else {
          const pred = filterPredicate();
          let add = 0;
          for (const en of e.payload) if (pred(en)) add++;
          if (add > 0) setPendingCount(pendingCount() + add);
        }
      }
      scheduleFlush();
    }).then((u) => (unlistenBatch = u));

    listen<{ message: string }>("logcat://error", (e) => {
      setErrorMsg(e.payload.message);
    }).then((u) => (unlistenError = u));

    // Kick the backend. logcat_open is idempotent — if the window
    // already had a stream (e.g. it was reopened from the main app),
    // this returns immediately without double-spawn.
    invoke("logcat_open", { serial, appLabel: appLabel ?? null }).catch((err) => {
      setErrorMsg(typeof err === "string" ? err : (err?.message ?? String(err)));
    });

    onCleanup(() => {
      unlistenBatch?.();
      unlistenError?.();
      if (rafHandle !== undefined) cancelAnimationFrame(rafHandle);
    });
  });

  // Auto-scroll to bottom when new entries arrive and the user hasn't
  // scrolled up.
  //
  // Depend on the full `visible()` reference, not just its length.
  // Once the ring buffer hits MAX_ENTRIES, length is pinned at the
  // cap and FIFO turnover only changes the array reference — depending
  // on length alone would freeze auto-scroll at saturation, which was
  // the "logcat stopped updating" bug users hit on long-running
  // sessions.
  //
  // We go through virtualizer.scrollToIndex rather than setting
  // scrollEl.scrollTop directly: a manual scrollTop write fires the
  // browser scroll event ASYNC, and until that event reaches the
  // virtualizer's offset observer its internal range is still pinned
  // at the previous position. For one or two frames the rendered
  // virtual rows live way above the new (taller) content's bottom and
  // the viewport paints blank — that was the per-second flicker users
  // saw in the firehose. scrollToIndex updates the virtualizer's own
  // offset signal coherently, so getVirtualItems() returns the new
  // tail range in the same render pass.
  createEffect(() => {
    void visible();
    if (!autoScroll() || !scrollEl) return;
    const count = visible().length;
    if (count === 0) return;
    queueMicrotask(() => {
      if (!scrollEl) return;
      virtualizer.scrollToIndex(count - 1, { align: "end" });
      lastScrollTop = scrollEl.scrollTop;
    });
  });

  // Detect user scroll-away from the bottom and switch auto-scroll
  // off; symmetrically, when the user scrolls back down to the
  // bottom, re-engage Follow and drain any held batches.
  //
  // We compare against lastScrollTop instead of computing "is at
  // bottom?" for the OFF transition — at firehose rates the
  // programmatic auto-scroll fires many times per second, and any
  // "is at bottom" check loses the race vs. the user's scroll-up
  // event. A 4px slack absorbs sub-pixel wheel jitter; anything
  // bigger than that going backwards is the user.
  //
  // The ON transition is safe to gate on "at bottom" because with
  // Follow OFF entries() is frozen — there are no programmatic
  // scrolls competing with the user's input.
  const onScroll = () => {
    if (!scrollEl) return;
    const cur = scrollEl.scrollTop;
    // Don't read a scroll event as "user scrolled up" when the
    // content just collapsed under them — e.g. Clear empties
    // entries(), the inner div height drops to 0, the browser snaps
    // scrollTop from its previous high value down to 0, and the OFF
    // transition below would interpret that as the user fleeing the
    // bottom and silently turn Tail off. Gate the OFF flip on there
    // being something to scroll through; with zero rows there is no
    // "scrolled up" state, only emptiness.
    if (cur < lastScrollTop - 4 && autoScroll() && visible().length > 0) {
      setAutoScroll(false);
    } else if (!autoScroll() && cur > lastScrollTop) {
      const atBottom =
        cur + scrollEl.clientHeight >= scrollEl.scrollHeight - 4;
      if (atBottom) {
        setAutoScroll(true);
        scheduleFlush();
      }
    }
    lastScrollTop = cur;
  };

  const togglePause = () => setPaused(!paused());
  const clearAll = () => {
    pending = [];
    pendingTotal = 0;
    setPendingCount(0);
    setEntries([]);
    // The scroll port is about to collapse to 0 height; reset the
    // baseline now so the post-collapse scroll event compares a
    // 0 against a 0 instead of a 0 against the old (huge) value.
    // Belt-and-braces with the visible().length > 0 guard in
    // onScroll — that one already blocks the OFF flip, this one
    // keeps lastScrollTop coherent for whatever scroll happens
    // next.
    lastScrollTop = 0;
    setSelected(new Set<LogEntry>());
    selectAnchor = -1;
  };

  // LogRabbit-style WHOLE-ROW selection (not text selection — rows are
  // `select-none`; for free-form substring/copy use the Text view). Rows
  // are translateY'd, so native cross-row text selection is janky and
  // copies cell-by-cell; a row model sidesteps that entirely.
  //
  // The visible() entries within [a, b] (inclusive), by identity.
  const rangeSet = (a: number, b: number): Set<LogEntry> => {
    const list = visible();
    const lo = Math.min(a, b);
    const hi = Math.max(a, b);
    const out = new Set<LogEntry>();
    for (let i = lo; i <= hi; i++) {
      const en = list[i];
      if (en) out.add(en);
    }
    return out;
  };

  // True while the mouse is held down after starting a drag-select.
  let rowDragging = false;

  // mousedown begins selection: Shift extends a range from the anchor,
  // ⌘/Ctrl toggles one row, a plain press selects one and arms a drag.
  const onRowMouseDown = (ev: MouseEvent, index: number) => {
    if (ev.button !== 0) return; // left button only
    const entry = visible()[index];
    if (!entry) return;
    if (ev.shiftKey && selectAnchor >= 0) {
      setSelected(rangeSet(selectAnchor, index));
    } else if (ev.metaKey || ev.ctrlKey) {
      const next = new Set(selected());
      if (next.has(entry)) next.delete(entry);
      else next.add(entry);
      setSelected(next);
      selectAnchor = index;
    } else {
      setSelected(new Set([entry]));
      selectAnchor = index;
      rowDragging = true;
    }
  };

  // While dragging, hovering a row extends the range from the anchor —
  // gives the click-and-drag-down full-row sweep LogRabbit has.
  const onRowMouseEnter = (index: number) => {
    if (!rowDragging || selectAnchor < 0) return;
    setSelected(rangeSet(selectAnchor, index));
  };

  // Right-click opens the row menu. If the clicked row isn't already in
  // the selection, make it the selection — so Copy acts on something
  // predictable; if it IS selected, the whole multi-selection is kept.
  const onRowContextMenu = (ev: MouseEvent, index: number) => {
    ev.preventDefault();
    rowDragging = false;
    const entry = visible()[index];
    if (!entry) return;
    if (!selected().has(entry)) {
      setSelected(new Set([entry]));
      selectAnchor = index;
    }
    menuViewEntry = entry;
    // Clamp to the viewport so the menu never spills past the bottom/right
    // edge of the window. The menu is a fixed 3-item list (~110px tall,
    // 200px wide); flip the anchor back inside if it would overflow.
    const MENU_W = 200;
    const MENU_H = 110;
    const x = Math.max(4, Math.min(ev.clientX, window.innerWidth - MENU_W));
    const y = Math.max(4, Math.min(ev.clientY, window.innerHeight - MENU_H));
    setRowMenu({ x, y });
  };

  const closeRowMenu = () => setRowMenu(null);
  const menuCopy = (messageOnly: boolean) => {
    copySelected(messageOnly);
    closeRowMenu();
  };
  // Most-recently-seen process name for a pid (the App column value).
  // Same logic as the row's inline appName, hoisted so the detail overlay
  // can show and copy it too.
  const appNameForPid = (pid: number): string => {
    const set = pidNames().get(pid);
    if (!set || set.size === 0) return "";
    let last = "";
    for (const n of set) last = n;
    return last;
  };

  const menuView = () => {
    // View the whole selection (the right-clicked row was folded into it
    // by onRowContextMenu), in visible() order; fall back to the clicked
    // row if somehow nothing is selected.
    const rows = visible().filter((en) => selected().has(en));
    if (rows.length > 0) openDetail(rows);
    else if (menuViewEntry) openDetail([menuViewEntry]);
    closeRowMenu();
  };

  // Copy the selected rows to the clipboard, in visible() order.
  // `messageOnly` (⇧⌘C) emits just the message field; otherwise (⌘C) the
  // full threadtime line. Returns false when nothing is selected so the
  // key handler can fall through. The clipboard write is AWAITED and the
  // "copied N" hint only shows on success — earlier it fired-and-forgot,
  // so a permission denial in the logcat window claimed success while the
  // clipboard kept its old contents.
  const copySelected = (messageOnly: boolean): boolean => {
    const sel = selected();
    if (sel.size === 0) return false;
    const ordered = visible().filter((en) => sel.has(en));
    if (ordered.length === 0) return false;
    const text = ordered
      .map((en) => (messageOnly ? en.message : formatEntryLine(en)))
      .join("\n");
    void writeClipboard(text)
      .then(() => {
        setCopyHint(tr("logcat.copied", { n: String(ordered.length) }));
        setTimeout(() => setCopyHint(null), 2000);
      })
      .catch((e: unknown) => {
        setCopyHint(
          tr("logcat.copy_failed", {
            message: (e as { message?: string })?.message ?? String(e),
          }),
        );
        setTimeout(() => setCopyHint(null), 4000);
      });
    return true;
  };

  // Toggle the plain-text view. Snapshots the (filter-narrowed, tail-
  // capped) lines on enter so the content stays put for selecting and
  // scrolling while the firehose keeps running underneath.
  const toggleTextMode = () => {
    const next = !textMode();
    if (next) {
      const v = visible();
      const slice = v.length > TEXT_VIEW_CAP ? v.slice(v.length - TEXT_VIEW_CAP) : v;
      setTextSnapshot(slice.map(formatEntryLine).join("\n"));
    }
    setTextMode(next);
  };

  /// Serialize the currently-visible entries (after filter + follow-app
  /// constraints) into a plain-text `.log` file. Format mirrors what
  /// `adb logcat -v threadtime` emits so the result drops into any
  /// log viewer (Android Studio, logbook, grep) unmodified.
  const exportLog = async () => {
    const lines = visible().map(formatEntryLine);
    const defaultName = appLabel
      ? `${appLabel}-${Date.now()}.log`
      : `logcat-${serial}-${Date.now()}.log`;
    const path = await save({
      defaultPath: defaultName,
      filters: [{ name: "Log", extensions: ["log"] }],
    });
    if (!path) return;
    try {
      // Backend command (Rust std::fs::write) instead of plugin-fs —
      // plugin-fs's write_text_file requires a per-capability scope
      // rule whitelisting the path, which doesn't make sense for a
      // user-chosen save dialog target. Same pattern as ca.save_to_file.
      await invoke("logcat_write_export", { path, content: lines.join("\n") + "\n" });
    } catch (e: unknown) {
      setErrorMsg(
        tr("logcat.export_failed", {
          message: (e as { message?: string })?.message ?? String(e),
        }),
      );
    }
  };

  const toggleAutoScroll = () => {
    const next = !autoScroll();
    setAutoScroll(next);
    if (next) {
      // Re-engaging Follow: drain any batches that piled up while
      // we were holding them, then snap to the bottom. The drain
      // commits into entries() on the next rAF and the auto-scroll
      // createEffect catches the visible() change to scroll to
      // the new bottom; the explicit scroll here just covers the
      // case where pending was empty.
      scheduleFlush();
      const count = visible().length;
      if (scrollEl && count > 0) {
        virtualizer.scrollToIndex(count - 1, { align: "end" });
        lastScrollTop = scrollEl.scrollTop;
      }
    }
  };

  // Hotkeys: Cmd/Ctrl-K clears, Cmd/Ctrl-F focuses the filter input.
  // Space-as-pause was removed — even with a `document.activeElement`
  // guard, Tauri's WebKit would intermittently eat the first space
  // after a click into the filter input, mangling sequences like
  // `app:foo tag:bar` into `app:footag:bar`. The toolbar Pause
  // button is right there; the global hotkey wasn't worth the
  // recurring bug.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && rowMenu()) {
        e.preventDefault();
        setRowMenu(null);
        return;
      }
      if (e.key === "Escape" && detail()) {
        e.preventDefault();
        setDetail(null);
        return;
      }
      if (e.key === "Escape" && selected().size > 0) {
        e.preventDefault();
        setSelected(new Set<LogEntry>());
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "c") {
        // Let inputs and the text-view textarea keep native copy, and
        // let any live text selection win; only fall back to copying
        // the selected rows. ⇧⌘C copies messages only, ⌘C full lines.
        const tag = (document.activeElement?.tagName || "").toLowerCase();
        if (tag === "textarea" || tag === "input") return;
        const tsel = window.getSelection();
        if (tsel && !tsel.isCollapsed && tsel.toString().length > 0) return;
        if (copySelected(e.shiftKey)) e.preventDefault();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        clearAll();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        // Cmd+Shift+F → quick substring search; Cmd+F (no shift) → DSL
        // filter. Two hotkeys because the two inputs serve different
        // mental modes: shape the firehose vs. find a specific token in
        // what's already on screen.
        if (e.shiftKey) searchInputRef?.focus();
        else filterInputRef?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    // End a row drag-select wherever the mouse is released.
    const onUp = () => {
      rowDragging = false;
    };
    window.addEventListener("mouseup", onUp);
    onCleanup(() => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mouseup", onUp);
    });
  });

  let filterInputRef: HTMLInputElement | undefined;
  let filterOverlayRef: HTMLDivElement | undefined;
  let searchInputRef: HTMLInputElement | undefined;

  // Saved-filters scope. The logcat window has its own filter list,
  // kept separate from captures by the `kind` column added in V005 —
  // captures and logcat use different DSLs, so the two scopes must
  // not bleed into each other (a captures query is almost always
  // invalid as a logcat query and vice versa).
  const savedStore = savedFiltersFor("logcat");
  const savedFilters = savedStore.filters;

  const loadSidebarWidth = (): number => {
    try {
      const n = Number(localStorage.getItem(SIDEBAR_STORAGE_KEY));
      if (Number.isFinite(n) && n >= SIDEBAR_MIN && n <= SIDEBAR_MAX) return n;
    } catch {
      /* storage unavailable */
    }
    return SIDEBAR_DEFAULT;
  };
  const [sidebarWidth, setSidebarWidth] = createSignal(loadSidebarWidth());
  createEffect(() => {
    try {
      localStorage.setItem(SIDEBAR_STORAGE_KEY, String(sidebarWidth()));
    } catch {
      /* storage unavailable */
    }
  });

  // Inline rename of a sidebar filter. `editingId` is the row being
  // edited; `editName` the in-progress text. Commit on Enter/blur, cancel
  // on Esc — a rename is just an upsert with the same id and a new name.
  const [editingId, setEditingId] = createSignal<string | null>(null);
  const [editName, setEditName] = createSignal("");
  const startRename = (f: { id: string; name: string }) => {
    setEditName(f.name);
    setEditingId(f.id);
  };
  const commitRename = (f: {
    id: string;
    name: string;
    query: string;
    color: string;
    pinned: boolean;
  }) => {
    if (editingId() !== f.id) return; // already committed or cancelled
    const name = editName().trim();
    setEditingId(null);
    if (!name || name === f.name) return;
    void savedStore
      .save({ id: f.id, name, query: f.query, color: f.color, pinned: f.pinned })
      .catch((e) => setErrorMsg((e as { message?: string })?.message ?? String(e)));
  };

  // Save-popover state. Mirrors CapturesView: name + colour + pin,
  // plus an "update vs save" decision based on a case-insensitive
  // exact name match against the existing list.
  const [saveOpen, setSaveOpen] = createSignal(false);
  const [saveName, setSaveName] = createSignal("");
  const [saveColor, setSaveColor] = createSignal(FILTER_PALETTE[0]!);
  const [savePinned, setSavePinned] = createSignal(false);
  const [saveBusy, setSaveBusy] = createSignal(false);
  let savePopoverRef: HTMLDivElement | undefined;

  const existingMatch = () => {
    const n = saveName().trim().toLowerCase();
    if (!n) return undefined;
    return savedFilters().find((f) => f.name.trim().toLowerCase() === n);
  };

  const openSave = () => {
    if (!filter().trim()) return;
    setSaveName("");
    setSaveColor(FILTER_PALETTE[0]!);
    setSavePinned(false);
    setSaveOpen(true);
    queueMicrotask(() => savePopoverRef?.querySelector("input")?.focus());
  };

  const doSave = async (e: Event) => {
    e.preventDefault();
    const name = saveName().trim();
    if (!name || saveBusy()) return;
    setSaveBusy(true);
    try {
      const match = existingMatch();
      await savedStore.save({
        id: match?.id,
        name,
        query: filter(),
        color: saveColor(),
        pinned: savePinned(),
      });
      setSaveOpen(false);
    } catch (err) {
      console.error("save logcat filter failed", err);
      alert(
        tr("logcat.save_failed", {
          message: (err as { message?: string })?.message ?? String(err),
        }),
      );
    } finally {
      setSaveBusy(false);
    }
  };

  // Outside-click closes both popovers. Initial fetch populates the
  // dropdown so the chevron shows up on first paint if the user
  // already has saved filters from a previous session.
  onMount(() => {
    const onDoc = (e: MouseEvent) => {
      const target = e.target as Node;
      if (saveOpen() && savePopoverRef && !savePopoverRef.contains(target)) {
        setSaveOpen(false);
      }
      if (
        headerMenuPos() &&
        headerMenuRef &&
        !headerMenuRef.contains(target)
      ) {
        setHeaderMenuPos(null);
      }
      if (rowMenu() && rowMenuRef && !rowMenuRef.contains(target)) {
        setRowMenu(null);
      }
    };
    document.addEventListener("mousedown", onDoc);
    onCleanup(() => document.removeEventListener("mousedown", onDoc));
    void savedStore.refresh();
  });

  // Keep the highlight overlay scrolled in step with the input —
  // when the typed text overflows, the input scrolls horizontally
  // and we mirror that scroll on the overlay so the colours stay
  // glued to the right characters.
  const syncFilterScroll = () => {
    if (filterInputRef && filterOverlayRef) {
      filterOverlayRef.scrollLeft = filterInputRef.scrollLeft;
    }
  };

  // Compute the highlighted HTML for the current filter text. Memo
  // so the assignment to `innerHTML` only fires when the text
  // actually changes. The HTML is a sequence of `<span class="...">`
  // chunks — token colours match CapturesView's filter pattern.
  const highlightedFilterHtml = createMemo(() => buildLogcatFilterHtml(filter()));

  // Stable virtualizer instance — `count` is a reactive getter so the
  // internal store recomputes virtual items as entries flow in, but
  // the Virtualizer object itself never gets reconstructed. Wrapping
  // this in `createMemo(() => createVirtualizer(...))` reconstructed
  // it on every batch (10×/sec during a firehose), which (a) wiped
  // scroll state, (b) saturated the main thread enough that toolbar
  // events stopped firing. `mergeProps` inside the lib makes the
  // option getters reactive without rebuilding the instance.
  // Row height tracks the root font-size so rows don't overlap when
  // the user bumps the text-size setting. The 22/16 ratio gives the
  // current 22px row at the default 16px root and scales linearly
  // from there (text-xs line-height is `1rem`, plus the `py-px`
  // padding — fits within this estimate at every scale step).
  const ROW_PX_PER_ROOT = 22 / 16;
  const virtualizer = createVirtualizer<HTMLDivElement, HTMLDivElement>({
    get count() {
      return visible().length;
    },
    getScrollElement: () => scrollEl ?? null,
    estimateSize: () => Math.round(ROOT_PX[fontScale()] * ROW_PX_PER_ROOT),
    overscan: 30,
  });

  // Force the virtualizer to remeasure when the user changes the font
  // scale. estimateSize() now depends on fontScale(), but the
  // virtualizer doesn't track that automatically — call .measure() so
  // already-positioned virtual items recompute against the new size.
  createEffect(() => {
    void fontScale();
    virtualizer.measure();
  });

  // Render a string with every occurrence of the active search term
  // wrapped in <mark>. Case-insensitive; returns the raw string when
  // search is empty so we don't pay the split/join cost on the firehose
  // hot path. The hot range slice keeps the original casing — only the
  // match boundary uses the lowered haystack.
  const renderHL = (text: string): JSX.Element => {
    const term = searchLower();
    if (!term || !text) return text;
    const lower = text.toLowerCase();
    if (!lower.includes(term)) return text;
    const out: JSX.Element[] = [];
    let i = 0;
    while (i < text.length) {
      const idx = lower.indexOf(term, i);
      if (idx < 0) {
        out.push(text.slice(i));
        break;
      }
      if (idx > i) out.push(text.slice(i, idx));
      out.push(
        <mark class="bg-warn/30 text-fg rounded-sm px-0.5">
          {text.slice(idx, idx + term.length)}
        </mark>,
      );
      i = idx + term.length;
    }
    return out;
  };


  return (
    <div class="flex h-screen bg-bg text-fg text-xs">
      {/* Saved-filters sidebar — mirrors the main window's FILTERS list.
          Click applies the query, hover reveals delete. Hidden while
          empty so the table keeps full width until the user stars a
          filter; replaces the old toolbar chevron dropdown. */}
      <Show when={savedFilters().length > 0}>
        <aside
          class="shrink-0 flex flex-col overflow-auto bg-bg-subtle/40 p-2"
          style={{ width: `${sidebarWidth()}px` }}
        >
          <div class="px-2 pt-1 pb-2 text-xs uppercase tracking-wide text-fg-muted">
            {t()("nav.filters")}
          </div>
          <For each={savedFilters()}>
            {(f) => (
              <div class="group px-2 py-1 rounded text-sm hover:bg-bg-muted flex items-center gap-2">
                <FilterIcon size={14} style={{ color: f.color }} class="shrink-0" />
                <Show
                  when={editingId() === f.id}
                  fallback={
                    <span
                      class="truncate flex-1 cursor-pointer"
                      title={tr("logcat.apply_filter", { query: f.query })}
                      onClick={() => setFilter(f.query)}
                    >
                      {f.name}
                    </span>
                  }
                >
                  <input
                    type="text"
                    autocapitalize="off"
                    autocomplete="off"
                    autocorrect="off"
                    spellcheck={false}
                    class="flex-1 min-w-0 bg-bg border border-border rounded px-1 py-0.5 text-sm"
                    value={editName()}
                    ref={(el) =>
                      queueMicrotask(() => {
                        el.focus();
                        el.select();
                      })
                    }
                    onInput={(e) => setEditName(e.currentTarget.value)}
                    onClick={(e) => e.stopPropagation()}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        commitRename(f);
                      } else if (e.key === "Escape") {
                        e.preventDefault();
                        setEditingId(null);
                      }
                    }}
                    onBlur={() => commitRename(f)}
                  />
                </Show>
                <Show when={editingId() !== f.id}>
                  <button
                    type="button"
                    class="opacity-0 group-hover:opacity-100 hover:text-accent shrink-0"
                    title={t()("logcat.rename_filter")}
                    onClick={(e) => {
                      e.stopPropagation();
                      startRename(f);
                    }}
                  >
                    <Pencil size={12} />
                  </button>
                  <button
                    type="button"
                    class="opacity-0 group-hover:opacity-100 hover:text-danger shrink-0"
                    title={t()("logcat.delete_filter")}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (
                        confirm(tr("logcat.delete_filter_confirm", { name: f.name }))
                      ) {
                        void savedStore.remove(f.id);
                      }
                    }}
                  >
                    <X size={12} />
                  </button>
                </Show>
              </div>
            )}
          </For>
        </aside>
        <VerticalResizer
          onResize={(dx) =>
            setSidebarWidth((w) =>
              Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, w + dx)),
            )
          }
          onReset={() => setSidebarWidth(SIDEBAR_DEFAULT)}
        />
      </Show>
      <div class="flex flex-col flex-1 min-w-0 min-h-0">
      {/* Toolbar */}
      <div class="flex items-center gap-2 px-3 py-2 border-b border-border bg-bg-subtle">
        <button
          class="inline-flex items-center gap-1 px-2 py-1 rounded hover:bg-bg-muted"
          onClick={togglePause}
          title={tr("logcat.pause_hotkey")}
        >
          {paused() ? <Play size={12} /> : <Pause size={12} />}
          {paused() ? t()("logcat.resume") : t()("logcat.pause")}
        </button>
        <button
          class="inline-flex items-center gap-1 px-2 py-1 rounded hover:bg-bg-muted text-danger"
          onClick={clearAll}
          title={tr("logcat.clear_hotkey")}
        >
          <Trash2 size={12} />
          {t()("logcat.clear")}
        </button>
        <button
          class="inline-flex items-center gap-1 px-2 py-1 rounded hover:bg-bg-muted"
          onClick={exportLog}
          title={t()("logcat.export_title")}
          disabled={visible().length === 0}
        >
          <Download size={12} />
          {t()("logcat.export")}
        </button>
        <button
          class={`inline-flex items-center gap-1 px-2 py-1 rounded ${
            autoScroll() ? "bg-accent/15 text-accent" : "hover:bg-bg-muted text-fg-muted"
          }`}
          onClick={toggleAutoScroll}
          title={t()("logcat.auto_scroll_title")}
        >
          <ArrowDown size={12} />
          {t()("logcat.auto_scroll")}
        </button>
        <button
          class={`inline-flex items-center gap-1 px-2 py-1 rounded ${
            textMode() ? "bg-accent/15 text-accent" : "hover:bg-bg-muted text-fg-muted"
          }`}
          onClick={toggleTextMode}
          title={t()("logcat.text_view_title")}
        >
          <Type size={12} />
          {t()("logcat.text_view")}
        </button>

        {/* Token-highlight overlay over a transparent input. The
            previous Solid-<For>-based overlay rendered only the
            first character of typed text — unclear why, since the
            same markup works in CapturesView. This version
            sidesteps Solid reactivity entirely: a memoized HTML
            string is rendered via `innerHTML`, so the DOM update
            is a single deterministic assignment. */}
        <div class="flex-1 relative flex items-center bg-bg-muted rounded focus-within:ring-1 focus-within:ring-accent">
          {/* Inset funnel icon — sits on the left edge of the field
              the same way the magnifier sits on the search field, so
              the two paired inputs look symmetric. `left-2` matches
              the input's `pl-7` left padding (7 = icon width 14px +
              gutter ~14px) so the typed text never overlaps the icon. */}
          <FilterIcon
            size={12}
            class="absolute left-2 inset-y-0 my-auto h-3 text-fg-muted pointer-events-none z-10"
          />
          <div
            ref={(el) => (filterOverlayRef = el)}
            aria-hidden="true"
            class="absolute inset-0 pointer-events-none text-xs font-mono overflow-hidden pl-7 pr-14 py-1 flex items-center"
          >
            {/* Wrap the highlight HTML in a single inline span so
                flexbox sees one item — without the wrapper, each
                top-level <span> becomes its own flex item and the
                anonymous text-node spaces between tokens (`app:foo `,
                ` tag:bar`) get collapsed by the layout, making
                multi-token filters look glued together visually
                even though `filter()` still has the spaces in it. */}
            <span
              class="whitespace-pre flex-shrink-0"
              innerHTML={highlightedFilterHtml()}
            />
          </div>
          <input
            ref={(el) => (filterInputRef = el)}
            type="text"
            class="relative w-full bg-transparent rounded pl-7 pr-14 py-1 outline-none text-xs font-mono text-transparent caret-fg placeholder:text-fg-muted"
            placeholder={tr("logcat.filter_placeholder", {
              hotkey: FILTER_HOTKEY_LABEL,
            })}
            value={filter()}
            onInput={(e) => {
              setFilter(e.currentTarget.value);
              syncFilterScroll();
            }}
            onScroll={syncFilterScroll}
            onKeyDown={(e) => {
              if (e.key === "Escape" && filter()) {
                e.preventDefault();
                setFilter("");
              }
            }}
            title={t()("logcat.filter_help")}
            autocapitalize="off"
            autocomplete="off"
            autocorrect="off"
            spellcheck={false}
          />
          {/* Right-aligned action cluster: star (save current filter —
              only when the filter is non-empty). The saved list itself
              now lives in the left sidebar, not a dropdown. */}
          <div class="absolute right-1 inset-y-0 flex items-center gap-0.5 z-10">
            <Show when={filter().trim()}>
              <button
                type="button"
                class="p-1 rounded text-fg-muted hover:text-warn hover:bg-bg-subtle"
                title={t()("logcat.save_filter_title")}
                aria-label={t()("logcat.save_filter")}
                onMouseDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  openSave();
                }}
              >
                <Star size={14} />
              </button>
            </Show>
          </div>

          {/* Save / Update popover. Anchored to wrapper's right edge,
              opens downward. Mirrors CapturesView popover so users
              who already know the captures flow recognise it. */}
          <Show when={saveOpen()}>
            <div
              ref={(el) => (savePopoverRef = el)}
              class="absolute right-0 top-full mt-1 w-72 z-30 bg-bg-subtle border border-border rounded shadow-lg p-3 text-xs"
              onMouseDown={(e) => e.stopPropagation()}
            >
              <form onSubmit={doSave} class="space-y-2">
                <div class="font-semibold text-fg-subtle uppercase tracking-wide">
                  {existingMatch()
                    ? t()("logcat.update_filter")
                    : t()("logcat.save_filter")}
                </div>
                <div class="font-mono text-fg-muted bg-bg-muted rounded px-2 py-1 truncate">
                  {filter()}
                </div>
                <input
                  type="text"
                  class="w-full px-2 py-1.5 rounded bg-bg-muted outline-none focus:ring-1 focus:ring-accent"
                  placeholder={t()("logcat.save_filter_name_placeholder")}
                  value={saveName()}
                  onInput={(e) => setSaveName(e.currentTarget.value)}
                  maxlength={64}
                />
                <Show when={existingMatch()}>
                  <div class="text-fg-muted text-[11px]">
                    {tr("logcat.update_filter_hint", {
                      name: existingMatch()!.name,
                    })}
                  </div>
                </Show>
                <div class="flex items-center justify-between">
                  <div class="flex items-center gap-1">
                    <For each={FILTER_PALETTE}>
                      {(c) => (
                        <button
                          type="button"
                          class={`w-5 h-5 rounded-full border ${
                            saveColor() === c ? "border-fg" : "border-transparent"
                          }`}
                          style={{ "background-color": c }}
                          onClick={() => setSaveColor(c)}
                          aria-label={tr("logcat.color_label", { color: c })}
                        />
                      )}
                    </For>
                  </div>
                  <label class="inline-flex items-center gap-1 cursor-pointer select-none">
                    <input
                      type="checkbox"
                      class="accent-accent"
                      checked={savePinned()}
                      onChange={(e) => setSavePinned(e.currentTarget.checked)}
                    />
                    <Pin size={11} /> {t()("logcat.pin")}
                  </label>
                </div>
                <div class="flex justify-end gap-2 pt-1">
                  <button
                    type="button"
                    class="px-2 py-1 rounded hover:bg-bg-muted text-fg-muted"
                    onClick={() => setSaveOpen(false)}
                  >
                    {t()("logcat.cancel")}
                  </button>
                  <button
                    type="submit"
                    class="px-3 py-1 rounded bg-accent text-white hover:opacity-90 disabled:opacity-50"
                    disabled={!saveName().trim() || saveBusy()}
                  >
                    {saveBusy()
                      ? existingMatch()
                        ? t()("logcat.updating")
                        : t()("logcat.saving")
                      : existingMatch()
                        ? t()("logcat.update")
                        : t()("logcat.save")}
                  </button>
                </div>
              </form>
            </div>
          </Show>

        </div>
        {/* Substring search — sits next to the DSL filter so the two are
            visually paired. Narrower than the filter (max-w-xs) since
            it's a simple term, not a query. Clears on Esc. */}
        <div class="relative flex items-center bg-bg-muted rounded focus-within:ring-1 focus-within:ring-accent w-48">
          <SearchIcon size={12} class="text-fg-muted shrink-0 ml-2" />
          <input
            ref={(el) => (searchInputRef = el)}
            type="text"
            class="w-full bg-transparent rounded px-2 py-1 pr-7 outline-none text-xs font-mono"
            placeholder={tr("logcat.search_placeholder", {
              hotkey: SEARCH_HOTKEY_LABEL,
            })}
            value={search()}
            onInput={(e) => setSearch(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && search()) {
                e.preventDefault();
                setSearch("");
              }
            }}
            title={t()("logcat.search_title")}
            autocapitalize="off"
            autocomplete="off"
            autocorrect="off"
            spellcheck={false}
          />
          <Show when={search()}>
            <button
              type="button"
              class="absolute right-1 inset-y-0 my-auto h-5 w-5 inline-flex items-center justify-center rounded text-fg-muted hover:text-fg hover:bg-bg-subtle"
              onClick={() => setSearch("")}
              title={t()("logcat.search_clear")}
              aria-label={t()("logcat.search_clear")}
            >
              <X size={11} />
            </button>
          </Show>
        </div>
      </div>

      {/* Error banner — soft, non-blocking. */}
      {errorMsg() && (
        <div class="px-3 py-1 bg-danger/10 text-danger border-b border-danger/30">
          {errorMsg()}
        </div>
      )}

      {/* Column header row. Lives outside the scroll container so it
          stays put when the user scrolls the firehose. The grid
          template comes from gridTemplate() and is shared with the
          row template below. Time/PID/Tag have drag handles on
          their right edge; Level is fixed (1 char) and Message
          takes the remainder (1fr). Vertical hairlines via
          per-cell border-r — no grid gap so the borders are
          column-edge aligned. Hidden in plain-text mode. */}
      <Show when={!textMode()}>
      <div
        class="grid font-mono text-fg-muted tracking-wide text-xs px-3 py-1 border-b border-border bg-bg-subtle/60"
        style={{ "grid-template-columns": gridTemplate() }}
        onContextMenu={openHeaderMenu}
        title={t()("logcat.col_menu_hint")}
      >
        <Show when={colVisible().time}>
          <HeaderCell
            label={t()("logcat.col_time")}
            onResize={(e) => startColResize("time", e)}
            onReset={() =>
              setColWidths({ ...colWidths(), time: COL_DEFAULTS.time })
            }
          />
        </Show>
        <Show when={colVisible().pid}>
          <HeaderCell
            label={t()("logcat.col_pid")}
            onResize={(e) => startColResize("pid", e)}
            onReset={() =>
              setColWidths({ ...colWidths(), pid: COL_DEFAULTS.pid })
            }
          />
        </Show>
        <Show when={colVisible().app}>
          <HeaderCell
            label={t()("logcat.col_app")}
            onResize={(e) => startColResize("app", e)}
            onReset={() =>
              setColWidths({ ...colWidths(), app: COL_DEFAULTS.app })
            }
          />
        </Show>
        <Show when={colVisible().level}>
          <span class="px-1 border-r border-border/40">
            {t()("logcat.col_level")}
          </span>
        </Show>
        <Show when={colVisible().tag}>
          <HeaderCell
            label={t()("logcat.col_tag")}
            onResize={(e) => startColResize("tag", e)}
            onReset={() =>
              setColWidths({ ...colWidths(), tag: COL_DEFAULTS.tag })
            }
          />
        </Show>
        <Show when={colVisible().message}>
          <span class="px-2">{t()("logcat.col_message")}</span>
        </Show>
      </div>
      </Show>

      {/* Plain-text view. A read-only textarea (wrap off → horizontal
          scroll) of the snapshotted filtered lines: native selection,
          scroll-to-end, and copy that the virtualized grid can't give.
          Swaps in for the header + table while active. */}
      <Show when={textMode()}>
        <textarea
          class="flex-1 w-full bg-bg text-fg text-xs font-mono p-3 outline-none resize-none select-text"
          readOnly
          wrap="off"
          spellcheck={false}
          value={textSnapshot()}
        />
      </Show>

      {/* Column show/hide menu. Anchored to the right-click position
          via `fixed` + inline `left/top`. We don't bother flipping
          if it would overflow the viewport bottom; the menu is small
          (~150px tall) and the header sits at the top of the window. */}
      <Show when={headerMenuPos()}>
        <div
          ref={(el) => (headerMenuRef = el)}
          class="fixed z-50 bg-bg-subtle border border-border rounded shadow-lg py-1 text-xs select-none"
          style={{
            left: `${headerMenuPos()!.x}px`,
            top: `${headerMenuPos()!.y}px`,
            "min-width": "180px",
          }}
          onMouseDown={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          <div class="px-3 py-1 text-fg-muted uppercase tracking-wide text-[10px]">
            {t()("logcat.col_menu_title")}
          </div>
          <For each={ALL_COLS}>
            {(key) => (
              <label class="flex items-center gap-2 px-3 py-1 cursor-pointer hover:bg-bg-muted">
                <input
                  type="checkbox"
                  class="accent-accent"
                  checked={colVisible()[key]}
                  onChange={(e) =>
                    setColVisible({
                      ...colVisible(),
                      [key]: e.currentTarget.checked,
                    })
                  }
                />
                <span>{t()(`logcat.col_${key}`)}</span>
              </label>
            )}
          </For>
        </div>
      </Show>

      {/* Virtualized table. <For> over the reactive virtual-items
          accessor keeps row identity stable across the firehose;
          .map would rebuild DOM nodes each batch. Hidden in text mode. */}
      <Show when={!textMode()}>
      <div
        ref={(el) => (scrollEl = el)}
        class="flex-1 overflow-auto"
        onScroll={onScroll}
      >
        <Show
          when={visible().length > 0}
          fallback={
            <div class="flex items-center justify-center h-full text-fg-muted italic">
              {entries().length === 0
                ? t()("logcat.empty_waiting")
                : t()("logcat.empty_filtered")}
            </div>
          }
        >
          <div
            style={{
              height: `${virtualizer.getTotalSize()}px`,
              position: "relative",
              width: "100%",
            }}
          >
            {/* <For> over virtualizer.getVirtualItems(). Earlier
                tried <Index> for less DOM churn during firehose,
                but it broke rendering — virtualizer + Index combo
                ended up with an empty body even when getTotalSize
                / count was non-zero. <For> works reliably; the
                flicker it causes on heavy update is the lesser
                evil and we'll revisit if it becomes a problem. */}
            <For each={virtualizer.getVirtualItems()}>
              {(vi) => {
                // Wrap the row entry in a memo so it tracks visible()
                // changes — at MAX_ENTRIES the virtualizer keeps
                // returning the same vi objects (count is pinned),
                // so the <For> callback never re-runs. Without this
                // memo, e was captured once and the row content
                // froze even though entries() kept turning over.
                const e = createMemo(() => visible()[vi.index]);
                return (
                  <Show when={e()}>
                    {(entry) => {
                      // Show the most-recently-seen name for this PID.
                      // Set preserves insertion order, so the last
                      // value is the latest name learned from the
                      // 10s poll.
                      const appName = () => {
                        const set = pidNames().get(entry().pid);
                        if (!set || set.size === 0) return "";
                        let last = "";
                        for (const n of set) last = n;
                        return last;
                      };
                      const pidStr = () =>
                        entry().pid > 0 ? String(entry().pid) : "";
                      return (
                        <div
                          class={`absolute left-0 right-0 grid font-mono whitespace-nowrap items-baseline px-3 py-px border-b border-border/30 select-none cursor-default ${
                            selected().has(entry())
                              ? "bg-accent/25"
                              : "hover:bg-bg-muted/40"
                          } ${LEVEL_ROW_COLOR[entry().level]}`}
                          style={{
                            transform: `translateY(${vi.start}px)`,
                            "grid-template-columns": gridTemplate(),
                          }}
                          onMouseDown={(ev) => onRowMouseDown(ev, vi.index)}
                          onMouseEnter={() => onRowMouseEnter(vi.index)}
                          onContextMenu={(ev) => onRowContextMenu(ev, vi.index)}
                          onDblClick={() => openDetail([entry()])}
                          title={t()("logcat.row_open_detail")}
                        >
                          <Show when={colVisible().time}>
                            <span class="truncate px-2 border-r border-border/30">
                              {renderHL(entry().timestamp)}
                            </span>
                          </Show>
                          <Show when={colVisible().pid}>
                            <span class="truncate px-2 border-r border-border/30">
                              {renderHL(pidStr())}
                            </span>
                          </Show>
                          <Show when={colVisible().app}>
                            <span
                              class="truncate px-2 border-r border-border/30"
                              title={appName()}
                            >
                              {renderHL(appName())}
                            </span>
                          </Show>
                          <Show when={colVisible().level}>
                            <span
                              class={`px-1 border-r border-border/30 ${LEVEL_COLOR[entry().level]}`}
                            >
                              {LEVEL_CHAR[entry().level]}
                            </span>
                          </Show>
                          <Show when={colVisible().tag}>
                            <span class="truncate px-2 border-r border-border/30">
                              {renderHL(entry().tag)}
                            </span>
                          </Show>
                          <Show when={colVisible().message}>
                            <span class="truncate px-2">
                              {renderHL(entry().message)}
                            </span>
                          </Show>
                        </div>
                      );
                    }}
                  </Show>
                );
              }}
            </For>
          </div>
        </Show>
      </div>
      </Show>

      {/* Status bar. LogRabbit-style row at the foot of the window
          carrying the counter on the left — keeps the toolbar uncluttered
          and the digits in one fixed spot. `pl-5` (= row's `px-3` + the
          first cell's `px-2`) aligns the counter's left edge with the
          start of the first column above it. The `+` suffix appears
          when the in-memory ring buffer has hit MAX_ENTRIES, signalling
          that older entries have been dropped FIFO and the visible
          total is the cap, not the actual log volume since attach. */}
      <div class="flex items-center pl-5 pr-3 py-1 border-t border-border bg-bg-subtle text-fg-muted text-xs tabular-nums">
        <span>
          {tr("logcat.counter", {
            shown: String(visible().length),
            total:
              entries().length >= MAX_ENTRIES
                ? `${MAX_ENTRIES}+`
                : String(entries().length),
          })}
        </span>
        <Show when={pendingCount() > 0}>
          <span class="ml-3 text-accent">
            {tr("logcat.pending_counter", { n: String(pendingCount()) })}
          </span>
        </Show>
        <Show when={selected().size > 0}>
          <span class="ml-3">
            {tr("logcat.selected_count", { n: String(selected().size) })}
          </span>
        </Show>
        <Show when={copyHint()}>
          <span class="ml-3 text-success">{copyHint()}</span>
        </Show>
      </div>

      {/* Row right-click menu. Copy / Copy message act on the whole
          selection; View message opens the row the menu was invoked on. */}
      <Show when={rowMenu()}>
        {(pos) => (
          <div
            ref={(el) => (rowMenuRef = el)}
            class="fixed z-50 bg-bg-subtle border border-border rounded shadow-lg py-1 text-xs select-none min-w-[180px]"
            style={{ left: `${pos().x}px`, top: `${pos().y}px` }}
          >
            <button
              type="button"
              class="w-full text-left px-3 py-1.5 hover:bg-bg-muted flex items-center justify-between gap-6"
              onClick={() => menuCopy(false)}
            >
              <span>{t()("logcat.menu_copy")}</span>
              <span class="text-fg-muted">{IS_MAC_PLATFORM ? "⌘C" : "Ctrl+C"}</span>
            </button>
            <button
              type="button"
              class="w-full text-left px-3 py-1.5 hover:bg-bg-muted flex items-center justify-between gap-6"
              onClick={() => menuCopy(true)}
            >
              <span>{t()("logcat.menu_copy_message")}</span>
              <span class="text-fg-muted">{IS_MAC_PLATFORM ? "⇧⌘C" : "Ctrl+Shift+C"}</span>
            </button>
            <button
              type="button"
              class="w-full text-left px-3 py-1.5 hover:bg-bg-muted"
              onClick={menuView}
            >
              {t()("logcat.menu_view_message")}
            </button>
          </div>
        )}
      </Show>

      {/* Row detail overlay. Resizable (drag the bottom-right corner) and
          scrollable. The content is a syntax-highlighted JSON editor with
          a Format button — a single row's message, or the selected rows
          as threadtime lines. Single row also lists its fields above the
          editor so each can be copied on its own. Backdrop or Esc closes. */}
      <Show when={detail()}>
        {(d) => {
          const rows = () => d();
          const single = () => (rows().length === 1 ? rows()[0]! : null);
          return (
            <div
              class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6"
              onClick={() => setDetail(null)}
            >
              <div
                class="flex flex-col bg-bg border border-border rounded-lg shadow-xl overflow-hidden"
                style={{
                  width: "760px",
                  height: "560px",
                  "min-width": "360px",
                  "min-height": "240px",
                  "max-width": "95vw",
                  "max-height": "92vh",
                  resize: "both",
                }}
                onClick={(e) => e.stopPropagation()}
              >
                <div class="flex items-center gap-2 px-4 py-2 border-b border-border text-xs font-mono text-fg-muted shrink-0">
                  <Show
                    when={single()}
                    fallback={
                      <span>{tr("logcat.detail_rows", { n: String(rows().length) })}</span>
                    }
                  >
                    {(e) => (
                      <span class="flex-1 min-w-0 select-text break-words text-fg">
                        <span class={`font-bold ${LEVEL_COLOR[e().level]}`}>
                          {LEVEL_CHAR[e().level]}
                        </span>{" "}
                        {e().timestamp}
                        <Show when={appNameForPid(e().pid)}>
                          {(app) => <> {app()}</>}
                        </Show>
                        <Show when={e().tag}>{` ${e().tag}`}</Show>
                      </span>
                    )}
                  </Show>
                  <Show when={detailFormatErr()}>
                    {(msg) => (
                      <span class="text-fg-muted truncate max-w-[220px]" title={msg()}>
                        {msg()}
                      </span>
                    )}
                  </Show>
                  <div class="ml-auto flex items-center gap-1">
                    <button
                      class="inline-flex items-center gap-1 px-2 py-1 rounded hover:bg-bg-muted"
                      onClick={formatDetail}
                      title={t()("logcat.detail_format_title")}
                    >
                      <Braces size={12} />
                      {t()("logcat.detail_format")}
                    </button>
                    <button
                      class="inline-flex items-center gap-1 px-2 py-1 rounded hover:bg-bg-muted"
                      onClick={() => void writeClipboard(detailText())}
                      title={t()("logcat.detail_copy")}
                    >
                      <Copy size={12} />
                      {t()("logcat.detail_copy")}
                    </button>
                    <button
                      class="p-1 rounded hover:bg-bg-muted"
                      onClick={() => setDetail(null)}
                      title={t()("logcat.detail_close")}
                      aria-label={t()("logcat.detail_close")}
                    >
                      <X size={14} />
                    </button>
                  </div>
                </div>
                <div class="flex-1 min-h-0 p-2">
                  <Show
                    when={detailLooksJson()}
                    fallback={
                      <textarea
                        class="w-full h-full bg-bg border border-border rounded px-2 py-1 text-xs font-mono leading-snug whitespace-pre-wrap break-words resize-none overflow-auto text-fg outline-none focus:border-accent"
                        spellcheck={false}
                        value={detailText()}
                        onInput={(e) => setDetailText(e.currentTarget.value)}
                      />
                    }
                  >
                    <JsonEditor value={detailText()} onInput={setDetailText} />
                  </Show>
                </div>
              </div>
            </div>
          );
        }}
      </Show>
      </div>
    </div>
  );
};

// ---- Filter syntax highlighting --------------------------------------------
//
// Builds an HTML string for the overlay <div innerHTML={...}> behind
// the transparent filter input. Solid <For> over a per-input parts
// array failed to update reliably here (worked in CapturesView, but
// inside the logcat toolbar's flex layout it stuck on the first
// char). innerHTML is a single deterministic DOM update — no
// reactive-list quirks possible.

const LOGCAT_VALID_KEYS = new Set(["tag", "msg", "message", "level", "pid", "app"]);

function escapeHtml(s: string): string {
  return s
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function buildLogcatFilterHtml(text: string): string {
  if (!text) return "";
  const out: string[] = [];
  const tokens = text.match(/\s+|\S+/g) ?? [];
  for (const tok of tokens) {
    if (/^\s+$/.test(tok)) {
      out.push(escapeHtml(tok));
      continue;
    }
    let body = tok;
    if (body.startsWith("!")) {
      out.push('<span class="text-danger">!</span>');
      body = body.slice(1);
    }
    if (body.startsWith("~")) {
      out.push('<span class="text-warn">~</span>');
      out.push(`<span class="text-fg">${escapeHtml(body.slice(1))}</span>`);
      continue;
    }
    const m = body.match(/^([a-zA-Z_]+)(:)(.*)$/);
    if (m) {
      const [, key, colon, valueRaw] = m;
      const known = LOGCAT_VALID_KEYS.has(key!.toLowerCase());
      const cls = known
        ? "text-accent"
        : "text-danger underline decoration-dotted";
      out.push(`<span class="${cls}">${escapeHtml(key!)}</span>`);
      out.push(`<span class="text-fg-muted">${escapeHtml(colon!)}</span>`);
      let value = valueRaw ?? "";
      // `key:!value` form — `!` after the colon is the negation
      // marker, same as a leading `!` on the whole token. Paint it
      // in danger-red so the user sees that it's structural, not
      // part of the value.
      if (value.startsWith("!")) {
        out.push('<span class="text-danger">!</span>');
        value = value.slice(1);
      }
      if (value) out.push(`<span class="text-fg">${escapeHtml(value)}</span>`);
    } else {
      out.push(`<span class="text-fg">${escapeHtml(body)}</span>`);
    }
  }
  return out.join("");
}

export default LogcatView;
