import { type Component, createSignal, createMemo, Show, For } from "solid-js";
import { ChevronRight, ChevronDown, Copy, Check } from "lucide-solid";
import type { CaptureBodyDto } from "@/ipc/types";
import { t } from "@/i18n";

type Mode = "tree" | "pretty" | "raw";

type Kind = "json" | "xml" | "text" | "binary";

function decodeBase64Utf8(b64: string): string | null {
  try {
    const bin = atob(b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
  } catch {
    return null;
  }
}

function detectKind(mime: string | null | undefined, text: string | null): Kind {
  if (text === null) return "binary";
  const m = (mime ?? "").toLowerCase();
  if (m.includes("json")) return "json";
  if (m.includes("xml") || m.includes("html")) return "xml";
  const t = text.trimStart();
  if (t.startsWith("{") || t.startsWith("[")) {
    try {
      JSON.parse(text);
      return "json";
    } catch {
      // fall through
    }
  }
  if (t.startsWith("<")) return "xml";
  return "text";
}

const BodyViewer: Component<{ body: CaptureBodyDto; onLoadFull?: () => void }> = (p) => {
  const text = createMemo(() => decodeBase64Utf8(p.body.bytes_base64));
  const kind = createMemo(() => detectKind(p.body.mime, text()));

  const defaultMode = (): Mode => {
    const k = kind();
    if (k === "json" || k === "xml") return "tree";
    return "raw";
  };
  const [mode, setMode] = createSignal<Mode>(defaultMode());

  const pretty = createMemo(() => {
    const t = text();
    if (t === null) return "";
    if (kind() === "json") {
      try {
        return JSON.stringify(JSON.parse(t), null, 2);
      } catch {
        return t;
      }
    }
    if (kind() === "xml") {
      return formatXml(t);
    }
    return t;
  });

  const [copied, setCopied] = createSignal(false);
  const copyAll = async () => {
    const t = mode() === "pretty" ? pretty() : (text() ?? "");
    await navigator.clipboard.writeText(t);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  return (
    <div>
      <div class="flex items-center gap-2 mb-2 text-fg-muted">
        <span>
          Body · {p.body.mime ?? "?"} · {p.body.total_size}B
          <Show when={p.body.truncated}> · truncated</Show>
        </span>
        <div class="ml-auto flex items-center gap-1">
          <Show when={p.body.truncated && p.onLoadFull}>
            <button
              class="text-xs px-2 py-1 rounded border border-warn/40 text-warn hover:bg-warn/10"
              title="Fetch the rest of the body"
              onClick={() => p.onLoadFull!()}
            >
              Load full ({Math.ceil(p.body.total_size / 1024)} KB)
            </button>
          </Show>
          <ModeToggle mode={mode()} setMode={setMode} kind={kind()} />
          <button
            class="text-xs px-2 py-1 rounded hover:bg-bg-muted flex items-center gap-1"
            title="Copy full body"
            onClick={copyAll}
          >
            <Show when={copied()} fallback={<Copy size={12} />}>
              <Check size={12} class="text-success" />
            </Show>
            Copy
          </button>
        </div>
      </div>

      <Show when={p.body.truncated}>
        <div class="mb-2 px-2 py-1 rounded bg-warn/10 text-warn border border-warn/30 text-xs">
          Body truncated — showing first part only. Tree/Pretty view may be unavailable until full body is loaded.
        </div>
      </Show>

      <Show when={text() === null}>
        <div class="text-fg-muted italic">&lt;binary, {p.body.total_size} bytes&gt;</div>
      </Show>

      <Show when={text() !== null}>
        <Show when={mode() === "tree" && kind() === "json"}>
          <JsonTree raw={text()!} truncated={p.body.truncated} />
        </Show>
        <Show when={mode() === "tree" && kind() === "xml"}>
          <XmlTree raw={text()!} />
        </Show>
        <Show when={mode() === "pretty"}>
          <pre class="whitespace-pre-wrap break-all leading-snug">{pretty()}</pre>
        </Show>
        <Show when={mode() === "raw"}>
          <pre class="whitespace-pre-wrap break-all leading-snug">{text()!}</pre>
        </Show>
      </Show>
    </div>
  );
};

const ModeToggle: Component<{ mode: Mode; setMode: (m: Mode) => void; kind: Kind }> = (p) => {
  const modes = createMemo<{ id: Mode; label: string; disabled: boolean }[]>(() => [
    { id: "tree", label: t()("body.view_tree"), disabled: p.kind !== "json" && p.kind !== "xml" },
    { id: "pretty", label: t()("body.view_pretty"), disabled: false },
    { id: "raw", label: t()("body.view_raw"), disabled: false },
  ]);
  return (
    <div class="flex border border-border rounded overflow-hidden text-xs">
      <For each={modes()}>
        {(m) => (
          <button
            disabled={m.disabled}
            class={`px-2 py-1 ${
              p.mode === m.id
                ? "bg-accent/15 text-accent"
                : m.disabled
                ? "text-fg-muted/50 cursor-not-allowed"
                : "text-fg-muted hover:bg-bg-muted"
            }`}
            onClick={() => !m.disabled && p.setMode(m.id)}
          >
            {m.label}
          </button>
        )}
      </For>
    </div>
  );
};

// ── JSON tree ──────────────────────────────────────────────────────────────

const JsonTree: Component<{ raw: string; truncated: boolean }> = (p) => {
  const parsed = createMemo(() => {
    try {
      return { ok: true as const, value: JSON.parse(p.raw) };
    } catch (e) {
      return { ok: false as const, error: String(e) };
    }
  });
  return (
    <Show
      when={parsed().ok}
      fallback={
        <div class="text-xs">
          <Show
            when={p.truncated}
            fallback={
              <div class="text-danger">
                Invalid JSON: {(parsed() as { error: string }).error}
              </div>
            }
          >
            <div class="text-fg-muted">
              JSON tree unavailable: body is truncated mid-document. Click "Load full" to fetch the
              rest, or switch to Raw.
            </div>
          </Show>
          <pre class="whitespace-pre-wrap break-all leading-snug mt-2 text-fg">{p.raw}</pre>
        </div>
      }
    >
      <div class="leading-snug">
        <JsonNode value={(parsed() as { value: unknown }).value} path="$" depth={0} isLast={true} />
      </div>
    </Show>
  );
};

type JsonNodeProps = {
  name?: string;
  value: unknown;
  path: string;
  depth: number;
  isLast: boolean;
};

const JsonNode: Component<JsonNodeProps> = (p) => {
  const t = jsonTypeOf(p.value);
  const isContainer = t === "object" || t === "array";
  const [open, setOpen] = createSignal(p.depth < 1);

  const keys = createMemo<string[]>(() => {
    if (t === "object") return Object.keys(p.value as Record<string, unknown>);
    if (t === "array") return (p.value as unknown[]).map((_, i) => String(i));
    return [];
  });
  const count = () => keys().length;

  const previewSummary = () => {
    if (t === "array") return `Array(${count()})`;
    if (t === "object") return `Object{${count()}}`;
    return "";
  };

  return (
    <div>
      <div class="group flex items-start gap-1 hover:bg-bg-subtle rounded px-1 -mx-1">
        <button
          class="w-3 h-4 flex items-center justify-center text-fg-muted shrink-0"
          onClick={() => isContainer && setOpen(!open())}
          aria-label={open() ? "Collapse" : "Expand"}
          disabled={!isContainer}
        >
          <Show when={isContainer}>
            <Show when={open()} fallback={<ChevronRight size={10} />}>
              <ChevronDown size={10} />
            </Show>
          </Show>
        </button>

        <div class="flex-1 break-all">
          <Show when={p.name !== undefined}>
            <span class="text-accent">{quoteKey(p.name!)}</span>
            <span class="text-fg-muted">: </span>
          </Show>
          <Show
            when={isContainer}
            fallback={<JsonPrimitive value={p.value} />}
          >
            <Show
              when={!open()}
              fallback={<span class="text-fg-muted">{t === "array" ? "[" : "{"}</span>}
            >
              <span class="text-fg-muted">
                {t === "array" ? "[" : "{"}
                <span class="mx-1 italic text-fg-muted/70">{previewSummary()}</span>
                {t === "array" ? "]" : "}"}
                <Show when={!p.isLast}>,</Show>
              </span>
            </Show>
          </Show>
        </div>

        <CopyButton
          getText={() => {
            if (isContainer) return JSON.stringify(p.value, null, 2);
            return jsonPrimitiveToString(p.value);
          }}
          getPath={() => p.path}
        />
      </div>

      <Show when={isContainer && open()}>
        <div class="ml-4 border-l border-border/60 pl-2">
          <For each={keys()}>
            {(k, i) => {
              const childValue =
                t === "object"
                  ? (p.value as Record<string, unknown>)[k]
                  : (p.value as unknown[])[Number(k)];
              const childPath = t === "array" ? `${p.path}[${k}]` : `${p.path}.${k}`;
              return (
                <JsonNode
                  name={t === "object" ? k : k}
                  value={childValue}
                  path={childPath}
                  depth={p.depth + 1}
                  isLast={i() === keys().length - 1}
                />
              );
            }}
          </For>
        </div>
        <div class="ml-4 text-fg-muted">
          {t === "array" ? "]" : "}"}
          <Show when={!p.isLast}>,</Show>
        </div>
      </Show>
    </div>
  );
};

const JsonPrimitive: Component<{ value: unknown }> = (p) => {
  const t = createMemo(() => jsonTypeOf(p.value));
  return (
    <>
      <Show when={t() === "string"}>
        <span class="text-success break-all">"{escapeJsonStr(String(p.value))}"</span>
      </Show>
      <Show when={t() === "number"}>
        <span class="text-warn">{String(p.value)}</span>
      </Show>
      <Show when={t() === "boolean"}>
        <span class="text-accent">{String(p.value)}</span>
      </Show>
      <Show when={t() === "null"}>
        <span class="text-fg-muted italic">null</span>
      </Show>
    </>
  );
};

function jsonTypeOf(v: unknown): "object" | "array" | "string" | "number" | "boolean" | "null" {
  if (v === null) return "null";
  if (Array.isArray(v)) return "array";
  const t = typeof v;
  if (t === "object") return "object";
  if (t === "number") return "number";
  if (t === "boolean") return "boolean";
  return "string";
}

function jsonPrimitiveToString(v: unknown): string {
  if (typeof v === "string") return v;
  return JSON.stringify(v);
}

function quoteKey(k: string): string {
  return `"${k.replace(/"/g, '\\"')}"`;
}

function escapeJsonStr(s: string): string {
  return s.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

// ── XML tree ───────────────────────────────────────────────────────────────

const XmlTree: Component<{ raw: string }> = (p) => {
  const doc = createMemo(() => {
    const parser = new DOMParser();
    const d = parser.parseFromString(p.raw, "application/xml");
    const err = d.getElementsByTagName("parsererror")[0];
    if (err) {
      const d2 = parser.parseFromString(p.raw, "text/html");
      return { ok: true as const, doc: d2, html: true };
    }
    return { ok: true as const, doc: d, html: false };
  });

  return (
    <div class="leading-snug">
      <For each={Array.from(doc().doc.childNodes).filter(isRenderable)}>
        {(n) => <XmlNode node={n} depth={0} />}
      </For>
    </div>
  );
};

function isRenderable(n: Node): boolean {
  if (n.nodeType === Node.ELEMENT_NODE) return true;
  if (n.nodeType === Node.TEXT_NODE) return !!n.textContent && n.textContent.trim().length > 0;
  if (n.nodeType === Node.COMMENT_NODE) return true;
  if (n.nodeType === Node.CDATA_SECTION_NODE) return true;
  return false;
}

const XmlNode: Component<{ node: Node; depth: number }> = (p) => {
  const nodeType = createMemo(() => p.node.nodeType);
  const el = createMemo(() => (nodeType() === Node.ELEMENT_NODE ? (p.node as Element) : null));
  const children = createMemo(() => {
    const e = el();
    return e ? Array.from(e.childNodes).filter(isRenderable) : [];
  });
  const onlyText = createMemo(() => {
    const c = children();
    return c.length > 0 && c.every((n) => n.nodeType === Node.TEXT_NODE);
  });
  const hasChildren = () => children().length > 0;
  const attrs = createMemo(() => {
    const e = el();
    return e ? Array.from(e.attributes) : [];
  });
  const [open, setOpen] = createSignal(p.depth < 1);

  const renderTagOpen = (selfClose: boolean) => (
    <>
      <span class="text-fg-muted">&lt;</span>
      <span class="text-accent">{el()!.tagName}</span>
      <For each={attrs()}>
        {(a) => (
          <>
            {" "}
            <span class="text-warn">{a.name}</span>
            <span class="text-fg-muted">=</span>
            <span class="text-success">"{a.value}"</span>
          </>
        )}
      </For>
      <span class="text-fg-muted">{selfClose ? " />" : ">"}</span>
    </>
  );

  return (
    <>
      <Show when={nodeType() === Node.TEXT_NODE}>
        <span class="text-fg">{p.node.textContent}</span>
      </Show>
      <Show when={nodeType() === Node.COMMENT_NODE}>
        <div class="text-fg-muted italic">&lt;!-- {p.node.textContent} --&gt;</div>
      </Show>
      <Show when={nodeType() === Node.CDATA_SECTION_NODE}>
        <div class="text-fg-muted">&lt;![CDATA[{p.node.textContent}]]&gt;</div>
      </Show>
      <Show when={el()}>
        <div>
          <div class="group flex items-start gap-1 hover:bg-bg-subtle rounded px-1 -mx-1">
            <button
              class="w-3 h-4 flex items-center justify-center text-fg-muted shrink-0"
              onClick={() => hasChildren() && setOpen(!open())}
              aria-label={open() ? "Collapse" : "Expand"}
              disabled={!hasChildren()}
            >
              <Show when={hasChildren()}>
                <Show when={open()} fallback={<ChevronRight size={10} />}>
                  <ChevronDown size={10} />
                </Show>
              </Show>
            </button>
            <div class="flex-1 break-all">
              <Show when={!hasChildren()}>{renderTagOpen(true)}</Show>
              <Show when={hasChildren() && onlyText() && !open()}>
                {renderTagOpen(false)}
                <span class="text-fg">{el()!.textContent}</span>
                <span class="text-fg-muted">&lt;/</span>
                <span class="text-accent">{el()!.tagName}</span>
                <span class="text-fg-muted">&gt;</span>
              </Show>
              <Show when={hasChildren() && onlyText() && open()}>
                {renderTagOpen(false)}
              </Show>
              <Show when={hasChildren() && !onlyText()}>
                {renderTagOpen(false)}
                <Show when={!open()}>
                  <span class="mx-1 italic text-fg-muted/70">… {children().length} children</span>
                  <span class="text-fg-muted">&lt;/</span>
                  <span class="text-accent">{el()!.tagName}</span>
                  <span class="text-fg-muted">&gt;</span>
                </Show>
              </Show>
            </div>
            <CopyButton
              getText={() => new XMLSerializer().serializeToString(el()!)}
              getPath={() => xmlPath(el()!)}
            />
          </div>

          <Show when={hasChildren() && open()}>
            <div class="ml-4 border-l border-border/60 pl-2">
              <For each={children()}>{(c) => <XmlNode node={c} depth={p.depth + 1} />}</For>
            </div>
            <div class="ml-4 text-fg-muted">
              &lt;/<span class="text-accent">{el()!.tagName}</span>&gt;
            </div>
          </Show>
        </div>
      </Show>
    </>
  );
};

function xmlPath(el: Element): string {
  const parts: string[] = [];
  let cur: Element | null = el;
  while (cur && cur.nodeType === Node.ELEMENT_NODE) {
    let idx = 1;
    let sib = cur.previousElementSibling;
    while (sib) {
      if (sib.tagName === cur.tagName) idx++;
      sib = sib.previousElementSibling;
    }
    parts.unshift(`${cur.tagName}[${idx}]`);
    cur = cur.parentElement;
  }
  return "/" + parts.join("/");
}

function formatXml(src: string): string {
  try {
    const parser = new DOMParser();
    const doc = parser.parseFromString(src, "application/xml");
    if (doc.getElementsByTagName("parsererror").length > 0) return src;
    return prettyPrintXmlNode(doc.documentElement, 0);
  } catch {
    return src;
  }
}

function prettyPrintXmlNode(node: Node, indent: number): string {
  const pad = "  ".repeat(indent);
  if (node.nodeType === Node.TEXT_NODE) {
    const t = node.textContent ?? "";
    return t.trim() ? pad + t.trim() : "";
  }
  if (node.nodeType === Node.COMMENT_NODE) {
    return `${pad}<!--${node.textContent}-->`;
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return "";
  const el = node as Element;
  const attrs = Array.from(el.attributes)
    .map((a) => ` ${a.name}="${a.value}"`)
    .join("");
  const children = Array.from(el.childNodes).filter(isRenderable);
  if (children.length === 0) return `${pad}<${el.tagName}${attrs}/>`;
  if (children.length === 1 && children[0].nodeType === Node.TEXT_NODE) {
    return `${pad}<${el.tagName}${attrs}>${children[0].textContent?.trim() ?? ""}</${el.tagName}>`;
  }
  const inner = children.map((c) => prettyPrintXmlNode(c, indent + 1)).filter(Boolean).join("\n");
  return `${pad}<${el.tagName}${attrs}>\n${inner}\n${pad}</${el.tagName}>`;
}

// ── Copy button ────────────────────────────────────────────────────────────

const CopyButton: Component<{ getText: () => string; getPath: () => string }> = (p) => {
  const [done, setDone] = createSignal(false);
  return (
    <div class="opacity-0 group-hover:opacity-100 flex items-center gap-1 shrink-0">
      <button
        class="text-[10px] px-1 py-0.5 rounded hover:bg-bg-muted text-fg-muted"
        title="Copy path"
        onClick={(e) => {
          e.stopPropagation();
          navigator.clipboard.writeText(p.getPath());
          setDone(true);
          setTimeout(() => setDone(false), 800);
        }}
      >
        path
      </button>
      <button
        class="text-fg-muted hover:text-fg p-0.5 rounded"
        title="Copy value"
        onClick={(e) => {
          e.stopPropagation();
          navigator.clipboard.writeText(p.getText());
          setDone(true);
          setTimeout(() => setDone(false), 800);
        }}
      >
        <Show when={done()} fallback={<Copy size={11} />}>
          <Check size={11} class="text-success" />
        </Show>
      </button>
    </div>
  );
};

export default BodyViewer;
