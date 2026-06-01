import { type Component, createSignal, createMemo, createEffect, onMount, onCleanup, For, Show } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { Search, Trash2, AlertTriangle, Lock, ShieldAlert, ArrowDownToLine, Pin, Star } from "lucide-solid";
import { api } from "@/ipc/client";
import { listenToCaptures } from "@/ipc/events";
import type { CaptureDto } from "@/ipc/types";
import DetailPanes from "@/components/DetailPanes";
import { VerticalResizer } from "@/components/VerticalResizer";
import {
  filter,
  setFilter,
  selectedId,
  setSelectedId,
  paused,
  setPaused,
} from "@/stores/captures";
import { refreshFilters, saveFilter } from "@/stores/saved-filters";
import HelpButton from "@/components/HelpButton";

const FILTER_PALETTE = [
  "#60a5fa", // blue
  "#f87171", // red
  "#facc15", // yellow
  "#34d399", // green
  "#a78bfa", // purple
  "#fb923c", // orange
];

const LIST_PANE_DEFAULT = 720;
const LIST_PANE_MIN = 320;
const LIST_PANE_MAX = 1600;
const LIST_PANE_STORAGE_KEY = "pane:list-pane-width";

function loadListPaneWidth(): number {
  try {
    const raw = localStorage.getItem(LIST_PANE_STORAGE_KEY);
    if (!raw) return LIST_PANE_DEFAULT;
    const n = JSON.parse(raw);
    if (typeof n === "number" && n >= LIST_PANE_MIN && n <= LIST_PANE_MAX) return n;
  } catch {
    /* fall through */
  }
  return LIST_PANE_DEFAULT;
}

// Columns: header label + default width. Widths are mutable px values
// (no `fr`) so the resize math stays predictable. Persisted to localStorage.
const COLUMNS = [
  { key: "idx", label: "#", width: 40 },
  { key: "method", label: "M", width: 64 },
  { key: "status", label: "St", width: 56 },
  { key: "host", label: "Host", width: 220 },
  { key: "path", label: "Path", width: 320 },
  { key: "ms", label: "ms", width: 64 },
  { key: "bytes", label: "bytes", width: 72 },
] as const;
const MIN_COL_WIDTH = 28;
const WIDTHS_STORAGE_KEY = "pane:captures-col-widths";
const AUTOFOLLOW_STORAGE_KEY = "pane:captures-autofollow";

const FILTER_HELP = [
  "Bare word: matches host OR path. E.g. 'google'.",
  "key:value — host, path, method, status, mime, size, duration, error.",
  "Wildcards: * inside the value (e.g. host:*google*).",
  "Negate with ! (e.g. !error:tls_handshake, !host:cdn.*).",
  "Range: status:500..599 — size:0..1024 — duration:..200.",
  "Multiple tokens are ANDed (e.g. host:rc3.test.dev-og.com method:post).",
].join("\n");

function loadAutoFollow(): boolean {
  try {
    const v = localStorage.getItem(AUTOFOLLOW_STORAGE_KEY);
    if (v === null) return true;
    return JSON.parse(v) === true;
  } catch {
    return true;
  }
}

function loadWidths(): number[] {
  try {
    const raw = localStorage.getItem(WIDTHS_STORAGE_KEY);
    if (!raw) return COLUMNS.map((c) => c.width);
    const parsed = JSON.parse(raw);
    if (
      Array.isArray(parsed) &&
      parsed.length === COLUMNS.length &&
      parsed.every((n) => typeof n === "number" && n >= MIN_COL_WIDTH)
    ) {
      return parsed;
    }
  } catch {
    // Corrupt JSON or storage disabled — fall through to defaults.
  }
  return COLUMNS.map((c) => c.width);
}

const CapturesView: Component = () => {
  const navigate = useNavigate();
  const [captures, setCaptures] = createSignal<CaptureDto[]>([]);
  const [filterError, setFilterError] = createSignal<string | null>(null);
  // Auto-follow tail: when true, new captures yank the list to the bottom.
  // The user's preference is persisted; manual scroll up/down also adjusts
  // it implicitly (Android Studio Logcat-style — scrolling away pauses
  // tailing, scrolling back to the bottom resumes).
  const [autoFollow, setAutoFollow] = createSignal(loadAutoFollow());
  createEffect(() => {
    try {
      localStorage.setItem(AUTOFOLLOW_STORAGE_KEY, JSON.stringify(autoFollow()));
    } catch {
      /* private mode */
    }
  });
  const [colWidths, setColWidths] = createSignal<number[]>(loadWidths());
  const gridTemplate = createMemo(() => colWidths().map((w) => `${w}px`).join(" "));
  const [listPaneWidth, setListPaneWidth] = createSignal(loadListPaneWidth());
  const splitTemplate = createMemo(() => `${listPaneWidth()}px 6px 1fr`);
  createEffect(() => {
    try {
      localStorage.setItem(LIST_PANE_STORAGE_KEY, JSON.stringify(listPaneWidth()));
    } catch {
      /* private mode */
    }
  });
  let scrollEl: HTMLDivElement | undefined;

  // Persist column widths whenever they change.
  createEffect(() => {
    try {
      localStorage.setItem(WIDTHS_STORAGE_KEY, JSON.stringify(colWidths()));
    } catch {
      // Storage disabled — widths stay in memory for this session.
    }
  });

  function startResize(idx: number, ev: MouseEvent) {
    ev.preventDefault();
    ev.stopPropagation();
    const startX = ev.clientX;
    const startW = colWidths()[idx]!;
    const onMove = (e: MouseEvent) => {
      const next = Math.max(MIN_COL_WIDTH, startW + (e.clientX - startX));
      setColWidths((ws) => {
        const copy = ws.slice();
        copy[idx] = next;
        return copy;
      });
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
  }

  const refresh = async () => {
    if (paused()) return;
    try {
      setCaptures(await api.captures.list(filter() || undefined, 500));
      setFilterError(null);
    } catch (e: any) {
      setFilterError(e?.message ?? "filter error");
    }
  };

  // Manual scroll implicitly toggles tailing: away from the bottom turns it
  // off, scrolling back to the bottom turns it on. `ignoreNextScroll` masks
  // the scroll event emitted by our own programmatic scrollTop write so we
  // don't toggle ourselves on. 8px slack absorbs sub-pixel rounding.
  let ignoreNextScroll = false;
  function handleScroll() {
    if (ignoreNextScroll) {
      ignoreNextScroll = false;
      return;
    }
    if (!scrollEl) return;
    const atBottom =
      scrollEl.scrollTop + scrollEl.clientHeight >= scrollEl.scrollHeight - 8;
    if (atBottom !== autoFollow()) setAutoFollow(atBottom);
  }

  // After each captures update, if tailing is on, scroll to the new bottom.
  // `queueMicrotask` lets the virtualizer measure new rows before scrolling.
  createEffect(() => {
    captures();
    if (!autoFollow() || !scrollEl) return;
    queueMicrotask(() => {
      if (!scrollEl) return;
      ignoreNextScroll = true;
      scrollEl.scrollTop = scrollEl.scrollHeight;
    });
  });

  // Save-filter popover state. Opens with the current filter pre-filled so
  // the user can name it without retyping. Kept local — closing on save or
  // outside-click is the popover's job, not anyone else's.
  const [saveOpen, setSaveOpen] = createSignal(false);
  const [saveName, setSaveName] = createSignal("");
  const [saveColor, setSaveColor] = createSignal(FILTER_PALETTE[0]!);
  const [savePinned, setSavePinned] = createSignal(false);
  const [saveBusy, setSaveBusy] = createSignal(false);
  let savePopover: HTMLDivElement | undefined;
  let filterInput: HTMLInputElement | undefined;
  let filterOverlay: HTMLPreElement | undefined;

  function syncFilterScroll() {
    if (filterInput && filterOverlay) {
      filterOverlay.scrollLeft = filterInput.scrollLeft;
    }
  }

  function openSave() {
    if (!filter().trim()) return;
    setSaveName("");
    setSaveColor(FILTER_PALETTE[0]!);
    setSavePinned(false);
    setSaveOpen(true);
    queueMicrotask(() => savePopover?.querySelector("input")?.focus());
  }

  async function doSave(e: Event) {
    e.preventDefault();
    const name = saveName().trim();
    if (!name || saveBusy()) return;
    setSaveBusy(true);
    try {
      await saveFilter({ name, query: filter(), color: saveColor(), pinned: savePinned() });
      setSaveOpen(false);
    } catch (err) {
      console.error("save filter failed", err);
      alert("Save failed: " + ((err as { message?: string })?.message ?? String(err)));
    } finally {
      setSaveBusy(false);
    }
  }

  // Close popover when user clicks outside it.
  onMount(() => {
    const onDoc = (e: MouseEvent) => {
      if (!saveOpen() || !savePopover) return;
      if (!savePopover.contains(e.target as Node)) setSaveOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    onCleanup(() => document.removeEventListener("mousedown", onDoc));
    // Make sure the sidebar's filter list is fresh on first mount even if
    // Layout hadn't yet populated it.
    refreshFilters();
  });

  // Explicit click on the tail-toggle button: if turning on, snap to bottom
  // immediately so the user sees the latest entries without waiting for the
  // next refresh tick.
  function toggleAutoFollow() {
    const next = !autoFollow();
    setAutoFollow(next);
    if (next && scrollEl) {
      ignoreNextScroll = true;
      scrollEl.scrollTop = scrollEl.scrollHeight;
    }
  }

  let refreshTimer: ReturnType<typeof setTimeout>;
  const debouncedRefresh = () => {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(refresh, 200);
  };

  onMount(() => {
    refresh();
    const off = listenToCaptures(() => debouncedRefresh());
    const t = setInterval(refresh, 1500);
    onCleanup(() => {
      off();
      clearInterval(t);
    });
  });

  const virtualizer = createMemo(() =>
    createVirtualizer({
      count: captures().length,
      getScrollElement: () => scrollEl ?? null,
      estimateSize: () => 32,
      overscan: 10,
    }),
  );

  const selected = createMemo(() => captures().find((c) => c.id === selectedId()) ?? null);

  const clearAll = async () => {
    if (!confirm("Clear all captures? This cannot be undone.")) return;
    try {
      await api.captures.clear();
    } catch (e: unknown) {
      const msg = (e as { message?: string })?.message ?? String(e);
      alert(`Clear failed: ${msg}`);
      return;
    }
    refresh();
  };

  return (
    <div class="h-full grid grid-rows-[auto_1fr] grid-cols-1">
      <div class="flex items-center gap-2 px-3 py-2 border-b border-border bg-bg-subtle">
        <Search size={14} class="text-fg-muted shrink-0" />
        <div class="flex-1 relative flex items-center">
          <pre
            ref={(el) => (filterOverlay = el)}
            aria-hidden="true"
            class="absolute inset-0 pointer-events-none text-sm font-mono whitespace-pre overflow-hidden pr-6 m-0 flex items-center"
          >
            <span class="flex-shrink-0">
              <FilterHighlight text={filter()} />
            </span>
          </pre>
          <input
            autocomplete="off"
            autocorrect="off"
            autocapitalize="off"
            spellcheck={false}
            ref={(el) => (filterInput = el)}
            class="w-full bg-transparent outline-none text-sm placeholder:text-fg-muted font-mono pr-6 text-transparent caret-fg relative"
            placeholder="google · host:api.example.com · status:5.. · !error:tls_handshake"
            value={filter()}
            onInput={(e) => {
              setFilter(e.currentTarget.value);
              debouncedRefresh();
              syncFilterScroll();
            }}
            onScroll={syncFilterScroll}
            onKeyDown={(e) => {
              if (e.key === "Escape" && filter()) {
                e.preventDefault();
                setFilter("");
                debouncedRefresh();
              }
            }}
            onKeyUp={syncFilterScroll}
            onClick={syncFilterScroll}
            title={FILTER_HELP}
          />
          <Show when={filter().trim()}>
            <button
              type="button"
              class="absolute right-0 text-fg-muted hover:text-warn p-0.5 rounded hover:bg-bg-muted"
              title="Save current filter to sidebar"
              aria-label="Save filter"
              onClick={openSave}
            >
              <Star size={14} />
            </button>
          </Show>
          <Show when={saveOpen()}>
            <div
              ref={(el) => (savePopover = el)}
              class="absolute right-0 top-full mt-1 w-72 z-30 bg-bg-subtle border border-border rounded shadow-lg p-3 text-xs"
              onMouseDown={(e) => e.stopPropagation()}
            >
              <form onSubmit={doSave} class="space-y-2">
                <div class="font-semibold text-fg-subtle uppercase tracking-wide">
                  Save filter
                </div>
                <div class="font-mono text-fg-muted bg-bg-muted rounded px-2 py-1 truncate">
                  {filter()}
                </div>
                <input
                  type="text"
                  class="w-full px-2 py-1.5 rounded bg-bg-muted outline-none focus:ring-1 focus:ring-accent"
                  placeholder="Name (e.g. my-backend)"
                  value={saveName()}
                  onInput={(e) => setSaveName(e.currentTarget.value)}
                  maxlength={64}
                />
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
                          aria-label={`Colour ${c}`}
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
                    <Pin size={11} /> Pin
                  </label>
                </div>
                <div class="flex justify-end gap-2 pt-1">
                  <button
                    type="button"
                    class="px-2 py-1 rounded hover:bg-bg-muted text-fg-muted"
                    onClick={() => setSaveOpen(false)}
                  >
                    Cancel
                  </button>
                  <button
                    type="submit"
                    class="px-3 py-1 rounded bg-accent text-white hover:opacity-90 disabled:opacity-50"
                    disabled={!saveName().trim() || saveBusy()}
                  >
                    {saveBusy() ? "Saving…" : "Save"}
                  </button>
                </div>
              </form>
            </div>
          </Show>
        </div>
        <Show when={filterError()}>
          <span class="text-xs text-danger">{filterError()}</span>
        </Show>
        <button
          class={`text-xs px-2 py-1 rounded inline-flex items-center gap-1 ${
            autoFollow()
              ? "bg-accent/15 text-accent hover:bg-accent/25"
              : "hover:bg-bg-muted text-fg-muted"
          }`}
          onClick={toggleAutoFollow}
          title={
            autoFollow()
              ? "Auto-scroll to newest is ON — click to lock the viewport"
              : "Auto-scroll is OFF — click to follow the tail"
          }
          aria-pressed={autoFollow()}
        >
          <ArrowDownToLine size={12} /> {autoFollow() ? "Tail" : "Tail off"}
        </button>
        <button
          class="text-xs px-2 py-1 rounded hover:bg-bg-muted"
          onClick={() => setPaused((p) => !p)}
          title="Pause UI updates"
        >
          {paused() ? "Resume" : "Pause"}
        </button>
        <button
          class="text-xs px-2 py-1 rounded hover:bg-bg-muted text-danger inline-flex items-center gap-1"
          onClick={clearAll}
        >
          <Trash2 size={12} /> Clear
        </button>
        <HelpButton path="/filtering/" title="Filter syntax: host, method, status, path, glob, ranges, negation" class="px-1" />
      </div>

      <div
        class="grid h-full overflow-hidden"
        style={{ "grid-template-columns": splitTemplate() }}
      >
        <div
          ref={(el) => (scrollEl = el)}
          onScroll={handleScroll}
          class="overflow-auto relative text-xs font-mono"
        >
          <div
            class="grid sticky top-0 bg-bg-subtle text-fg-muted z-10 border-b border-border select-none"
            style={{ "grid-template-columns": gridTemplate() }}
          >
            <For each={COLUMNS}>
              {(col, i) => (
                <div class="relative px-2 py-1 overflow-hidden">
                  <span class="truncate block">{col.label}</span>
                  <Show when={i() < COLUMNS.length - 1}>
                    {/*
                      6px-wide hit area for the cursor; the 1px visible line
                      sits at its right edge via `border-r`. Default colour =
                      border token; on hover/active it brightens to accent so
                      it's obvious you can grab it.
                    */}
                    <div
                      class="group absolute top-0 bottom-0 -right-[3px] w-1.5 cursor-col-resize z-20"
                      onMouseDown={(e) => startResize(i(), e)}
                      onDblClick={(e) => {
                        e.stopPropagation();
                        // Double-click resets this column to its default width.
                        setColWidths((ws) => {
                          const copy = ws.slice();
                          copy[i()] = COLUMNS[i()]!.width;
                          return copy;
                        });
                      }}
                      title="Drag to resize · double-click to reset"
                    >
                      <div class="absolute right-1/2 top-1 bottom-1 w-px bg-border group-hover:bg-accent group-active:bg-accent" />
                    </div>
                  </Show>
                </div>
              )}
            </For>
          </div>
          <Show
            when={captures().length > 0}
            fallback={
              <div class="px-4 py-8 text-center text-fg-muted">
                <AlertTriangle class="inline mr-1" size={14} />
                No captures yet. Start the proxy and pair a device.
              </div>
            }
          >
            <div class="relative" style={{ height: `${virtualizer().getTotalSize()}px` }}>
              <For each={virtualizer().getVirtualItems()}>
                {(row) => {
                  const cap = captures()[row.index];
                  return (
                    <div
                      class={`grid absolute left-0 right-0 items-center cursor-pointer ${
                        selectedId() === cap.id ? "bg-bg-muted" : "hover:bg-bg-subtle"
                      }`}
                      style={{
                        transform: `translateY(${row.start}px)`,
                        height: "32px",
                        "grid-template-columns": gridTemplate(),
                      }}
                      onClick={() => setSelectedId(cap.id)}
                      onDblClick={() => navigate(`/replay/${cap.id}`)}
                    >
                      <div class="px-2 truncate text-fg-muted">{row.index + 1}</div>
                      <div class="px-2 truncate">{cap.method}</div>
                      <div
                        class={`px-2 truncate ${statusColor(cap.status, cap.error_kind)}`}
                        title={errorHint(cap.error_kind)}
                      >
                        {cap.error_kind === "pinning" ? (
                          <Lock size={12} />
                        ) : cap.error_kind === "tls_handshake" ? (
                          <ShieldAlert size={12} />
                        ) : (
                          cap.status ?? "—"
                        )}
                      </div>
                      <div class="px-2 truncate">{cap.server_host}</div>
                      <div class="px-2 truncate">{cap.url_path}</div>
                      <div class="px-2 truncate text-fg-muted">{cap.duration_ms ?? "—"}</div>
                      <div class="px-2 truncate text-fg-muted">{fmtBytes(cap.total_bytes)}</div>
                    </div>
                  );
                }}
              </For>
            </div>
          </Show>
        </div>

        <VerticalResizer
          onResize={(dx) =>
            setListPaneWidth((w) =>
              Math.min(LIST_PANE_MAX, Math.max(LIST_PANE_MIN, w + dx)),
            )
          }
          onReset={() => setListPaneWidth(LIST_PANE_DEFAULT)}
        />

        <div class="overflow-hidden min-w-[180px]">
          <DetailPanes capture={selected()} />
        </div>
      </div>
    </div>
  );
};

function statusColor(status: number | null, errorKind: string | null) {
  if (errorKind === "pinning") return "text-warn";
  if (errorKind === "tls_handshake") return "text-warn";
  if (errorKind) return "text-danger";
  if (status === null) return "text-fg-muted";
  if (status >= 500) return "text-danger";
  if (status >= 400) return "text-warn";
  if (status >= 300) return "text-accent";
  return "text-success";
}

function errorHint(errorKind: string | null): string | undefined {
  switch (errorKind) {
    case "tls_handshake":
      return "TLS handshake failed — device doesn't trust Pane CA. Install root CA in system store (root/Magisk) or configure network_security_config.";
    case "pinning":
      return "Certificate pinning detected — app rejects Pane's leaf cert. Bypass requires Frida or similar.";
    case "upstream":
      return "Upstream server connection failed.";
    case "connect_pipelined":
      return "Client sent bytes before the CONNECT handshake completed — protocol violation.";
    case "connection_refused":
      return "Could not connect to upstream host.";
    default:
      return errorKind ?? undefined;
  }
}

function fmtBytes(n: number) {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}K`;
  return `${(n / 1024 / 1024).toFixed(1)}M`;
}

// Filter DSL keys recognised by the backend (see crates/pane-storage/src/filter_dsl.rs).
// Used for live key validation in the input.
const VALID_FILTER_KEYS = new Set([
  "host",
  "method",
  "status",
  "mime",
  "path",
  "size",
  "duration",
  "error",
]);

/// Render the filter input's text with per-token colours. Mirrors the input
/// character-for-character (same font + whitespace) so it can sit behind a
/// transparent input.
const FilterHighlight: import("solid-js").Component<{ text: string }> = (p) => {
  const parts = () => tokenizeForHighlight(p.text);
  return (
    <For each={parts()}>
      {(part) => <span class={part.cls}>{part.text}</span>}
    </For>
  );
};

type HlPart = { text: string; cls: string };

function tokenizeForHighlight(text: string): HlPart[] {
  const out: HlPart[] = [];
  const tokens = text.match(/\s+|\S+/g) ?? [];
  for (const tok of tokens) {
    if (/^\s+$/.test(tok)) {
      out.push({ text: tok, cls: "" });
      continue;
    }
    const m = tok.match(/^(!?)([a-zA-Z_]+)(:)(.*)$/);
    if (m) {
      const [, bang, key, colon, value] = m;
      if (bang) out.push({ text: bang, cls: "text-danger" });
      const known = VALID_FILTER_KEYS.has(key.toLowerCase());
      out.push({
        text: key,
        cls: known ? "text-accent" : "text-danger underline decoration-dotted",
      });
      out.push({ text: colon, cls: "text-fg-muted" });
      if (value) out.push({ text: value, cls: "text-fg" });
    } else {
      out.push({ text: tok, cls: "text-fg" });
    }
  }
  return out;
}

export default CapturesView;
