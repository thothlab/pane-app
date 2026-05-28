import { type Component, createResource, createSignal, For, Show, onMount } from "solid-js";
import { useParams, useNavigate } from "@solidjs/router";
import { Play, Plus, Trash2 } from "lucide-solid";
import { api } from "@/ipc/client";
import type { HeaderDto } from "@/ipc/types";

const ReplayView: Component = () => {
  const params = useParams<{ id: string }>();
  const navigate = useNavigate();
  const [source] = createResource(() => params.id, (id) => api.captures.get(id));

  const [method, setMethod] = createSignal("GET");
  const [url, setUrl] = createSignal("");
  const [headers, setHeaders] = createSignal<HeaderDto[]>([]);
  const [body, setBody] = createSignal("");
  const [result, setResult] = createSignal<string | null>(null);
  const [sending, setSending] = createSignal(false);

  onMount(async () => {
    const s = await api.captures.get(params.id);
    setMethod(s.method);
    setUrl(`${s.scheme}://${s.server_host}:${s.server_port}${s.url_path}`);
    setHeaders(s.req_headers ?? []);
    if (s.req_body_id) {
      const b = await api.captures.body(s.req_body_id, 65536);
      try {
        setBody(atob(b.bytes_base64));
      } catch {
        /* binary — leave empty */
      }
    }
  });

  const send = async () => {
    setSending(true);
    setResult(null);
    try {
      const r = await api.replay.send(
        {
          method: method(),
          url: url(),
          headers: headers(),
          body_text: body() || undefined,
        },
        params.id,
      );
      setResult(`Sent. New capture: ${r.result_capture_id}`);
    } catch (e: any) {
      setResult(`Error: ${e?.message ?? e}`);
    } finally {
      setSending(false);
    }
  };

  return (
    <div class="h-full overflow-auto p-6 space-y-4 max-w-4xl">
      <header class="flex items-center justify-between">
        <h1 class="text-xl font-semibold">Replay</h1>
        <button class="text-xs text-fg-muted" onClick={() => navigate("/")}>← back to captures</button>
      </header>

      <Show when={source()}>
        <div class="text-xs text-fg-muted">
          Source: <span class="font-mono">{source()!.method} {source()!.server_host}{source()!.url_path}</span>
        </div>
      </Show>

      <div class="flex gap-2">
        <select
          class="bg-bg-subtle border border-border rounded px-2 py-1.5 text-sm"
          value={method()}
          onChange={(e) => setMethod(e.currentTarget.value)}
        >
          <For each={["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]}>
            {(m) => <option value={m}>{m}</option>}
          </For>
        </select>
        <input
          class="flex-1 bg-bg-subtle border border-border rounded px-2 py-1.5 text-sm font-mono"
          value={url()}
          onInput={(e) => setUrl(e.currentTarget.value)}
          placeholder="https://api.example.com/path"
        />
        <button
          class="bg-accent text-white rounded px-3 py-1.5 text-sm font-medium inline-flex items-center gap-1 disabled:opacity-50"
          onClick={send}
          disabled={sending()}
        >
          <Play size={14} /> {sending() ? "Sending…" : "Send"}
        </button>
      </div>

      <section>
        <div class="flex items-center justify-between mb-2">
          <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">Headers</h2>
          <button
            class="text-xs px-2 py-1 rounded hover:bg-bg-muted inline-flex items-center gap-1"
            onClick={() => setHeaders([...headers(), { name: "", value: "" }])}
          >
            <Plus size={12} /> Add
          </button>
        </div>
        <div class="space-y-1">
          <For each={headers()}>
            {(h, i) => (
              <div class="flex gap-2">
                <input
                  class="flex-1 bg-bg-subtle border border-border rounded px-2 py-1 text-xs font-mono"
                  placeholder="Name"
                  value={h.name}
                  onInput={(e) => {
                    const copy = [...headers()];
                    copy[i()] = { ...h, name: e.currentTarget.value };
                    setHeaders(copy);
                  }}
                />
                <input
                  class="flex-[2] bg-bg-subtle border border-border rounded px-2 py-1 text-xs font-mono"
                  placeholder="Value"
                  value={h.value}
                  onInput={(e) => {
                    const copy = [...headers()];
                    copy[i()] = { ...h, value: e.currentTarget.value };
                    setHeaders(copy);
                  }}
                />
                <button
                  class="text-xs text-danger px-2"
                  onClick={() => setHeaders(headers().filter((_, idx) => idx !== i()))}
                >
                  <Trash2 size={12} />
                </button>
              </div>
            )}
          </For>
        </div>
      </section>

      <section>
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle mb-2">Body</h2>
        <textarea
          class="w-full h-48 bg-bg-subtle border border-border rounded p-2 text-xs font-mono"
          value={body()}
          onInput={(e) => setBody(e.currentTarget.value)}
          placeholder='{"hello": "world"}'
        />
      </section>

      <Show when={result()}>
        <div class="text-sm font-mono bg-bg-subtle border border-border rounded p-2">{result()}</div>
      </Show>
    </div>
  );
};

export default ReplayView;
