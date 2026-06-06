import { type Component, createSignal, createResource, For, Show } from "solid-js";
import { Smartphone, Plus, RefreshCw, RotateCw, Trash2, AlertCircle, CheckCircle, Copy, Check, ChevronDown, ChevronRight } from "lucide-solid";
import { api } from "@/ipc/client";
import HelpButton from "@/components/HelpButton";
import type { DeviceDto, DiscoveredDeviceDto } from "@/ipc/types";

/** What ca_install_state means visually — see pane-android::add_usb. */
type CaInstallState = "auto_succeeded" | "manual_required" | "failed" | "unknown";

function caState(d: DeviceDto): CaInstallState {
  const v = (d.capabilities as Record<string, unknown>)?.["ca_install_state"];
  return v === "auto_succeeded" || v === "manual_required" || v === "failed"
    ? v
    : "unknown";
}

function caPath(d: DeviceDto): string {
  const v = (d.capabilities as Record<string, unknown>)?.["ca_install_path"];
  return typeof v === "string" ? v : "/sdcard/Pane/pane-ca.pem";
}

const DevicesView: Component = () => {
  const [devices, { refetch }] = createResource(() => api.devices.list());
  const [attached, { refetch: refetchAttached }] = createResource(() =>
    api.devices.listAttachedUsb(),
  );
  const [adbStatus, { refetch: refetchAdbStatus }] = createResource(() =>
    api.devices.androidToolingStatus(),
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

  // Re-runs the pairing flow on an already-paired device. Useful when the
  // adb-reverse mapping is lost (device unplugged, adb server restarted) —
  // the DB row is updated in place via ON CONFLICT(platform, serial).
  const resync = async (d: DeviceDto) => {
    setBusy(d.serial);
    setError(null);
    try {
      if (d.platform === "ios") await api.devices.addIosUsb(d.serial);
      else await api.devices.addAndroidUsb(d.serial);
      await refetch();
    } catch (e: unknown) {
      setError((e as { message?: string })?.message ?? "re-sync failed");
    } finally {
      setBusy(null);
    }
  };

  return (
    <div class="h-full overflow-auto p-6 space-y-6">
      <header class="flex items-center justify-between">
        <div class="flex items-center gap-2">
          <h1 class="text-xl font-semibold">Devices</h1>
          <HelpButton path="/getting-started/#first-device" title="USB pairing: iOS / Android setup walkthrough" />
        </div>
        <button
          class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1"
          onClick={() => {
            refetch();
            refetchAttached();
            refetchAdbStatus();
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

      <Show when={adbStatus() && !adbStatus()!.ok}>
        <div class="rounded border border-warn/40 bg-warn/10 text-warn px-3 py-2 text-sm flex items-start gap-2">
          <AlertCircle size={14} class="mt-0.5 shrink-0" />
          <div>
            <div class="font-medium">Android tooling not found</div>
            <div class="text-fg-muted mt-1">{adbStatus()!.error}</div>
          </div>
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
              {(d) => (
                <DeviceRow
                  device={d}
                  busy={busy() === d.serial}
                  onResync={() => resync(d)}
                  onRemove={() => remove(d.id)}
                />
              )}
            </For>
          </ul>
        </Show>
      </section>

      <section class="text-xs text-fg-muted">
        <h3 class="text-sm text-fg-subtle font-semibold mb-1">Use only on devices you own.</h3>
        <p>
          Pane is intended for inspecting your own apps and authorized security work. Don't
          point it at devices or applications you lack permission to inspect.
        </p>
      </section>
    </div>
  );
};

const DeviceRow: Component<{
  device: DeviceDto;
  busy: boolean;
  onResync: () => void;
  onRemove: () => void;
}> = (p) => {
  const state = () => caState(p.device);
  const isFullyReady = () => p.device.state === "ready" && state() === "auto_succeeded";
  // Expand the manual-install guide by default for fresh manual_required
  // devices — the user just clicked Add and needs to see what to do next.
  // Once they've installed and there's no warning to surface, collapse it.
  const [guideOpen, setGuideOpen] = createSignal(state() === "manual_required");

  return (
    <li class="rounded border border-border bg-bg-subtle">
      <div class="flex items-start justify-between p-3 gap-3">
        <div class="flex items-start gap-3 min-w-0 flex-1">
          <Show
            when={isFullyReady()}
            fallback={<AlertCircle size={16} class="text-warn shrink-0 mt-0.5" />}
          >
            <CheckCircle size={16} class="text-success shrink-0 mt-0.5" />
          </Show>
          <div class="min-w-0 flex-1">
            <div class="text-sm font-medium truncate">{p.device.display_name}</div>
            <div class="text-xs text-fg-muted font-mono">
              {p.device.platform} · {p.device.state}
            </div>
            <Show when={state() === "manual_required"}>
              <div class="text-xs text-warn mt-1">
                Almost there — finish CA install on the device.
              </div>
            </Show>
            <Show when={state() === "failed"}>
              <div class="text-xs text-danger mt-1">{p.device.last_error}</div>
            </Show>
          </div>
        </div>
        <div class="flex items-center gap-1">
          <button
            class="text-xs px-2 py-1 rounded hover:bg-bg-muted inline-flex items-center gap-1 disabled:opacity-50"
            onClick={p.onResync}
            disabled={p.busy}
            title="Re-apply USB port-forwarding + proxy setup"
          >
            <RotateCw size={12} class={p.busy ? "animate-spin" : ""} /> Re-sync
          </button>
          <button
            class="text-xs px-2 py-1 rounded hover:bg-bg-muted text-danger inline-flex items-center gap-1"
            onClick={p.onRemove}
          >
            <Trash2 size={12} /> Remove
          </button>
        </div>
      </div>

      {/* Expanded manual-install guide. Collapsible because once the user
          has done it, the row should compress back to just the headline. */}
      <Show when={state() === "manual_required"}>
        <button
          type="button"
          class="w-full px-3 py-2 border-t border-border text-xs text-fg-muted hover:bg-bg-muted flex items-center gap-1"
          onClick={() => setGuideOpen(!guideOpen())}
        >
          <Show when={guideOpen()} fallback={<ChevronRight size={12} />}>
            <ChevronDown size={12} />
          </Show>
          How to install the CA certificate
        </button>
        <Show when={guideOpen()}>
          <ManualInstallGuide path={caPath(p.device)} />
        </Show>
      </Show>
    </li>
  );
};

/** Step-by-step manual CA install instructions. Shown when programmatic
 *  install was blocked (Samsung One UI on Android 16, primarily). */
const ManualInstallGuide: Component<{ path: string }> = (p) => {
  const [copied, setCopied] = createSignal(false);
  const copyPath = async () => {
    try {
      await navigator.clipboard.writeText(p.path);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* no clipboard permission — fine, path is visible inline */
    }
  };

  return (
    <div class="px-3 pb-3 pt-1 border-t border-border space-y-3 text-xs">
      <div class="text-fg-muted">
        Your Android build (most commonly Samsung One UI on Android 16+)
        blocks programmatic CA installs. Pane has already pushed the
        certificate to your device — finish the install yourself:
      </div>

      <ol class="list-decimal pl-5 space-y-1.5 text-fg">
        <li>
          On the phone, open <strong>Settings → Biometrics & security →
          Other security settings → Install from device storage → CA
          certificate</strong>.
        </li>
        <li>
          On the warning screen tap <strong>Install anyway</strong>.
        </li>
        <li>
          In the file picker, open <strong>Internal storage → Pane</strong>{" "}
          and pick <code class="font-mono">pane-ca.pem</code>.
        </li>
        <li>
          Enter your screen-lock PIN/pattern when prompted.
        </li>
      </ol>

      <div class="rounded bg-bg-muted px-2 py-1.5 flex items-center justify-between gap-2">
        <code class="font-mono text-fg truncate">{p.path}</code>
        <button
          type="button"
          class="text-fg-muted hover:text-fg shrink-0 inline-flex items-center gap-1"
          onClick={() => void copyPath()}
          title="Copy path"
        >
          <Show when={copied()} fallback={<Copy size={12} />}>
            <Check size={12} class="text-success" />
          </Show>
        </button>
      </div>

      <div class="text-fg-muted">
        Without a lock-screen PIN/pattern, Android refuses user CA
        installs — set one first if needed. After installation, debug
        builds with <code class="font-mono">network_security_config</code>{" "}
        trusting user CAs will accept Pane. Release builds with TLS
        pinning need extra bypass.
      </div>
    </div>
  );
};

export default DevicesView;
