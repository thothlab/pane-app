import { type Component, createResource, createSignal, For, Show } from "solid-js";
import { RefreshCw, Download } from "lucide-solid";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "@/ipc/client";
import HelpButton from "@/components/HelpButton";
import { setTheme, theme, type Theme } from "@/stores/theme";
import { t, tr, locale, setLocale, LOCALES } from "@/i18n";

// Theme button labels are looked up via i18n in the JSX. Statically
// listed key names keep the i18n key set discoverable by grep and let
// the translator type-check each call site.
const THEME_OPTIONS: Array<{ value: Theme; labelKey: "settings.theme_light" | "settings.theme_dark" | "settings.theme_system" }> = [
  { value: "light", labelKey: "settings.theme_light" },
  { value: "dark", labelKey: "settings.theme_dark" },
  { value: "system", labelKey: "settings.theme_system" },
];

type CaFormat = "pem" | "der" | "qr" | "mobileconfig";

// File-format metadata. `labelKey` is resolved against the dictionary
// at the call site (file picker dialog label) so it tracks the locale.
const FORMAT_META: Record<
  CaFormat,
  { ext: string; defaultName: string; labelKey: "settings.ca_format_pem" | "settings.ca_format_der" | "settings.ca_format_qr" | "settings.ca_format_mobileconfig" }
> = {
  pem: { ext: "pem", defaultName: "pane-root-ca.pem", labelKey: "settings.ca_format_pem" },
  der: { ext: "der", defaultName: "pane-root-ca.der", labelKey: "settings.ca_format_der" },
  qr: { ext: "svg", defaultName: "pane-root-ca-qr.svg", labelKey: "settings.ca_format_qr" },
  mobileconfig: {
    ext: "mobileconfig",
    defaultName: "pane-root-ca.mobileconfig",
    labelKey: "settings.ca_format_mobileconfig",
  },
};

const SettingsView: Component = () => {
  const [ca, { refetch }] = createResource(() => api.ca.current());
  const [busy, setBusy] = createSignal(false);
  const [exported, setExported] = createSignal<string | null>(null);

  const rotate = async () => {
    if (!confirm(tr("settings.ca_rotate_confirm"))) return;
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
      filters: [{ name: tr(meta.labelKey), extensions: [meta.ext] }],
    });
    if (!path) return;
    try {
      const r = await api.ca.saveToFile(format, path);
      setExported(tr("settings.ca_export_success", { bytes: r.bytes_written, path: r.path }));
    } catch (e) {
      setExported(tr("settings.ca_export_failed", { message: (e as { message?: string })?.message ?? String(e) }));
    }
  };

  return (
    <div class="h-full overflow-auto p-6 space-y-6 max-w-3xl">
      <div class="flex items-center gap-2">
        <h1 class="text-xl font-semibold">{t()("settings.title")}</h1>
        <HelpButton path="/getting-started/" title={t()("settings.help_title")} />
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
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">
          {t()("settings.ca_section")}
        </h2>
        <Show when={ca()} fallback={<p class="text-fg-muted">{t()("settings.ca_loading")}</p>}>
          <dl class="grid grid-cols-[120px_1fr] gap-y-1 text-sm font-mono">
            <dt class="text-fg-muted">{t()("settings.ca_subject")}</dt><dd>{ca()!.subject}</dd>
            <dt class="text-fg-muted">{t()("settings.ca_serial")}</dt><dd>{ca()!.serial}</dd>
            <dt class="text-fg-muted">{t()("settings.ca_fingerprint")}</dt><dd class="break-all">{ca()!.sha256_fp}</dd>
            <dt class="text-fg-muted">{t()("settings.ca_valid_from")}</dt><dd>{ca()!.valid_from}</dd>
            <dt class="text-fg-muted">{t()("settings.ca_valid_to")}</dt><dd>{ca()!.valid_to}</dd>
          </dl>
        </Show>
        <div class="flex flex-wrap gap-2">
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("pem")}>
            <Download size={12} /> {t()("settings.ca_export_pem")}
          </button>
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("der")}>
            <Download size={12} /> {t()("settings.ca_export_der")}
          </button>
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("qr")}>
            <Download size={12} /> {t()("settings.ca_export_qr")}
          </button>
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("mobileconfig")}>
            <Download size={12} /> {t()("settings.ca_export_mobileconfig")}
          </button>
          <button
            class="text-xs px-3 py-1.5 rounded bg-warn/15 text-warn hover:bg-warn/25 inline-flex items-center gap-1"
            disabled={busy()}
            onClick={rotate}
          >
            <RefreshCw size={12} /> {t()("settings.ca_rotate")}
          </button>
        </div>
        <Show when={exported()}>
          <p class="text-xs text-fg-muted">{exported()}</p>
        </Show>
      </section>

      <section class="space-y-3">
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">
          {t()("settings.privacy_section")}
        </h2>
        <p class="text-sm text-fg-subtle">{t()("settings.privacy_body")}</p>
      </section>
    </div>
  );
};

export default SettingsView;
