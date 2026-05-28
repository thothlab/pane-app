import { type Component, createSignal, createResource, For, Show } from "solid-js";
import { Smartphone, Plus, RefreshCw, Trash2, AlertCircle, CheckCircle } from "lucide-solid";
import { api } from "@/ipc/client";
import type { DeviceDto, DiscoveredDeviceDto } from "@/ipc/types";

const DevicesView: Component = () => {
  const [devices, { refetch }] = createResource(() => api.devices.list());
  const [attached, { refetch: refetchAttached }] = createResource(() =>
    api.devices.listAttachedUsb(),
  );
  const [busy, setBusy] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  const add = async (d: DiscoveredDeviceDto) => {
    setBusy(d.serial);
    setError(null);
    try {
      if (d.platform === "ios") await api.devices.addIosUsb(d.serial);
      else await api.devices.addAndroidUsb(d.serial);
      await refetch();
    } catch (e: any) {
      setError(e?.message ?? "add failed");
    } finally {
      setBusy(null);
    }
  };

  const remove = async (id: string) => {
    if (!confirm("Remove device and revoke setup?")) return;
    await api.devices.remove(id);
    await refetch();
  };

  return (
    <div class="h-full overflow-auto p-6 space-y-6">
      <header class="flex items-center justify-between">
        <h1 class="text-xl font-semibold">Devices</h1>
        <button
          class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1"
          onClick={() => {
            refetch();
            refetchAttached();
          }}
        >
          <RefreshCw size={12} /> Refresh
        </button>
      </header>

      <Show when={error()}>
        <div class="rounded border border-danger/40 bg-danger/10 text-danger px-3 py-2 text-sm flex items-start gap-2">
          <AlertCircle size={14} class="mt-0.5" />
          <div>{error()}</div>
        </div>
      </Show>

      <section>
        <h2 class="text-sm font-semibold text-fg-subtle mb-2 uppercase tracking-wide">Attached over USB</h2>
        <Show
          when={attached() && attached()!.length > 0}
          fallback={
            <p class="text-fg-muted text-sm">
              No devices detected. Plug in your iPhone or Android, allow trust / USB debugging.
            </p>
          }
        >
          <ul class="space-y-2">
            <For each={attached()}>
              {(d) => (
                <li class="flex items-center justify-between p-3 rounded border border-border bg-bg-subtle">
                  <div class="flex items-center gap-3">
                    <Smartphone size={16} class="text-accent" />
                    <div>
                      <div class="text-sm font-medium">{d.name}</div>
                      <div class="text-xs text-fg-muted font-mono">{d.platform} · {d.serial}</div>
                    </div>
                  </div>
                  <button
                    class="text-xs px-3 py-1.5 rounded bg-accent text-white inline-flex items-center gap-1 disabled:opacity-50"
                    disabled={busy() === d.serial}
                    onClick={() => add(d)}
                  >
                    <Plus size={12} /> {busy() === d.serial ? "Adding…" : "Add"}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </section>

      <section>
        <h2 class="text-sm font-semibold text-fg-subtle mb-2 uppercase tracking-wide">Paired</h2>
        <Show
          when={devices() && devices()!.length > 0}
          fallback={<p class="text-fg-muted text-sm">No paired devices yet.</p>}
        >
          <ul class="space-y-2">
            <For each={devices()}>
              {(d) => <DeviceRow device={d} onRemove={() => remove(d.id)} />}
            </For>
          </ul>
        </Show>
      </section>

      <section class="text-xs text-fg-muted">
        <h3 class="text-sm text-fg-subtle font-semibold mb-1">Use only on devices you own.</h3>
        <p>
          my-charles is intended for inspecting your own apps and authorized security work. Don't
          point it at devices or applications you lack permission to inspect.
        </p>
      </section>
    </div>
  );
};

const DeviceRow: Component<{ device: DeviceDto; onRemove: () => void }> = (p) => (
  <li class="flex items-center justify-between p-3 rounded border border-border bg-bg-subtle">
    <div class="flex items-center gap-3">
      <Show when={p.device.state === "ready"} fallback={<AlertCircle size={16} class="text-warn" />}>
        <CheckCircle size={16} class="text-success" />
      </Show>
      <div>
        <div class="text-sm font-medium">{p.device.display_name}</div>
        <div class="text-xs text-fg-muted font-mono">
          {p.device.platform} · {p.device.state}
          <Show when={p.device.last_error}> · {p.device.last_error}</Show>
        </div>
      </div>
    </div>
    <button
      class="text-xs px-2 py-1 rounded hover:bg-bg-muted text-danger inline-flex items-center gap-1"
      onClick={p.onRemove}
    >
      <Trash2 size={12} /> Remove
    </button>
  </li>
);

export default DevicesView;
