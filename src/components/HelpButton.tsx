import { type Component, Show } from "solid-js";
import { HelpCircle } from "lucide-solid";
import { open } from "@tauri-apps/plugin-shell";

const DOCS_BASE = "https://pane.thothlab.tech/docs";

export function docsUrl(path: string): string {
  if (!path || path === "/") return `${DOCS_BASE}/`;
  const trimmed = path.startsWith("/") ? path : `/${path}`;
  return `${DOCS_BASE}${trimmed}`;
}

type Props = {
  path: string;
  title?: string;
  size?: number;
  class?: string;
  label?: string;
};

const HelpButton: Component<Props> = (p) => {
  const handle = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    void open(docsUrl(p.path));
  };
  return (
    <button
      type="button"
      onClick={handle}
      title={p.title ?? "Open documentation"}
      class={`inline-flex items-center gap-1 text-fg-muted hover:text-accent transition ${p.class ?? ""}`}
    >
      <HelpCircle size={p.size ?? 14} />
      <Show when={p.label}>
        <span class="text-xs">{p.label}</span>
      </Show>
    </button>
  );
};

export default HelpButton;
