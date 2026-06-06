import { type Component, createResource, createSignal, For, Show } from "solid-js";
import { RefreshCw, Download } from "lucide-solid";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "@/ipc/client";
import HelpButton from "@/components/HelpButton";
import { setTheme, theme, type Theme } from "@/stores/theme";
import { t, locale, setLocale, LOCALES } from "@/i18n";

// Theme button labels are looked up via i18n in the JSX. Statically
// listed key names keep the i18n key set discoverable by grep and let
// the translator type-check each call site.
const THEME_OPTIONS: Array<{ value: Theme; labelKey: "settings.theme_light" | "settings.theme_dark" | "settings.theme_system" }> = [
  { value: "light", labelKey: "settings.theme_light" },
  { value: "dark", labelKey: "settings.theme_dark" },
  { value: "system", labelKey: "settings.theme_system" },
];

type CaFormat = "pem" | "der" | "qr" | "mobileconfig";

const FORMAT_META: Record<CaFormat, { ext: string; defaultName: string; label: string }> = {
  pem: { ext: "pem", defaultName: "pane-root-ca.pem", label: "PEM certificate" },
  der: { ext: "der", defaultName: "pane-root-ca.der", label: "DER certificate" },
  qr: { ext: "svg", defaultName: "pane-root-ca-qr.svg", label: "QR code (SVG)" },
  mobileconfig: {
    ext: "mobileconfig",
    defaultName: "pane-root-ca.mobileconfig",
    label: "Apple Configuration Profile",
  },
};

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

  const exportCa = async (format: CaFormat) => {
    const meta = FORMAT_META[format];
    const path = await save({
      defaultPath: meta.defaultName,
      filters: [{ name: meta.label, extensions: [meta.ext] }],
    });
    if (!path) return;
    try {
      const r = await api.ca.saveToFile(format, path);
      setExported(`Saved ${r.bytes_written} bytes → ${r.path}`);
    } catch (e) {
      setExported(`Save failed: ${(e as { message?: string })?.message ?? String(e)}`);
    }
  };

  return (
    <div class="h-full overflow-auto p-6 space-y-6 max-w-3xl">
      <div class="flex items-center gap-2">
        <h1 class="text-xl font-semibold">{t()("settings.title")}</h1>
        <HelpButton path="/getting-started/" title="First-time setup: install, CA export, proxy on device" />
      </div>

      <section class="space-y-3">
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">
          {t()("settings.appearance_section")}
        </h2>

        <div class="flex items-center gap-3">
          <div class="text-sm w-24">{t()("settings.theme_label")}</div>
          <div
            role="radiogroup"
            aria-label={t()("settings.theme_label")}
            class="inline-flex rounded border border-border overflow-hidden text-xs"
          >
            <For each={THEME_OPTIONS}>
              {(opt) => (
                <button
                  role="radio"
                  aria-checked={theme() === opt.value}
                  onClick={() => setTheme(opt.value)}
                  class="px-3 py-1.5 hover:bg-bg-muted aria-checked:bg-accent aria-checked:text-white not-[:last-child]:border-r not-[:last-child]:border-border"
                >
                  {t()(opt.labelKey)}
                </button>
              )}
            </For>
          </div>
        </div>

        <div class="flex items-center gap-3">
          <div class="text-sm w-24">{t()("settings.language_label")}</div>
          <div
            role="radiogroup"
            aria-label={t()("settings.language_label")}
            class="inline-flex rounded border border-border overflow-hidden text-xs"
          >
            <For each={LOCALES}>
              {(opt) => (
                <button
                  role="radio"
                  aria-checked={locale() === opt.code}
                  onClick={() => setLocale(opt.code)}
                  class="px-3 py-1.5 hover:bg-bg-muted aria-checked:bg-accent aria-checked:text-white not-[:last-child]:border-r not-[:last-child]:border-border"
                >
                  {opt.label}
                </button>
              )}
            </For>
          </div>
        </div>
      </section>

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
          Pane collects zero telemetry. No data leaves your machine unless you explicitly
          export it. Crash reports stay in <code class="font-mono text-xs bg-bg-muted px-1 rounded">logs/</code> next to the data dir.
        </p>
      </section>
    </div>
  );
};

export default SettingsView;
