import { type Component, createSignal, createMemo, For, Index, Show, onMount } from "solid-js";
import {
  Plus,
  Trash2,
  Pencil,
  Shuffle,
  X,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  FolderPlus,
} from "lucide-solid";
import { api } from "@/ipc/client";
import HelpButton from "@/components/HelpButton";
import { t, tr } from "@/i18n";
import type {
  RuleDto,
  RuleUpsertArgs,
  RuleHeaderDto,
  RuleParamDto,
  RulePatchOpDto,
  RulePatchOpKind,
  RuleMode,
  RuleCollectionDto,
} from "@/ipc/types";

const UNGROUPED_KEY = "__ungrouped__";

/// Spread onto every editable <input>/<textarea> in this view to disable the
/// browser's autocorrect / smart-quote substitution / spellcheck / autocomplete
/// suggestions. Smart quotes on macOS otherwise mangle JSON bodies and rule
/// names ("error" -> «error», etc.).
const NO_AC = {
  autocomplete: "off",
  autocorrect: "off",
  autocapitalize: "off",
  spellcheck: false,
} as const;

type Editing = { kind: "rule"; collectionId: string | null; id: string | "new" } | null;

const RulesView: Component = () => {
  const [rules, setRules] = createSignal<RuleDto[]>([]);
  const [collections, setCollections] = createSignal<RuleCollectionDto[]>([]);
  const [editing, setEditing] = createSignal<Editing>(null);
  const [loading, setLoading] = createSignal(true);
  const [collapsed, setCollapsed] = createSignal<Record<string, boolean>>({});

  // Inline collection-creation form. `creatingName === null` means hidden.
  // `creatingCallback` is invoked with the newly-created id (used when the
  // form is opened from a rule editor's collection dropdown).
  const [creatingName, setCreatingName] = createSignal<string | null>(null);
  const [creatingCallback, setCreatingCallback] = createSignal<
    ((id: string) => void) | null
  >(null);
  const [renamingId, setRenamingId] = createSignal<string | null>(null);
  const [renamingName, setRenamingName] = createSignal("");

  // Drag state. `dragOverKey` highlights the section currently under the
  // pointer; UNGROUPED_KEY is used for the Ungrouped section.
  const [draggingRuleId, setDraggingRuleId] = createSignal<string | null>(null);
  const [dragOverKey, setDragOverKey] = createSignal<string | null>(null);

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

  const openCreateForm = (cb?: (id: string) => void) => {
    setCreatingCallback(() => cb ?? null);
    setCreatingName("");
  };

  const cancelCreate = () => {
    setCreatingName(null);
    setCreatingCallback(null);
  };

  const confirmCreate = async () => {
    const n = creatingName()?.trim();
    if (!n) {
      cancelCreate();
      return;
    }
    const saved = await api.collections.upsert({
      name: n,
      enabled: true,
      priority: collections().length,
    });
    const cb = creatingCallback();
    cancelCreate();
    await refresh();
    if (cb) cb(saved.id);
  };

  const startRename = (c: RuleCollectionDto) => {
    setRenamingId(c.id);
    setRenamingName(c.name);
  };

  const confirmRename = async () => {
    const id = renamingId();
    const name = renamingName().trim();
    if (!id || !name) {
      setRenamingId(null);
      return;
    }
    const c = collections().find((x) => x.id === id);
    if (c && name !== c.name) {
      await api.collections.upsert({
        id: c.id,
        name,
        enabled: c.enabled,
        priority: c.priority,
      });
      await refresh();
    }
    setRenamingId(null);
  };

  const deleteCollection = async (c: RuleCollectionDto) => {
    if (!confirm(tr("rules.delete_collection_confirm", { name: c.name }))) return;
    await api.collections.delete(c.id);
    await refresh();
  };

  const toggleRule = async (r: RuleDto) => {
    await api.rules.setEnabled(r.id, !r.enabled);
    await refresh();
  };

  const moveRule = async (r: RuleDto, collectionId: string | null) => {
    if (r.collection_id === collectionId) return;
    await api.rules.upsert({
      id: r.id,
      collection_id: collectionId,
      name: r.name,
      enabled: r.enabled,
      priority: r.priority,
      mode: r.mode,
      patches: r.patches,
      match_host_glob: r.match_host_glob,
      match_method: r.match_method,
      match_path_glob: r.match_path_glob,
      match_params: r.match_params,
      res_status: r.res_status,
      res_headers: r.res_headers,
      res_body_id: r.res_body_id,
      res_body_base64: null,
      res_body_mime: r.res_body_mime,
      res_delay_ms: r.res_delay_ms,
    });
    await refresh();
  };

  const removeRule = async (r: RuleDto) => {
    if (!confirm(tr("rules.delete_rule_confirm", { name: r.name }))) return;
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

  // Drag handlers shared by every CollectionSection.
  const onDragStartRule = (id: string) => setDraggingRuleId(id);
  const onDragEndRule = () => {
    setDraggingRuleId(null);
    setDragOverKey(null);
  };
  const onDragOverSection = (key: string) => setDragOverKey(key);
  const onDragLeaveSection = () => setDragOverKey(null);
  const onDropOnSection = async (key: string, ruleId: string | null) => {
    setDragOverKey(null);
    const id = ruleId ?? draggingRuleId();
    if (!id) return;
    const rule = rules().find((r) => r.id === id);
    if (!rule) return;
    const targetCollectionId = key === UNGROUPED_KEY ? null : key;
    await moveRule(rule, targetCollectionId);
  };

  return (
    <div class="h-full grid grid-rows-[auto_1fr]">
      <header class="border-b border-border bg-bg-subtle px-4 py-3 flex items-center gap-3">
        <Shuffle size={16} class="text-accent" />
        <div>
          <div class="text-sm font-semibold flex items-center gap-1.5">
            {t()("rules.title")}
            <HelpButton path="/rules/" title={t()("rules.help_title")} />
          </div>
        </div>
        <button
          class="ml-auto inline-flex items-center gap-1 text-sm rounded px-3 py-1.5 border border-border hover:bg-bg-muted"
          onClick={() => openCreateForm()}
        >
          <FolderPlus size={14} /> {t()("rules.new_collection")}
        </button>
      </header>

      <div class="overflow-auto p-4 space-y-4">
        <Show when={creatingName() !== null}>
          <div class="border border-accent/40 rounded p-3 bg-bg-subtle/40 flex items-center gap-2">
            <FolderPlus size={14} class="text-accent shrink-0" />
            <input {...NO_AC}
              ref={(el) => setTimeout(() => el?.focus(), 0)}
              class="flex-1 bg-bg border border-border rounded px-2 py-1 text-sm"
              placeholder={t()("rules.new_collection")}
              value={creatingName() ?? ""}
              onInput={(e) => setCreatingName(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void confirmCreate();
                }
                if (e.key === "Escape") cancelCreate();
              }}
            />
            <button
              class="text-sm px-3 py-1 rounded bg-accent text-white hover:opacity-90 disabled:opacity-50"
              disabled={!creatingName()?.trim()}
              onClick={confirmCreate}
            >
              {t()("rules.save")}
            </button>
            <button
              class="text-sm px-3 py-1 rounded hover:bg-bg-muted text-fg-muted"
              onClick={cancelCreate}
            >
              {t()("rules.cancel")}
            </button>
          </div>
        </Show>
        <For each={collections()}>
          {(c) => (
            <CollectionSection
              collection={c}
              rules={rulesByCollection().get(c.id) ?? []}
              collapsed={isCollapsed(c.id)}
              onToggleCollapsed={() => toggleSection(c.id)}
              onRename={() => startRename(c)}
              onDelete={() => deleteCollection(c)}
              onAddRule={() => startNewRule(c.id)}
              onEditRule={startEditRule}
              onToggleRule={toggleRule}
              onDeleteRule={removeRule}
              editing={editing()}
              onSaved={onRuleSaved}
              onCancel={cancelEdit}
                  onStartRename={startRename}
              renamingId={renamingId()}
              renamingName={renamingName()}
              onRenamingNameChange={setRenamingName}
              onConfirmRename={confirmRename}
              onCancelRename={() => setRenamingId(null)}
              draggingRuleId={draggingRuleId()}
              dragOverKey={dragOverKey()}
              onDragStartRule={onDragStartRule}
              onDragEndRule={onDragEndRule}
              onDragOverSection={onDragOverSection}
              onDragLeaveSection={onDragLeaveSection}
              onDropOnSection={(rid) => onDropOnSection(c.id, rid)}
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
          onStartRename={startRename}
          renamingId={renamingId()}
          renamingName={renamingName()}
          onRenamingNameChange={setRenamingName}
          onConfirmRename={confirmRename}
          onCancelRename={() => setRenamingId(null)}
          draggingRuleId={draggingRuleId()}
          dragOverKey={dragOverKey()}
          onDragStartRule={onDragStartRule}
          onDragEndRule={onDragEndRule}
          onDragOverSection={onDragOverSection}
          onDragLeaveSection={onDragLeaveSection}
          onDropOnSection={(rid) => onDropOnSection(UNGROUPED_KEY, rid)}
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
  onRename?: () => void;
  onDelete?: () => void;
  onAddRule: () => void;
  onEditRule: (r: RuleDto) => void;
  onToggleRule: (r: RuleDto) => void;
  onDeleteRule: (r: RuleDto) => void;
  editing: Editing;
  onSaved: (r: RuleDto) => void;
  onCancel: () => void;
  onStartRename: (c: RuleCollectionDto) => void;
  renamingId: string | null;
  renamingName: string;
  onRenamingNameChange: (s: string) => void;
  onConfirmRename: () => void;
  onCancelRename: () => void;
  draggingRuleId: string | null;
  dragOverKey: string | null;
  onDragStartRule: (id: string) => void;
  onDragEndRule: () => void;
  onDragOverSection: (key: string) => void;
  onDragLeaveSection: () => void;
  onDropOnSection: (ruleId: string | null) => void;
}> = (p) => {
  const isUngrouped = () => p.collection === null;
  const sectionKey = () => p.collection?.id ?? UNGROUPED_KEY;
  const isRenaming = () => p.collection !== null && p.renamingId === p.collection.id;
  const isDragOver = () => p.draggingRuleId !== null && p.dragOverKey === sectionKey();
  const editingNewHere = () => {
    const ed = p.editing;
    return (
      ed?.kind === "rule" && ed.id === "new" && ed.collectionId === (p.collection?.id ?? null)
    );
  };

  return (
    <section
      class={`border rounded transition-colors ${
        isDragOver()
          ? "border-accent ring-2 ring-accent/40 bg-accent/5"
          : "border-border"
      }`}
      onDragEnter={(e) => {
        e.preventDefault();
        p.onDragOverSection(sectionKey());
      }}
      onDragOver={(e) => {
        // Unconditionally allow drop. We can't reliably read p.draggingRuleId
        // here in time across native DnD events, so just accept the drop and
        // let onDropOnSection bail out if there's no active rule drag.
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
        p.onDragOverSection(sectionKey());
      }}
      onDragLeave={(e) => {
        // Only clear if leaving the section entirely (not a child element).
        const related = e.relatedTarget as Node | null;
        if (!related || !(e.currentTarget as Node).contains(related)) {
          p.onDragLeaveSection();
        }
      }}
      onDrop={(e) => {
        e.preventDefault();
        e.stopPropagation();
        const id = e.dataTransfer?.getData("text/plain") || null;
        p.onDropOnSection(id);
      }}
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

        <Show
          when={isRenaming()}
          fallback={
            <>
              <div class="font-medium text-sm">{p.collection?.name ?? t()("rules.ungrouped")}</div>
              <div class="text-xs text-fg-muted">({p.rules.length})</div>
            </>
          }
        >
          <input {...NO_AC}
            ref={(el) => setTimeout(() => el?.focus(), 0)}
            class="bg-bg border border-border rounded px-2 py-0.5 text-sm flex-1 max-w-xs"
            value={p.renamingName}
            onInput={(e) => p.onRenamingNameChange(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                p.onConfirmRename();
              }
              if (e.key === "Escape") p.onCancelRename();
            }}
            onBlur={() => p.onConfirmRename()}
          />
        </Show>

        <div class="ml-auto flex items-center gap-1">
          <button
            class="text-xs px-2 py-1 rounded hover:bg-bg-muted inline-flex items-center gap-1"
            onClick={p.onAddRule}
          >
            <Plus size={12} /> {t()("rules.new_rule")}
          </button>
          <Show when={!isUngrouped()}>
            <button
              class="text-xs p-1 rounded hover:bg-bg-muted text-fg-muted"
              title={t()("rules.rename")}
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
                    isDragging={p.draggingRuleId === rule.id}
                    onToggle={() => p.onToggleRule(rule)}
                    onEdit={() => p.onEditRule(rule)}
                    onDelete={() => p.onDeleteRule(rule)}
                    onDragStart={() => p.onDragStartRule(rule.id)}
                    onDragEnd={p.onDragEndRule}
                  />
                }
              >
                <RuleEditor
                  initial={rule}
                  defaultCollectionId={rule.collection_id}
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
  isDragging: boolean;
  onToggle: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onDragStart: () => void;
  onDragEnd: () => void;
}> = (p) => {
  const effectivelyOn = () => p.rule.enabled;
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
      draggable={true}
      onDragStart={(e) => {
        if (e.dataTransfer) {
          e.dataTransfer.effectAllowed = "move";
          e.dataTransfer.setData("text/plain", p.rule.id);
        }
        p.onDragStart();
      }}
      onDragEnd={p.onDragEnd}
      class={`border rounded p-2 flex items-start gap-3 cursor-move ${effectivelyOn() ? "border-border bg-bg" : "border-border/50 bg-bg-subtle/30 opacity-70"} ${p.isDragging ? "opacity-40" : ""}`}
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
          <div class="font-medium text-sm truncate">{p.rule.name || t()("rules.unnamed_rule")}</div>
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
  mode: RuleMode;
  patches: RulePatchOpDto[];
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
  mode: "stub",
  patches: [],
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

/// Parse the patch row's `value` string into a JSON value. Numbers, booleans,
/// null, arrays and objects are recognised; anything else (including bare
/// words) falls back to a plain string. Empty raw value → JSON null. The
/// `delete` op doesn't carry a value.
function normalisePatch(p: RulePatchOpDto): RulePatchOpDto {
  if (p.op === "delete") return { op: "delete", path: p.path };
  const raw = typeof p.value === "string" ? p.value : JSON.stringify(p.value ?? "");
  let value: unknown = raw;
  try {
    value = JSON.parse(raw);
  } catch {
    // not valid JSON → keep as a string
  }
  return { op: p.op, path: p.path, value };
}

/// Parse a single "Name: value" line into a header pair. Returns null if the
/// input doesn't look like a header line (no `:`, or empty name). Used by
/// the header editor's paste handler so users can copy a header from the
/// captures view and paste it as a stub header in one keystroke.
function splitHeaderPair(text: string): RuleHeaderDto | null {
  const line = text.trim().split(/\r?\n/)[0];
  if (!line) return null;
  const idx = line.indexOf(":");
  if (idx <= 0) return null;
  const name = line.slice(0, idx).trim();
  const value = line.slice(idx + 1).trim();
  if (!name) return null;
  return { name, value };
}

function utf8ToBase64(s: string): string {
  const bytes = new TextEncoder().encode(s);
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin);
}

const RuleEditor: Component<{
  initial: RuleDto | null;
  defaultCollectionId: string | null;
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
      mode: r.mode,
      patches: r.patches.slice(),
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
  const [bodyLoading, setBodyLoading] = createSignal(false);

  const patch = (q: Partial<DraftState>) => setD({ ...d(), ...q });

  const existingBodyId = createMemo(() => p.initial?.res_body_id ?? null);
  const existingBodySize = createMemo(() => p.initial?.res_body_size ?? 0);

  // Load the existing response body into the textarea on open so the user
  // sees what's currently stored and can edit it in place.
  onMount(async () => {
    const id = existingBodyId();
    if (!id) return;
    try {
      setBodyLoading(true);
      const body = await api.captures.body(id, 8 * 1024 * 1024);
      const bin = atob(body.bytes_base64);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      const text = new TextDecoder("utf-8", { fatal: false }).decode(bytes);
      // Don't clobber if the user already started typing while we were loading.
      if (d().res_body_text.length === 0) {
        patch({ res_body_text: text });
      }
    } catch (e) {
      console.warn("failed to load existing body for editing", e);
    } finally {
      setBodyLoading(false);
    }
  });

  const save = async () => {
    setBusy(true);
    setErr(null);
    try {
      const draft = d();
      const hasBody = draft.res_body_text.length > 0;
      const args: RuleUpsertArgs = {
        id: draft.id ?? undefined,
        collection_id: draft.collection_id,
        name: draft.name || "Unnamed rule",
        enabled: draft.enabled,
        priority: draft.priority,
        mode: draft.mode,
        patches: draft.mode === "patch"
          ? draft.patches.filter((p) => p.path.trim().length > 0).map(normalisePatch)
          : [],
        match_host_glob: draft.match_host_glob.trim() ? draft.match_host_glob.trim() : null,
        match_method: draft.match_method === "ANY" ? null : draft.match_method,
        match_path_glob: draft.match_path_glob.trim() ? draft.match_path_glob.trim() : null,
        match_params: draft.match_params.filter((q) => q.name.length > 0),
        res_status: draft.res_status,
        res_headers: draft.res_headers.filter((h) => h.name.length > 0),
        // The textarea is pre-filled from existing body on open, so we always
        // send what's in it — non-empty text becomes the new body, empty text
        // clears it. (Storage dedupes identical bodies by sha256.)
        res_body_id: null,
        res_body_base64: hasBody ? utf8ToBase64(draft.res_body_text) : null,
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
        <button
          class="text-fg-muted hover:text-fg shrink-0 p-0.5 rounded hover:bg-bg-muted"
          title="Collapse without saving"
          onClick={p.onCancel}
        >
          <ChevronUp size={14} />
        </button>
        <input {...NO_AC}
          class="flex-1 min-w-48 bg-bg border border-border rounded px-2 py-1 text-sm"
          placeholder="Rule name"
          value={d().name}
          onInput={(e) => patch({ name: e.currentTarget.value })}
        />
        <label class="flex items-center gap-1 text-xs text-fg-muted">
          priority
          <input {...NO_AC}
            type="number"
            class="w-16 bg-bg border border-border rounded px-1 py-1 text-sm"
            value={d().priority}
            onInput={(e) => patch({ priority: Number(e.currentTarget.value) || 0 })}
          />
        </label>
        <label class="flex items-center gap-1 text-xs text-fg-muted">
          <input {...NO_AC}
            type="checkbox"
            checked={d().enabled}
            onChange={(e) => patch({ enabled: e.currentTarget.checked })}
          />
          enabled
        </label>
      </div>

      <Section title="Match">
        <FieldRow label="Host (glob)">
          <input {...NO_AC}
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
          <input {...NO_AC}
            class="flex-1 bg-bg border border-border rounded px-2 py-1 text-sm font-mono"
            placeholder="/api/v1/document or /api/v1/*"
            value={d().match_path_glob}
            onInput={(e) => patch({ match_path_glob: e.currentTarget.value })}
          />
        </FieldRow>
        <FieldRow label="Params (all AND, query or body)">
          <div class="flex-1 space-y-1">
            <Index each={d().match_params}>
              {(q, i) => (
                <div class="flex items-center gap-2">
                  <input {...NO_AC}
                    class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder="name"
                    value={q().name}
                    onInput={(e) => {
                      const arr = d().match_params.slice();
                      arr[i] = { ...arr[i], name: e.currentTarget.value };
                      patch({ match_params: arr });
                    }}
                  />
                  <input {...NO_AC}
                    class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder="value"
                    value={q().value}
                    onInput={(e) => {
                      const arr = d().match_params.slice();
                      arr[i] = { ...arr[i], value: e.currentTarget.value };
                      patch({ match_params: arr });
                    }}
                  />
                  <button
                    class="text-fg-muted hover:text-danger"
                    onClick={() =>
                      patch({ match_params: d().match_params.filter((_, j) => j !== i) })
                    }
                  >
                    <X size={12} />
                  </button>
                </div>
              )}
            </Index>
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
        <FieldRow label="Mode" help={{ path: "/rules/", title: "Stub vs Patch — explained in the docs" }}>
          <select
            class="bg-bg border border-border rounded px-2 py-1 text-sm"
            value={d().mode}
            onChange={(e) => patch({ mode: e.currentTarget.value as RuleMode })}
          >
            <option value="stub">Stub — replace whole response</option>
            <option value="patch">Patch — forward, then mutate</option>
          </select>
          <label class="flex items-center gap-1 text-xs text-fg-muted ml-3">
            delay (ms)
            <input {...NO_AC}
              type="number"
              class="w-20 bg-bg border border-border rounded px-2 py-1 text-sm"
              value={d().res_delay_ms}
              onInput={(e) => patch({ res_delay_ms: Number(e.currentTarget.value) || 0 })}
            />
          </label>
        </FieldRow>
        <Show when={d().mode === "stub"}>
        <FieldRow label="Status">
          <input {...NO_AC}
            type="number"
            class="w-20 bg-bg border border-border rounded px-2 py-1 text-sm"
            value={d().res_status}
            onInput={(e) => patch({ res_status: Number(e.currentTarget.value) || 200 })}
          />
        </FieldRow>
        <FieldRow label="Headers">
          <div class="flex-1 space-y-1">
            <Index each={d().res_headers}>
              {(h, i) => (
                <div class="flex items-center gap-2">
                  <input {...NO_AC}
                    class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder='name (paste "name: value" to split)'
                    value={h().name}
                    onInput={(e) => {
                      const arr = d().res_headers.slice();
                      arr[i] = { ...arr[i], name: e.currentTarget.value };
                      patch({ res_headers: arr });
                    }}
                    onPaste={(e) => {
                      const text = e.clipboardData?.getData("text/plain") ?? "";
                      const split = splitHeaderPair(text);
                      if (split) {
                        e.preventDefault();
                        const arr = d().res_headers.slice();
                        arr[i] = split;
                        patch({ res_headers: arr });
                      }
                    }}
                  />
                  <input {...NO_AC}
                    class="flex-[2] bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder="value"
                    value={h().value}
                    onInput={(e) => {
                      const arr = d().res_headers.slice();
                      arr[i] = { ...arr[i], value: e.currentTarget.value };
                      patch({ res_headers: arr });
                    }}
                    onPaste={(e) => {
                      const text = e.clipboardData?.getData("text/plain") ?? "";
                      const split = splitHeaderPair(text);
                      if (split) {
                        e.preventDefault();
                        const arr = d().res_headers.slice();
                        arr[i] = split;
                        patch({ res_headers: arr });
                      }
                    }}
                  />
                  <button
                    class="text-fg-muted hover:text-danger"
                    onClick={() =>
                      patch({ res_headers: d().res_headers.filter((_, j) => j !== i) })
                    }
                  >
                    <X size={12} />
                  </button>
                </div>
              )}
            </Index>
            <div class="flex items-center gap-3">
              <button
                class="text-xs text-accent hover:underline"
                onClick={() => patch({ res_headers: [...d().res_headers, { name: "", value: "" }] })}
              >
                + add header
              </button>
              <button
                class="text-xs text-accent hover:underline"
                title='Read clipboard, parse "name: value", insert as a new header'
                onClick={async () => {
                  try {
                    const text = await navigator.clipboard.readText();
                    const split = splitHeaderPair(text);
                    if (split) {
                      patch({ res_headers: [...d().res_headers, split] });
                    }
                  } catch {
                    // Clipboard access may be denied; user can fall back to manual paste.
                  }
                }}
              >
                + paste header
              </button>
            </div>
          </div>
        </FieldRow>
        <FieldRow label="Body mime">
          <input {...NO_AC}
            class="flex-1 bg-bg border border-border rounded px-2 py-1 text-sm font-mono"
            placeholder="application/json"
            value={d().res_body_mime}
            onInput={(e) => patch({ res_body_mime: e.currentTarget.value })}
          />
        </FieldRow>
        <FieldRow label="Body">
          <div class="flex-1">
            <textarea {...NO_AC}
              class="w-full bg-bg border border-border rounded px-2 py-1 text-xs font-mono min-h-32"
              placeholder={
                bodyLoading()
                  ? "Loading existing body…"
                  : existingBodyId()
                  ? "Body is empty — type to set a response body"
                  : "Paste response body here"
              }
              disabled={bodyLoading()}
              value={d().res_body_text}
              onInput={(e) => patch({ res_body_text: e.currentTarget.value })}
            />
            <Show when={!!existingBodyId() && !bodyLoading()}>
              <div class="text-xs text-fg-muted mt-1">
                Stored body: {existingBodySize()}B. Edits replace it on save.
              </div>
            </Show>
          </div>
        </FieldRow>
        </Show>
        <Show when={d().mode === "patch"}>
          <FieldRow label="Patches" help={{ path: "/rules/#синтаксис-path", title: "Path syntax, ops, examples" }}>
            <PatchesEditor
              patches={d().patches}
              onChange={(arr) => patch({ patches: arr })}
            />
          </FieldRow>
        </Show>
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
          class="text-sm px-3 py-1.5 rounded bg-accent text-white hover:opacity-90 inline-flex items-center gap-1 disabled:opacity-50"
          onClick={save}
          disabled={busy() || bodyLoading()}
        >
          <Check size={14} /> {busy() ? "Saving…" : bodyLoading() ? "Loading…" : "Save"}
        </button>
      </div>
    </div>
  );
};

const PATCH_PATH_EXAMPLES = "user.fio · list[0] · headers.Content-Type · status";

const PatchesEditor: Component<{
  patches: RulePatchOpDto[];
  onChange: (next: RulePatchOpDto[]) => void;
}> = (p) => {
  const setRow = (i: number, patch: Partial<RulePatchOpDto>) => {
    const arr = p.patches.slice();
    arr[i] = { ...arr[i], ...patch };
    p.onChange(arr);
  };
  const removeRow = (i: number) => p.onChange(p.patches.filter((_, j) => j !== i));
  const addRow = () => p.onChange([...p.patches, { op: "set", path: "", value: "" }]);

  return (
    <div class="flex-1 space-y-1">
      <Index each={p.patches}>
        {(op, i) => (
          <div class="flex items-center gap-2">
            <select
              class="bg-bg border border-border rounded px-2 py-1 text-xs font-mono w-24 shrink-0"
              value={op().op}
              onChange={(e) => setRow(i, { op: e.currentTarget.value as RulePatchOpKind })}
            >
              <option value="set">set</option>
              <option value="delete">delete</option>
              <option value="append">append</option>
            </select>
            <input
              {...NO_AC}
              class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
              placeholder={PATCH_PATH_EXAMPLES}
              value={op().path}
              onInput={(e) => setRow(i, { path: e.currentTarget.value })}
            />
            <Show when={op().op !== "delete"}>
              <input
                {...NO_AC}
                class="flex-[2] bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                placeholder='value (JSON: "X", 777, true, null, {"a":1})'
                value={
                  typeof op().value === "string"
                    ? (op().value as string)
                    : op().value === undefined
                    ? ""
                    : JSON.stringify(op().value)
                }
                onInput={(e) => setRow(i, { value: e.currentTarget.value })}
              />
            </Show>
            <button
              class="text-fg-muted hover:text-danger"
              onClick={() => removeRow(i)}
            >
              <X size={12} />
            </button>
          </div>
        )}
      </Index>
      <button class="text-xs text-accent hover:underline" onClick={addRow}>
        + add patch
      </button>
      <div class="text-xs text-fg-muted italic">
        Path heads: <code>status</code>, <code>headers.&lt;Name&gt;</code>,{" "}
        <code>body.&lt;dot.path&gt;</code> — the <code>body.</code> prefix is optional, so{" "}
        <code>user.fio</code> means the same as <code>body.user.fio</code>. Use <code>[i]</code> for
        array index and <code>[-]</code> to append. Value is parsed as JSON; non-JSON text is
        treated as a string.
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

const FieldRow: Component<{ label: string; help?: { path: string; title?: string }; children: any }> = (p) => (
  <div class="flex items-start gap-3">
    <div class="w-40 text-xs text-fg-muted pt-1.5 shrink-0 inline-flex items-center gap-1">
      {p.label}
      <Show when={p.help}>
        {(h) => <HelpButton path={h().path} title={h().title} size={12} />}
      </Show>
    </div>
    <div class="flex-1 flex items-center gap-2 flex-wrap">{p.children}</div>
  </div>
);

export default RulesView;
