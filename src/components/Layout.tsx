import { type ParentComponent, createSignal, onMount, For, Show } from "solid-js";
import { A } from "@solidjs/router";
import { Activity, Smartphone, Settings, Info, Play, Square, Filter as FilterIcon } from "lucide-solid";
import { api } from "@/ipc/client";
import type { FilterDto, ProxyStatusDto } from "@/ipc/types";

const Layout: ParentComponent = (props) => {
  const [status, setStatus] = createSignal<ProxyStatusDto | null>(null);
  const [filters, setFilters] = createSignal<FilterDto[]>([]);

  const refresh = async () => {
    try {
      setStatus(await api.proxy.status());
      setFilters(await api.filters.list());
    } catch (e) {
      console.warn("status refresh failed", e);
    }
  };

  onMount(() => {
    refresh();
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
    <div class="h-full grid grid-cols-[240px_1fr] bg-bg text-fg">
      <aside class="border-r border-border bg-bg-subtle flex flex-col">
        <div class="px-4 py-4 border-b border-border">
          <div class="font-semibold text-lg">my-charles</div>
          <div class="text-xs text-fg-muted">v0.1.0-dev</div>
        </div>
        <nav class="flex-1 overflow-auto p-2 space-y-1">
          <NavLink href="/" icon={<Activity size={16} />}>Captures</NavLink>
          <NavLink href="/devices" icon={<Smartphone size={16} />}>Devices</NavLink>
          <NavLink href="/settings" icon={<Settings size={16} />}>Settings</NavLink>
          <NavLink href="/about" icon={<Info size={16} />}>About</NavLink>

          <Show when={filters().length > 0}>
            <div class="mt-4 px-2 text-xs uppercase tracking-wide text-fg-muted">Filters</div>
            <For each={filters()}>
              {(f) => (
                <div class="px-2 py-1 rounded text-sm hover:bg-bg-muted cursor-pointer flex items-center gap-2">
                  <FilterIcon size={14} style={{ color: f.color }} />
                  <span class="truncate">{f.name}</span>
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
