import { type Component, createMemo, createResource, createSignal, For, Show } from "solid-js";
import { RefreshCw, Upload, ChevronRight, X } from "lucide-solid";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "@/ipc/client";
import HelpButton from "@/components/HelpButton";
import { setTheme, theme, type Theme } from "@/stores/theme";
import {
  fontScale,
  setFontScale,
  FONT_SCALE_OPTIONS,
  type FontScale,
} from "@/stores/font-scale";
import { t, tr, locale, setLocale, LOCALES } from "@/i18n";
import { groupByBaseDomain } from "@/lib/host-grouping";
import {
  capturesPoll,
  setCapturesPollEnabled,
  setCapturesPollSeconds,
  MIN_POLL_SECONDS,
  MAX_POLL_SECONDS,
} from "@/stores/captures-poll";

// Theme button labels are looked up via i18n in the JSX. Statically
// listed key names keep the i18n key set discoverable by grep and let
// the translator type-check each call site.
const THEME_OPTIONS: Array<{ value: Theme; labelKey: "settings.theme_light" | "settings.theme_dark" | "settings.theme_system" }> = [
  { value: "light", labelKey: "settings.theme_light" },
  { value: "dark", labelKey: "settings.theme_dark" },
  { value: "system", labelKey: "settings.theme_system" },
];

const FONT_SCALE_LABEL_KEY: Record<
  FontScale,
  "settings.font_scale_sm" | "settings.font_scale_md" | "settings.font_scale_lg" | "settings.font_scale_xl"
> = {
  sm: "settings.font_scale_sm",
  md: "settings.font_scale_md",
  lg: "settings.font_scale_lg",
  xl: "settings.font_scale_xl",
};

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

  const [tunneled, { refetch: refetchTunneled }] = createResource(() =>
    api.passthrough.list(),
  );

  const rotate = async () => {
    if (!confirm(tr("settings.ca_rotate_confirm"))) return;
    setBusy(true);
    try {
      await api.ca.rotate();
      await refetch();
      // Rotating the CA invalidates every earlier "won't accept our cert"
      // verdict, and the backend clears them — reflect that here rather than
      // leaving a list the user has to reload by hand to trust.
      await refetchTunneled();
    } finally {
      setBusy(false);
    }
  };

  const resetTunneled = async () => {
    await api.passthrough.reset();
    await refetchTunneled();
  };

  const forgetTunneled = async (host: string) => {
    await api.passthrough.forget(host);
    await refetchTunneled();
  };

  // Sequential rather than concurrent: each call is a tiny mutation and the
  // refetch happens once at the end, so there is nothing to gain from racing
  // them and a partial failure stays easier to reason about.
  const forgetGroup = async (hosts: string[]) => {
    for (const host of hosts) await api.passthrough.forget(host);
    await refetchTunneled();
  };

  const tunneledGroups = createMemo(() =>
    groupByBaseDomain(tunneled()?.learned ?? [], (h) => h.host),
  );

  // Collapsed by default — the panel's first question is "what is being
  // tunnelled", which the domain answers. Only the domains the user opened are
  // tracked, so a refetch never silently re-collapses what they expanded.
  const [openGroups, setOpenGroups] = createSignal<Set<string>>(new Set());
  const isGroupOpen = (domain: string) => openGroups().has(domain);
  const toggleGroup = (domain: string) => {
    setOpenGroups((prev) => {
      const next = new Set(prev);
      if (!next.delete(domain)) next.add(domain);
      return next;
    });
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
          <div class="text-sm w-24">{t()("settings.font_scale_label")}</div>
          <div
            role="radiogroup"
            aria-label={t()("settings.font_scale_label")}
            class="inline-flex rounded border border-border overflow-hidden text-xs"
          >
            <For each={FONT_SCALE_OPTIONS}>
              {(opt) => (
                <button
                  role="radio"
                  aria-checked={fontScale() === opt}
                  onClick={() => setFontScale(opt)}
                  class="px-3 py-1.5 hover:bg-bg-muted aria-checked:bg-accent aria-checked:text-white not-[:last-child]:border-r not-[:last-child]:border-border"
                >
                  {t()(FONT_SCALE_LABEL_KEY[opt])}
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
          {/* Label column sized to its content, not to a pixel count: the labels
              are translated and the root font scales, so any fixed width is wrong
              for some combination of the two. At Extra large in English,
              "Fingerprint" already ran under its own value. */}
          <dl class="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-sm font-mono">
            <dt class="text-fg-muted">{t()("settings.ca_subject")}</dt><dd class="break-all min-w-0">{ca()!.subject}</dd>
            <dt class="text-fg-muted">{t()("settings.ca_serial")}</dt><dd class="break-all min-w-0">{ca()!.serial}</dd>
            <dt class="text-fg-muted">{t()("settings.ca_fingerprint")}</dt><dd class="break-all min-w-0">{ca()!.sha256_fp}</dd>
            <dt class="text-fg-muted">{t()("settings.ca_valid_from")}</dt><dd class="break-all min-w-0">{ca()!.valid_from}</dd>
            <dt class="text-fg-muted">{t()("settings.ca_valid_to")}</dt><dd class="break-all min-w-0">{ca()!.valid_to}</dd>
          </dl>
        </Show>
        <div class="flex flex-wrap gap-2">
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("pem")}>
            <Upload size={12} /> {t()("settings.ca_export_pem")}
          </button>
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("der")}>
            <Upload size={12} /> {t()("settings.ca_export_der")}
          </button>
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("qr")}>
            <Upload size={12} /> {t()("settings.ca_export_qr")}
          </button>
          <button class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1" onClick={() => exportCa("mobileconfig")}>
            <Upload size={12} /> {t()("settings.ca_export_mobileconfig")}
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
        <div class="flex items-center justify-between gap-2">
          <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">
            {t()("settings.tunneled_section")}
          </h2>
          <div class="flex items-center gap-2">
            <button
              class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1"
              onClick={() => refetchTunneled()}
            >
              <RefreshCw size={12} /> {t()("settings.tunneled_refresh")}
            </button>
            <button
              class="text-xs px-3 py-1.5 rounded bg-warn/15 text-warn hover:bg-warn/25 disabled:opacity-40"
              disabled={(tunneled()?.learned.length ?? 0) === 0}
              onClick={resetTunneled}
            >
              {t()("settings.tunneled_reset")}
            </button>
          </div>
        </div>
        <p class="text-sm text-fg-subtle">{t()("settings.tunneled_body")}</p>
        <Show
          when={(tunneled()?.learned.length ?? 0) > 0}
          fallback={<p class="text-sm text-fg-muted">{t()("settings.tunneled_empty")}</p>}
        >
          {/* One row per subdomain turned a handful of services into a
              screenful, so hosts are grouped by registrable domain and each
              group collapses. Collapsed is the default: the question this
              panel answers first is "what is being tunnelled", and the domain
              answers it — the individual hosts matter only when forgetting
              one. */}
          <ul class="text-sm divide-y divide-border/40 border border-border/40 rounded">
            <For each={tunneledGroups()}>
              {(group) => (
                <li>
                  {/* Two buttons side by side rather than one nested in the
                      other: a control inside a control is invalid markup and
                      leaves the inner one unreachable by keyboard. */}
                  <div class="flex items-center gap-2 px-2 py-1.5 hover:bg-bg-muted">
                    <button
                      type="button"
                      class="flex items-center gap-2 min-w-0 flex-1 text-left"
                      aria-expanded={isGroupOpen(group.domain)}
                      onClick={() => toggleGroup(group.domain)}
                    >
                      <ChevronRight
                        size={12}
                        class={`shrink-0 text-fg-muted transition-transform ${
                          isGroupOpen(group.domain) ? "rotate-90" : ""
                        }`}
                      />
                      <span class="font-mono truncate">{group.domain}</span>
                      <span class="text-xs text-fg-muted tabular-nums shrink-0">
                        {group.items.length}
                      </span>
                    </button>
                    <button
                      type="button"
                      class="text-xs px-2 py-0.5 rounded border border-border hover:bg-bg-subtle shrink-0"
                      title={t()("settings.tunneled_forget_group")}
                      onClick={() => void forgetGroup(group.items.map((h) => h.host))}
                    >
                      {t()("settings.tunneled_forget")}
                    </button>
                  </div>
                  <Show when={isGroupOpen(group.domain)}>
                    <ul class="pb-1">
                      <For each={group.items}>
                        {(h) => (
                          <li class="flex items-baseline gap-2 pl-6 pr-2 py-0.5 hover:bg-bg-muted/50">
                            <span class="font-mono text-xs truncate flex-1" title={h.host}>
                              {h.host}
                            </span>
                            <span
                              class="text-xs text-fg-muted shrink-0"
                              title={h.detail || undefined}
                            >
                              {h.reason === "cert_rejected"
                                ? t()("settings.tunneled_reason_cert")
                                : t()("settings.tunneled_reason_repeated")}
                            </span>
                            <button
                              class="text-xs text-fg-muted hover:text-fg px-1 shrink-0"
                              title={t()("settings.tunneled_forget")}
                              aria-label={t()("settings.tunneled_forget")}
                              onClick={() => forgetTunneled(h.host)}
                            >
                              <X size={12} />
                            </button>
                          </li>
                        )}
                      </For>
                    </ul>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <Show when={(tunneled()?.seeded.length ?? 0) > 0}>
          <p class="text-xs text-fg-muted">
            {t()("settings.tunneled_seeded")}: {tunneled()!.seeded.join(", ")}
          </p>
        </Show>
      </section>

      <section class="space-y-3">
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">
          {t()("settings.poll_section")}
        </h2>
        <p class="text-sm text-fg-subtle">{t()("settings.poll_body")}</p>
        <div class="flex items-center gap-3 flex-wrap">
          <label class="inline-flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={capturesPoll().enabled}
              onChange={(e) => setCapturesPollEnabled(e.currentTarget.checked)}
            />
            {t()("settings.poll_enabled")}
          </label>
          <label class="inline-flex items-center gap-2 text-sm">
            <span class={capturesPoll().enabled ? "" : "text-fg-muted"}>
              {t()("settings.poll_interval")}
            </span>
            <input
              type="number"
              class="w-20 px-2 py-1 rounded border border-border bg-bg-subtle text-sm disabled:opacity-40"
              min={MIN_POLL_SECONDS}
              max={MAX_POLL_SECONDS}
              step="1"
              disabled={!capturesPoll().enabled}
              value={capturesPoll().seconds}
              // Commit on change, not on input: typing "30" passes through "3",
              // and re-arming the timer on every keystroke is both pointless
              // and briefly wrong.
              onChange={(e) => setCapturesPollSeconds(Number(e.currentTarget.value))}
            />
            <span class={capturesPoll().enabled ? "text-fg-muted" : "text-fg-muted/50"}>
              {t()("settings.poll_seconds")}
            </span>
          </label>
        </div>
        <p class="text-xs text-fg-muted">{t()("settings.poll_hint")}</p>
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
