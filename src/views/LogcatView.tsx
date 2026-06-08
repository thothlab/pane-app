import {
  type Component,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { Pause, Play, Trash2, ArrowDown, Search as SearchIcon } from "lucide-solid";
import { t, tr } from "@/i18n";
import { compileLogcatFilter } from "@/lib/logcat-filter";

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
    return entries().filter(m);
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

  const virtualizer = createMemo(() =>
    createVirtualizer({
      count: visible().length,
      getScrollElement: () => scrollEl ?? null,
      estimateSize: () => 22,
      overscan: 30,
    }),
  );

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
          class={`inline-flex items-center gap-1 px-2 py-1 rounded ${
            autoScroll() ? "bg-accent/15 text-accent" : "hover:bg-bg-muted text-fg-muted"
          }`}
          onClick={toggleAutoScroll}
          title={t()("logcat.auto_scroll_title")}
        >
          <ArrowDown size={12} />
          {t()("logcat.auto_scroll")}
        </button>
        <div class="flex-1 relative">
          <SearchIcon
            size={12}
            class="absolute left-2 top-1/2 -translate-y-1/2 text-fg-muted pointer-events-none"
          />
          <input
            ref={(el) => (filterInputRef = el)}
            type="text"
            class="w-full pl-7 pr-2 py-1 rounded bg-bg-muted outline-none focus:ring-1 focus:ring-accent font-mono"
            placeholder={t()("logcat.filter_placeholder")}
            value={filter()}
            onInput={(e) => setFilter(e.currentTarget.value)}
            title={t()("logcat.filter_help")}
          />
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

      {/* Virtualized table */}
      <div
        ref={(el) => (scrollEl = el)}
        class="flex-1 overflow-auto"
        onScroll={onScroll}
      >
        <div
          style={{
            height: `${virtualizer().getTotalSize()}px`,
            position: "relative",
            width: "100%",
          }}
        >
          {virtualizer()
            .getVirtualItems()
            .map((vi) => {
              const e = visible()[vi.index]!;
              return (
                <div
                  class="absolute left-0 right-0 grid font-mono whitespace-nowrap items-baseline gap-2 px-3 py-px"
                  style={{
                    transform: `translateY(${vi.start}px)`,
                    "grid-template-columns": "90px 60px 14px 180px 1fr",
                  }}
                >
                  <span class="text-fg-muted truncate">{e.timestamp}</span>
                  <span class="text-fg-muted truncate">{e.pid > 0 ? e.pid : ""}</span>
                  <span class={LEVEL_COLOR[e.level]}>{LEVEL_CHAR[e.level]}</span>
                  <span class="truncate">{e.tag}</span>
                  <span class="truncate">{e.message}</span>
                </div>
              );
            })}
        </div>
        {visible().length === 0 && (
          <div class="flex items-center justify-center h-full text-fg-muted italic">
            {entries().length === 0
              ? t()("logcat.empty_waiting")
              : t()("logcat.empty_filtered")}
          </div>
        )}
      </div>
    </div>
  );
};

export default LogcatView;
