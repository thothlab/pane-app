import {
  type Component,
  createEffect,
  createMemo,
  createSignal,
  For,
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
  ChevronDown,
  Download,
  Filter as FilterIcon,
  Pause,
  Pin,
  Play,
  Search as SearchIcon,
  Star,
  Trash2,
  X,
} from "lucide-solid";
import { t, tr } from "@/i18n";
import { compileLogcatFilter } from "@/lib/logcat-filter";
import { savedFiltersFor } from "@/stores/saved-filters";
import { fontScale, ROOT_PX } from "@/stores/font-scale";

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

const LEVEL_CHAR: Record<LogLevel, string> = {
  verbose: "V",
  debug: "D",
  info: "I",
  warn: "W",
  error: "E",
  fatal: "F",
  silent: "S",
};

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
  const [errorMsg, setErrorMsg] = createSignal<string | null>(null);
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
  const [pidNames, setPidNames] = createSignal<Map<number, string>>(new Map());

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
    for (const [pid, name] of pidNames()) {
      const lower = name.toLowerCase();
      if (pos.some((n) => lower.includes(n))) include.add(pid);
      if (neg.some((n) => lower.includes(n))) exclude.add(pid);
    }
    return { include, exclude, hasPositive: pos.length > 0 };
  });

  const visible = createMemo(() => {
    const { predicate, appPackages } = matcher();
    const { include, exclude, hasPositive } = appPids();
    const all = entries();
    if (appPackages.length === 0) return all.filter(predicate);
    // Positive app:X is in the filter but the package isn't currently
    // running → include is empty → nothing matches, surfacing the
    // "app not running" state via an empty list.
    if (hasPositive && include.size === 0) return [];
    return all.filter((e) => {
      if (hasPositive && !include.has(e.pid)) return false;
      if (exclude.has(e.pid)) return false;
      return predicate(e);
    });
  });

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
            next.set(Number(k), v);
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

  // Subscribe to the per-window batched stream. Backend emits
  // `logcat://batch` with payload Vec<LogEntry> every 100ms / 50
  // entries (whichever first) on this WebviewWindow only — so the
  // main window never sees the firehose.
  //
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
  onMount(() => {
    let unlistenBatch: UnlistenFn | undefined;
    let unlistenError: UnlistenFn | undefined;
    let pending: LogEntry[][] = [];
    let flushScheduled = false;
    let rafHandle: number | undefined;

    const flush = () => {
      flushScheduled = false;
      rafHandle = undefined;
      if (pending.length === 0) return;
      const merged: LogEntry[] =
        pending.length === 1 ? pending[0]! : pending.flat();
      pending = [];

      // FIFO-shift compensation. Once the buffer hits MAX_ENTRIES,
      // every new batch pushes the same count off the front — the
      // virtualizer's count stays at MAX, scrollTop stays put, but
      // the rows at every fixed pixel offset have rotated forward.
      // Result for an autoScroll-off user: the content visibly
      // "scrolls down" under their viewport even though they didn't
      // ask for it. Anchor to the entry currently at the top of the
      // viewport, then after the buffer turns over re-locate that
      // exact entry and restore the same screen position.
      let anchor:
        | { entry: LogEntry; pxOffset: number }
        | undefined;
      if (!autoScroll() && scrollEl) {
        const rowH = Math.max(
          1,
          Math.round(ROOT_PX[fontScale()] * ROW_PX_PER_ROOT),
        );
        const topIdx = Math.floor(scrollEl.scrollTop / rowH);
        const e = visible()[topIdx];
        if (e) anchor = { entry: e, pxOffset: scrollEl.scrollTop - topIdx * rowH };
      }

      setEntries((prev) => {
        const next = prev.length === 0 ? merged : prev.concat(merged);
        return next.length > MAX_ENTRIES
          ? next.slice(next.length - MAX_ENTRIES)
          : next;
      });

      if (anchor && scrollEl) {
        queueMicrotask(() => {
          if (!scrollEl || !anchor) return;
          // visible() is reactive — after setEntries above it points
          // at the updated array. indexOf is reference-equality, so
          // we find the exact same LogEntry object regardless of
          // how the buffer turned over.
          const newVisible = visible();
          const idx = newVisible.indexOf(anchor.entry);
          if (idx < 0) return; // entry filtered out — nothing to anchor to
          const rowH = Math.max(
            1,
            Math.round(ROOT_PX[fontScale()] * ROW_PX_PER_ROOT),
          );
          scrollEl.scrollTop = idx * rowH + anchor.pxOffset;
          lastScrollTop = scrollEl.scrollTop;
        });
      }
    };

    const scheduleFlush = () => {
      if (flushScheduled) return;
      flushScheduled = true;
      rafHandle = requestAnimationFrame(flush);
    };

    listen<LogEntry[]>("logcat://batch", (e) => {
      if (paused()) return;
      pending.push(e.payload);
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
  createEffect(() => {
    void visible();
    if (!autoScroll() || !scrollEl) return;
    queueMicrotask(() => {
      if (!scrollEl) return;
      scrollEl.scrollTop = scrollEl.scrollHeight;
      lastScrollTop = scrollEl.scrollTop;
    });
  });

  // Detect user scroll-away from the bottom and switch auto-scroll
  // off. Re-engaging is via the toolbar toggle.
  //
  // We compare against lastScrollTop instead of computing "is at
  // bottom?" — at firehose rates the programmatic auto-scroll fires
  // many times per second, and any "is at bottom" check loses the
  // race vs. the user's scroll-up event. A 4px slack absorbs
  // sub-pixel wheel jitter; anything bigger than that going
  // backwards is the user.
  const onScroll = () => {
    if (!scrollEl) return;
    const cur = scrollEl.scrollTop;
    if (cur < lastScrollTop - 4 && autoScroll()) {
      setAutoScroll(false);
    }
    lastScrollTop = cur;
  };

  const togglePause = () => setPaused(!paused());
  const clearAll = () => setEntries([]);

  /// Serialize the currently-visible entries (after filter + follow-app
  /// constraints) into a plain-text `.log` file. Format mirrors what
  /// `adb logcat -v threadtime` emits so the result drops into any
  /// log viewer (Android Studio, logbook, grep) unmodified.
  const exportLog = async () => {
    const lines = visible().map((e) => {
      // "MM-DD HH:MM:SS.mmm  PID  TID L Tag: Message"
      const ts = e.timestamp || "";
      const pid = String(e.pid).padStart(5);
      const tid = String(e.tid).padStart(5);
      const lvl = LEVEL_CHAR[e.level];
      return `${ts} ${pid} ${tid} ${lvl} ${e.tag}: ${e.message}`;
    });
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
    if (next && scrollEl) {
      scrollEl.scrollTop = scrollEl.scrollHeight;
      lastScrollTop = scrollEl.scrollTop;
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
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        clearAll();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        filterInputRef?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  let filterInputRef: HTMLInputElement | undefined;
  let filterOverlayRef: HTMLDivElement | undefined;

  // Saved-filters scope. The logcat window has its own filter list,
  // kept separate from captures by the `kind` column added in V005 —
  // captures and logcat use different DSLs, so the two scopes must
  // not bleed into each other (a captures query is almost always
  // invalid as a logcat query and vice versa).
  const savedStore = savedFiltersFor("logcat");
  const savedFilters = savedStore.filters;

  // Save-popover state. Mirrors CapturesView: name + colour + pin,
  // plus an "update vs save" decision based on a case-insensitive
  // exact name match against the existing list.
  const [saveOpen, setSaveOpen] = createSignal(false);
  const [saveName, setSaveName] = createSignal("");
  const [saveColor, setSaveColor] = createSignal(FILTER_PALETTE[0]!);
  const [savePinned, setSavePinned] = createSignal(false);
  const [saveBusy, setSaveBusy] = createSignal(false);
  const [savedListOpen, setSavedListOpen] = createSignal(false);
  let savePopoverRef: HTMLDivElement | undefined;
  let savedListRef: HTMLDivElement | undefined;

  const existingMatch = () => {
    const n = saveName().trim().toLowerCase();
    if (!n) return undefined;
    return savedFilters().find((f) => f.name.trim().toLowerCase() === n);
  };

  const openSave = () => {
    if (!filter().trim()) return;
    setSavedListOpen(false);
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
      if (savedListOpen() && savedListRef && !savedListRef.contains(target)) {
        setSavedListOpen(false);
      }
      if (
        headerMenuPos() &&
        headerMenuRef &&
        !headerMenuRef.contains(target)
      ) {
        setHeaderMenuPos(null);
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

  return (
    <div class="flex flex-col h-screen bg-bg text-fg text-xs">
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
          class="inline-flex items-center gap-1 px-2 py-1 rounded hover:bg-bg-muted"
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

        {/* Token-highlight overlay over a transparent input. The
            previous Solid-<For>-based overlay rendered only the
            first character of typed text — unclear why, since the
            same markup works in CapturesView. This version
            sidesteps Solid reactivity entirely: a memoized HTML
            string is rendered via `innerHTML`, so the DOM update
            is a single deterministic assignment. */}
        <SearchIcon size={14} class="text-fg-muted shrink-0" />
        <div class="flex-1 relative flex items-center bg-bg-muted rounded focus-within:ring-1 focus-within:ring-accent">
          <div
            ref={(el) => (filterOverlayRef = el)}
            aria-hidden="true"
            class="absolute inset-0 pointer-events-none text-xs font-mono overflow-hidden px-2 py-1 pr-14 flex items-center"
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
            class="relative w-full bg-transparent rounded px-2 py-1 pr-14 outline-none text-xs font-mono text-transparent caret-fg placeholder:text-fg-muted"
            placeholder={t()("logcat.filter_placeholder")}
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
          {/* Right-aligned action cluster: chevron (saved-filter
              dropdown — only when something to show), star (save
              current — only when filter is non-empty). Sits on top
              of the input via z-10. Stacked icons are spaced by
              gap-0.5; mr-1 keeps them off the rounded corner. */}
          <div class="absolute right-1 inset-y-0 flex items-center gap-0.5 z-10">
            <Show when={savedFilters().length > 0}>
              <button
                type="button"
                class={`p-1 rounded hover:bg-bg-subtle ${
                  savedListOpen() ? "text-accent" : "text-fg-muted"
                }`}
                title={t()("logcat.saved_filters_title")}
                aria-label={t()("logcat.saved_filters_title")}
                // stopPropagation on mousedown so the document-level
                // outside-click handler doesn't fire BEFORE the click
                // toggle below, which would close-then-reopen the
                // popover and visually do nothing.
                onMouseDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  setSaveOpen(false);
                  setSavedListOpen((v) => !v);
                }}
              >
                <ChevronDown size={14} />
              </button>
            </Show>
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

          {/* Saved-filters dropdown. Lists logcat-scoped entries with
              colour dot + name. Click applies (writes to `filter`),
              hover reveals delete. Pinned first, then alphabetical —
              ordering comes from the backend. */}
          <Show when={savedListOpen()}>
            <div
              ref={(el) => (savedListRef = el)}
              class="absolute right-0 top-full mt-1 w-64 z-30 bg-bg-subtle border border-border rounded shadow-lg py-1 text-xs max-h-80 overflow-auto"
              onMouseDown={(e) => e.stopPropagation()}
            >
              <Show
                when={savedFilters().length > 0}
                fallback={
                  <div class="px-3 py-2 text-fg-muted italic">
                    {t()("logcat.saved_filters_empty")}
                  </div>
                }
              >
                <For each={savedFilters()}>
                  {(f) => (
                    <div
                      class="group px-3 py-1.5 hover:bg-bg-muted cursor-pointer flex items-center gap-2"
                      title={tr("logcat.apply_filter", { query: f.query })}
                      onClick={() => {
                        setFilter(f.query);
                        setSavedListOpen(false);
                      }}
                    >
                      <FilterIcon size={12} style={{ color: f.color }} />
                      <span class="truncate flex-1">{f.name}</span>
                      <button
                        type="button"
                        class="opacity-0 group-hover:opacity-100 hover:text-danger shrink-0 p-0.5"
                        title={t()("logcat.delete_filter")}
                        onClick={(e) => {
                          e.stopPropagation();
                          if (
                            confirm(
                              tr("logcat.delete_filter_confirm", { name: f.name }),
                            )
                          ) {
                            void savedStore.remove(f.id);
                          }
                        }}
                      >
                        <X size={11} />
                      </button>
                    </div>
                  )}
                </For>
              </Show>
            </div>
          </Show>
        </div>
        <span class="text-fg-muted whitespace-nowrap">
          {tr("logcat.counter", {
            shown: String(visible().length),
            total: String(entries().length),
          })}
        </span>
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
          column-edge aligned. */}
      <div
        class="grid font-mono text-fg-muted tracking-wide text-[10px] px-3 py-1 border-b border-border bg-bg-subtle/60"
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
          .map would rebuild DOM nodes each batch. */}
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
                    {(entry) => (
                      <div
                        class={`absolute left-0 right-0 grid font-mono whitespace-nowrap items-baseline px-3 py-px border-b border-border/30 ${LEVEL_ROW_COLOR[entry().level]}`}
                        style={{
                          transform: `translateY(${vi.start}px)`,
                          "grid-template-columns": gridTemplate(),
                        }}
                      >
                        <Show when={colVisible().time}>
                          <span class="truncate px-2 border-r border-border/30">
                            {entry().timestamp}
                          </span>
                        </Show>
                        <Show when={colVisible().pid}>
                          <span class="truncate px-2 border-r border-border/30">
                            {entry().pid > 0 ? entry().pid : ""}
                          </span>
                        </Show>
                        <Show when={colVisible().app}>
                          <span
                            class="truncate px-2 border-r border-border/30"
                            title={pidNames().get(entry().pid) ?? ""}
                          >
                            {pidNames().get(entry().pid) ?? ""}
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
                            {entry().tag}
                          </span>
                        </Show>
                        <Show when={colVisible().message}>
                          <span class="truncate px-2">{entry().message}</span>
                        </Show>
                      </div>
                    )}
                  </Show>
                );
              }}
            </For>
          </div>
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
