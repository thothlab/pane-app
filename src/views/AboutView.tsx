import { type Component, createSignal, Show, onMount } from "solid-js";
import { getVersion } from "@tauri-apps/api/app";
import { RefreshCw, CheckCircle, Download } from "lucide-solid";
import {
  checkForUpdatesNow,
  installPendingUpdate,
  isCheckingForUpdates,
  pendingUpdate,
  updaterLastCheckedAt,
} from "@/lib/updater";
import { t } from "@/i18n";

const AboutView: Component = () => {
  const [version, setVersion] = createSignal<string>("");
  const [installing, setInstalling] = createSignal(false);
  const [lastResultMsg, setLastResultMsg] = createSignal<string | null>(null);

  onMount(() => {
    getVersion().then(setVersion).catch(() => {});
  });

  const onCheckClick = async () => {
    setLastResultMsg(null);
    const res = await checkForUpdatesNow();
    if (res.error) {
      setLastResultMsg(t()("updates.server_unreachable"));
    } else if (!res.found) {
      setLastResultMsg(t()("updates.up_to_date"));
    }
  };

  const onInstallClick = async () => {
    setInstalling(true);
    try {
      await installPendingUpdate();
    } finally {
      setInstalling(false);
    }
  };

  return (
    <div class="h-full overflow-auto p-6 space-y-6 max-w-3xl">
      <h1 class="text-xl font-semibold">{t()("about.title")}</h1>

      <section class="space-y-3 text-sm leading-6">
        <div class="flex items-center gap-3">
          <div>
            <div class="font-medium">{t()("about.version_label")}</div>
            <div class="text-fg-muted font-mono text-xs">{version() || "—"}</div>
          </div>
          <div class="ml-auto flex items-center gap-2">
            <Show when={pendingUpdate()}>
              <button
                type="button"
                class="text-xs px-3 py-1.5 rounded bg-accent text-white inline-flex items-center gap-1 disabled:opacity-60"
                disabled={installing()}
                onClick={() => void onInstallClick()}
              >
                <Download size={12} />
                {installing()
                  ? t()("updates.installing")
                  : t()("updates.update_to", { version: pendingUpdate()!.version })}
              </button>
            </Show>
            <button
              type="button"
              class="text-xs px-3 py-1.5 rounded border border-border hover:bg-bg-muted inline-flex items-center gap-1 disabled:opacity-60"
              disabled={isCheckingForUpdates()}
              onClick={() => void onCheckClick()}
            >
              <RefreshCw
                size={12}
                class={isCheckingForUpdates() ? "animate-spin" : ""}
              />
              {isCheckingForUpdates()
                ? t()("updates.checking")
                : t()("updates.check_for_updates")}
            </button>
          </div>
        </div>
        <Show when={lastResultMsg()}>
          <div class="text-xs text-fg-muted inline-flex items-center gap-1">
            <CheckCircle size={12} class="text-success" />
            {lastResultMsg()}
          </div>
        </Show>
        <Show when={updaterLastCheckedAt() && !lastResultMsg()}>
          <div class="text-xs text-fg-muted">
            {t()("updates.last_checked", {
              time: updaterLastCheckedAt()!.toLocaleTimeString(),
            })}
          </div>
        </Show>
      </section>

      <section class="space-y-2 text-sm leading-6">
        <p innerHTML={t()("about.intro")} />
      </section>

      <section class="space-y-2 text-sm leading-6">
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">
          {t()("about.boundaries_title")}
        </h2>
        <ul class="list-disc pl-5 space-y-1 text-fg-subtle">
          <li innerHTML={t()("about.boundaries_1")} />
          <li innerHTML={t()("about.boundaries_2")} />
          <li innerHTML={t()("about.boundaries_3")} />
        </ul>
      </section>

      <section class="space-y-2 text-sm leading-6">
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">
          {t()("about.pinning_title")}
        </h2>
        <p>{t()("about.pinning_para1")}</p>
        <p>{t()("about.pinning_para2")}</p>
      </section>

      <section class="space-y-2 text-sm">
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">
          {t()("about.license_title")}
        </h2>
        <p class="text-fg-subtle">{t()("about.license_body")}</p>
      </section>
    </div>
  );
};

export default AboutView;
