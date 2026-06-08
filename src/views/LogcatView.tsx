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
import { writeTextFile } from "@tauri-apps/plugin-fs";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { Pause, Play, Trash2, ArrowDown, Search as SearchIcon, Target, Download } from "lucide-solid";
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
  // Follow-app state. When `followApp` is set, we periodically resolve
  // the package's current PID via `android_pidof` and additionally
  // filter visible entries to that pid only. Resolves transparently
  // when the app restarts (PID changes) — that's the whole point of
  // "follow", as opposed to a stale `pid:1234` literal in the filter.
  const [packages, setPackages] = createSignal<string[]>([]);
  const [followApp, setFollowApp] = createSignal<string | null>(null);
  const [followedPid, setFollowedPid] = createSignal<number | null>(null);

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
      await writeTextFile(path, lines.join("\n") + "\n");
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

      {/* Column header row. Lives outside the scroll container so it
          stays put when the user scrolls the firehose. Grid template
          matches the row template below — keep them in sync. */}
      <div
        class="grid font-mono text-fg-muted uppercase tracking-wide text-[10px] gap-2 px-3 py-1 border-b border-border bg-bg-subtle/60"
        style={{ "grid-template-columns": "90px 60px 14px 180px 1fr" }}
      >
        <span>{t()("logcat.col_time")}</span>
        <span>{t()("logcat.col_pid")}</span>
        <span>{t()("logcat.col_level")}</span>
        <span>{t()("logcat.col_tag")}</span>
        <span>{t()("logcat.col_message")}</span>
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
            <For each={virtualizer.getVirtualItems()}>
              {(vi) => {
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
                    <span class="text-fg-muted truncate">
                      {e.pid > 0 ? e.pid : ""}
                    </span>
                    <span class={LEVEL_COLOR[e.level]}>{LEVEL_CHAR[e.level]}</span>
                    <span class="truncate">{e.tag}</span>
                    <span class="truncate">{e.message}</span>
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

export default LogcatView;
