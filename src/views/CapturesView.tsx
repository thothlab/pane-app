import { type Component, createSignal, createMemo, createEffect, createResource, onMount, onCleanup, For, Show } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { Search, Trash2, AlertTriangle, Lock, ShieldAlert, ArrowDownToLine, Copy, Pin, Star, FolderPlus, Shuffle, ChevronDown } from "lucide-solid";
import { api } from "@/ipc/client";
import { listenToCaptures } from "@/ipc/events";
import type {
  CaptureDto,
  RuleCollectionDto,
  RuleDto,
  RuleUpsertArgs,
} from "@/ipc/types";
import {
  setRulesEditing,
  rulesCollapsed,
  setRulesCollapsed,
} from "@/stores/rules-ui";
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
import { filters, refreshFilters, saveFilter } from "@/stores/saved-filters";
import HelpButton from "@/components/HelpButton";
import { writeClipboard } from "@/lib/clipboard";
import { t, tr } from "@/i18n";

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

// Columns: i18n key + default width. Widths are mutable px values
// (no `fr`) so the resize math stays predictable. Persisted to
// localStorage. Header cells truncate with ellipsis when the column
// is narrower than the label — full text shows on hover via title.
const COLUMNS = [
  { key: "idx", labelKey: "captures.column_idx", width: 40 },
  { key: "device", labelKey: "captures.column_device", width: 140 },
  { key: "method", labelKey: "captures.column_method", width: 80 },
  { key: "status", labelKey: "captures.column_status", width: 72 },
  { key: "host", labelKey: "captures.column_host", width: 220 },
  { key: "path", labelKey: "captures.column_path", width: 320 },
  { key: "ms", labelKey: "captures.column_ms", width: 64 },
  { key: "bytes", labelKey: "captures.column_bytes", width: 72 },
] as const;
const MIN_COL_WIDTH = 28;
// v2: bumped when `device` moved to position 1 — the old positional width
// array is in the pre-reorder order, so reset rather than mis-map widths.
const WIDTHS_STORAGE_KEY = "pane:captures-col-widths-v2";
const VISIBLE_STORAGE_KEY = "pane:captures-col-visible";
const AUTOFOLLOW_STORAGE_KEY = "pane:captures-autofollow";

// Columns the header context-menu can hide. `idx` (the row number) is the
// anchor and always stays — it guarantees there's a place to right-click.
const TOGGLEABLE_COLS = ["device", "method", "status", "host", "path", "ms", "bytes"] as const;
type ToggleableCol = (typeof TOGGLEABLE_COLS)[number];
const VISIBLE_DEFAULTS: Record<ToggleableCol, boolean> = {
  device: true,
  method: true,
  status: true,
  host: true,
  path: true,
  ms: true,
  bytes: true,
};
function loadColVisible(): Record<ToggleableCol, boolean> {
  try {
    const raw = localStorage.getItem(VISIBLE_STORAGE_KEY);
    if (!raw) return { ...VISIBLE_DEFAULTS };
    const parsed = JSON.parse(raw) as Partial<Record<ToggleableCol, boolean>>;
    const out = { ...VISIBLE_DEFAULTS };
    for (const k of TOGGLEABLE_COLS) {
      if (typeof parsed[k] === "boolean") out[k] = parsed[k] as boolean;
    }
    return out;
  } catch {
    return { ...VISIBLE_DEFAULTS };
  }
}

// Filter help text — read via i18n at render time so it switches with
// the locale. The Russian translation mirrors the same examples.

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

  // Known devices, for resolving capture.device_id → display_name in the
  // table and populating the device-filter dropdown. Same source the Devices
  // view uses (api.devices.list). Refreshed on mount; the list is small and
  // changes rarely, so we don't poll it.
  const [devices, { refetch: refetchDevices }] = createResource(() =>
    api.devices.list(),
  );
  const deviceName = (id: string | null): string | null => {
    if (!id) return null;
    const d = (devices() ?? []).find((x) => x.id === id);
    return d?.display_name ?? null;
  };
  // display_name is "<vendor> <model> · Android <ver> · <serial-tail>".
  // `deviceHead` = "<vendor> <model>" (e.g. "CipherLab RS35") — shown in the
  // dropdown. `deviceModel` drops the vendor (first word) → just "RS35",
  // used in the table cell and the filter token. Single-word heads (vendor
  // unknown) keep the whole head as the model.
  const deviceHead = (full: string): string => full.split(" · ")[0]!.trim();
  const deviceModel = (full: string): string => {
    const head = deviceHead(full);
    const parts = head.split(/\s+/);
    return parts.length > 1 ? parts.slice(1).join(" ") : head;
  };
  const shortDeviceName = (id: string | null): string | null => {
    // The host sentinel has no device row — label it "This computer".
    if (id === "__host__") return t()("captures.device_this_computer");
    const full = deviceName(id);
    return full ? deviceModel(full) : null;
  };
  // The filter matches device by name/serial substring (see filter_dsl), so
  // the token is the short model — not the UUID. Quote it when it has spaces.
  const deviceFilterToken = (model: string): string =>
    model.includes(" ") ? `"${model}"` : model;

  // The device filter is driven by a custom button+menu (not a native
  // <select>, which desynced from the filter string). The current value is
  // derived from the device: token in the filter — `__host__` is the host
  // sentinel ("This computer"), any other value is a device model, none means
  // "Not selected".
  const selectedDeviceModel = createMemo(() => {
    // Match device:"quoted value" OR device:bareword.
    const m = filter().match(/(?:^|\s)device:("([^"]*)"|\S+)/);
    if (!m) return "";
    return (m[2] !== undefined ? m[2] : m[1]!).trim();
  });
  // Human label for the dropdown button, derived from the filter token.
  const selectedDeviceLabel = createMemo(() => {
    const v = selectedDeviceModel();
    if (!v) return t()("captures.device_filter_none");
    if (v === "__host__") return t()("captures.device_this_computer");
    return v;
  });
  // Replace the device: token (quoted or bare) with `tok` (or remove it when
  // empty). Does NOT touch host capture — turning host capture off is reserved
  // for Stop proxy / app exit, so "Not selected" still shows Mac traffic if it
  // was enabled.
  const setDeviceToken = (tok: string) => {
    const stripped = filter()
      .replace(/(?:^|\s)!?device:("[^"]*"|\S+)/g, "")
      .trim();
    const next = tok ? (stripped ? `${stripped} ${tok}` : tok) : stripped;
    setFilter(next);
    debouncedRefresh(true);
  };
  const onPickDevice = (model: string) => {
    setDeviceToken(model ? `device:${deviceFilterToken(model)}` : "");
    setDeviceMenuOpen(false);
  };
  // "This computer": enable host capture (trust CA + set system proxy), then
  // filter the list to host-local traffic via the __host__ sentinel token.
  const onPickThisComputer = async () => {
    setDeviceMenuOpen(false);
    try {
      await api.host.enable();
    } catch (e: unknown) {
      alert(
        tr("captures.host_capture_failed", {
          message: (e as { message?: string })?.message ?? String(e),
        }),
      );
      return;
    }
    setDeviceToken("device:__host__");
  };
  // Custom device dropdown open/close state. Outside-click close is wired into
  // the shared onMount mousedown handler alongside the other menus.
  const [deviceMenuOpen, setDeviceMenuOpen] = createSignal(false);
  let deviceMenuRef: HTMLDivElement | undefined;
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

  // Per-column show/hide (header right-click menu). `idx` is never in the
  // map — it's always shown. Persisted separately from widths.
  const [colVisible, setColVisibleRaw] = createSignal(loadColVisible());
  const isColVisible = (key: string): boolean =>
    key === "idx" || colVisible()[key as ToggleableCol] !== false;
  const setColVisible = (next: Record<ToggleableCol, boolean>) => {
    setColVisibleRaw(next);
    try {
      localStorage.setItem(VISIBLE_STORAGE_KEY, JSON.stringify(next));
    } catch {
      /* storage unavailable */
    }
  };
  // gridTemplate skips hidden columns; `colWidths` stays positionally
  // aligned with COLUMNS (8 entries) so resize-by-index keeps working.
  const gridTemplate = createMemo(() =>
    COLUMNS.map((c, i) => (isColVisible(c.key) ? `${colWidths()[i]}px` : null))
      .filter((p): p is string => p !== null)
      .join(" "),
  );
  // Index of the rightmost visible column — its header gets no resize
  // handle (nothing to its right to push against).
  const lastVisibleColIdx = createMemo(() => {
    let last = 0;
    COLUMNS.forEach((c, i) => {
      if (isColVisible(c.key)) last = i;
    });
    return last;
  });

  // Header context-menu (column toggles). Right-click the header row to
  // open at the cursor; outside click closes it (handled in onMount).
  const [headerMenuPos, setHeaderMenuPos] = createSignal<
    { x: number; y: number } | null
  >(null);
  let headerMenuRef: HTMLDivElement | undefined;
  const openHeaderMenu = (e: MouseEvent) => {
    e.preventDefault();
    setHeaderMenuPos({ x: e.clientX, y: e.clientY });
  };
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
  // Monotonic guard against out-of-order list responses. The 1.5s interval
  // and a filter keystroke can both have a `captures.list` in flight; if the
  // older (broader-filter) request resolves last it would clobber the newer
  // results with stale rows — e.g. typing `mtsm` but still seeing `mts`
  // matches. Stamp each call and drop any response that isn't the latest.
  let refreshSeq = 0;

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

  const refresh = async (force = false) => {
    if (paused()) return;
    // Poll-driven and backend-event-driven refreshes freeze while
    // Follow is off — otherwise every tick replaces captures(), the
    // virtualizer reflows, and the user is yanked away from what
    // they're reading. User-initiated calls (mount, filter change,
    // toggle Follow back on) pass force=true and always go through.
    if (!force && !autoFollow() && captures().length > 0) return;
    const seq = ++refreshSeq;
    try {
      const rows = await api.captures.list(filter() || undefined, 500);
      // A newer refresh started while we awaited — its results win.
      if (seq !== refreshSeq) return;
      setCaptures(rows);
      setFilterError(null);
    } catch (e: any) {
      if (seq !== refreshSeq) return;
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

  // When the typed name matches an existing saved filter (case-insensitive
  // exact, trimmed), submit becomes an update instead of an insert.
  // Lets the user overwrite an existing saved filter by name without
  // first finding-and-deleting it. UI text + button label switch to
  // "Update" / "Updating…" + show a "will overwrite «X»" hint.
  const existingMatch = () => {
    const n = saveName().trim().toLowerCase();
    if (!n) return undefined;
    return filters().find((f) => f.name.trim().toLowerCase() === n);
  };

  async function doSave(e: Event) {
    e.preventDefault();
    const name = saveName().trim();
    if (!name || saveBusy()) return;
    setSaveBusy(true);
    try {
      const match = existingMatch();
      await saveFilter({
        id: match?.id,
        name,
        query: filter(),
        color: saveColor(),
        pinned: savePinned(),
      });
      setSaveOpen(false);
    } catch (err) {
      console.error("save filter failed", err);
      alert(tr("captures.save_failed", { message: (err as { message?: string })?.message ?? String(err) }));
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
    if (next) {
      // Pull the latest entries right away so the user doesn't sit on
      // a stale view until the next 1.5s tick. Then snap to bottom.
      void refresh(true);
      if (scrollEl) {
        ignoreNextScroll = true;
        scrollEl.scrollTop = scrollEl.scrollHeight;
      }
    }
  }

  let refreshTimer: ReturnType<typeof setTimeout>;
  // `force` controls whether the refresh ignores the Follow-off gate.
  // Filter input passes true (user expects results immediately), the
  // backend `listenToCaptures` event passes false (let the gate decide).
  const debouncedRefresh = (force = false) => {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(() => void refresh(force), 200);
  };

  onMount(() => {
    // Initial load always goes through — even if the user had Follow
    // off from a previous session, they need something to look at.
    void refresh(true);
    const off = listenToCaptures(() => debouncedRefresh());
    const t = setInterval(refresh, 1500);
    onCleanup(() => {
      off();
      clearInterval(t);
    });
  });

  // Stable virtualizer instance. The earlier `createMemo(() =>
  // createVirtualizer({...}))` re-created the whole instance on every
  // count change, which wiped its scroll state — that's why the list
  // snapped back to the top a second after the user scrolled. Reading
  // `captures().length` via a getter keeps the count reactive without
  // rebuilding the virtualizer.
  const virtualizer = createVirtualizer<HTMLDivElement, HTMLDivElement>({
    get count() {
      return captures().length;
    },
    getScrollElement: () => scrollEl ?? null,
    estimateSize: () => 32,
    overscan: 10,
  });

  // ── Add-to-Rules context menu ──────────────────────────────────────
  // Right-clicking a row opens a small picker rather than the browser
  // context menu. It lists current rule collections and a "new
  // collection" action. Selecting any of them creates a stub rule
  // pre-filled from the capture (method + host + path + captured
  // response) — same shape the manual rule editor produces, so the
  // user can refine it in the Rules view if needed.
  const [addMenuPos, setAddMenuPos] = createSignal<
    { x: number; y: number; captureId: string } | null
  >(null);
  const [addCollections, setAddCollections] = createSignal<RuleCollectionDto[]>([]);
  const [addBusy, setAddBusy] = createSignal(false);
  const [addToast, setAddToast] = createSignal<string | null>(null);
  let addMenuRef: HTMLDivElement | undefined;

  const openAddMenu = async (e: MouseEvent, captureId: string) => {
    e.preventDefault();
    e.stopPropagation();
    setAddMenuPos({ x: e.clientX, y: e.clientY, captureId });
    // Refresh collections on each open — cheap call, and the user
    // may have created/renamed collections in the Rules tab since
    // we last looked.
    try {
      setAddCollections(await api.collections.list());
    } catch {
      setAddCollections([]);
    }
  };

  const closeAddMenu = () => setAddMenuPos(null);

  // After the menu renders (and re-renders when collections finish
  // loading), check if it spills past the viewport bottom/right and
  // flip the anchor up/left if so. Right-click near the bottom of
  // the captures list used to push the menu below the window so
  // its items got clipped — measure the actual rect post-layout
  // and reposition rather than guessing a height upfront.
  createEffect(() => {
    const pos = addMenuPos();
    // Track collections so we re-clamp once the async list arrives
    // and grows the menu.
    void addCollections();
    if (!pos) return;
    queueMicrotask(() => {
      if (!addMenuRef) return;
      const r = addMenuRef.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const margin = 8;
      let { x, y, captureId } = pos;
      let changed = false;
      if (r.bottom > vh - margin) {
        y = Math.max(margin, vh - r.height - margin);
        changed = true;
      }
      if (r.right > vw - margin) {
        x = Math.max(margin, vw - r.width - margin);
        changed = true;
      }
      if (changed) setAddMenuPos({ x, y, captureId });
    });
  });

  // Build a RuleUpsertArgs from a captured request — used by the
  // context menu's "Add to <collection>" path. Fetches the full
  // capture (headers) and response body via the regular APIs.
  const buildRuleFromCapture = async (
    captureId: string,
    collectionId: string | null,
  ): Promise<RuleUpsertArgs> => {
    const cap = await api.captures.get(captureId);
    let bodyBase64: string | null = null;
    let bodyMime: string | null = null;
    if (cap.res_body_id) {
      try {
        const body = await api.captures.body(cap.res_body_id, 8 * 1024 * 1024);
        bodyBase64 = body.bytes_base64;
        bodyMime = body.mime;
      } catch {
        /* body fetch failed — leave empty, user can fill in editor */
      }
    }
    // Carry the captured request body into the rule as a JSON match
    // template. The engine deep-subset-matches it against the request
    // body (nested objects/arrays included), so the full structure is
    // preserved — unlike the old top-level-only flattening into params.
    // Pretty-printed so the user can read and prune it (e.g. drop the
    // per-request requestId) in the editor; the editor flags UUIDs.
    let matchReqBody: string | null = null;
    if (cap.req_body_id) {
      try {
        const rb = await api.captures.body(cap.req_body_id, 8 * 1024 * 1024);
        if ((rb.mime ?? "").includes("json")) {
          const text = decodeBodyAsText(rb);
          if (text !== null) {
            try {
              matchReqBody = JSON.stringify(JSON.parse(text), null, 2);
            } catch {
              matchReqBody = text;
            }
          }
        }
      } catch {
        /* body fetch failed — leave it empty, user can fill the editor */
      }
    }
    // Strip query string from the path glob; match_params is the
    // intended slot for query matching, but we don't auto-populate it
    // from the query — wildcard query matches are the more common case,
    // and the user can pin specific params in the Rules editor.
    const qIdx = cap.url_path.indexOf("?");
    const pathGlob = qIdx >= 0 ? cap.url_path.slice(0, qIdx) : cap.url_path;
    // Append the capture's local timestamp so repeated "Add to rules"
    // of the same endpoint produce distinguishable names instead of N
    // identical "POST host/path" rows — the stamp also tells the user
    // which capture each rule came from. Reserve the suffix length so
    // the 120-char cap trims the path, never the timestamp.
    const stamp = captureStamp(cap.started_at);
    const suffix = ` · ${stamp}`;
    const base = `${cap.method} ${cap.server_host}${pathGlob}`;
    return {
      collection_id: collectionId,
      name: `${base.slice(0, 120 - suffix.length)}${suffix}`,
      enabled: true,
      priority: 0,
      mode: "stub",
      patches: [],
      match_method: cap.method || null,
      match_host_glob: cap.server_host || null,
      match_path_glob: pathGlob || null,
      match_params: [],
      match_req_body: matchReqBody,
      res_status: cap.status ?? 200,
      res_headers: (cap.res_headers ?? []).map((h) => ({
        name: h.name,
        value: h.value,
      })),
      res_body_base64: bodyBase64,
      res_body_mime: bodyMime,
      res_delay_ms: 0,
    };
  };

  // Resolve a human-readable collection label for the toast. Looks up
  // by id in the snapshot used to render the menu so we don't have to
  // hit the backend again.
  const collectionLabel = (collectionId: string | null): string => {
    if (collectionId === null) return tr("captures.add_to_rules_ungrouped");
    const c = addCollections().find((x) => x.id === collectionId);
    return c?.name ?? tr("captures.add_to_rules_ungrouped");
  };

  // Shared post-success path: navigate state so the Rules tab opens
  // the editor for this rule, expand the target collection (otherwise
  // the editor would render inside a collapsed section and the user
  // wouldn't see it), and show a toast that names BOTH the rule and
  // its destination collection so it's unambiguous where it landed.
  const finishAdd = (rule: RuleDto, targetCollectionLabel: string) => {
    setRulesEditing({
      kind: "rule",
      collectionId: rule.collection_id,
      id: rule.id,
    });
    const sectionKey = rule.collection_id ?? "__ungrouped__";
    if (rulesCollapsed()[sectionKey]) {
      setRulesCollapsed({ ...rulesCollapsed(), [sectionKey]: false });
    }
    setAddToast(
      tr("captures.add_to_rules_done", {
        name: rule.name,
        collection: targetCollectionLabel,
      }),
    );
    setTimeout(() => setAddToast(null), 3000);
    closeAddMenu();
  };

  // ── Copy as OkHttp-style dump ─────────────────────────────────────
  // Builds a single text blob that contains the full request and
  // response (start line, every header, body) in the same shape
  // OkHttp's HttpLoggingInterceptor emits, so a developer can paste
  // it straight into a bug report alongside their own logs and the
  // two streams are visually consistent.
  //
  // Bodies: we ask the backend for up to 8 MiB. JSON/text/XML/form
  // bodies are pasted as-is; anything that doesn't decode as UTF-8
  // is replaced with a "[N-byte binary body]" placeholder so the
  // user's clipboard doesn't get pages of mojibake.
  const COPY_BODY_LIMIT = 8 * 1024 * 1024;

  const buildHttpDump = async (captureId: string): Promise<string> => {
    const cap = await api.captures.get(captureId);
    const port =
      (cap.scheme === "http" && cap.server_port === 80) ||
      (cap.scheme === "https" && cap.server_port === 443)
        ? ""
        : `:${cap.server_port}`;
    const url = `${cap.scheme}://${cap.server_host}${port}${cap.url_path}`;

    const [reqBody, resBody] = await Promise.all([
      cap.req_body_id ? safeBody(cap.req_body_id) : Promise.resolve(null),
      cap.res_body_id ? safeBody(cap.res_body_id) : Promise.resolve(null),
    ]);

    const lines: string[] = [];
    lines.push(`--> ${cap.method} ${url}`);
    lines.push("");
    for (const h of cap.req_headers ?? []) {
      lines.push(`${h.name}: ${h.value}`);
    }
    if (reqBody && reqBody.text !== null) {
      lines.push("");
      lines.push(reqBody.text);
    } else if (reqBody) {
      lines.push("");
      lines.push(`[${reqBody.total_size}-byte binary body]`);
    }
    lines.push("");
    lines.push(
      reqBody
        ? `--> END ${cap.method} (${reqBody.total_size}-byte body)`
        : `--> END ${cap.method}`,
    );
    lines.push("");

    if (cap.status !== null) {
      const reason = HTTP_REASON[cap.status] ?? "";
      const tail = reason ? ` ${reason}` : "";
      lines.push(`<-- ${cap.status}${tail} ${url}`);
    } else {
      lines.push(`<-- (no response) ${url}`);
    }
    lines.push("");
    for (const h of cap.res_headers ?? []) {
      lines.push(`${h.name}: ${h.value}`);
    }
    if (resBody && resBody.text !== null) {
      lines.push("");
      lines.push(resBody.text);
    } else if (resBody) {
      lines.push("");
      lines.push(`[${resBody.total_size}-byte binary body]`);
    }
    lines.push("");
    lines.push(
      resBody
        ? `<-- END HTTP (${resBody.total_size}-byte body)`
        : `<-- END HTTP`,
    );

    return lines.join("\n");
  };

  const safeBody = async (
    bodyId: string,
  ): Promise<{ text: string | null; total_size: number } | null> => {
    try {
      const body = await api.captures.body(bodyId, COPY_BODY_LIMIT);
      return { text: decodeBodyAsText(body), total_size: body.total_size };
    } catch {
      return null;
    }
  };

  const copyDump = async (captureId: string) => {
    try {
      const text = await buildHttpDump(captureId);
      await writeClipboard(text);
      setAddToast(
        tr("captures.copy_dump_done", { bytes: String(text.length) }),
      );
      setTimeout(() => setAddToast(null), 2500);
      closeAddMenu();
    } catch (e: unknown) {
      setAddToast(
        tr("captures.copy_dump_failed", {
          message: (e as { message?: string })?.message ?? String(e),
        }),
      );
      setTimeout(() => setAddToast(null), 3500);
    }
  };

  const addToCollection = async (collectionId: string | null) => {
    const pos = addMenuPos();
    if (!pos || addBusy()) return;
    setAddBusy(true);
    try {
      const label = collectionLabel(collectionId);
      const args = await buildRuleFromCapture(pos.captureId, collectionId);
      const rule = await api.rules.upsert(args);
      finishAdd(rule, label);
    } catch (e: unknown) {
      alert(
        tr("captures.add_to_rules_failed", {
          message: (e as { message?: string })?.message ?? String(e),
        }),
      );
    } finally {
      setAddBusy(false);
    }
  };

  const addToNewCollection = async () => {
    const pos = addMenuPos();
    if (!pos || addBusy()) return;
    setAddBusy(true);
    try {
      const created = await api.collections.upsert({
        name: tr("captures.add_to_rules_default_collection"),
        enabled: true,
        priority: 0,
      });
      // Inline the rule creation here rather than recursing into
      // `addToCollection`. The recursive call short-circuited on its
      // own `addBusy()` guard, silently leaving an empty collection
      // and no rule — exactly the symptom users reported.
      const args = await buildRuleFromCapture(pos.captureId, created.id);
      const rule = await api.rules.upsert(args);
      finishAdd(rule, created.name);
    } catch (e: unknown) {
      alert(
        tr("captures.add_to_rules_failed", {
          message: (e as { message?: string })?.message ?? String(e),
        }),
      );
    } finally {
      setAddBusy(false);
    }
  };

  // Outside click closes the menu. Mounted alongside the existing
  // document handlers.
  onMount(() => {
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node;
      if (addMenuPos() && addMenuRef && !addMenuRef.contains(t)) {
        closeAddMenu();
      }
      if (headerMenuPos() && headerMenuRef && !headerMenuRef.contains(t)) {
        setHeaderMenuPos(null);
      }
      if (deviceMenuOpen() && deviceMenuRef && !deviceMenuRef.contains(t)) {
        setDeviceMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", onDoc);
    onCleanup(() => document.removeEventListener("mousedown", onDoc));
  });

  const selected = createMemo(() => captures().find((c) => c.id === selectedId()) ?? null);

  const clearAll = async () => {
    if (!confirm(tr("captures.clear_confirm"))) return;
    try {
      await api.captures.clear();
    } catch (e: unknown) {
      const msg = (e as { message?: string })?.message ?? String(e);
      alert(tr("captures.clear_failed", { message: msg }));
      return;
    }
    void refresh(true);
  };

  return (
    <div class="h-full grid grid-rows-[auto_1fr] grid-cols-1">
      <div class="flex items-center gap-2 px-3 py-2 border-b border-border bg-bg-subtle">
        <Search size={14} class="text-fg-muted shrink-0" />
        <div class="flex-1 relative flex items-center">
          <pre
            ref={(el) => (filterOverlay = el)}
            aria-hidden="true"
            class="absolute left-0 right-6 top-0 bottom-0 pointer-events-none text-sm font-mono whitespace-pre overflow-hidden m-0 flex items-center"
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
            placeholder={t()("captures.filter_placeholder")}
            value={filter()}
            onInput={(e) => {
              setFilter(e.currentTarget.value);
              debouncedRefresh(true);
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
            title={t()("captures.filter_help")}
          />
          <Show when={filter().trim()}>
            <button
              type="button"
              class="absolute right-0 bg-bg-subtle text-fg-muted hover:text-warn pl-1 p-0.5 rounded hover:bg-bg-muted"
              title={t()("captures.save_filter_title")}
              aria-label={t()("captures.save_filter")}
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
                  {existingMatch() ? t()("captures.update_filter") : t()("captures.save_filter")}
                </div>
                <div class="font-mono text-fg-muted bg-bg-muted rounded px-2 py-1 truncate">
                  {filter()}
                </div>
                <input
                  type="text"
                  class="w-full px-2 py-1.5 rounded bg-bg-muted outline-none focus:ring-1 focus:ring-accent"
                  placeholder={t()("captures.save_filter_name_placeholder")}
                  value={saveName()}
                  onInput={(e) => setSaveName(e.currentTarget.value)}
                  maxlength={64}
                />
                <Show when={existingMatch()}>
                  <div class="text-fg-muted text-[11px]">
                    {tr("captures.update_filter_hint", { name: existingMatch()!.name })}
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
                          aria-label={tr("captures.color_label", { color: c })}
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
                    <Pin size={11} /> {t()("captures.pin")}
                  </label>
                </div>
                <div class="flex justify-end gap-2 pt-1">
                  <button
                    type="button"
                    class="px-2 py-1 rounded hover:bg-bg-muted text-fg-muted"
                    onClick={() => setSaveOpen(false)}
                  >
                    {t()("captures.cancel")}
                  </button>
                  <button
                    type="submit"
                    class="px-3 py-1 rounded bg-accent text-white hover:opacity-90 disabled:opacity-50"
                    disabled={!saveName().trim() || saveBusy()}
                  >
                    {saveBusy()
                      ? existingMatch()
                        ? t()("captures.updating")
                        : t()("captures.saving")
                      : existingMatch()
                        ? t()("captures.update")
                        : t()("captures.save")}
                  </button>
                </div>
              </form>
            </div>
          </Show>
        </div>
        <Show when={filterError()}>
          <span class="text-xs text-danger">{filterError()}</span>
        </Show>
        <div class="relative" ref={(el) => (deviceMenuRef = el)}>
          <button
            type="button"
            class="text-xs px-2 py-1 rounded bg-bg-muted text-fg outline-none focus:ring-1 focus:ring-accent max-w-[180px] truncate inline-flex items-center gap-1"
            title={t()("captures.column_device")}
            onClick={() => {
              if (!deviceMenuOpen()) void refetchDevices();
              setDeviceMenuOpen((o) => !o);
            }}
          >
            <span class="truncate">{selectedDeviceLabel()}</span>
            <ChevronDown size={12} class="shrink-0 text-fg-muted" />
          </button>
          <Show when={deviceMenuOpen()}>
            <div
              class="absolute right-0 top-full mt-1 z-30 min-w-[180px] max-w-[260px] bg-bg-subtle border border-border rounded shadow-lg py-1 text-xs select-none"
              onMouseDown={(e) => e.stopPropagation()}
            >
              <button
                type="button"
                class={`w-full text-left px-3 py-1.5 hover:bg-bg-muted truncate ${
                  selectedDeviceModel() === "" ? "text-accent" : ""
                }`}
                onClick={() => onPickDevice("")}
              >
                {t()("captures.device_filter_none")}
              </button>
              <button
                type="button"
                class={`w-full text-left px-3 py-1.5 hover:bg-bg-muted truncate ${
                  selectedDeviceModel() === "__host__" ? "text-accent" : ""
                }`}
                onClick={() => void onPickThisComputer()}
              >
                {t()("captures.device_this_computer")}
              </button>
              <Show when={(devices() ?? []).length > 0}>
                <div class="border-t border-border my-1" />
                <For each={devices() ?? []}>
                  {(d) => {
                    const model = deviceModel(d.display_name);
                    return (
                      <button
                        type="button"
                        class={`w-full text-left px-3 py-1.5 hover:bg-bg-muted truncate ${
                          selectedDeviceModel() === model ? "text-accent" : ""
                        }`}
                        title={d.display_name}
                        onClick={() => onPickDevice(model)}
                      >
                        {deviceHead(d.display_name)}
                      </button>
                    );
                  }}
                </For>
              </Show>
            </div>
          </Show>
        </div>
        <button
          class={`text-xs px-2 py-1 rounded inline-flex items-center gap-1 ${
            autoFollow()
              ? "bg-accent/15 text-accent hover:bg-accent/25"
              : "hover:bg-bg-muted text-fg-muted"
          }`}
          onClick={toggleAutoFollow}
          title={autoFollow() ? t()("captures.tail_on_title") : t()("captures.tail_off_title")}
          aria-pressed={autoFollow()}
        >
          <ArrowDownToLine size={12} />{" "}
          {t()("captures.tail_on")}
        </button>
        <button
          class="text-xs px-2 py-1 rounded hover:bg-bg-muted"
          onClick={() => setPaused((p) => !p)}
          title={t()("captures.pause_title")}
        >
          {paused() ? t()("captures.resume") : t()("captures.pause")}
        </button>
        <button
          class="text-xs px-2 py-1 rounded hover:bg-bg-muted text-danger inline-flex items-center gap-1"
          onClick={clearAll}
        >
          <Trash2 size={12} /> {t()("captures.clear")}
        </button>
        <HelpButton path="/filtering/" title={t()("captures.filter_help_title")} class="px-1" />
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
            onContextMenu={openHeaderMenu}
          >
            <For each={COLUMNS}>
              {(col, i) => (
                <Show when={isColVisible(col.key)}>
                  <div class="relative px-2 py-1 overflow-hidden" title={t()(col.labelKey)}>
                    <span class="truncate block">{t()(col.labelKey)}</span>
                    <Show when={i() < lastVisibleColIdx()}>
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
                        title={t()("captures.column_resize_title")}
                      >
                        <div class="absolute right-1/2 top-1 bottom-1 w-px bg-border group-hover:bg-accent group-active:bg-accent" />
                      </div>
                    </Show>
                  </div>
                </Show>
              )}
            </For>
          </div>
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
                {t()("captures.col_menu_title")}
              </div>
              <For each={TOGGLEABLE_COLS}>
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
                    <span>{t()(`captures.column_${key}`)}</span>
                  </label>
                )}
              </For>
            </div>
          </Show>
          <Show
            when={captures().length > 0}
            fallback={
              <div class="px-4 py-8 text-center text-fg-muted">
                <AlertTriangle class="inline mr-1" size={14} />
                {t()("captures.empty_state")}
              </div>
            }
          >
            <div class="relative" style={{ height: `${virtualizer.getTotalSize()}px` }}>
              <For each={virtualizer.getVirtualItems()}>
                {(row) => {
                  const cap = captures()[row.index];
                  return (
                    <div
                      class={`grid absolute left-0 right-0 items-center cursor-pointer border-b border-border/30 ${rowColor(cap.status, cap.error_kind)} ${
                        selectedId() === cap.id ? "bg-bg-muted" : "hover:bg-bg-subtle"
                      }`}
                      style={{
                        transform: `translateY(${row.start}px)`,
                        height: "32px",
                        "grid-template-columns": gridTemplate(),
                      }}
                      onClick={() => setSelectedId(cap.id)}
                      onDblClick={() => navigate(`/replay/${cap.id}`)}
                      onContextMenu={(e) => openAddMenu(e, cap.id)}
                    >
                      <div class="px-2 truncate text-fg-muted">{row.index + 1}</div>
                      <Show when={isColVisible("device")}>
                        <div
                          class="px-2 truncate text-fg-muted"
                          title={deviceName(cap.device_id) ?? cap.device_id ?? t()("captures.device_unknown")}
                        >
                          {shortDeviceName(cap.device_id) ?? t()("captures.device_unknown")}
                        </div>
                      </Show>
                      <Show when={isColVisible("method")}>
                        <div class="px-2 truncate">{cap.method}</div>
                      </Show>
                      <Show when={isColVisible("status")}>
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
                      </Show>
                      <Show when={isColVisible("host")}>
                        <div class="px-2 truncate">{cap.server_host}</div>
                      </Show>
                      <Show when={isColVisible("path")}>
                        <div class="px-2 truncate">{cap.url_path}</div>
                      </Show>
                      <Show when={isColVisible("ms")}>
                        <div class="px-2 truncate text-fg-muted">{cap.duration_ms ?? "—"}</div>
                      </Show>
                      <Show when={isColVisible("bytes")}>
                        <div class="px-2 truncate text-fg-muted">{fmtBytes(cap.total_bytes)}</div>
                      </Show>
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

      {/* Add-to-Rules popover. Positioned at the click coordinates;
          clamped only by the viewport — content is short (~6 lines)
          and right-clicks near the edge are rare. Outside-click and
          Escape close it; menu items dispatch directly. */}
      <Show when={addMenuPos()}>
        <div
          ref={(el) => (addMenuRef = el)}
          class="fixed z-50 bg-bg-subtle border border-border rounded shadow-lg py-1 text-xs select-none"
          style={{
            left: `${addMenuPos()!.x}px`,
            top: `${addMenuPos()!.y}px`,
            "min-width": "220px",
            "max-width": "320px",
          }}
          onMouseDown={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          <button
            type="button"
            class="w-full text-left px-3 py-1.5 hover:bg-bg-muted flex items-center gap-2 disabled:opacity-50"
            disabled={addBusy()}
            title={t()("captures.copy_dump_title")}
            onClick={() => void copyDump(addMenuPos()!.captureId)}
          >
            <Copy size={12} class="text-fg-muted shrink-0" />
            <span class="truncate flex-1">{t()("captures.copy_dump")}</span>
          </button>
          <div class="border-t border-border my-1" />
          <div class="px-3 py-1 text-fg-muted uppercase tracking-wide text-[10px]">
            {t()("captures.add_to_rules_title")}
          </div>
          <Show when={addCollections().length > 0}>
            <For each={addCollections()}>
              {(c) => (
                <button
                  type="button"
                  class="w-full text-left px-3 py-1.5 hover:bg-bg-muted flex items-center gap-2 disabled:opacity-50"
                  disabled={addBusy()}
                  onClick={() => void addToCollection(c.id)}
                >
                  <Shuffle size={12} class="text-accent shrink-0" />
                  <span class="truncate flex-1">{c.name}</span>
                </button>
              )}
            </For>
          </Show>
          <button
            type="button"
            class="w-full text-left px-3 py-1.5 hover:bg-bg-muted flex items-center gap-2 disabled:opacity-50"
            disabled={addBusy()}
            onClick={() => void addToCollection(null)}
          >
            <Shuffle size={12} class="text-fg-muted shrink-0" />
            <span class="truncate flex-1 text-fg-muted">
              {t()("captures.add_to_rules_ungrouped")}
            </span>
          </button>
          <div class="border-t border-border my-1" />
          <button
            type="button"
            class="w-full text-left px-3 py-1.5 hover:bg-bg-muted flex items-center gap-2 disabled:opacity-50"
            disabled={addBusy()}
            onClick={() => void addToNewCollection()}
          >
            <FolderPlus size={12} class="text-accent shrink-0" />
            <span class="truncate flex-1">
              {t()("captures.add_to_rules_new_collection")}
            </span>
          </button>
        </div>
      </Show>

      {/* Add-to-Rules confirmation toast. Bottom-right of the view,
          auto-dismisses via setTimeout in addToCollection. */}
      <Show when={addToast()}>
        <div class="fixed bottom-4 right-4 z-50 bg-bg-subtle border border-accent/40 rounded shadow-lg px-3 py-2 text-xs text-fg max-w-sm">
          {addToast()}
        </div>
      </Show>
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

// Whole-row tint — only failures (5xx and transport errors) light up.
// Everything else stays default so the list reads as quiet baseline
// noise and red rows truly stand out.
function rowColor(status: number | null, errorKind: string | null): string {
  if (errorKind) return "text-danger";
  if (status !== null && status >= 500) return "text-danger";
  return "";
}

function errorHint(errorKind: string | null): string | undefined {
  // Map backend error_kind to a localized hint. tr() reads the active
  // locale; this is called inline as a row's `title=` attribute, which
  // recomputes when SolidJS reconciles, so locale switches do propagate.
  switch (errorKind) {
    case "tls_handshake":
      return tr("captures.error_tls_handshake");
    case "pinning":
      return tr("captures.error_pinning");
    case "upstream":
      return tr("captures.error_upstream");
    case "connect_pipelined":
      return tr("captures.error_connect_pipelined");
    case "connection_refused":
      return tr("captures.error_connection_refused");
    default:
      return errorKind ?? undefined;
  }
}

function fmtBytes(n: number) {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}K`;
  return `${(n / 1024 / 1024).toFixed(1)}M`;
}

// Decode a CaptureBodyDto into a plain string suitable for an
// OkHttp-style dump, or `null` when the bytes don't look like text.
// We try UTF-8 with `fatal: true` first — if the body is genuinely
// binary the decode throws and we surface a "[N-byte binary body]"
// placeholder upstream instead of pasting pages of replacement chars
// into the user's clipboard. Truncated bodies (server > COPY_BODY_LIMIT)
// get a trailing note so the user knows they're not seeing the full
// payload.
// Compact local timestamp (MM-DD HH:MM:SS) for disambiguating
// auto-generated rule names. Local time so it matches what the captures
// list shows; seconds included so two captures a moment apart still
// differ. Falls back to the raw string if started_at doesn't parse.
function captureStamp(startedAt: string): string {
  // `started_at` may be ISO ("...T..Z") or the Rust OffsetDateTime Display
  // form ("2026-06-22 6:38:38.0 +00:00:00"), which `new Date()` can't
  // parse — normalise that to ISO first so we get a real local time
  // instead of falling back to the raw, seconds-and-timezone-noisy string.
  let d = new Date(startedAt);
  if (Number.isNaN(d.getTime())) {
    const m = startedAt.match(
      /(\d{4})-(\d{2})-(\d{2})[ T](\d{1,2}):(\d{2}):(\d{2})(?:\.\d+)?\s*([+-]\d{2}):?(\d{2})/,
    );
    if (m) {
      const [, y, mo, da, h, mi, s, offh, offm] = m;
      d = new Date(`${y}-${mo}-${da}T${h.padStart(2, "0")}:${mi}:${s}${offh}:${offm}`);
    }
  }
  if (Number.isNaN(d.getTime())) return startedAt;
  const p = (n: number) => String(n).padStart(2, "0");
  // Local YYYY-MM-DD HH:MM — drop seconds and timezone (just disambiguates
  // captures of the same endpoint; minute precision is plenty).
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

function decodeBodyAsText(body: {
  mime: string | null;
  bytes_base64: string;
  truncated: boolean;
  total_size: number;
}): string | null {
  if (!body.bytes_base64) return body.truncated ? "[body truncated]" : "";
  let bytes: Uint8Array;
  try {
    const bin = atob(body.bytes_base64);
    bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  } catch {
    return null;
  }
  try {
    const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    return body.truncated
      ? `${text}\n[body truncated to ${bytes.length} of ${body.total_size} bytes]`
      : text;
  } catch {
    return null;
  }
}

// Minimal HTTP reason-phrase map. Covers the codes a developer is
// realistically going to see in captured app traffic — anything
// else falls back to an empty string and the dump shows just the
// numeric status. We don't pull the full IANA list because the
// dump is a debugging aid, not an RFC artefact.
const HTTP_REASON: Record<number, string> = {
  100: "Continue",
  101: "Switching Protocols",
  200: "OK",
  201: "Created",
  202: "Accepted",
  204: "No Content",
  206: "Partial Content",
  301: "Moved Permanently",
  302: "Found",
  303: "See Other",
  304: "Not Modified",
  307: "Temporary Redirect",
  308: "Permanent Redirect",
  400: "Bad Request",
  401: "Unauthorized",
  403: "Forbidden",
  404: "Not Found",
  405: "Method Not Allowed",
  409: "Conflict",
  410: "Gone",
  413: "Payload Too Large",
  415: "Unsupported Media Type",
  422: "Unprocessable Entity",
  429: "Too Many Requests",
  500: "Internal Server Error",
  501: "Not Implemented",
  502: "Bad Gateway",
  503: "Service Unavailable",
  504: "Gateway Timeout",
};

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
  "device",
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
