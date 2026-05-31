import { type Component, createSignal, createMemo, For, Show, onMount } from "solid-js";
import {
  Plus,
  Trash2,
  Pencil,
  Shuffle,
  X,
  Check,
  ChevronDown,
  ChevronRight,
  FolderPlus,
} from "lucide-solid";
import { api } from "@/ipc/client";
import type {
  RuleDto,
  RuleUpsertArgs,
  RuleHeaderDto,
  RuleParamDto,
  RuleCollectionDto,
} from "@/ipc/types";

const UNGROUPED_KEY = "__ungrouped__";

type Editing = { kind: "rule"; collectionId: string | null; id: string | "new" } | null;

const RulesView: Component = () => {
  const [rules, setRules] = createSignal<RuleDto[]>([]);
  const [collections, setCollections] = createSignal<RuleCollectionDto[]>([]);
  const [editing, setEditing] = createSignal<Editing>(null);
  const [loading, setLoading] = createSignal(true);
  const [collapsed, setCollapsed] = createSignal<Record<string, boolean>>({});

  const refresh = async () => {
    const [r, c] = await Promise.all([api.rules.list(), api.collections.list()]);
    setRules(r);
    setCollections(c);
  };

  onMount(async () => {
    try {
      await refresh();
    } finally {
      setLoading(false);
    }
  });

  const rulesByCollection = createMemo(() => {
    const map = new Map<string, RuleDto[]>();
    for (const r of rules()) {
      const k = r.collection_id ?? UNGROUPED_KEY;
      const arr = map.get(k) ?? [];
      arr.push(r);
      map.set(k, arr);
    }
    return map;
  });

  const newCollection = async () => {
    const name = prompt("Collection name", "New collection");
    if (!name?.trim()) return;
    await api.collections.upsert({
      name: name.trim(),
      enabled: true,
      priority: collections().length,
    });
    await refresh();
  };

  const renameCollection = async (c: RuleCollectionDto) => {
    const name = prompt("Collection name", c.name);
    if (!name?.trim() || name === c.name) return;
    await api.collections.upsert({
      id: c.id,
      name: name.trim(),
      enabled: c.enabled,
      priority: c.priority,
    });
    await refresh();
  };

  const deleteCollection = async (c: RuleCollectionDto) => {
    if (!confirm(`Delete collection "${c.name}"? Its rules will move to Ungrouped.`)) return;
    await api.collections.delete(c.id);
    await refresh();
  };

  const toggleCollection = async (c: RuleCollectionDto) => {
    await api.collections.setEnabled(c.id, !c.enabled);
    await refresh();
  };

  const toggleRule = async (r: RuleDto) => {
    await api.rules.setEnabled(r.id, !r.enabled);
    await refresh();
  };

  const removeRule = async (r: RuleDto) => {
    if (!confirm(`Delete rule "${r.name}"?`)) return;
    await api.rules.delete(r.id);
    const ed = editing();
    if (ed?.kind === "rule" && ed.id === r.id) setEditing(null);
    await refresh();
  };

  const startNewRule = (collectionId: string | null) =>
    setEditing({ kind: "rule", collectionId, id: "new" });
  const startEditRule = (r: RuleDto) =>
    setEditing({ kind: "rule", collectionId: r.collection_id, id: r.id });
  const cancelEdit = () => setEditing(null);

  const onRuleSaved = async (saved: RuleDto) => {
    await refresh();
    setEditing({ kind: "rule", collectionId: saved.collection_id, id: saved.id });
  };

  const isCollapsed = (k: string) => collapsed()[k] === true;
  const toggleSection = (k: string) =>
    setCollapsed({ ...collapsed(), [k]: !collapsed()[k] });

  return (
    <div class="h-full grid grid-rows-[auto_1fr]">
      <header class="border-b border-border bg-bg-subtle px-4 py-3 flex items-center gap-3">
        <Shuffle size={16} class="text-accent" />
        <div>
          <div class="text-sm font-semibold">Response stubs</div>
          <div class="text-xs text-fg-muted">
            Group rules into collections. A rule fires only when both the rule and its collection
            are enabled.
          </div>
        </div>
        <button
          class="ml-auto inline-flex items-center gap-1 text-sm rounded px-3 py-1.5 border border-border hover:bg-bg-muted"
          onClick={newCollection}
        >
          <FolderPlus size={14} /> New collection
        </button>
      </header>

      <div class="overflow-auto p-4 space-y-4">
        <For each={collections()}>
          {(c) => (
            <CollectionSection
              collection={c}
              rules={rulesByCollection().get(c.id) ?? []}
              collapsed={isCollapsed(c.id)}
              onToggleCollapsed={() => toggleSection(c.id)}
              onToggleEnabled={() => toggleCollection(c)}
              onRename={() => renameCollection(c)}
              onDelete={() => deleteCollection(c)}
              onAddRule={() => startNewRule(c.id)}
              onEditRule={startEditRule}
              onToggleRule={toggleRule}
              onDeleteRule={removeRule}
              editing={editing()}
              onSaved={onRuleSaved}
              onCancel={cancelEdit}
              collections={collections()}
            />
          )}
        </For>

        <CollectionSection
          collection={null}
          rules={rulesByCollection().get(UNGROUPED_KEY) ?? []}
          collapsed={isCollapsed(UNGROUPED_KEY)}
          onToggleCollapsed={() => toggleSection(UNGROUPED_KEY)}
          onAddRule={() => startNewRule(null)}
          onEditRule={startEditRule}
          onToggleRule={toggleRule}
          onDeleteRule={removeRule}
          editing={editing()}
          onSaved={onRuleSaved}
          onCancel={cancelEdit}
          collections={collections()}
        />

        <Show when={!loading() && rules().length === 0 && collections().length === 0}>
          <div class="text-center text-fg-muted text-sm py-12">
            No rules or collections yet. Create a collection to organize stubs, or add an Ungrouped
            rule.
          </div>
        </Show>
      </div>
    </div>
  );
};

const CollectionSection: Component<{
  collection: RuleCollectionDto | null;
  rules: RuleDto[];
  collapsed: boolean;
  onToggleCollapsed: () => void;
  onToggleEnabled?: () => void;
  onRename?: () => void;
  onDelete?: () => void;
  onAddRule: () => void;
  onEditRule: (r: RuleDto) => void;
  onToggleRule: (r: RuleDto) => void;
  onDeleteRule: (r: RuleDto) => void;
  editing: Editing;
  onSaved: (r: RuleDto) => void;
  onCancel: () => void;
  collections: RuleCollectionDto[];
}> = (p) => {
  const isUngrouped = () => p.collection === null;
  const enabled = () => p.collection?.enabled ?? true;
  const editingNewHere = () => {
    const ed = p.editing;
    return (
      ed?.kind === "rule" && ed.id === "new" && ed.collectionId === (p.collection?.id ?? null)
    );
  };

  return (
    <section
      class={`border rounded ${enabled() ? "border-border" : "border-border/40 bg-bg-subtle/30"}`}
    >
      <header class="flex items-center gap-2 px-3 py-2">
        <button
          class="text-fg-muted hover:text-fg shrink-0"
          onClick={p.onToggleCollapsed}
          aria-label={p.collapsed ? "Expand" : "Collapse"}
        >
          <Show when={p.collapsed} fallback={<ChevronDown size={14} />}>
            <ChevronRight size={14} />
          </Show>
        </button>

        <Show when={!isUngrouped()}>
          <button
            class={`w-9 h-5 rounded-full relative transition shrink-0 ${enabled() ? "bg-accent" : "bg-bg-muted"}`}
            title={enabled() ? "Disable collection" : "Enable collection"}
            onClick={p.onToggleEnabled}
          >
            <span
              class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition"
              style={{ left: enabled() ? "1.125rem" : "0.125rem" }}
            />
          </button>
        </Show>

        <div class="font-medium text-sm">{p.collection?.name ?? "Ungrouped"}</div>
        <div class="text-xs text-fg-muted">({p.rules.length})</div>

        <div class="ml-auto flex items-center gap-1">
          <button
            class="text-xs px-2 py-1 rounded hover:bg-bg-muted inline-flex items-center gap-1"
            onClick={p.onAddRule}
          >
            <Plus size={12} /> Rule
          </button>
          <Show when={!isUngrouped()}>
            <button
              class="text-xs p-1 rounded hover:bg-bg-muted text-fg-muted"
              title="Rename"
              onClick={p.onRename}
            >
              <Pencil size={12} />
            </button>
            <button
              class="text-xs p-1 rounded hover:bg-danger/10 text-danger"
              title="Delete collection"
              onClick={p.onDelete}
            >
              <Trash2 size={12} />
            </button>
          </Show>
        </div>
      </header>

      <Show when={!p.collapsed}>
        <div class="px-3 pb-3 space-y-2">
          <Show when={editingNewHere()}>
            <RuleEditor
              initial={null}
              defaultCollectionId={p.collection?.id ?? null}
              collections={p.collections}
              onCancel={p.onCancel}
              onSaved={p.onSaved}
            />
          </Show>

          <Show when={p.rules.length === 0 && !editingNewHere()}>
            <div class="text-xs text-fg-muted italic px-2 py-2">No rules in this collection.</div>
          </Show>

          <For each={p.rules}>
            {(rule) => (
              <Show
                when={p.editing?.kind === "rule" && p.editing.id === rule.id}
                fallback={
                  <RuleRow
                    rule={rule}
                    collectionEnabled={enabled()}
                    onToggle={() => p.onToggleRule(rule)}
                    onEdit={() => p.onEditRule(rule)}
                    onDelete={() => p.onDeleteRule(rule)}
                  />
                }
              >
                <RuleEditor
                  initial={rule}
                  defaultCollectionId={rule.collection_id}
                  collections={p.collections}
                  onCancel={p.onCancel}
                  onSaved={p.onSaved}
                />
              </Show>
            )}
          </For>
        </div>
      </Show>
    </section>
  );
};

const RuleRow: Component<{
  rule: RuleDto;
  collectionEnabled: boolean;
  onToggle: () => void;
  onEdit: () => void;
  onDelete: () => void;
}> = (p) => {
  const effectivelyOn = () => p.rule.enabled && p.collectionEnabled;
  const summary = () => {
    const r = p.rule;
    const m = r.match_method ?? "ANY";
    const h = r.match_host_glob ?? "*";
    const path = r.match_path_glob ?? "*";
    const params =
      r.match_params.length > 0
        ? ` · ${r.match_params.map((q) => `${q.name}=${q.value}`).join(" & ")}`
        : "";
    return `${m} ${h}${path}${params}`;
  };
  return (
    <div
      class={`border rounded p-2 flex items-start gap-3 ${effectivelyOn() ? "border-border bg-bg" : "border-border/50 bg-bg-subtle/30 opacity-70"}`}
    >
      <button
        class={`mt-0.5 w-9 h-5 rounded-full relative transition shrink-0 ${p.rule.enabled ? "bg-accent" : "bg-bg-muted"}`}
        title={p.rule.enabled ? "Disable" : "Enable"}
        onClick={p.onToggle}
      >
        <span
          class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition"
          style={{ left: p.rule.enabled ? "1.125rem" : "0.125rem" }}
        />
      </button>
      <div class="flex-1 min-w-0">
        <div class="flex items-baseline gap-2">
          <div class="font-medium text-sm truncate">{p.rule.name || "(unnamed)"}</div>
          <Show when={p.rule.enabled && !p.collectionEnabled}>
            <div class="text-xs text-warn" title="Collection disabled">
              · collection off
            </div>
          </Show>
        </div>
        <div class="text-xs font-mono text-fg-subtle truncate mt-0.5">{summary()}</div>
        <div class="text-xs text-fg-muted mt-0.5">
          → {p.rule.res_status} · {p.rule.res_body_mime ?? "no body"} · {p.rule.res_body_size}B
          <Show when={p.rule.res_delay_ms > 0}> · delay {p.rule.res_delay_ms}ms</Show>
        </div>
      </div>
      <div class="flex items-center gap-1 shrink-0">
        <button class="text-xs px-2 py-1 rounded hover:bg-bg-muted text-fg-muted" onClick={p.onEdit}>
          <Pencil size={12} />
        </button>
        <button class="text-xs px-2 py-1 rounded hover:bg-danger/10 text-danger" onClick={p.onDelete}>
          <Trash2 size={12} />
        </button>
      </div>
    </div>
  );
};

// ── Editor ─────────────────────────────────────────────────────────────────

type DraftState = {
  id: string | null;
  collection_id: string | null;
  name: string;
  enabled: boolean;
  priority: number;
  match_host_glob: string;
  match_method: string;
  match_path_glob: string;
  match_params: RuleParamDto[];
  res_status: number;
  res_headers: RuleHeaderDto[];
  res_body_text: string;
  res_body_mime: string;
  res_delay_ms: number;
};

const emptyDraft = (collectionId: string | null): DraftState => ({
  id: null,
  collection_id: collectionId,
  name: "",
  enabled: true,
  priority: 0,
  match_host_glob: "",
  match_method: "ANY",
  match_path_glob: "",
  match_params: [],
  res_status: 200,
  res_headers: [{ name: "Content-Type", value: "application/json; charset=UTF-8" }],
  res_body_text: "",
  res_body_mime: "application/json",
  res_delay_ms: 0,
});

function utf8ToBase64(s: string): string {
  const bytes = new TextEncoder().encode(s);
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin);
}

const RuleEditor: Component<{
  initial: RuleDto | null;
  defaultCollectionId: string | null;
  collections: RuleCollectionDto[];
  onCancel: () => void;
  onSaved: (saved: RuleDto) => void;
}> = (p) => {
  const init = (): DraftState => {
    const r = p.initial;
    if (!r) return emptyDraft(p.defaultCollectionId);
    return {
      id: r.id,
      collection_id: r.collection_id,
      name: r.name,
      enabled: r.enabled,
      priority: r.priority,
      match_host_glob: r.match_host_glob ?? "",
      match_method: r.match_method ?? "ANY",
      match_path_glob: r.match_path_glob ?? "",
      match_params: r.match_params.slice(),
      res_status: r.res_status,
      res_headers:
        r.res_headers.length > 0
          ? r.res_headers.slice()
          : [{ name: "Content-Type", value: "application/json" }],
      res_body_text: "",
      res_body_mime: r.res_body_mime ?? "application/json",
      res_delay_ms: r.res_delay_ms,
    };
  };
  const [d, setD] = createSignal<DraftState>(init());
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);

  const patch = (q: Partial<DraftState>) => setD({ ...d(), ...q });

  const existingBodyId = createMemo(() => p.initial?.res_body_id ?? null);
  const existingBodySize = createMemo(() => p.initial?.res_body_size ?? 0);

  const save = async () => {
    setBusy(true);
    setErr(null);
    try {
      const draft = d();
      const hasNewBody = draft.res_body_text.length > 0;
      const args: RuleUpsertArgs = {
        id: draft.id ?? undefined,
        collection_id: draft.collection_id,
        name: draft.name || "Unnamed rule",
        enabled: draft.enabled,
        priority: draft.priority,
        match_host_glob: draft.match_host_glob.trim() ? draft.match_host_glob.trim() : null,
        match_method: draft.match_method === "ANY" ? null : draft.match_method,
        match_path_glob: draft.match_path_glob.trim() ? draft.match_path_glob.trim() : null,
        match_params: draft.match_params.filter((q) => q.name.length > 0),
        res_status: draft.res_status,
        res_headers: draft.res_headers.filter((h) => h.name.length > 0),
        res_body_id: hasNewBody ? null : existingBodyId(),
        res_body_base64: hasNewBody ? utf8ToBase64(draft.res_body_text) : null,
        res_body_mime: draft.res_body_mime || null,
        res_delay_ms: draft.res_delay_ms,
      };
      const saved = await api.rules.upsert(args);
      p.onSaved(saved);
    } catch (e: any) {
      setErr(e?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="border border-accent/40 rounded p-3 bg-bg-subtle/40 space-y-4">
      <div class="flex items-center gap-2 flex-wrap">
        <input
          class="flex-1 min-w-48 bg-bg border border-border rounded px-2 py-1 text-sm"
          placeholder="Rule name"
          value={d().name}
          onInput={(e) => patch({ name: e.currentTarget.value })}
        />
        <label class="flex items-center gap-1 text-xs text-fg-muted">
          collection
          <select
            class="bg-bg border border-border rounded px-2 py-1 text-sm"
            value={d().collection_id ?? ""}
            onChange={(e) => patch({ collection_id: e.currentTarget.value || null })}
          >
            <option value="">— Ungrouped —</option>
            <For each={p.collections}>{(c) => <option value={c.id}>{c.name}</option>}</For>
          </select>
        </label>
        <label class="flex items-center gap-1 text-xs text-fg-muted">
          priority
          <input
            type="number"
            class="w-16 bg-bg border border-border rounded px-1 py-1 text-sm"
            value={d().priority}
            onInput={(e) => patch({ priority: Number(e.currentTarget.value) || 0 })}
          />
        </label>
        <label class="flex items-center gap-1 text-xs text-fg-muted">
          <input
            type="checkbox"
            checked={d().enabled}
            onChange={(e) => patch({ enabled: e.currentTarget.checked })}
          />
          enabled
        </label>
      </div>

      <Section title="Match">
        <FieldRow label="Host (glob)">
          <input
            class="flex-1 bg-bg border border-border rounded px-2 py-1 text-sm font-mono"
            placeholder="* or rc1.test.dev-og.com or *.dev-og.com"
            value={d().match_host_glob}
            onInput={(e) => patch({ match_host_glob: e.currentTarget.value })}
          />
        </FieldRow>
        <FieldRow label="Method">
          <select
            class="bg-bg border border-border rounded px-2 py-1 text-sm"
            value={d().match_method}
            onChange={(e) => patch({ match_method: e.currentTarget.value })}
          >
            <For each={["ANY", "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]}>
              {(m) => <option value={m}>{m}</option>}
            </For>
          </select>
        </FieldRow>
        <FieldRow label="Path (glob)">
          <input
            class="flex-1 bg-bg border border-border rounded px-2 py-1 text-sm font-mono"
            placeholder="/api/v1/document or /api/v1/*"
            value={d().match_path_glob}
            onInput={(e) => patch({ match_path_glob: e.currentTarget.value })}
          />
        </FieldRow>
        <FieldRow label="Params (all AND, query or body)">
          <div class="flex-1 space-y-1">
            <For each={d().match_params}>
              {(q, i) => (
                <div class="flex items-center gap-2">
                  <input
                    class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder="name"
                    value={q.name}
                    onInput={(e) => {
                      const arr = d().match_params.slice();
                      arr[i()] = { ...arr[i()], name: e.currentTarget.value };
                      patch({ match_params: arr });
                    }}
                  />
                  <input
                    class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder="value"
                    value={q.value}
                    onInput={(e) => {
                      const arr = d().match_params.slice();
                      arr[i()] = { ...arr[i()], value: e.currentTarget.value };
                      patch({ match_params: arr });
                    }}
                  />
                  <button
                    class="text-fg-muted hover:text-danger"
                    onClick={() =>
                      patch({ match_params: d().match_params.filter((_, j) => j !== i()) })
                    }
                  >
                    <X size={12} />
                  </button>
                </div>
              )}
            </For>
            <button
              class="text-xs text-accent hover:underline"
              onClick={() =>
                patch({ match_params: [...d().match_params, { name: "", value: "" }] })
              }
            >
              + add param
            </button>
            <div class="text-xs text-fg-muted italic">
              Each row must be found either in the URL query, or at the top level of a JSON
              request body.
            </div>
          </div>
        </FieldRow>
      </Section>

      <Section title="Response">
        <FieldRow label="Status">
          <input
            type="number"
            class="w-20 bg-bg border border-border rounded px-2 py-1 text-sm"
            value={d().res_status}
            onInput={(e) => patch({ res_status: Number(e.currentTarget.value) || 200 })}
          />
          <label class="flex items-center gap-1 text-xs text-fg-muted ml-3">
            delay (ms)
            <input
              type="number"
              class="w-20 bg-bg border border-border rounded px-2 py-1 text-sm"
              value={d().res_delay_ms}
              onInput={(e) => patch({ res_delay_ms: Number(e.currentTarget.value) || 0 })}
            />
          </label>
        </FieldRow>
        <FieldRow label="Headers">
          <div class="flex-1 space-y-1">
            <For each={d().res_headers}>
              {(h, i) => (
                <div class="flex items-center gap-2">
                  <input
                    class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder="name"
                    value={h.name}
                    onInput={(e) => {
                      const arr = d().res_headers.slice();
                      arr[i()] = { ...arr[i()], name: e.currentTarget.value };
                      patch({ res_headers: arr });
                    }}
                  />
                  <input
                    class="flex-[2] bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder="value"
                    value={h.value}
                    onInput={(e) => {
                      const arr = d().res_headers.slice();
                      arr[i()] = { ...arr[i()], value: e.currentTarget.value };
                      patch({ res_headers: arr });
                    }}
                  />
                  <button
                    class="text-fg-muted hover:text-danger"
                    onClick={() =>
                      patch({ res_headers: d().res_headers.filter((_, j) => j !== i()) })
                    }
                  >
                    <X size={12} />
                  </button>
                </div>
              )}
            </For>
            <button
              class="text-xs text-accent hover:underline"
              onClick={() => patch({ res_headers: [...d().res_headers, { name: "", value: "" }] })}
            >
              + add header
            </button>
          </div>
        </FieldRow>
        <FieldRow label="Body mime">
          <input
            class="flex-1 bg-bg border border-border rounded px-2 py-1 text-sm font-mono"
            placeholder="application/json"
            value={d().res_body_mime}
            onInput={(e) => patch({ res_body_mime: e.currentTarget.value })}
          />
        </FieldRow>
        <FieldRow label="Body">
          <div class="flex-1">
            <textarea
              class="w-full bg-bg border border-border rounded px-2 py-1 text-xs font-mono min-h-32"
              placeholder={existingBodyId() ? "Leave empty to keep the existing body" : "Paste response body here"}
              value={d().res_body_text}
              onInput={(e) => patch({ res_body_text: e.currentTarget.value })}
            />
            <Show when={!!existingBodyId() && d().res_body_text.length === 0}>
              <div class="text-xs text-fg-muted mt-1">
                Keeping existing body ({existingBodySize()}B). Type above to replace.
              </div>
            </Show>
          </div>
        </FieldRow>
      </Section>

      <Show when={err()}>
        <div class="text-xs text-danger">{err()}</div>
      </Show>

      <div class="flex items-center justify-end gap-2 pt-2 border-t border-border">
        <button
          class="text-sm px-3 py-1.5 rounded hover:bg-bg-muted text-fg-muted"
          onClick={p.onCancel}
          disabled={busy()}
        >
          Cancel
        </button>
        <button
          class="text-sm px-3 py-1.5 rounded bg-accent text-white hover:opacity-90 inline-flex items-center gap-1"
          onClick={save}
          disabled={busy()}
        >
          <Check size={14} /> {busy() ? "Saving…" : "Save"}
        </button>
      </div>
    </div>
  );
};

const Section: Component<{ title: string; children: any }> = (p) => (
  <div>
    <div class="text-xs uppercase tracking-wide text-fg-muted mb-2">{p.title}</div>
    <div class="space-y-2">{p.children}</div>
  </div>
);

const FieldRow: Component<{ label: string; children: any }> = (p) => (
  <div class="flex items-start gap-3">
    <div class="w-40 text-xs text-fg-muted pt-1.5 shrink-0">{p.label}</div>
    <div class="flex-1 flex items-center gap-2 flex-wrap">{p.children}</div>
  </div>
);

export default RulesView;
