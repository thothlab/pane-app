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
      setLastResultMsg("Couldn't reach the update server. Try again later.");
    } else if (!res.found) {
      setLastResultMsg("You're on the latest version.");
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
      <h1 class="text-xl font-semibold">About Pane</h1>

      <section class="space-y-3 text-sm leading-6">
        <div class="flex items-center gap-3">
          <div>
            <div class="font-medium">Version</div>
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
                  ? "Installing…"
                  : `Update to v${pendingUpdate()!.version}`}
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
              {isCheckingForUpdates() ? "Checking…" : "Check for updates"}
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
            Last checked: {updaterLastCheckedAt()!.toLocaleTimeString()}
          </div>
        </Show>
      </section>

      <section class="space-y-2 text-sm leading-6">
        <p>
          A modern HTTPS network debugger focused on one thing: <strong>making device setup take
          30 seconds instead of 15 minutes.</strong> No certificate trust dance, no Wi-Fi proxy
          editing — plug your iPhone or Android in over USB and click Add.
        </p>
      </section>

      <section class="space-y-2 text-sm leading-6">
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">Boundaries</h2>
        <ul class="list-disc pl-5 space-y-1 text-fg-subtle">
          <li>Designed for inspecting <strong>your own</strong> apps and authorized security work.</li>
          <li>Doesn't bypass certificate pinning. When pinning blocks inspection, you'll see why.</li>
          <li>Not a production traffic monitor. Not a packet-level capture tool.</li>
        </ul>
      </section>

      <section class="space-y-2 text-sm leading-6">
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">Cert pinning</h2>
        <p>
          Certificate pinning is a security feature where an app refuses to talk to anyone whose
          cert doesn't match a pre-baked fingerprint. Our MITM proxy can't impersonate those
          endpoints — that's by design.
        </p>
        <p>
          For your own apps, disable pinning in the debug build. For owned-device security
          research, tools like Frida or Magisk can bypass pinning at runtime; Pane doesn't
          bundle them.
        </p>
      </section>

      <section class="space-y-2 text-sm">
        <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">License</h2>
        <p class="text-fg-subtle">
          Apache-2.0. Built on top of rustls, rcgen, libimobiledevice, and the Android Platform
          Tools.
        </p>
      </section>
    </div>
  );
};

export default AboutView;
