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
  Target,
  Trash2,
  X,
} from "lucide-solid";
import { t, tr } from "@/i18n";
import { compileLogcatFilter } from "@/lib/logcat-filter";
import { savedFiltersFor } from "@/stores/saved-filters";

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

// Hard cap to bound memory on a chatty device. 10k entries × ~200B
// average = ~2 MB resident, fits comfortably and gives ~30s of history
// even on a verbose logcat firehose. Older entries shift out FIFO.
const MAX_ENTRIES = 10_000;

const LEVEL_COLOR: Record<LogLevel, string> = {
  verbose: "text-fg-muted",
  debug: "text-accent",
  info: "text-success",
  warn: "text-warn",
  error: "text-danger",
  fatal: "text-danger font-bold",
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
  // Follow-app state. When `followApp` is set, we periodically resolve
  // the package's current PID via `android_pidof` and additionally
  // filter visible entries to that pid only. Resolves transparently
  // when the app restarts (PID changes) — that's the whole point of
  // "follow", as opposed to a stale `pid:1234` literal in the filter.
  const [packages, setPackages] = createSignal<string[]>([]);
  const [followApp, setFollowApp] = createSignal<string | null>(null);
  const [followedPid, setFollowedPid] = createSignal<number | null>(null);

  // Resizable column widths. Persisted in localStorage so a user's
  // preferred layout survives close/reopen of the logcat window.
  // `level` is fixed-1-char; `message` takes whatever space is left
  // (`1fr`). Drag handles live only on Time/PID/Tag.
  type ColKey = "time" | "pid" | "tag";
  const COL_DEFAULTS: Record<ColKey, number> = { time: 90, pid: 60, tag: 180 };
  const COL_MIN = 40;
  const COL_STORAGE_KEY = "pane.logcat.col-widths";

  const loadColWidths = (): Record<ColKey, number> => {
    try {
      const raw = localStorage.getItem(COL_STORAGE_KEY);
      if (!raw) return { ...COL_DEFAULTS };
      const parsed = JSON.parse(raw) as Partial<Record<ColKey, number>>;
      return {
        time: clampWidth(parsed.time ?? COL_DEFAULTS.time),
        pid: clampWidth(parsed.pid ?? COL_DEFAULTS.pid),
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
    return `${w.time}px ${w.pid}px 14px ${w.tag}px 1fr`;
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
  let scrollEl: HTMLDivElement | undefined;
  let ignoreNextScroll = false;

  // Compile the filter once per typed input — cheap regex/parse, then
  // we apply the predicate over the buffer on every render tick.
  const matcher = createMemo(() => {
    try {
      setErrorMsg(null);
      return compileLogcatFilter(filter());
    } catch (e: unknown) {
      setErrorMsg((e as { message?: string })?.message ?? String(e));
      return () => true;
    }
  });

  const visible = createMemo(() => {
    const m = matcher();
    const pid = followedPid();
    const all = entries();
    if (pid === null) return all.filter(m);
    return all.filter((e) => e.pid === pid && m(e));
  });

  // Union of `packages()` (running, third-party — refreshed every
  // 10s) with the currently-followed app, so the `<select>` always
  // has an option whose `value` matches the bound value. Without
  // this, every refresh tick can transiently drop the selected
  // option and reset the select to "(off)" — visible to the user
  // as the dropdown text flipping even though `followApp` signal
  // is unchanged.
  const dropdownPackages = createMemo(() => {
    const list = [...packages()];
    const cur = followApp();
    if (cur && !list.includes(cur)) list.unshift(cur);
    return list;
  });

  // Re-fetch the running-app list every 10s so newly-launched apps
  // appear in the dropdown without the user having to reopen the
  // window. ps -A roundtrip is ~50ms over USB — barely noticeable.
  // The active selection is preserved across refreshes; if the user
  // had picked an app that's since exited, the "(not running)"
  // indicator next to the dropdown surfaces that, the dropdown
  // option itself stays as-typed.
  onMount(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const list = await invoke<string[]>("android_list_packages", { serial });
        if (!cancelled) setPackages(list);
      } catch {
        if (!cancelled) setPackages([]);
      }
    };
    tick();
    const handle = setInterval(tick, 10000);
    onCleanup(() => {
      cancelled = true;
      clearInterval(handle);
    });
  });

  // While Follow-app is on, resolve the PID periodically. Polling is
  // crude vs. an event-based hook (PROCESS_STATE_BROADCAST etc. would
  // be cleaner), but adb shell pidof is a 10ms round-trip and the
  // user can't tell the difference at 5s granularity.
  createEffect(() => {
    const pkg = followApp();
    if (!pkg) {
      setFollowedPid(null);
      return;
    }
    let cancelled = false;
    const tick = async () => {
      try {
        const pid = await invoke<number | null>("android_pidof", { serial, package: pkg });
        if (!cancelled) setFollowedPid(pid);
      } catch {
        if (!cancelled) setFollowedPid(null);
      }
    };
    tick();
    const handle = setInterval(tick, 5000);
    onCleanup(() => {
      cancelled = true;
      clearInterval(handle);
    });
  });

  // Subscribe to the per-window batched stream. Backend emits
  // `logcat://batch` with payload Vec<LogEntry> every 100ms / 50
  // entries (whichever first) on this WebviewWindow only — so the
  // main window never sees the firehose.
  onMount(() => {
    let unlistenBatch: UnlistenFn | undefined;
    let unlistenError: UnlistenFn | undefined;

    listen<LogEntry[]>("logcat://batch", (e) => {
      if (paused()) return;
      setEntries((prev) => {
        // Append + truncate to MAX_ENTRIES. Slicing once per batch is
        // cheap; doing it per-entry would thrash GC on a firehose.
        const next = prev.concat(e.payload);
        return next.length > MAX_ENTRIES ? next.slice(next.length - MAX_ENTRIES) : next;
      });
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
    });
  });

  // Auto-scroll to bottom when new entries arrive and the user hasn't
  // scrolled up.
  createEffect(() => {
    void visible().length;
    if (!autoScroll() || !scrollEl) return;
    queueMicrotask(() => {
      if (!scrollEl) return;
      ignoreNextScroll = true;
      scrollEl.scrollTop = scrollEl.scrollHeight;
    });
  });

  // Detect user scroll-away from the bottom and switch auto-scroll
  // off. Re-engaging is via the toolbar toggle.
  const onScroll = () => {
    if (!scrollEl) return;
    if (ignoreNextScroll) {
      ignoreNextScroll = false;
      return;
    }
    const atBottom =
      scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight < 4;
    if (!atBottom && autoScroll()) setAutoScroll(false);
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
      ignoreNextScroll = true;
      scrollEl.scrollTop = scrollEl.scrollHeight;
    }
  };

  // Hotkeys. Space toggles pause; Cmd/Ctrl-K clears; Cmd/Ctrl-F focuses
  // the filter input. Active only when no input is focused (so typing
  // a space inside the filter doesn't pause).
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName ?? "";
      const inTextField = tag === "INPUT" || tag === "TEXTAREA";
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        clearAll();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        filterInputRef?.focus();
        return;
      }
      if (e.key === " " && !inTextField) {
        e.preventDefault();
        togglePause();
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
  const virtualizer = createVirtualizer<HTMLDivElement, HTMLDivElement>({
    get count() {
      return visible().length;
    },
    getScrollElement: () => scrollEl ?? null,
    estimateSize: () => 22,
    overscan: 30,
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

        {/* Follow-app dropdown. The options list is the union of
            currently-running third-party packages PLUS whatever the
            user has selected. Putting `followApp()` in the list
            unconditionally (when set) prevents the browser from
            silently resetting the `<select>` to the first option
            on the 10s `setPackages` refresh — without this, every
            refresh tick momentarily breaks the value-binding when
            the prior option set is replaced. The "(not running)"
            indicator beside the dropdown surfaces a stale pick. */}
        <select
          class={`text-xs px-2 py-1 rounded outline-none max-w-[200px] ${
            followApp() ? "bg-accent/15 text-accent" : "bg-bg-muted text-fg-muted"
          }`}
          value={followApp() ?? ""}
          onChange={(e) => {
            const v = e.currentTarget.value;
            setFollowApp(v === "" ? null : v);
          }}
          title={t()("logcat.follow_app_title")}
        >
          <option value="">{t()("logcat.follow_app_none")}</option>
          <For each={dropdownPackages()}>
            {(pkg) => <option value={pkg}>{pkg}</option>}
          </For>
        </select>
        <Show when={followApp()}>
          <span class="inline-flex items-center gap-1 text-fg-muted whitespace-nowrap">
            <Target size={12} />
            <Show when={followedPid()} fallback={<i>{t()("logcat.follow_app_no_pid")}</i>}>
              pid {followedPid()}
            </Show>
          </span>
        </Show>

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
            class="absolute inset-0 pointer-events-none text-xs font-mono whitespace-pre overflow-hidden px-2 py-1 pr-14 flex items-center"
            innerHTML={highlightedFilterHtml()}
          />
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
        class="grid font-mono text-fg-muted uppercase tracking-wide text-[10px] px-3 py-1 border-b border-border bg-bg-subtle/60"
        style={{ "grid-template-columns": gridTemplate() }}
      >
        <HeaderCell
          label={t()("logcat.col_time")}
          onResize={(e) => startColResize("time", e)}
          onReset={() => setColWidths({ ...colWidths(), time: COL_DEFAULTS.time })}
        />
        <HeaderCell
          label={t()("logcat.col_pid")}
          onResize={(e) => startColResize("pid", e)}
          onReset={() => setColWidths({ ...colWidths(), pid: COL_DEFAULTS.pid })}
        />
        <span class="px-1 border-r border-border/40">{t()("logcat.col_level")}</span>
        <HeaderCell
          label={t()("logcat.col_tag")}
          onResize={(e) => startColResize("tag", e)}
          onReset={() => setColWidths({ ...colWidths(), tag: COL_DEFAULTS.tag })}
        />
        <span class="px-2">{t()("logcat.col_message")}</span>
      </div>

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
                const e = visible()[vi.index];
                if (!e) return null;
                return (
                  <div
                    class="absolute left-0 right-0 grid font-mono whitespace-nowrap items-baseline px-3 py-px"
                    style={{
                      transform: `translateY(${vi.start}px)`,
                      "grid-template-columns": gridTemplate(),
                    }}
                  >
                    <span class="text-fg-muted truncate px-2 border-r border-border/30">
                      {e.timestamp}
                    </span>
                    <span class="text-fg-muted truncate px-2 border-r border-border/30">
                      {e.pid > 0 ? e.pid : ""}
                    </span>
                    <span class={`px-1 border-r border-border/30 ${LEVEL_COLOR[e.level]}`}>
                      {LEVEL_CHAR[e.level]}
                    </span>
                    <span class="truncate px-2 border-r border-border/30">{e.tag}</span>
                    <span class="truncate px-2">{e.message}</span>
                  </div>
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

const LOGCAT_VALID_KEYS = new Set(["tag", "msg", "message", "level", "pid"]);

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
      const [, key, colon, value] = m;
      const known = LOGCAT_VALID_KEYS.has(key!.toLowerCase());
      const cls = known
        ? "text-accent"
        : "text-danger underline decoration-dotted";
      out.push(`<span class="${cls}">${escapeHtml(key!)}</span>`);
      out.push(`<span class="text-fg-muted">${escapeHtml(colon!)}</span>`);
      if (value) out.push(`<span class="text-fg">${escapeHtml(value)}</span>`);
    } else {
      out.push(`<span class="text-fg">${escapeHtml(body)}</span>`);
    }
  }
  return out.join("");
}

export default LogcatView;
