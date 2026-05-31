import { type ParentComponent, createEffect, createMemo, createSignal, onMount, For, Show } from "solid-js";
import { A } from "@solidjs/router";
import { Activity, Smartphone, Settings, Info, Play, Square, Filter as FilterIcon, X, Shuffle } from "lucide-solid";
import { api } from "@/ipc/client";
import { VerticalResizer } from "@/components/VerticalResizer";
import { setFilter } from "@/stores/captures";
import { filters, deleteFilter, refreshFilters } from "@/stores/saved-filters";
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
    const t = setInterval(refresh, 2000);
    return () => clearInterval(t);
  });

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
          <div class="text-xs text-fg-muted">v0.1.0-dev</div>
        </div>
        <nav class="flex-1 overflow-auto p-2 space-y-1">
          <NavLink href="/" icon={<Activity size={16} />}>Captures</NavLink>
          <NavLink href="/devices" icon={<Smartphone size={16} />}>Devices</NavLink>
          <NavLink href="/rules" icon={<Shuffle size={16} />}>Rules</NavLink>
          <NavLink href="/settings" icon={<Settings size={16} />}>Settings</NavLink>
          <NavLink href="/about" icon={<Info size={16} />}>About</NavLink>

          <Show when={filters().length > 0}>
            <div class="mt-4 px-2 text-xs uppercase tracking-wide text-fg-muted">Filters</div>
            <For each={filters()}>
              {(f) => (
                <div
                  class="group px-2 py-1 rounded text-sm hover:bg-bg-muted cursor-pointer flex items-center gap-2"
                  title={`Apply "${f.query}"`}
                  onClick={() => setFilter(f.query)}
                >
                  <FilterIcon size={14} style={{ color: f.color }} />
                  <span class="truncate flex-1">{f.name}</span>
                  <button
                    class="opacity-0 group-hover:opacity-100 hover:text-danger shrink-0"
                    title="Delete filter"
                    onClick={(e) => {
                      e.stopPropagation();
                      if (confirm(`Delete filter "${f.name}"?`)) {
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
        <div class="p-3 border-t border-border space-y-2">
          <button
            class={`w-full inline-flex items-center justify-center gap-2 rounded px-3 py-2 text-sm font-medium transition
              ${status()?.running ? "bg-danger/15 text-danger hover:bg-danger/25" : "bg-accent text-white hover:opacity-90"}`}
            onClick={toggleProxy}
          >
            {status()?.running ? <Square size={14} /> : <Play size={14} />}
            {status()?.running ? "Stop proxy" : "Start proxy"}
          </button>
          <div class="text-xs text-fg-muted text-center">
            {status()?.running ? status()?.listen ?? "running" : "stopped"}
            <span class="mx-1">·</span>
            <span>{status()?.captures_count ?? 0} captures</span>
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
