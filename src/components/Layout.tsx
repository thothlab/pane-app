import { type ParentComponent, createEffect, createMemo, createSignal, onMount, For, Show } from "solid-js";
import { A } from "@solidjs/router";
import { Activity, Smartphone, Settings, Info, Play, Square, Filter as FilterIcon, X, Shuffle, BookOpen, Download } from "lucide-solid";
import { open as openExternal } from "@tauri-apps/plugin-shell";
import { getVersion } from "@tauri-apps/api/app";
import { api } from "@/ipc/client";
import { VerticalResizer } from "@/components/VerticalResizer";
import { docsUrl } from "@/components/HelpButton";
import { setFilter } from "@/stores/captures";
import { filters, deleteFilter, refreshFilters } from "@/stores/saved-filters";
import { t, tr } from "@/i18n";
import {
  checkForUpdatesNow,
  checkForUpdatesOnStartup,
  installPendingUpdate,
  pendingUpdate,
} from "@/lib/updater";
import type { ProxyStatusDto } from "@/ipc/types";

const SIDEBAR_DEFAULT = 240;
const SIDEBAR_MIN = 180;
const SIDEBAR_MAX = 480;
const SIDEBAR_STORAGE_KEY = "pane:sidebar-width";

function loadSidebarWidth(): number {
  try {
    const raw = localStorage.getItem(SIDEBAR_STORAGE_KEY);
    if (!raw) return SIDEBAR_DEFAULT;
    const n = JSON.parse(raw);
    if (typeof n === "number" && n >= SIDEBAR_MIN && n <= SIDEBAR_MAX) return n;
  } catch {
    /* fall through */
  }
  return SIDEBAR_DEFAULT;
}

const Layout: ParentComponent = (props) => {
  const [status, setStatus] = createSignal<ProxyStatusDto | null>(null);
  const [appVersion, setAppVersion] = createSignal<string>("");
  const [sidebarWidth, setSidebarWidth] = createSignal(loadSidebarWidth());
  const gridTemplate = createMemo(() => `${sidebarWidth()}px 6px 1fr`);
  createEffect(() => {
    try {
      localStorage.setItem(SIDEBAR_STORAGE_KEY, JSON.stringify(sidebarWidth()));
    } catch {
      /* private mode */
    }
  });

  const refresh = async () => {
    try {
      setStatus(await api.proxy.status());
    } catch (e) {
      console.warn("status refresh failed", e);
    }
  };

  onMount(() => {
    refresh();
    refreshFilters();
    getVersion().then(setAppVersion).catch(() => {});
    void checkForUpdatesOnStartup();
    const t = setInterval(refresh, 2000);

    // Poll the update endpoint hourly so long-running sessions notice
    // new releases without needing a restart. Cheap (one HTTP GET to a
    // GitHub release asset) and only triggers a UI change if a newer
    // version is offered.
    const updateTimer = setInterval(() => void checkForUpdatesNow(), 60 * 60 * 1000);

    // Also re-check when the window regains focus — covers the common
    // "left it overnight, came back in the morning" case immediately
    // instead of up to an hour later.
    const onFocus = () => void checkForUpdatesNow();
    window.addEventListener("focus", onFocus);

    return () => {
      clearInterval(t);
      clearInterval(updateTimer);
      window.removeEventListener("focus", onFocus);
    };
  });

  const [installing, setInstalling] = createSignal(false);
  const onInstallUpdate = async () => {
    setInstalling(true);
    try {
      await installPendingUpdate();
    } finally {
      setInstalling(false);
    }
  };

  const toggleProxy = async () => {
    const s = status();
    if (s?.running) {
      await api.proxy.stop();
    } else {
      await api.proxy.start();
    }
    await refresh();
  };

  return (
    <div
      class="h-full grid bg-bg text-fg"
      style={{ "grid-template-columns": gridTemplate() }}
    >
      <aside class="bg-bg-subtle flex flex-col overflow-hidden">
        <div class="px-4 py-4 border-b border-border">
          <div class="font-semibold text-lg">Pane</div>
          <div class="text-xs text-fg-muted">{appVersion() ? `v${appVersion()}` : ""}</div>
          <Show when={pendingUpdate()}>
            <button
              type="button"
              class="mt-2 w-full inline-flex items-center gap-1 rounded border border-accent/40 bg-accent/10 text-accent px-2 py-1 text-xs font-medium hover:bg-accent/20 disabled:opacity-60"
              onClick={() => void onInstallUpdate()}
              disabled={installing()}
              title={t()("updates.install_title", { version: pendingUpdate()!.version })}
            >
              <Download size={12} />
              {installing()
                ? t()("updates.installing")
                : t()("updates.update_to", { version: pendingUpdate()!.version })}
            </button>
          </Show>
        </div>
        <nav class="flex-1 overflow-auto p-2 space-y-1">
          <NavLink href="/" icon={<Activity size={16} />}>{t()("nav.captures")}</NavLink>
          <NavLink href="/rules" icon={<Shuffle size={16} />}>{t()("nav.rules")}</NavLink>
          <NavLink href="/devices" icon={<Smartphone size={16} />}>{t()("nav.devices")}</NavLink>

          <Show when={filters().length > 0}>
            <div class="mt-8 pt-3 border-t border-border px-2 text-xs uppercase tracking-wide text-fg-muted">
              {t()("nav.filters")}
            </div>
            <For each={filters()}>
              {(f) => (
                <div
                  class="group px-2 py-1 rounded text-sm hover:bg-bg-muted cursor-pointer flex items-center gap-2"
                  title={t()("nav.apply_filter", { query: f.query })}
                  onClick={() => setFilter(f.query)}
                >
                  <FilterIcon size={14} style={{ color: f.color }} />
                  <span class="truncate flex-1">{f.name}</span>
                  <button
                    class="opacity-0 group-hover:opacity-100 hover:text-danger shrink-0"
                    title={t()("nav.delete_filter")}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (confirm(tr("nav.delete_filter_confirm", { name: f.name }))) {
                        void deleteFilter(f.id);
                      }
                    }}
                  >
                    <X size={12} />
                  </button>
                </div>
              )}
            </For>
          </Show>
        </nav>
        {/* Secondary nav (settings / help) pinned above the proxy control —
            separated from the primary navigation so the main list stays
            focused on workflow surfaces (Captures, Rules, Devices). */}
        <div class="p-2 border-t border-border space-y-1">
          <NavLink href="/settings" icon={<Settings size={16} />}>{t()("nav.settings")}</NavLink>
          <button
            type="button"
            class="w-full flex items-center gap-2 px-2 py-1.5 rounded text-sm hover:bg-bg-muted text-fg"
            title={t()("nav.docs_title")}
            onClick={() => void openExternal(docsUrl("/"))}
          >
            <BookOpen size={16} />
            {t()("nav.docs")}
          </button>
          <NavLink href="/about" icon={<Info size={16} />}>{t()("nav.about")}</NavLink>
        </div>
        <div class="p-3 border-t border-border space-y-2">
          <button
            class={`w-full inline-flex items-center justify-center gap-2 rounded px-3 py-2 text-sm font-medium transition
              ${status()?.running ? "bg-danger/15 text-danger hover:bg-danger/25" : "bg-accent text-white hover:opacity-90"}`}
            onClick={toggleProxy}
          >
            {status()?.running ? <Square size={14} /> : <Play size={14} />}
            {status()?.running ? t()("proxy.stop") : t()("proxy.start")}
          </button>
          <div class="text-xs text-fg-muted text-center">
            {status()?.running ? status()?.listen ?? t()("proxy.running") : t()("proxy.stopped")}
          </div>
        </div>
      </aside>
      <VerticalResizer
        onResize={(dx) =>
          setSidebarWidth((w) =>
            Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, w + dx)),
          )
        }
        onReset={() => setSidebarWidth(SIDEBAR_DEFAULT)}
      />
      <main class="overflow-hidden">{props.children}</main>
    </div>
  );
};

const NavLink: ParentComponent<{ href: string; icon: any }> = (props) => (
  <A
    href={props.href}
    end={props.href === "/"}
    class="flex items-center gap-2 px-2 py-1.5 rounded text-sm hover:bg-bg-muted"
    activeClass="bg-bg-muted text-accent"
  >
    {props.icon}
    {props.children}
  </A>
);

export default Layout;
