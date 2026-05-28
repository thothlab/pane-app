import { type Component, createSignal, createMemo, createEffect, Show, For } from "solid-js";
import { Lock } from "lucide-solid";
import { useNavigate } from "@solidjs/router";
import { api } from "@/ipc/client";
import type { CaptureBodyDto, CaptureDto } from "@/ipc/types";

type Tab = "overview" | "request" | "response" | "timing" | "tls";

const DetailPanes: Component<{ capture: CaptureDto | null }> = (props) => {
  const navigate = useNavigate();
  const [tab, setTab] = createSignal<Tab>("overview");
  const [full, setFull] = createSignal<CaptureDto | null>(null);
  const [body, setBody] = createSignal<CaptureBodyDto | null>(null);

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
      setBody(await api.captures.body(f.res_body_id, 262144));
    } else if (tab() === "request" && f.req_body_id) {
      setBody(await api.captures.body(f.req_body_id, 262144));
    } else {
      setBody(null);
    }
  });

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

        <div class="overflow-auto p-3 font-mono text-xs">
          <Show when={isPinning()}>
            <div class="mb-3 p-3 rounded border border-warn/40 bg-warn/10 text-warn">
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
          </Show>

          <Show when={tab() === "request"}>
            <HeadersList headers={full()!.req_headers ?? []} />
            <Show when={body()}>
              <BodyView body={body()!} />
            </Show>
          </Show>

          <Show when={tab() === "response"}>
            <HeadersList headers={full()!.res_headers ?? []} />
            <Show when={body()}>
              <BodyView body={body()!} />
            </Show>
          </Show>

          <Show when={tab() === "timing"}>
            <TimingWaterfall capture={full()!} />
          </Show>

          <Show when={tab() === "tls"}>
            <Row k="SNI">{full()!.server_host}</Row>
            <Row k="Version">{full()!.http_version}</Row>
            <p class="text-fg-muted mt-3">
              Detailed TLS information requires the engine's decrypted-TLS path.
              CONNECT tunnels capture host metadata only.
            </p>
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
    <For each={p.headers}>
      {(h) => (
        <div
          class="flex gap-2 py-0.5 cursor-pointer hover:bg-bg-subtle"
          onClick={() => navigator.clipboard.writeText(`${h.name}: ${h.value}`)}
        >
          <div class="text-accent">{h.name}</div>
          <div class="text-fg-subtle break-all">{h.value}</div>
        </div>
      )}
    </For>
  </div>
);

const BodyView: Component<{ body: CaptureBodyDto }> = (p) => {
  const text = createMemo(() => {
    try {
      return atob(p.body.bytes_base64);
    } catch {
      return "<binary>";
    }
  });
  const pretty = createMemo(() => {
    if (p.body.mime?.includes("json")) {
      try {
        return JSON.stringify(JSON.parse(text()), null, 2);
      } catch {
        return text();
      }
    }
    return text();
  });
  return (
    <div class="border-t border-border mt-2 pt-2">
      <div class="text-fg-muted mb-1">
        Body · {p.body.mime ?? "?"} · {p.body.total_size}B
        <Show when={p.body.truncated}> · truncated</Show>
      </div>
      <pre class="whitespace-pre-wrap break-all">{pretty()}</pre>
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
