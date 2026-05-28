import { type Component, createSignal, createMemo, onMount, onCleanup, For, Show } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { Search, Trash2, AlertTriangle, Lock } from "lucide-solid";
import { api } from "@/ipc/client";
import { listenToCaptures } from "@/ipc/events";
import type { CaptureDto } from "@/ipc/types";
import DetailPanes from "@/components/DetailPanes";

const CapturesView: Component = () => {
  const navigate = useNavigate();
  const [captures, setCaptures] = createSignal<CaptureDto[]>([]);
  const [filter, setFilter] = createSignal("");
  const [filterError, setFilterError] = createSignal<string | null>(null);
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [paused, setPaused] = createSignal(false);
  let scrollEl: HTMLDivElement | undefined;

  const refresh = async () => {
    if (paused()) return;
    try {
      setCaptures(await api.captures.list(filter() || undefined, 500));
      setFilterError(null);
    } catch (e: any) {
      setFilterError(e?.message ?? "filter error");
    }
  };

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
    await api.captures.clear();
    refresh();
  };

  return (
    <div class="h-full grid grid-rows-[auto_1fr] grid-cols-1">
      <div class="flex items-center gap-2 px-3 py-2 border-b border-border bg-bg-subtle">
        <Search size={14} class="text-fg-muted" />
        <input
          class="flex-1 bg-transparent outline-none text-sm placeholder:text-fg-muted font-mono"
          placeholder="host:api.example.com status:5.. !host:cdn.*"
          value={filter()}
          onInput={(e) => {
            setFilter(e.currentTarget.value);
            debouncedRefresh();
          }}
        />
        <Show when={filterError()}>
          <span class="text-xs text-danger">{filterError()}</span>
        </Show>
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
      </div>

      <div class="grid grid-cols-[1.4fr_1fr] h-full overflow-hidden">
        <div ref={(el) => (scrollEl = el)} class="overflow-auto border-r border-border">
          <table class="w-full text-xs font-mono">
            <thead class="sticky top-0 bg-bg-subtle">
              <tr class="text-left text-fg-muted">
                <th class="px-2 py-1 w-10">#</th>
                <th class="px-2 py-1 w-12">M</th>
                <th class="px-2 py-1 w-12">St</th>
                <th class="px-2 py-1">Host</th>
                <th class="px-2 py-1">Path</th>
                <th class="px-2 py-1 w-16">ms</th>
                <th class="px-2 py-1 w-16">bytes</th>
              </tr>
            </thead>
            <tbody style={{ height: `${virtualizer().getTotalSize()}px`, position: "relative" }}>
              <For each={virtualizer().getVirtualItems()}>
                {(row) => {
                  const cap = captures()[row.index];
                  return (
                    <tr
                      class={`absolute left-0 right-0 cursor-pointer ${
                        selectedId() === cap.id ? "bg-bg-muted" : "hover:bg-bg-subtle"
                      }`}
                      style={{ transform: `translateY(${row.start}px)`, height: "32px" }}
                      onClick={() => setSelectedId(cap.id)}
                      onDblClick={() => navigate(`/replay/${cap.id}`)}
                    >
                      <td class="px-2 py-1 text-fg-muted">{row.index + 1}</td>
                      <td class="px-2 py-1">{cap.method}</td>
                      <td class={`px-2 py-1 ${statusColor(cap.status, cap.error_kind)}`}>
                        {cap.error_kind === "pinning" ? <Lock size={12} /> : cap.status ?? "—"}
                      </td>
                      <td class="px-2 py-1 truncate">{cap.server_host}</td>
                      <td class="px-2 py-1 truncate">{cap.url_path}</td>
                      <td class="px-2 py-1 text-fg-muted">{cap.duration_ms ?? "—"}</td>
                      <td class="px-2 py-1 text-fg-muted">{fmtBytes(cap.total_bytes)}</td>
                    </tr>
                  );
                }}
              </For>
              <Show when={captures().length === 0}>
                <tr>
                  <td colSpan={7} class="px-4 py-8 text-center text-fg-muted">
                    <AlertTriangle class="inline mr-1" size={14} />
                    No captures yet. Start the proxy and pair a device.
                  </td>
                </tr>
              </Show>
            </tbody>
          </table>
        </div>

        <div class="overflow-hidden">
          <DetailPanes capture={selected()} />
        </div>
      </div>
    </div>
  );
};

function statusColor(status: number | null, errorKind: string | null) {
  if (errorKind === "pinning") return "text-warn";
  if (errorKind) return "text-danger";
  if (status === null) return "text-fg-muted";
  if (status >= 500) return "text-danger";
  if (status >= 400) return "text-warn";
  if (status >= 300) return "text-accent";
  return "text-success";
}

function fmtBytes(n: number) {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}K`;
  return `${(n / 1024 / 1024).toFixed(1)}M`;
}

export default CapturesView;
