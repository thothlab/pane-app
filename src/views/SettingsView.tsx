import { type Component, createResource, createSignal, Show } from "solid-js";
import { RefreshCw, Download } from "lucide-solid";
import { api } from "@/ipc/client";

const SettingsView: Component = () => {
  const [ca, { refetch }] = createResource(() => api.ca.current());
  const [busy, setBusy] = createSignal(false);
  const [exported, setExported] = createSignal<string | null>(null);

  const rotate = async () => {
    if (!confirm("Rotate CA? Paired devices will need to re-trust.")) return;
    setBusy(true);
    try {
      await api.ca.rotate();
      await refetch();
    } finally {
      setBusy(false);
    }
  };

  const exportCa = async (format: "pem" | "der" | "qr" | "mobileconfig") => {
    const r = await api.ca.export(format);
    setExported(`${format} (${r.mime}) — copied as base64`);
    if (r.data_base64) navigator.clipboard.writeText(r.data_base64);
  };

  return (
    <div class="h-full overflow-auto p-6 space-y-6 max-w-3xl">
      <h1 class="text-xl font-semibold">Settings</h1>

      <section class="space-y-3">
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">Root CA</h2>
        <Show when={ca()} fallback={<p class="text-fg-muted">Loading…</p>}>
          <dl class="grid grid-cols-[120px_1fr] gap-y-1 text-sm font-mono">
            <dt class="text-fg-muted">Subject</dt><dd>{ca()!.subject}</dd>
            <dt class="text-fg-muted">Serial</dt><dd>{ca()!.serial}</dd>
            <dt class="text-fg-muted">Fingerprint</dt><dd class="break-all">{ca()!.sha256_fp}</dd>
            <dt class="text-fg-muted">Valid from</dt><dd>{ca()!.valid_from}</dd>
            <dt class="text-fg-muted">Valid to</dt><dd>{ca()!.valid_to}</dd>
          </dl>
        </Show>
        <div class="flex flex-wrap gap-2">
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("pem")}>
            <Download size={12} /> Export PEM
          </button>
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("der")}>
            <Download size={12} /> Export DER
          </button>
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("qr")}>
            <Download size={12} /> Export QR
          </button>
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("mobileconfig")}>
            <Download size={12} /> Export mobileconfig
          </button>
          <button
            class="text-xs px-3 py-1.5 rounded bg-warn/15 text-warn hover:bg-warn/25 inline-flex items-center gap-1"
            disabled={busy()}
            onClick={rotate}
          >
            <RefreshCw size={12} /> Rotate CA
          </button>
        </div>
        <Show when={exported()}>
          <p class="text-xs text-fg-muted">{exported()}</p>
        </Show>
      </section>

      <section class="space-y-3">
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">Privacy</h2>
        <p class="text-sm text-fg-subtle">
          my-charles collects zero telemetry. No data leaves your machine unless you explicitly
          export it. Crash reports stay in <code class="font-mono text-xs bg-bg-muted px-1 rounded">logs/</code> next to the data dir.
        </p>
      </section>
    </div>
  );
};

export default SettingsView;
