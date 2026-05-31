import { type Component, createSignal, createMemo, createEffect, Show, For } from "solid-js";
import { Lock, ChevronDown, ChevronRight, Copy, Check } from "lucide-solid";
import { useNavigate } from "@solidjs/router";
import { api } from "@/ipc/client";
import type { CaptureBodyDto, CaptureDto } from "@/ipc/types";
import BodyViewer from "./BodyViewer";
import { HorizontalResizer } from "./HorizontalResizer";

type Tab = "overview" | "request" | "response" | "timing" | "tls";

const DetailPanes: Component<{ capture: CaptureDto | null }> = (props) => {
  const navigate = useNavigate();
  const [tab, setTab] = createSignal<Tab>("overview");
  const [full, setFull] = createSignal<CaptureDto | null>(null);
  const [body, setBody] = createSignal<CaptureBodyDto | null>(null);

  const DEFAULT_BODY_LIMIT = 4 * 1024 * 1024; // 4 MB

  createEffect(async () => {
    const c = props.capture;
    if (!c) {
      setFull(null);
      setBody(null);
      return;
    }
    const f = await api.captures.get(c.id);
    setFull(f);
    if (tab() === "response" && f.res_body_id) {
      setBody(await api.captures.body(f.res_body_id, DEFAULT_BODY_LIMIT));
    } else if (tab() === "request" && f.req_body_id) {
      setBody(await api.captures.body(f.req_body_id, DEFAULT_BODY_LIMIT));
    } else {
      setBody(null);
    }
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
            {(t) => (
              <button
                class={`px-3 py-2 text-xs uppercase tracking-wide ${
                  tab() === t ? "text-accent border-b-2 border-accent" : "text-fg-muted hover:text-fg"
                }`}
                onClick={() => setTab(t)}
              >
                {t}
              </button>
            )}
          </For>
          <div class="ml-auto flex gap-1">
            <button
              class="text-xs px-2 py-1 rounded hover:bg-bg-muted"
              onClick={() => navigate(`/replay/${full()!.id}`)}
            >
              Replay
            </button>
            <button
              class="text-xs px-2 py-1 rounded hover:bg-bg-muted"
              onClick={async () => {
                const r = await api.captures.exportOne(full()!.id, "curl");
                navigator.clipboard.writeText(r.text);
              }}
            >
              cURL
            </button>
          </div>
        </div>

        <div class="min-h-0 font-mono text-xs flex flex-col">
          <Show when={isPinning()}>
            <div class="mx-3 mt-3 mb-0 p-3 rounded border border-warn/40 bg-warn/10 text-warn">
              <div class="flex items-center gap-2 font-semibold">
                <Lock size={14} /> Cert pinning detected
              </div>
              <p class="text-fg-subtle mt-1">
                {full()?.server_host} uses certificate pinning. Inspection isn't possible without
                bypassing it on the device (e.g. Frida). For your own apps, disable pinning in the
                debug build. <a class="underline" href="/about">Learn more</a>
              </p>
            </div>
          </Show>

          <Show when={tab() === "overview"}>
            <div class="overflow-auto p-3 flex-1 min-h-0">
              <Row k="ID">{full()!.id}</Row>
              <Row k="Method">{full()!.method}</Row>
              <Row k="URL">{`${full()!.scheme}://${full()!.server_host}:${full()!.server_port}${full()!.url_path}`}</Row>
              <Row k="Status">{full()!.status ?? "—"}</Row>
              <Row k="HTTP">{full()!.http_version}</Row>
              <Row k="State">{full()!.state}</Row>
              <Row k="Error">{full()!.error_kind ?? "—"}</Row>
              <Row k="Started">{full()!.started_at}</Row>
              <Row k="Duration">{full()!.duration_ms ?? "—"} ms</Row>
              <Row k="Size">{full()!.total_bytes} B</Row>
            </div>
          </Show>

          <Show when={tab() === "request"}>
            <HeadersBodySplit
              headers={full()!.req_headers ?? []}
              body={body()}
              onLoadFull={loadFullBody}
            />
          </Show>

          <Show when={tab() === "response"}>
            <HeadersBodySplit
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
              <Row k="Version">{full()!.http_version}</Row>
              <p class="text-fg-muted mt-3">
                Detailed TLS information requires the engine's decrypted-TLS path.
                CONNECT tunnels capture host metadata only.
              </p>
            </div>
          </Show>
        </div>
      </div>
    </Show>
  );
};

const EmptyDetail: Component = () => (
  <div class="h-full flex items-center justify-center text-fg-muted text-sm">
    Select a capture to see details.
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
    navigator.clipboard.writeText(p.header.name);
    flash("name");
  };
  const copyValue = (e: MouseEvent) => {
    e.stopPropagation();
    navigator.clipboard.writeText(p.header.value);
    flash("value");
  };
  const copyPair = (e: MouseEvent) => {
    e.stopPropagation();
    navigator.clipboard.writeText(`${p.header.name}: ${p.header.value}`);
    flash("pair");
  };
  return (
    <div class="group flex gap-2 py-0.5 hover:bg-bg-subtle rounded px-1 -mx-1">
      <button
        class="text-accent hover:underline text-left shrink-0"
        title="Copy header name"
        onClick={copyName}
      >
        {p.header.name}
      </button>
      <button
        class="text-fg-subtle break-all hover:underline text-left flex-1 min-w-0"
        title="Copy header value"
        onClick={copyValue}
      >
        {p.header.value}
      </button>
      <button
        class="opacity-0 group-hover:opacity-100 text-fg-muted hover:text-fg shrink-0 p-0.5 rounded"
        title='Copy "name: value"'
        onClick={copyPair}
      >
        <Show when={copied() !== null} fallback={<Copy size={11} />}>
          <Check size={11} class="text-success" />
        </Show>
      </button>
    </div>
  );
};

const HeadersBodySplit: Component<{
  headers: { name: string; value: string }[];
  body: CaptureBodyDto | null;
  onLoadFull: () => void;
}> = (p) => {
  const [headersCollapsed, setHeadersCollapsed] = createSignal(false);
  // Headers area height in px. Default to ~40% of the surrounding pane.
  const [headersHeight, setHeadersHeight] = createSignal(220);
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
            title={headersCollapsed() ? "Expand headers" : "Collapse headers"}
          >
            <Show when={headersCollapsed()} fallback={<ChevronDown size={12} />}>
              <ChevronRight size={12} />
            </Show>
            Headers <span class="text-fg-muted/70">({p.headers.length})</span>
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
        <HorizontalResizer onResize={resize} onReset={() => setHeadersHeight(220)} />
      </Show>

      <div class="min-h-0 overflow-auto px-3 py-2">
        <Show
          when={p.body}
          fallback={<div class="text-fg-muted italic">No body</div>}
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
      <Row k="Total">{total} ms</Row>
      <div class="mt-3 h-3 bg-bg-muted rounded overflow-hidden">
        <div class="h-full bg-accent" style={{ width: "100%" }} />
      </div>
      <p class="text-fg-muted mt-2 text-xs">
        Per-phase breakdown (DNS/connect/TLS/send/wait/receive) is wired through the engine's
        timing events; populated once the decrypted-TLS path lands.
      </p>
    </div>
  );
};

export default DetailPanes;
