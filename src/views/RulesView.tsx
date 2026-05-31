import { type Component, createSignal, createMemo, For, Show, onMount } from "solid-js";
import { Plus, Trash2, Pencil, Shuffle, X, Check } from "lucide-solid";
import { api } from "@/ipc/client";
import type { RuleDto, RuleUpsertArgs, RuleHeaderDto, RuleQueryParamDto } from "@/ipc/types";

const RulesView: Component = () => {
  const [rules, setRules] = createSignal<RuleDto[]>([]);
  const [editingId, setEditingId] = createSignal<string | "new" | null>(null);
  const [loading, setLoading] = createSignal(true);

  const refresh = async () => {
    setRules(await api.rules.list());
  };

  onMount(async () => {
    try {
      await refresh();
    } finally {
      setLoading(false);
    }
  });

  const startNew = () => setEditingId("new");
  const startEdit = (id: string) => setEditingId(id);
  const cancelEdit = () => setEditingId(null);

  const onSaved = async (saved: RuleDto) => {
    await refresh();
    setEditingId(saved.id);
  };

  const toggle = async (rule: RuleDto) => {
    await api.rules.setEnabled(rule.id, !rule.enabled);
    await refresh();
  };

  const remove = async (rule: RuleDto) => {
    if (!confirm(`Delete rule "${rule.name}"?`)) return;
    await api.rules.delete(rule.id);
    if (editingId() === rule.id) setEditingId(null);
    await refresh();
  };

  return (
    <div class="h-full grid grid-rows-[auto_1fr]">
      <header class="border-b border-border bg-bg-subtle px-4 py-3 flex items-center gap-3">
        <Shuffle size={16} class="text-accent" />
        <div>
          <div class="text-sm font-semibold">Response stubs</div>
          <div class="text-xs text-fg-muted">
            Match incoming requests and serve a canned response instead of forwarding upstream.
          </div>
        </div>
        <button
          class="ml-auto inline-flex items-center gap-1 bg-accent text-white text-sm rounded px-3 py-1.5 hover:opacity-90"
          onClick={startNew}
        >
          <Plus size={14} /> New rule
        </button>
      </header>

      <div class="overflow-auto p-4 space-y-3">
        <Show when={editingId() === "new"}>
          <RuleEditor
            initial={null}
            onCancel={cancelEdit}
            onSaved={onSaved}
          />
        </Show>

        <Show when={!loading() && rules().length === 0 && editingId() !== "new"}>
          <div class="text-center text-fg-muted text-sm py-12">
            No rules yet. Click <span class="text-accent">New rule</span> to add one.
          </div>
        </Show>

        <For each={rules()}>
          {(rule) => (
            <Show
              when={editingId() === rule.id}
              fallback={
                <RuleRow rule={rule} onToggle={() => toggle(rule)} onEdit={() => startEdit(rule.id)} onDelete={() => remove(rule)} />
              }
            >
              <RuleEditor initial={rule} onCancel={cancelEdit} onSaved={onSaved} />
            </Show>
          )}
        </For>
      </div>
    </div>
  );
};

const RuleRow: Component<{
  rule: RuleDto;
  onToggle: () => void;
  onEdit: () => void;
  onDelete: () => void;
}> = (p) => {
  const summary = () => {
    const r = p.rule;
    const m = r.match_method ?? "ANY";
    const h = r.match_host_glob ?? "*";
    const path = r.match_path_glob ?? "*";
    return `${m} ${h}${path}`;
  };
  return (
    <div class={`border rounded p-3 flex items-start gap-3 ${p.rule.enabled ? "border-border bg-bg" : "border-border/50 bg-bg-subtle/30 opacity-70"}`}>
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
          <div class="text-xs text-fg-muted">priority {p.rule.priority}</div>
        </div>
        <div class="text-xs font-mono text-fg-subtle truncate mt-0.5">{summary()}</div>
        <div class="text-xs text-fg-muted mt-1">
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
  name: string;
  enabled: boolean;
  priority: number;
  match_host_glob: string;
  match_method: string;
  match_path_glob: string;
  match_query: RuleQueryParamDto[];
  res_status: number;
  res_headers: RuleHeaderDto[];
  res_body_text: string;
  res_body_mime: string;
  res_delay_ms: number;
};

const emptyDraft = (): DraftState => ({
  id: null,
  name: "",
  enabled: true,
  priority: 0,
  match_host_glob: "",
  match_method: "ANY",
  match_path_glob: "",
  match_query: [],
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
  onCancel: () => void;
  onSaved: (saved: RuleDto) => void;
}> = (p) => {
  const init = (): DraftState => {
    const r = p.initial;
    if (!r) return emptyDraft();
    return {
      id: r.id,
      name: r.name,
      enabled: r.enabled,
      priority: r.priority,
      match_host_glob: r.match_host_glob ?? "",
      match_method: r.match_method ?? "ANY",
      match_path_glob: r.match_path_glob ?? "",
      match_query: r.match_query.slice(),
      res_status: r.res_status,
      res_headers: r.res_headers.length > 0 ? r.res_headers.slice() : [{ name: "Content-Type", value: "application/json" }],
      res_body_text: "",
      res_body_mime: r.res_body_mime ?? "application/json",
      res_delay_ms: r.res_delay_ms,
    };
  };
  const [d, setD] = createSignal<DraftState>(init());
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);

  const patch = (p: Partial<DraftState>) => setD({ ...d(), ...p });

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
        name: draft.name || "Unnamed rule",
        enabled: draft.enabled,
        priority: draft.priority,
        match_host_glob: draft.match_host_glob.trim() ? draft.match_host_glob.trim() : null,
        match_method: draft.match_method === "ANY" ? null : draft.match_method,
        match_path_glob: draft.match_path_glob.trim() ? draft.match_path_glob.trim() : null,
        match_query: draft.match_query.filter((q) => q.name.length > 0),
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
    <div class="border border-accent/40 rounded p-4 bg-bg-subtle/40 space-y-4">
      <div class="flex items-center gap-2">
        <input
          class="flex-1 bg-bg border border-border rounded px-2 py-1 text-sm"
          placeholder="Rule name"
          value={d().name}
          onInput={(e) => patch({ name: e.currentTarget.value })}
        />
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
        <FieldRow label="Query (all required)">
          <div class="flex-1 space-y-1">
            <For each={d().match_query}>
              {(q, i) => (
                <div class="flex items-center gap-2">
                  <input
                    class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder="name"
                    value={q.name}
                    onInput={(e) => {
                      const arr = d().match_query.slice();
                      arr[i()] = { ...arr[i()], name: e.currentTarget.value };
                      patch({ match_query: arr });
                    }}
                  />
                  <input
                    class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder="value"
                    value={q.value}
                    onInput={(e) => {
                      const arr = d().match_query.slice();
                      arr[i()] = { ...arr[i()], value: e.currentTarget.value };
                      patch({ match_query: arr });
                    }}
                  />
                  <button
                    class="text-fg-muted hover:text-danger"
                    onClick={() => patch({ match_query: d().match_query.filter((_, j) => j !== i()) })}
                  >
                    <X size={12} />
                  </button>
                </div>
              )}
            </For>
            <button
              class="text-xs text-accent hover:underline"
              onClick={() => patch({ match_query: [...d().match_query, { name: "", value: "" }] })}
            >
              + add query param
            </button>
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
                    onClick={() => patch({ res_headers: d().res_headers.filter((_, j) => j !== i()) })}
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
            <Show when={existingBodyId() && d().res_body_text.length === 0}>
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
        <button class="text-sm px-3 py-1.5 rounded hover:bg-bg-muted text-fg-muted" onClick={p.onCancel} disabled={busy()}>
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
    <div class="w-32 text-xs text-fg-muted pt-1.5 shrink-0">{p.label}</div>
    <div class="flex-1 flex items-center gap-2 flex-wrap">{p.children}</div>
  </div>
);

export default RulesView;
