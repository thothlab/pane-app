import { type Component, createSignal, createMemo, createEffect, Show, For } from "solid-js";
import { Lock, ChevronDown, ChevronRight, Copy, Check } from "lucide-solid";
import { useNavigate } from "@solidjs/router";
import { api } from "@/ipc/client";
import type { CaptureBodyDto, CaptureDto } from "@/ipc/types";
import BodyViewer from "./BodyViewer";
import { HorizontalResizer } from "./HorizontalResizer";
import { writeClipboard } from "@/lib/clipboard";
import { t } from "@/i18n";

type Tab = "overview" | "request" | "response" | "timing" | "tls";

// Tab label keys — keeps the i18n key set discoverable by grep.
const TAB_LABELS: Record<Tab, "detail.overview" | "detail.request" | "detail.response" | "detail.timing" | "detail.tls"> = {
  overview: "detail.overview",
  request: "detail.request",
  response: "detail.response",
  timing: "detail.timing",
  tls: "detail.tls",
};

const DetailPanes: Component<{ capture: CaptureDto | null }> = (props) => {
  const navigate = useNavigate();
  const [tab, setTab] = createSignal<Tab>("overview");
  const [full, setFull] = createSignal<CaptureDto | null>(null);
  const [body, setBody] = createSignal<CaptureBodyDto | null>(null);

  const DEFAULT_BODY_LIMIT = 4 * 1024 * 1024; // 4 MB

  // Load the full capture (headers + body ids) whenever the selection
  // changes. Kept separate from the body load below so each effect tracks
  // exactly one concern synchronously.
  let fullGen = 0;
  createEffect(() => {
    const c = props.capture;
    if (!c) {
      setFull(null);
      return;
    }
    const gen = ++fullGen;
    void api.captures.get(c.id).then((f) => {
      if (gen === fullGen) setFull(f);
    });
  });

  // (Re)load the body for the ACTIVE tab. Both full() and tab() are read
  // synchronously so Solid tracks them — previously tab() was read only
  // after an `await` inside a single async effect, so it was never a
  // dependency and the body pane kept showing whichever tab was active when
  // the row was selected (e.g. the 60B request body under the Response tab,
  // even though the response's content-length was 17 KB). The generation
  // guard drops out-of-order async results from rapid tab/row switching.
  let bodyGen = 0;
  createEffect(() => {
    const f = full();
    const activeTab = tab();
    const bodyId =
      activeTab === "response" ? f?.res_body_id
      : activeTab === "request" ? f?.req_body_id
      : null;
    if (!f || !bodyId) {
      setBody(null);
      return;
    }
    const gen = ++bodyGen;
    void api.captures.body(bodyId, DEFAULT_BODY_LIMIT).then((b) => {
      if (gen === bodyGen) setBody(b);
    });
  });

  const loadFullBody = async () => {
    const f = full();
    if (!f) return;
    const id = tab() === "request" ? f.req_body_id : f.res_body_id;
    if (!id) return;
    setBody(await api.captures.body(id));
  };

  const isPinning = createMemo(() => full()?.error_kind === "pinning");

  return (
    <Show when={full()} fallback={<EmptyDetail />}>
      <div class="h-full grid grid-rows-[auto_1fr]">
        <div class="border-b border-border flex items-center px-2 bg-bg-subtle">
          <For each={["overview", "request", "response", "timing", "tls"] as Tab[]}>
            {(tabKey) => (
              <button
                class={`px-3 py-2 text-xs uppercase tracking-wide ${
                  tab() === tabKey ? "text-accent border-b-2 border-accent" : "text-fg-muted hover:text-fg"
                }`}
                onClick={() => setTab(tabKey)}
              >
                {t()(TAB_LABELS[tabKey])}
              </button>
            )}
          </For>
          <div class="ml-auto flex gap-1">
            <button
              class="text-xs px-2 py-1 rounded hover:bg-bg-muted"
              onClick={() => navigate(`/replay/${full()!.id}`)}
            >
              {t()("detail.replay")}
            </button>
            <button
              class="text-xs px-2 py-1 rounded hover:bg-bg-muted"
              onClick={async () => {
                const r = await api.captures.exportOne(full()!.id, "curl");
                await writeClipboard(r.text);
              }}
            >
              {t()("detail.curl")}
            </button>
          </div>
        </div>

        <div class="min-h-0 font-mono text-xs flex flex-col">
          <Show when={isPinning()}>
            <div class="mx-3 mt-3 mb-0 p-3 rounded border border-warn/40 bg-warn/10 text-warn">
              <div class="flex items-center gap-2 font-semibold">
                <Lock size={14} /> {t()("detail.pinning_detected")}
              </div>
              <p class="text-fg-subtle mt-1">
                {t()("detail.pinning_body", { host: full()?.server_host ?? "" })}{" "}
                <a class="underline" href="/about">{t()("detail.learn_more")}</a>
              </p>
            </div>
          </Show>

          <Show when={tab() === "overview"}>
            <div class="overflow-auto p-3 flex-1 min-h-0">
              <Row k={t()("detail.id")}>{full()!.id}</Row>
              <Row k={t()("detail.method")}>{full()!.method}</Row>
              <Row k={t()("detail.url")}>{`${full()!.scheme}://${full()!.server_host}:${full()!.server_port}${full()!.url_path}`}</Row>
              <Row k={t()("detail.status")}>{full()!.status ?? "—"}</Row>
              <Row k="HTTP">{full()!.http_version}</Row>
              <Row k={t()("detail.state")}>{full()!.state}</Row>
              <Row k={t()("detail.error")}>{full()!.error_kind ?? "—"}</Row>
              <Row k={t()("detail.started")}>{full()!.started_at}</Row>
              <Row k={t()("detail.duration")}>{full()!.duration_ms ?? "—"} ms</Row>
              <Row k={t()("detail.size")}>{full()!.total_bytes} B</Row>
            </div>
          </Show>

          <Show when={tab() === "request"}>
            <HeadersBodySplit
              kind="request"
              headers={full()!.req_headers ?? []}
              body={body()}
              onLoadFull={loadFullBody}
            />
          </Show>

          <Show when={tab() === "response"}>
            <HeadersBodySplit
              kind="response"
              headers={full()!.res_headers ?? []}
              body={body()}
              onLoadFull={loadFullBody}
            />
          </Show>

          <Show when={tab() === "timing"}>
            <div class="overflow-auto p-3 flex-1 min-h-0">
              <TimingWaterfall capture={full()!} />
            </div>
          </Show>

          <Show when={tab() === "tls"}>
            <div class="overflow-auto p-3 flex-1 min-h-0">
              <Row k="SNI">{full()!.server_host}</Row>
              <Row k={t()("detail.version")}>{full()!.http_version}</Row>
              <p class="text-fg-muted mt-3">{t()("detail.tls_note")}</p>
            </div>
          </Show>
        </div>
      </div>
    </Show>
  );
};

const EmptyDetail: Component = () => (
  <div class="h-full flex items-center justify-center text-fg-muted text-sm">
    {t()("detail.select_capture")}
  </div>
);

const Row: Component<{ k: string; children: any }> = (p) => (
  <div class="flex gap-2 py-0.5">
    <div class="w-20 text-fg-muted">{p.k}</div>
    <div class="flex-1 break-all">{p.children}</div>
  </div>
);

const HeadersList: Component<{ headers: { name: string; value: string }[] }> = (p) => (
  <div class="mb-3">
    <For each={p.headers}>{(h) => <HeaderRow header={h} />}</For>
  </div>
);

const HeaderRow: Component<{ header: { name: string; value: string } }> = (p) => {
  const [copied, setCopied] = createSignal<"name" | "value" | "pair" | null>(null);
  const flash = (k: "name" | "value" | "pair") => {
    setCopied(k);
    setTimeout(() => setCopied(null), 900);
  };
  const copyName = (e: MouseEvent) => {
    e.stopPropagation();
    void writeClipboard(p.header.name);
    flash("name");
  };
  const copyValue = (e: MouseEvent) => {
    e.stopPropagation();
    void writeClipboard(p.header.value);
    flash("value");
  };
  const copyPair = (e: MouseEvent) => {
    e.stopPropagation();
    void writeClipboard(`${p.header.name}: ${p.header.value}`);
    flash("pair");
  };
  return (
    <div class="group flex items-start gap-2 py-0.5 hover:bg-bg-subtle rounded px-1 -mx-1">
      <button
        class="text-accent hover:underline text-left shrink-0"
        title={t()("detail.copy_header_name")}
        onClick={copyName}
      >
        {p.header.name}
      </button>
      <button
        class="text-fg-subtle break-all hover:underline text-left flex-1 min-w-0"
        title={t()("detail.copy_header_value")}
        onClick={copyValue}
      >
        {p.header.value}
      </button>
      <button
        class="opacity-0 group-hover:opacity-100 text-fg-muted hover:text-fg shrink-0 p-0.5 rounded"
        title={t()("detail.copy_header_pair")}
        onClick={copyPair}
      >
        <Show when={copied() !== null} fallback={<Copy size={11} />}>
          <Check size={11} class="text-success" />
        </Show>
      </button>
    </div>
  );
};

/// Keyed localStorage prefix for per-pane Headers/Body split height.
/// One entry per pane kind ("request" / "response") so the user's
/// chosen split sticks per pane across captures and app restarts.
const HEADERS_HEIGHT_KEY_PREFIX = "pane.detail.headers-height";
const DEFAULT_HEADERS_HEIGHT_PX = 220;

const HeadersBodySplit: Component<{
  kind: "request" | "response";
  headers: { name: string; value: string }[];
  body: CaptureBodyDto | null;
  onLoadFull: () => void;
}> = (p) => {
  const [headersCollapsed, setHeadersCollapsed] = createSignal(false);
  // Headers area height in px, persisted per `kind`. Single localStorage
  // read on mount; subsequent updates write through on every change so
  // a drag-resize survives reload + restart.
  const storageKey = `${HEADERS_HEIGHT_KEY_PREFIX}.${p.kind}`;
  const readStoredHeight = (): number => {
    const raw = (() => {
      try {
        return localStorage.getItem(storageKey);
      } catch {
        return null; // safari private mode / SSR / etc.
      }
    })();
    const n = raw === null ? NaN : parseInt(raw, 10);
    return Number.isFinite(n) && n > 0 ? n : DEFAULT_HEADERS_HEIGHT_PX;
  };
  const writeStoredHeight = (v: number) => {
    try {
      localStorage.setItem(storageKey, String(v));
    } catch {
      // swallow — storage unavailable, just lose persistence
    }
  };
  const [headersHeight, setHeadersHeightRaw] = createSignal(readStoredHeight());
  const setHeadersHeight = (next: number | ((prev: number) => number)) => {
    const v = typeof next === "function" ? next(headersHeight()) : next;
    writeStoredHeight(v);
    setHeadersHeightRaw(v);
  };

  const COLLAPSED_PX = 28;
  const MIN_HEADERS_PX = 60;
  const MIN_BODY_PX = 100;
  let containerRef: HTMLDivElement | undefined;

  const resize = (delta: number) => {
    if (headersCollapsed()) return;
    const total = containerRef?.clientHeight ?? 600;
    const max = total - MIN_BODY_PX;
    setHeadersHeight((h) => Math.max(MIN_HEADERS_PX, Math.min(max, h + delta)));
  };

  const topRowSize = () => (headersCollapsed() ? `${COLLAPSED_PX}px` : `${headersHeight()}px`);

  return (
    <div
      ref={containerRef}
      class="grid flex-1 min-h-0"
      style={{ "grid-template-rows": `${topRowSize()} auto 1fr` }}
    >
      <div class="min-h-0 flex flex-col">
        <div class="flex items-center gap-2 px-3 py-1 bg-bg-subtle/60 text-fg-muted">
          <button
            class="flex items-center gap-1 hover:text-fg"
            onClick={() => setHeadersCollapsed(!headersCollapsed())}
            title={headersCollapsed() ? t()("detail.expand_headers") : t()("detail.collapse_headers")}
          >
            <Show when={headersCollapsed()} fallback={<ChevronDown size={12} />}>
              <ChevronRight size={12} />
            </Show>
            {t()("detail.headers")} <span class="text-fg-muted/70">({p.headers.length})</span>
          </button>
        </div>
        <Show when={!headersCollapsed()}>
          <div class="overflow-auto px-3 py-1">
            <HeadersList headers={p.headers} />
          </div>
        </Show>
      </div>

      <Show
        when={!headersCollapsed()}
        fallback={<div />}
      >
        <HorizontalResizer
          onResize={resize}
          onReset={() => setHeadersHeight(DEFAULT_HEADERS_HEIGHT_PX)}
        />
      </Show>

      {/*
        No padding here — the BodyViewer owns its horizontal padding
        so its sticky header bar can pin flush to the scroll port's
        top edge without a visible JSON-text strip leaking through
        any padding-top region above it.
      */}
      <div class="min-h-0 overflow-auto">
        <Show
          when={p.body}
          fallback={<div class="text-fg-muted italic px-3 py-2">{t()("detail.no_body")}</div>}
        >
          <BodyViewer body={p.body!} onLoadFull={p.onLoadFull} />
        </Show>
      </div>
    </div>
  );
};

const TimingWaterfall: Component<{ capture: CaptureDto }> = (p) => {
  const total = p.capture.duration_ms ?? 0;
  return (
    <div>
      <Row k={t()("detail.total")}>{total} ms</Row>
      <div class="mt-3 h-3 bg-bg-muted rounded overflow-hidden">
        <div class="h-full bg-accent" style={{ width: "100%" }} />
      </div>
      <p class="text-fg-muted mt-2 text-xs">{t()("detail.timing_note")}</p>
    </div>
  );
};

export default DetailPanes;
