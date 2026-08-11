import { type Component, createSignal, createMemo, For, Index, Show, ErrorBoundary, onMount, onCleanup } from "solid-js";
import { useBeforeLeave } from "@solidjs/router";
import {
  Plus,
  Trash2,
  Pencil,
  Copy,
  GripVertical,
  Eraser,
  Shuffle,
  X,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  FolderPlus,
  Download,
  Upload,
  Maximize2,
  Minimize2,
  Braces,
  FilePlus,
  Minus,
  AlertTriangle,
} from "lucide-solid";
import { open, save } from "@tauri-apps/plugin-dialog";
import { readClipboard } from "@/lib/clipboard";
import { api } from "@/ipc/client";
import HelpButton from "@/components/HelpButton";
import JsonEditor from "@/components/JsonEditor";
import { t, tr } from "@/i18n";
import {
  applyImport,
  buildCollectionExport,
  buildLibraryExport,
  buildRuleExport,
  slugifyForFilename,
  parseImportFile,
  type PortFile,
} from "@/lib/rules-portfile";
import {
  rulesCollapsed,
  setRulesCollapsed,
  rulesEditing,
  setRulesEditing,
  type RulesEditing,
  ruleDraftKey,
  loadRuleDraft,
  saveRuleDraft,
  clearRuleDraft,
  ruleEditorDirty,
  setRuleEditorDirty,
  ruleEditorPendingNav,
  setRuleEditorPendingNav,
  registerEditorSaveFn,
  triggerEditorSave,
} from "@/stores/rules-ui";
import type {
  RuleDto,
  RuleUpsertArgs,
  RuleHeaderDto,
  RuleParamDto,
  RulePatchOpDto,
  RulePatchOpKind,
  RuleMode,
  RuleConditionDto,
  RuleConditionOp,
  RuleCollectionDto,
} from "@/ipc/types";

const UNGROUPED_KEY = "__ungrouped__";

// Condition operators. Comparison symbols are language-neutral; `contains`
// gets a translated label at render time.
const CONDITION_OPS: { value: RuleConditionOp; symbol: string }[] = [
  { value: "eq", symbol: "=" },
  { value: "ne", symbol: "≠" },
  { value: "gt", symbol: ">" },
  { value: "gte", symbol: "≥" },
  { value: "lt", symbol: "<" },
  { value: "lte", symbol: "≤" },
  { value: "contains", symbol: "contains" },
];

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

/// A param value that's a canonical UUID is almost always a per-request
/// token (requestId, traceId, correlationId…) that changes on every call.
/// The matcher compares param values exactly, so pinning one makes the
/// rule fire for that single captured request only. We flag these in the
/// editor so the user removes them. Stable identifiers like a numeric
/// userId aren't UUID-shaped, so they're left alone.
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
function looksVolatile(value: string): boolean {
  return UUID_RE.test(value.trim());
}

// Human-readable byte size: B under 1 KiB, then KB / MB with one decimal.
function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

// Same UUID shape, but un-anchored — for spotting a per-request token
// buried anywhere inside the JSON request-body matcher. The body editor
// is free text, so we can't flag a single field; we warn on the whole
// blob instead (the anti-silent-match cue that the per-row highlight in
// Params would otherwise provide).
const UUID_ANYWHERE_RE = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;
function bodyHasVolatile(text: string): boolean {
  return UUID_ANYWHERE_RE.test(text);
}

type Editing = RulesEditing;

const RulesView: Component = () => {
  const [rules, setRules] = createSignal<RuleDto[]>([]);
  const [collections, setCollections] = createSignal<RuleCollectionDto[]>([]);
  // `editing` and `collapsed` are hoisted into module-level signals
  // (stores/rules-ui) so they survive view remount across nav and
  // app restart. Local aliases keep the call-sites below unchanged.
  const editing = rulesEditing;
  const setEditing = setRulesEditing;
  const collapsed = rulesCollapsed;
  const setCollapsed = setRulesCollapsed;
  const [loading, setLoading] = createSignal(true);
  // Surfaces a failed rules/collections load instead of silently leaving an
  // empty list (the previous behaviour swallowed the rejection).
  const [loadError, setLoadError] = createSignal<string | null>(null);

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
  // Id of the collection currently being dragged by its header grip, or null.
  // Drives collection-reorder detection via a signal (not dataTransfer.types)
  // because WKWebView doesn't reliably expose custom drag types during
  // dragover — a signal is bulletproof and mirrors draggingRuleId.
  const [draggingCollectionId, setDraggingCollectionId] = createSignal<string | null>(null);

  const refresh = async () => {
    try {
      const [r, c] = await Promise.all([api.rules.list(), api.collections.list()]);
      setRules(r);
      setCollections(c);
      setLoadError(null);
    } catch (e) {
      // Keep the previous list on screen rather than blanking it, and show a
      // retry banner. Without this the rejection was swallowed and the list
      // just vanished (e.g. on remount after a tab switch).
      console.error("rules refresh failed", e);
      setLoadError((e as { message?: string })?.message ?? String(e));
    }
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

  // Reflect an enabled toggle that an open RuleEditor already persisted. We
  // mutate the matching rule in place (keeping its object identity so the
  // <For> doesn't remount the open editor) and hand back a fresh array so the
  // collection-header tri-state memo recomputes.
  const syncRuleEnabled = (id: string, enabled: boolean) => {
    setRules((prev) =>
      prev.map((r) => (r.id === id ? Object.assign(r, { enabled }) : r)),
    );
  };

  // Bulk-toggle every rule in `list` to `targetEnabled`. Issues only the
  // changes that actually differ from the current state, in parallel,
  // then refreshes once. The backend has no batch endpoint for this, so
  // we just fan out IPC calls — even for collections of 50 rules the
  // round-trips finish within a frame on the same machine.
  const toggleCollection = async (list: RuleDto[], targetEnabled: boolean) => {
    const toFlip = list.filter((r) => r.enabled !== targetEnabled);
    if (toFlip.length === 0) return;
    await Promise.all(
      toFlip.map((r) => api.rules.setEnabled(r.id, targetEnabled)),
    );
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
      match_req_body: r.match_req_body,
      match_conditions: r.match_conditions,
      res_status: r.res_status,
      res_headers: r.res_headers,
      res_body_id: r.res_body_id,
      res_body_base64: null,
      res_body_mime: r.res_body_mime,
      res_delay_ms: r.res_delay_ms,
    });
    await refresh();
  };

  // Clone a rule in place: same collection, same everything, name suffixed
  // with " - copy"/" - копия". The stored body is shared by id (storage
  // dedupes bodies by sha256, so referencing res_body_id is safe — no
  // re-upload). Keeping the source priority lands the copy right next to
  // the original in the list.
  const duplicateRule = async (r: RuleDto) => {
    await api.rules.upsert({
      collection_id: r.collection_id,
      name: `${r.name || "Unnamed rule"}${t()("rules.copy_suffix")}`,
      enabled: r.enabled,
      priority: r.priority,
      mode: r.mode,
      patches: r.patches,
      match_host_glob: r.match_host_glob,
      match_method: r.match_method,
      match_path_glob: r.match_path_glob,
      match_params: r.match_params,
      match_req_body: r.match_req_body,
      match_conditions: r.match_conditions,
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
    // Drop any persisted draft for this rule — its key would otherwise
    // orphan in localStorage forever.
    clearRuleDraft(ruleDraftKey(r.id, r.collection_id));
    await refresh();
  };

  // ── Import / export ──────────────────────────────────────────────
  //
  // Three export entry points (one rule, one collection, full
  // library) share the save-dialog → backend-write boilerplate via
  // savePortFile. Import is the inverse: open-dialog → backend-read
  // → validate → applyImport → refresh.
  const savePortFile = async (file: PortFile, defaultName: string) => {
    const path = await save({
      defaultPath: defaultName,
      filters: [{ name: "Pane rules", extensions: ["json"] }],
    });
    if (!path) return;
    try {
      await api.rules.exportWrite(path, JSON.stringify(file, null, 2));
    } catch (e) {
      alert(
        tr("rules.export_failed", {
          message: (e as { message?: string })?.message ?? String(e),
        }),
      );
    }
  };

  const exportRule = async (r: RuleDto) => {
    const file = await buildRuleExport(r);
    await savePortFile(
      file,
      `${slugifyForFilename(r.name || "rule")}.pane-rule.json`,
    );
  };

  const exportCollection = async (c: RuleCollectionDto) => {
    const file = await buildCollectionExport(
      c,
      rulesByCollection().get(c.id) ?? [],
    );
    await savePortFile(
      file,
      `${slugifyForFilename(c.name)}.pane-collection.json`,
    );
  };

  const exportLibrary = async () => {
    const file = await buildLibraryExport(collections(), rules());
    await savePortFile(file, "pane-rules.json");
  };

  const importFile = async () => {
    const path = await open({
      multiple: false,
      filters: [{ name: "Pane rules", extensions: ["json"] }],
    });
    if (!path || typeof path !== "string") return;
    try {
      const raw = await api.rules.importRead(path);
      const parsed = JSON.parse(raw) as unknown;
      // Accepts our own port file and the readable dump shape; see
      // `parseImportFile` for why the second one is worth supporting.
      const validated = parseImportFile(parsed);
      if (typeof validated === "string") {
        alert(tr("rules.import_invalid", { message: validated }));
        return;
      }
      const summary = await applyImport(validated);
      await refresh();
      alert(
        tr("rules.import_success", {
          collections: String(summary.collections),
          rules: String(summary.rules),
        }),
      );
    } catch (e) {
      alert(
        tr("rules.import_failed", {
          message: (e as { message?: string })?.message ?? String(e),
        }),
      );
    }
  };

  // Show the unsaved-changes modal if the editor is dirty; otherwise run
  // `action` immediately. The modal's Save/Cancel buttons drive
  // `saveAndProceed` and `cancelNav`.
  const guard = (action: () => void) => {
    if (ruleEditorDirty()) {
      setRuleEditorPendingNav(() => action);
    } else {
      action();
    }
  };

  const cancelNav = () => setRuleEditorPendingNav(null);

  const discardAndProceed = () => {
    const ed = editing();
    if (ed) clearRuleDraft(ruleDraftKey(ed.id === "new" ? null : ed.id, ed.collectionId));
    setRuleEditorDirty(false);
    const action = ruleEditorPendingNav();
    setRuleEditorPendingNav(null);
    action?.();
  };

  const saveAndProceed = async () => {
    await triggerEditorSave();
    if (!ruleEditorDirty()) {
      const action = ruleEditorPendingNav();
      setRuleEditorPendingNav(null);
      action?.();
    }
  };

  // Intercept tab/route navigation away from this view when dirty.
  useBeforeLeave((e) => {
    if (ruleEditorDirty()) {
      e.preventDefault();
      setRuleEditorPendingNav(() => () => e.retry(true));
    }
  });

  const startNewRule = (collectionId: string | null) =>
    guard(() => setEditing({ kind: "rule", collectionId, id: "new" }));
  const startEditRule = (r: RuleDto) =>
    guard(() => setEditing({ kind: "rule", collectionId: r.collection_id, id: r.id }));
  const cancelEdit = () => setEditing(null);

  const onRuleSaved = (saved: RuleDto) => {
    // Don't `refresh()` here. A full refresh replaces every rule AND every
    // collection object with freshly-deserialised ones, so the open editor's
    // row — and its whole `CollectionSection` (rendered inside the outer
    // `<For each={collections()}>`) — gets recreated. That remounted the
    // RuleEditor, whose onMount re-loads the response body, making the panel
    // flash and jump on every Save.
    //
    // Instead patch the saved rule in place, preserving its object identity
    // (same trick as `syncRuleEnabled`) so the reference-keyed `<For>` reuses
    // the row, and leave `collections()` untouched (a save never changes them)
    // so the outer section list stays stable too.
    setRules((prev) =>
      prev.some((r) => r.id === saved.id)
        ? prev.map((r) => (r.id === saved.id ? Object.assign(r, saved) : r))
        : [...prev, saved],
    );
    setEditing({ kind: "rule", collectionId: saved.collection_id, id: saved.id });
  };

  const isCollapsed = (k: string) => collapsed()[k] === true;
  const toggleSection = (k: string) => {
    const next = { ...collapsed(), [k]: !collapsed()[k] };
    const collapsing = !collapsed()[k];
    const ed = editing();
    if (collapsing && ed && ruleEditorDirty()) {
      const inThisSection = k === (ed.collectionId ?? UNGROUPED_KEY);
      if (inThisSection) {
        guard(() => setCollapsed(next));
        return;
      }
    }
    setCollapsed(next);
  };

  // Drag handlers shared by every CollectionSection.
  const onDragStartRule = (id: string) => setDraggingRuleId(id);
  const onDragEndRule = () => {
    setDraggingRuleId(null);
    setDragOverKey(null);
  };
  const onDragStartCollection = (id: string) => setDraggingCollectionId(id);
  const onDragEndCollection = () => setDraggingCollectionId(null);
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

  // Drop a dragged rule directly onto another rule to reorder it within the
  // same collection. Rule order *is* match precedence (engine takes the first
  // match in priority order), so this lets the user promote/demote a rule by
  // hand. Cross-collection drops still fall through to moveRule — reordering
  // is intentionally a within-list operation.
  const reorderRule = async (
    draggedId: string,
    targetId: string,
    position: "before" | "after",
  ) => {
    setDraggingRuleId(null);
    setDragOverKey(null);
    if (draggedId === targetId) return;
    const dragged = rules().find((r) => r.id === draggedId);
    const target = rules().find((r) => r.id === targetId);
    if (!dragged || !target) return;
    if (dragged.collection_id !== target.collection_id) {
      // Different collection — keep the existing "move into this collection"
      // semantics; positional sort is left to a follow-up drag within it.
      await moveRule(dragged, target.collection_id);
      return;
    }
    const sectionKey = dragged.collection_id ?? UNGROUPED_KEY;
    const current = rulesByCollection().get(sectionKey) ?? [];
    const ordered = current.map((r) => r.id).filter((rid) => rid !== draggedId);
    const targetIdx = ordered.indexOf(targetId);
    if (targetIdx < 0) return;
    ordered.splice(position === "after" ? targetIdx + 1 : targetIdx, 0, draggedId);
    // Renumber to a dense 0..N-1 sequence, but only persist the rows whose
    // priority actually changes (mirrors toggleCollection's diff-then-fan-out).
    const priorityOf = new Map(current.map((r) => [r.id, r.priority]));
    const changed = ordered
      .map((rid, idx) => ({ rid, idx }))
      .filter(({ rid, idx }) => priorityOf.get(rid) !== idx);
    if (changed.length === 0) return;
    await Promise.all(changed.map(({ rid, idx }) => api.rules.setPriority(rid, idx)));
    await refresh();
  };

  // Reorder whole collections by dragging their headers. Collection order is
  // also precedence — list_active_rules sorts by collection priority first —
  // so this lets the user prioritise an entire group. Same diff-then-fan-out
  // shape as reorderRule. The Ungrouped pseudo-section has no collection row
  // and is always rendered last, so it never participates.
  const reorderCollection = async (
    draggedId: string,
    targetId: string,
    position: "before" | "after",
  ) => {
    if (draggedId === targetId) return;
    const ordered = collections().map((c) => c.id).filter((cid) => cid !== draggedId);
    const targetIdx = ordered.indexOf(targetId);
    if (targetIdx < 0) return;
    ordered.splice(position === "after" ? targetIdx + 1 : targetIdx, 0, draggedId);
    const priorityOf = new Map(collections().map((c) => [c.id, c.priority]));
    const changed = ordered
      .map((cid, idx) => ({ cid, idx }))
      .filter(({ cid, idx }) => priorityOf.get(cid) !== idx);
    if (changed.length === 0) return;
    await Promise.all(changed.map(({ cid, idx }) => api.collections.setPriority(cid, idx)));
    await refresh();
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
        <div class="ml-auto flex items-center gap-2">
          <button
            class="inline-flex items-center gap-1 text-sm rounded px-3 py-1.5 border border-border hover:bg-bg-muted"
            onClick={importFile}
            title={t()("rules.import_title")}
          >
            <Upload size={14} /> {t()("rules.import")}
          </button>
          <button
            class="inline-flex items-center gap-1 text-sm rounded px-3 py-1.5 border border-border hover:bg-bg-muted disabled:opacity-40"
            onClick={exportLibrary}
            disabled={rules().length === 0 && collections().length === 0}
            title={t()("rules.export_all_title")}
          >
            <Download size={14} /> {t()("rules.export_all")}
          </button>
          <button
            class="inline-flex items-center gap-1 text-sm rounded px-3 py-1.5 border border-border hover:bg-bg-muted"
            onClick={() => openCreateForm()}
          >
            <FolderPlus size={14} /> {t()("rules.new_collection")}
          </button>
        </div>
      </header>

      <div class="overflow-auto p-4 space-y-4">
        <Show when={loadError()}>
          {(msg) => (
            <div class="flex items-center gap-2 border border-danger/40 bg-danger/10 text-danger rounded px-3 py-2 text-sm">
              <AlertTriangle size={14} class="shrink-0" />
              <span class="flex-1 min-w-0 truncate" title={msg()}>
                {t()("rules.load_error", { error: msg() })}
              </span>
              <button
                class="shrink-0 px-2 py-0.5 rounded border border-danger/40 hover:bg-danger/15"
                onClick={() => void refresh()}
              >
                {t()("rules.load_error_retry")}
              </button>
            </div>
          )}
        </Show>
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
              onExportCollection={() => exportCollection(c)}
              onExportRule={exportRule}
              onAddRule={() => startNewRule(c.id)}
              onEditRule={startEditRule}
              onCopyRule={duplicateRule}
              onReorderRule={reorderRule}
              onReorderCollection={reorderCollection}
              onToggleRule={toggleRule}
              onToggleCollection={(en) => toggleCollection(rulesByCollection().get(c.id) ?? [], en)}
              onDeleteRule={removeRule}
              editing={editing()}
              onSaved={onRuleSaved}
              onCancel={cancelEdit}
              onLiveSync={syncRuleEnabled}
                  onStartRename={startRename}
              renamingId={renamingId()}
              renamingName={renamingName()}
              onRenamingNameChange={setRenamingName}
              onConfirmRename={confirmRename}
              onCancelRename={() => setRenamingId(null)}
              draggingRuleId={draggingRuleId()}
              draggingCollectionId={draggingCollectionId()}
              dragOverKey={dragOverKey()}
              onDragStartRule={onDragStartRule}
              onDragEndRule={onDragEndRule}
              onDragStartCollection={onDragStartCollection}
              onDragEndCollection={onDragEndCollection}
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
          onExportRule={exportRule}
          onAddRule={() => startNewRule(null)}
          onEditRule={startEditRule}
          onCopyRule={duplicateRule}
          onReorderRule={reorderRule}
          onReorderCollection={reorderCollection}
          onToggleRule={toggleRule}
          onToggleCollection={(en) => toggleCollection(rulesByCollection().get(UNGROUPED_KEY) ?? [], en)}
          onDeleteRule={removeRule}
          editing={editing()}
          onSaved={onRuleSaved}
          onCancel={cancelEdit}
          onLiveSync={syncRuleEnabled}
          onStartRename={startRename}
          renamingId={renamingId()}
          renamingName={renamingName()}
          onRenamingNameChange={setRenamingName}
          onConfirmRename={confirmRename}
          onCancelRename={() => setRenamingId(null)}
          draggingRuleId={draggingRuleId()}
          draggingCollectionId={draggingCollectionId()}
          dragOverKey={dragOverKey()}
          onDragStartRule={onDragStartRule}
          onDragEndRule={onDragEndRule}
          onDragStartCollection={onDragStartCollection}
          onDragEndCollection={onDragEndCollection}
          onDragOverSection={onDragOverSection}
          onDragLeaveSection={onDragLeaveSection}
          onDropOnSection={(rid) => onDropOnSection(UNGROUPED_KEY, rid)}
        />

        <Show when={!loading() && rules().length === 0 && collections().length === 0}>
          <div class="text-center text-fg-muted text-sm py-12">
            {t()("rules.empty_state")}
          </div>
        </Show>
      </div>

      <Show when={ruleEditorPendingNav()}>
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div class="bg-bg border border-border rounded-lg p-5 shadow-xl max-w-sm w-full mx-4 space-y-4">
            <div class="flex items-start gap-3">
              <AlertTriangle size={28} class="text-danger shrink-0 mt-0.5" />
              <div class="font-medium text-sm">{t()("rules.unsaved_changes_prompt")}</div>
            </div>
            <div class="flex justify-between gap-2">
              <button
                class="text-sm px-3 py-1.5 rounded hover:bg-bg-muted text-fg-muted"
                onClick={cancelNav}
              >
                {t()("rules.close_dialog")}
              </button>
              <div class="flex gap-2">
                <button
                  class="text-sm px-3 py-1.5 rounded hover:bg-bg-muted text-fg-muted"
                  onClick={discardAndProceed}
                >
                  {t()("rules.no")}
                </button>
                <button
                  class="text-sm px-3 py-1.5 rounded bg-accent text-white hover:opacity-90"
                  onClick={() => void saveAndProceed()}
                >
                  {t()("rules.yes")}
                </button>
              </div>
            </div>
          </div>
        </div>
      </Show>
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
  onCopyRule: (r: RuleDto) => void;
  onReorderRule: (draggedId: string, targetId: string, position: "before" | "after") => void;
  onReorderCollection: (draggedId: string, targetId: string, position: "before" | "after") => void;
  onToggleRule: (r: RuleDto) => void;
  onToggleCollection: (enable: boolean) => void;
  onDeleteRule: (r: RuleDto) => void;
  editing: Editing;
  onSaved: (r: RuleDto) => void | Promise<void>;
  onCancel: () => void;
  onLiveSync: (id: string, enabled: boolean) => void;
  onStartRename: (c: RuleCollectionDto) => void;
  onExportCollection?: () => void;
  onExportRule: (r: RuleDto) => void;
  renamingId: string | null;
  renamingName: string;
  onRenamingNameChange: (s: string) => void;
  onConfirmRename: () => void;
  onCancelRename: () => void;
  draggingRuleId: string | null;
  draggingCollectionId: string | null;
  dragOverKey: string | null;
  onDragStartRule: (id: string) => void;
  onDragEndRule: () => void;
  onDragStartCollection: (id: string) => void;
  onDragEndCollection: () => void;
  onDragOverSection: (key: string) => void;
  onDragLeaveSection: () => void;
  onDropOnSection: (ruleId: string | null) => void;
}> = (p) => {
  const isUngrouped = () => p.collection === null;
  const sectionKey = () => p.collection?.id ?? UNGROUPED_KEY;
  const isRenaming = () => p.collection !== null && p.renamingId === p.collection.id;
  const isDragOver = () => p.draggingRuleId !== null && p.dragOverKey === sectionKey();
  // Which edge this section would drop a dragged *collection* against. Rule
  // drags and collection drags share the section as a drop target; we tell
  // them apart by the draggingCollectionId signal (not dataTransfer types,
  // which WKWebView doesn't expose reliably on dragover) so the two systems
  // never cross-fire.
  const [collectionDropEdge, setCollectionDropEdge] = createSignal<"before" | "after" | null>(null);
  const COLLECTION_DND = "application/x-pane-collection";
  const isCollectionDrag = () => p.draggingCollectionId !== null;
  // This collection is the one currently being dragged (dim it like a rule row).
  const isBeingDragged = () => p.collection !== null && p.draggingCollectionId === p.collection.id;
  // The header DOM node, snapshotted as the drag image so dragging a collection
  // looks like dragging a rule (a translucent row), not the bare grip icon.
  let headerEl: HTMLElement | undefined;
  const editingNewHere = () => {
    const ed = p.editing;
    return (
      ed?.kind === "rule" && ed.id === "new" && ed.collectionId === (p.collection?.id ?? null)
    );
  };

  return (
    <section
      class={`relative border rounded transition-colors ${
        isDragOver()
          ? "border-accent ring-2 ring-accent/40 bg-accent/5"
          : "border-border"
      }`}
      onDragEnter={(e) => {
        e.preventDefault();
        if (!isCollectionDrag()) p.onDragOverSection(sectionKey());
      }}
      onDragOver={(e) => {
        // A collection is being dragged onto this one → reorder mode: pick the
        // edge by pointer half and show an insertion line. (Ungrouped has no
        // collection row, so it never accepts a reorder.)
        if (!isUngrouped() && isCollectionDrag()) {
          e.preventDefault();
          e.stopPropagation();
          if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
          const rect = e.currentTarget.getBoundingClientRect();
          setCollectionDropEdge(e.clientY < rect.top + rect.height / 2 ? "before" : "after");
          return;
        }
        // Otherwise it's a rule drag. Unconditionally allow drop. We can't
        // reliably read p.draggingRuleId here in time across native DnD events,
        // so just accept the drop and let onDropOnSection bail if there's no
        // active rule drag.
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
        p.onDragOverSection(sectionKey());
      }}
      onDragLeave={(e) => {
        // Only clear if leaving the section entirely (not a child element).
        const related = e.relatedTarget as Node | null;
        if (!related || !(e.currentTarget as Node).contains(related)) {
          p.onDragLeaveSection();
          setCollectionDropEdge(null);
        }
      }}
      onDrop={(e) => {
        if (!isUngrouped() && isCollectionDrag()) {
          e.preventDefault();
          e.stopPropagation();
          const edge = collectionDropEdge();
          const draggedId = p.draggingCollectionId;
          setCollectionDropEdge(null);
          if (edge && draggedId && p.collection && draggedId !== p.collection.id) {
            p.onReorderCollection(draggedId, p.collection.id, edge);
          }
          return;
        }
        e.preventDefault();
        e.stopPropagation();
        const id = e.dataTransfer?.getData("text/plain") || null;
        p.onDropOnSection(id);
      }}
    >
      <Show when={collectionDropEdge()}>
        {(edge) => (
          <div
            class={`pointer-events-none absolute inset-x-0 h-0.5 bg-accent rounded ${
              edge() === "before" ? "-top-px" : "-bottom-px"
            }`}
          />
        )}
      </Show>
      <header
        ref={headerEl}
        class={`flex items-center gap-2 px-3 py-2 ${isBeingDragged() ? "opacity-40" : ""}`}
      >
        <Show when={!isUngrouped()}>
          <div
            draggable={true}
            class="cursor-grab active:cursor-grabbing text-fg-muted hover:text-fg shrink-0"
            title={t()("rules.drag_collection")}
            onDragStart={(e) => {
              if (e.dataTransfer) {
                e.dataTransfer.effectAllowed = "move";
                // Set data for native DnD validity; detection itself rides on
                // the draggingCollectionId signal, not this type.
                e.dataTransfer.setData(COLLECTION_DND, p.collection!.id);
                // Drag the whole header as the preview (like a rule row), not
                // the bare grip. Offset keeps the cursor over the grip area.
                if (headerEl) {
                  e.dataTransfer.setDragImage(headerEl, 16, headerEl.offsetHeight / 2);
                }
              }
              p.onDragStartCollection(p.collection!.id);
            }}
            onDragEnd={() => {
              p.onDragEndCollection();
              setCollectionDropEdge(null);
            }}
          >
            <GripVertical size={14} />
          </div>
        </Show>
        <button
          class="text-fg-muted hover:text-fg shrink-0"
          onClick={p.onToggleCollapsed}
          aria-label={p.collapsed ? t()("rules.expand") : t()("rules.collapse")}
        >
          <Show when={p.collapsed} fallback={<ChevronDown size={14} />}>
            <ChevronRight size={14} />
          </Show>
        </button>

        {/*
          Collection-level toggle. State reflects the aggregate:
          - all rules enabled  → "on", clicking disables all
          - all rules disabled → "off", clicking enables all
          - mixed              → "mixed" (minus sign), clicking enables all
                                 (most useful default — turning everything
                                 on tends to be what the user wants when
                                 they reach for a top-level switch).
          Hidden for an empty collection: there's nothing to toggle and a
          standalone unchecked box reads as "this collection is off",
          which would be misleading.
        */}
        <Show when={p.rules.length > 0}>
          <Checkbox
            state={
              p.rules.every((r) => r.enabled)
                ? "on"
                : p.rules.every((r) => !r.enabled)
                ? "off"
                : "mixed"
            }
            title={
              p.rules.every((r) => r.enabled)
                ? t()("rules.disable")
                : t()("rules.enable")
            }
            onClick={() => p.onToggleCollection(!p.rules.every((r) => r.enabled))}
          />
        </Show>

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
              title={t()("rules.export_collection_title")}
              onClick={p.onExportCollection}
            >
              <Download size={12} />
            </button>
            <button
              class="text-xs p-1 rounded hover:bg-bg-muted text-fg-muted"
              title={t()("rules.rename")}
              onClick={p.onRename}
            >
              <Pencil size={12} />
            </button>
            <button
              class="text-xs p-1 rounded hover:bg-danger/10 text-danger"
              title={t()("rules.delete_collection_title")}
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
            <EditorErrorBoundary onClose={p.onCancel}>
              <RuleEditor
                initial={null}
                defaultCollectionId={p.collection?.id ?? null}
                onCancel={p.onCancel}
                onSaved={p.onSaved}
              />
            </EditorErrorBoundary>
          </Show>

          <Show when={p.rules.length === 0 && !editingNewHere()}>
            <div class="text-xs text-fg-muted italic px-2 py-2">{t()("rules.empty_collection")}</div>
          </Show>

          {/* `<For>` keyed by object reference: the open editor's row must NOT
              be recreated on a save/refresh, so callers preserve rule object
              identity (see `onRuleSaved` / `syncRuleEnabled`). */}
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
                    onCopy={() => p.onCopyRule(rule)}
                    onExport={() => p.onExportRule(rule)}
                    onDelete={() => p.onDeleteRule(rule)}
                    onDragStart={() => p.onDragStartRule(rule.id)}
                    onDragEnd={p.onDragEndRule}
                    onReorder={(draggedId, position) =>
                      p.onReorderRule(draggedId, rule.id, position)
                    }
                  />
                }
              >
                <EditorErrorBoundary onClose={p.onCancel}>
                  <RuleEditor
                    initial={rule}
                    defaultCollectionId={rule.collection_id}
                    onCancel={p.onCancel}
                    onSaved={p.onSaved}
                    onLiveSync={p.onLiveSync}
                  />
                </EditorErrorBoundary>
              </Show>
            )}
          </For>
        </div>
      </Show>
    </section>
  );
};

// Contains a render crash inside the inline editor to just its own row,
// instead of letting it take down the whole rules `<For>` (which made the
// list "disappear"). Shows the error and a button to close the editor so
// the user can recover and the cause is visible.
const EditorErrorBoundary: Component<{ onClose: () => void; children: any }> = (p) => (
  <ErrorBoundary
    fallback={(err) => (
      <div class="border border-danger/40 bg-danger/10 text-danger rounded p-3 text-xs space-y-2">
        <div class="font-medium">{t()("rules.editor_crash")}</div>
        <div class="font-mono break-all opacity-80">
          {String((err as { message?: string })?.message ?? err)}
        </div>
        <button
          class="px-2 py-1 rounded border border-danger/40 hover:bg-danger/15"
          onClick={() => p.onClose()}
        >
          {t()("rules.close_dialog")}
        </button>
      </div>
    )}
  >
    {p.children}
  </ErrorBoundary>
);

const RuleRow: Component<{
  rule: RuleDto;
  isDragging: boolean;
  onToggle: () => void;
  onEdit: () => void;
  onCopy: () => void;
  onExport: () => void;
  onDelete: () => void;
  onDragStart: () => void;
  onDragEnd: () => void;
  onReorder: (draggedId: string, position: "before" | "after") => void;
}> = (p) => {
  const effectivelyOn = () => p.rule.enabled;
  // Which edge the dragged row would drop against, or null when nothing is
  // hovering this row. Drives the insertion indicator line.
  const [dropEdge, setDropEdge] = createSignal<"before" | "after" | null>(null);
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
      onDragEnd={() => {
        setDropEdge(null);
        p.onDragEnd();
      }}
      onDragOver={(e) => {
        // A rule is being dragged over this row — claim the drop for
        // reordering (the section's own drop is for moving between
        // collections). Pick the edge by which half the pointer is in.
        // Ignore collection drags (no text/plain) so they bubble to the
        // section's collection-reorder handler instead.
        if (p.isDragging) return;
        if (!e.dataTransfer?.types.includes("text/plain")) return;
        e.preventDefault();
        e.stopPropagation();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
        const rect = e.currentTarget.getBoundingClientRect();
        setDropEdge(e.clientY < rect.top + rect.height / 2 ? "before" : "after");
      }}
      onDragLeave={(e) => {
        const related = e.relatedTarget as Node | null;
        if (!related || !(e.currentTarget as Node).contains(related)) {
          setDropEdge(null);
        }
      }}
      onDrop={(e) => {
        // Collection drags (no text/plain) belong to the section handler.
        if (!e.dataTransfer?.types.includes("text/plain")) return;
        const edge = dropEdge();
        setDropEdge(null);
        if (p.isDragging || !edge) return;
        e.preventDefault();
        // Stop the section's onDrop from also firing — that would move the
        // rule between collections on top of the reorder.
        e.stopPropagation();
        const draggedId = e.dataTransfer?.getData("text/plain");
        if (draggedId) p.onReorder(draggedId, edge);
      }}
      class={`relative border rounded p-2 flex items-start gap-3 cursor-move ${effectivelyOn() ? "border-border bg-bg" : "border-border/50 bg-bg-subtle/30 opacity-70"} ${p.isDragging ? "opacity-40" : ""}`}
    >
      <Show when={dropEdge()}>
        {(edge) => (
          <div
            class={`pointer-events-none absolute inset-x-1 h-0.5 bg-accent rounded ${edge() === "before" ? "-top-px" : "-bottom-px"}`}
          />
        )}
      </Show>
      <div class="mt-0.5">
        <Checkbox
          state={p.rule.enabled ? "on" : "off"}
          title={p.rule.enabled ? t()("rules.disable") : t()("rules.enable")}
          onClick={p.onToggle}
        />
      </div>
      <div class="flex-1 min-w-0">
        <div class="flex items-baseline gap-2">
          <div class="font-medium text-sm truncate">{p.rule.name || t()("rules.unnamed_rule")}</div>
        </div>
        <div class="text-xs font-mono text-fg-subtle truncate mt-0.5">{summary()}</div>
        <div class="text-xs text-fg-muted mt-0.5">
          → {p.rule.res_status} · {p.rule.res_body_mime ?? t()("rules.summary_no_body")} · {p.rule.res_body_size}B
          <Show when={p.rule.res_delay_ms > 0}> · {t()("rules.summary_delay")} {p.rule.res_delay_ms}ms</Show>
        </div>
      </div>
      <div class="flex items-center gap-1 shrink-0">
        <button
          class="text-xs px-2 py-1 rounded hover:bg-bg-muted text-fg-muted"
          title={t()("rules.edit")}
          onClick={p.onEdit}
        >
          <Pencil size={12} />
        </button>
        <button
          class="text-xs px-2 py-1 rounded hover:bg-bg-muted text-fg-muted"
          title={t()("rules.duplicate_rule_title")}
          onClick={p.onCopy}
        >
          <Copy size={12} />
        </button>
        <button
          class="text-xs px-2 py-1 rounded hover:bg-bg-muted text-fg-muted"
          title={t()("rules.export_rule_title")}
          onClick={p.onExport}
        >
          <Download size={12} />
        </button>
        <button
          class="text-xs px-2 py-1 rounded hover:bg-danger/10 text-danger"
          title={t()("rules.delete")}
          onClick={p.onDelete}
        >
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
  match_req_body: string;
  match_conditions: RuleConditionDto[];
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
  match_req_body: "",
  match_conditions: [],
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

/// Smallest height (px) the body editor can be dragged down to — matches
/// the collapsed `min-h-10` so the corner grip can't hide the field.
const MIN_BODY_HEIGHT = 40;

/// JSON editor field: the syntax-highlighted `JsonEditor` plus the Format
/// (pretty-print) and Expand/Collapse controls and a format-error flash.
/// Shared by the response-body and request-body-match fields so the two
/// stay in lockstep — `expandKey` namespaces the persisted expand flag
/// per field.
const JsonFieldEditor: Component<{
  value: string;
  onInput: (v: string) => void;
  placeholder?: string;
  disabled?: boolean;
  expandKey: string;
}> = (p) => {
  const [expanded, setExpandedRaw] = createSignal(
    (() => {
      try {
        return localStorage.getItem(p.expandKey) === "1";
      } catch {
        return false;
      }
    })(),
  );
  // Custom drag-resized height in px. `null` ⇒ fall back to the class-based
  // expand/collapse presets. Persisted under its own key so a hand-sized
  // editor survives reloads. Set by the corner grip; cleared whenever
  // Expand/Collapse is clicked so those presets keep working unchanged.
  const heightKey = `${p.expandKey}-h`;
  const [dragHeight, setDragHeightRaw] = createSignal<number | null>(
    (() => {
      try {
        const n = parseInt(localStorage.getItem(heightKey) ?? "", 10);
        return Number.isFinite(n) && n >= MIN_BODY_HEIGHT ? n : null;
      } catch {
        return null;
      }
    })(),
  );
  const setDragHeight = (v: number | null) => {
    setDragHeightRaw(v);
    try {
      if (v == null) localStorage.removeItem(heightKey);
      else localStorage.setItem(heightKey, String(v));
    } catch {
      /* private mode — signal still works, just no persistence */
    }
  };
  const setExpanded = (v: boolean) => {
    setExpandedRaw(v);
    setDragHeight(null); // preset wins; resume class-based sizing
    try {
      localStorage.setItem(p.expandKey, v ? "1" : "0");
    } catch {
      /* private mode — signal still works, just no persistence */
    }
  };
  let wrapperEl: HTMLDivElement | undefined;
  const startBodyResize = (ev: MouseEvent) => {
    ev.preventDefault();
    ev.stopPropagation();
    const startY = ev.clientY;
    const startH = wrapperEl?.getBoundingClientRect().height ?? MIN_BODY_HEIGHT;
    const onMove = (e: MouseEvent) => {
      setDragHeight(Math.max(MIN_BODY_HEIGHT, Math.round(startH + (e.clientY - startY))));
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    document.body.style.cursor = "ns-resize";
    document.body.style.userSelect = "none";
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };
  const [formatErr, setFormatErr] = createSignal<string | null>(null);
  const loadFromFile = async () => {
    const path = await open({
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path || typeof path !== "string") return;
    try {
      const raw = await api.rules.importRead(path);
      p.onInput(raw);
      setFormatErr(null);
    } catch (e) {
      setFormatErr((e as Error).message ?? "read failed");
      setTimeout(() => setFormatErr(null), 2500);
    }
  };
  const format = () => {
    const raw = p.value;
    if (!raw.trim()) {
      setFormatErr("empty");
      setTimeout(() => setFormatErr(null), 1800);
      return;
    }
    try {
      const pretty = JSON.stringify(JSON.parse(raw), null, 2);
      if (pretty !== raw) p.onInput(pretty);
      setFormatErr(null);
    } catch (e) {
      setFormatErr((e as Error).message ?? "invalid JSON");
      setTimeout(() => setFormatErr(null), 2500);
    }
  };
  return (
    <div class="flex-1">
      <div class="flex items-center justify-end gap-1 mb-1">
        <Show when={formatErr()}>
          {(msg) => (
            <span class="text-xs text-danger truncate max-w-[260px]" title={msg()}>
              {msg() === "empty"
                ? t()("rules.json_body_empty")
                : `${t()("rules.json_invalid")}: ${msg()}`}
            </span>
          )}
        </Show>
        <button
          type="button"
          class="text-xs text-fg-muted hover:text-fg inline-flex items-center gap-1 px-1.5 py-0.5 rounded hover:bg-bg-muted disabled:opacity-50"
          title={t()("rules.json_format_title")}
          disabled={p.disabled}
          onClick={format}
        >
          <Braces size={12} />
          {t()("rules.json_format")}
        </button>
        <button
          type="button"
          class="text-xs text-fg-muted hover:text-fg inline-flex items-center gap-1 px-1.5 py-0.5 rounded hover:bg-bg-muted disabled:opacity-50"
          title={t()("rules.json_load_file_title")}
          disabled={p.disabled}
          onClick={loadFromFile}
        >
          <FilePlus size={12} />
          {t()("rules.json_load_file")}
        </button>
        <button
          type="button"
          class="text-xs text-danger hover:text-danger inline-flex items-center gap-1 px-1.5 py-0.5 rounded hover:bg-danger/10 disabled:opacity-50"
          title={t()("rules.json_clear_title")}
          disabled={p.disabled || p.value.length === 0}
          onClick={() => {
            p.onInput("");
            setFormatErr(null);
          }}
        >
          <Eraser size={12} />
          {t()("rules.json_clear")}
        </button>
        <button
          type="button"
          class="text-xs text-fg-muted hover:text-fg inline-flex items-center gap-1 px-1.5 py-0.5 rounded hover:bg-bg-muted"
          title={expanded() ? t()("rules.json_collapse_title") : t()("rules.json_expand_title")}
          onClick={() => setExpanded(!expanded())}
        >
          <Show when={expanded()} fallback={<Maximize2 size={12} />}>
            <Minimize2 size={12} />
          </Show>
          {expanded() ? t()("rules.json_collapse") : t()("rules.json_expand")}
        </button>
      </div>
      <div
        ref={wrapperEl}
        class={`relative w-full ${
          dragHeight() != null
            ? ""
            : expanded()
              ? "h-[70vh] min-h-[400px]"
              : "h-14 min-h-10"
        }`}
        style={dragHeight() != null ? { height: `${dragHeight()}px` } : undefined}
      >
        <JsonEditor
          value={p.value}
          onInput={p.onInput}
          placeholder={p.placeholder}
          disabled={p.disabled}
        />
        {/* Drag the bottom-right corner to set a custom height. Sits above
            the editor; clicking Expand/Collapse clears it (see setExpanded). */}
        <div
          class="group absolute bottom-0 right-0 w-4 h-4 z-20 cursor-ns-resize flex items-end justify-end p-0.5"
          title={t()("rules.json_resize_title")}
          onMouseDown={startBodyResize}
        >
          <div class="w-2 h-2 border-b-2 border-r-2 border-fg-muted/40 group-hover:border-accent rounded-br-sm" />
        </div>
      </div>
    </div>
  );
};

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
  onSaved: (saved: RuleDto) => void | Promise<void>;
  // Reflect a live (non-Save) enabled toggle in the parent list so the
  // collection-header switch re-renders in sync — without remounting this
  // open editor (a full refetch would, and could race the async body load).
  onLiveSync?: (id: string, enabled: boolean) => void;
}> = (p) => {
  // Stable storage key for this editor instance. Drafts are persisted
  // per-(rule|new+collection) so switching between two rules doesn't
  // merge their in-progress edits.
  const draftKey = ruleDraftKey(p.initial?.id ?? null, p.defaultCollectionId);

  // Track whether init() restored from a persisted draft. If it did,
  // the saved draft is authoritative — we skip the body-load below so
  // we don't clobber user-typed text (including a deliberately empty
  // body) with the version on disk.
  let hadInitialDraft = false;

  const init = (): DraftState => {
    const saved = loadRuleDraft<DraftState>(draftKey);
    if (saved) {
      hadInitialDraft = true;
      return saved;
    }
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
      match_req_body: r.match_req_body ?? "",
      match_conditions: r.match_conditions.slice(),
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
  // Collapse the (often long) response-headers list to reclaim vertical
  // space. Persisted globally, like the JSON-body expand flags.
  const [headersCollapsed, setHeadersCollapsedRaw] = createSignal(
    (() => {
      try {
        return localStorage.getItem("pane.rules.res-headers-collapsed") === "1";
      } catch {
        return false;
      }
    })(),
  );
  const setHeadersCollapsed = (v: boolean) => {
    setHeadersCollapsedRaw(v);
    try {
      localStorage.setItem("pane.rules.res-headers-collapsed", v ? "1" : "0");
    } catch {
      /* private mode — signal still works, just no persistence */
    }
  };
  // `ruleEditorDirty` (module-level) goes true on the first user edit and
  // back to false after save. Drives the Save button colour and the
  // unsaved-changes guard. Reset to false on every fresh mount so a
  // restored localStorage draft doesn't start the button red — the user
  // hasn't typed anything yet in this editor instance.

  // Persisting the in-progress draft is tied to actual user edits (one
  // funnel: `patch`). The earlier `createEffect`-based approach fired on
  // every `d()` change — including the post-mount body-load `setD` — and
  // so wrote a no-edits snapshot to localStorage on first open. Next open
  // saw "had draft" and started the Save button red without anything to
  // actually save.
  const patch = (q: Partial<DraftState>) => {
    const next = { ...d(), ...q };
    setD(next);
    setRuleEditorDirty(true);
    saveRuleDraft(draftKey, next);
  };

  const existingBodyId = createMemo(() => p.initial?.res_body_id ?? null);
  const existingBodySize = createMemo(() => p.initial?.res_body_size ?? 0);

  // Load the existing response body into the textarea on open so the user
  // sees what's currently stored and can edit it in place. Skipped if
  // we restored a draft above — the draft's body text is what the
  // user last had, which beats whatever's on disk.
  onMount(async () => {
    registerEditorSaveFn(save);
    setRuleEditorDirty(false);
    if (hadInitialDraft) return;
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
        // Bypass `patch` to avoid marking the editor dirty — loading the
        // existing body on open is not a user edit.
        setD({ ...d(), res_body_text: text });
      }
    } catch (e) {
      console.warn("failed to load existing body for editing", e);
    } finally {
      setBodyLoading(false);
    }
  });

  // ChevronUp ("collapse"): navigational intent — warns if dirty.
  const dismiss = () => {
    if (ruleEditorDirty()) {
      setRuleEditorPendingNav(() => p.onCancel);
    } else {
      clearRuleDraft(draftKey);
      p.onCancel();
    }
  };

  onCleanup(() => {
    registerEditorSaveFn(null);
    // The editor is unmounting (closed, deleted, or the whole Rules view
    // navigated away). `ruleEditorDirty` / `ruleEditorPendingNav` are
    // module-level and otherwise survive this unmount — a dirty editor that
    // disappeared would leave the list's nav-guard armed, so every Edit
    // click would pop the unsaved-changes modal instead of opening the
    // editor, and a stale pending-nav modal could reappear on remount.
    // Clear both so a closed editor always leaves a clean slate.
    setRuleEditorDirty(false);
    setRuleEditorPendingNav(null);
  });

  // The `enabled` flag behaves like the standalone RuleRow checkbox: for an
  // already-saved rule it commits immediately so the collapsed row and the
  // collection-header tri-state switch reflect it at once, without waiting
  // for a full Save of the open draft. A brand-new (unsaved) rule has no row
  // to sync yet, so it just rides along in the draft until first Save.
  const toggleEnabled = async (checked: boolean) => {
    const id = d().id;
    if (!id) {
      patch({ enabled: checked });
      return;
    }
    // Keep the draft in lockstep but don't mark it dirty — the change is
    // persisted below, so the Save button shouldn't flag it as unsaved.
    const next = { ...d(), enabled: checked };
    setD(next);
    saveRuleDraft(draftKey, next);
    try {
      await api.rules.setEnabled(id, checked);
      p.onLiveSync?.(id, checked);
    } catch (e: any) {
      setErr(e?.message ?? String(e));
    }
  };

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
        // Blank → null (no body matching). Storage trims and re-checks.
        match_req_body: draft.match_req_body.trim() ? draft.match_req_body : null,
        // Drop half-filled rows: a condition needs both a path and a value.
        // (An empty value would behave inconsistently by op — `contains ""`
        // matches everything, numeric ops fail closed.)
        match_conditions: draft.match_conditions.filter(
          (c) => c.path.trim().length > 0 && c.value.trim().length > 0,
        ),
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
      // Saved state is authoritative now — drop the in-progress draft
      // so the editor doesn't reopen on the stale pre-save snapshot.
      clearRuleDraft(draftKey);
      setRuleEditorDirty(false);
      await p.onSaved(saved);
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
          title={t()("rules.collapse_without_saving")}
          onClick={dismiss}
        >
          <ChevronUp size={14} />
        </button>
        <input {...NO_AC}
          class="flex-1 min-w-48 bg-bg border border-border rounded px-2 py-1 text-sm"
          placeholder={t()("rules.name_placeholder")}
          value={d().name}
          onInput={(e) => patch({ name: e.currentTarget.value })}
        />
        <label class="flex items-center gap-1 text-xs text-fg-muted">
          {t()("rules.priority_label")}
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
            onChange={(e) => toggleEnabled(e.currentTarget.checked)}
          />
          {t()("rules.enabled_label")}
        </label>
      </div>

      <Section title={t()("rules.match_section")}>
        <FieldRow label={t()("rules.host_label")}>
          <input {...NO_AC}
            class="flex-1 bg-bg border border-border rounded px-2 py-1 text-sm font-mono"
            placeholder={t()("rules.host_placeholder")}
            value={d().match_host_glob}
            onInput={(e) => patch({ match_host_glob: e.currentTarget.value })}
          />
        </FieldRow>
        <FieldRow label={t()("rules.method_label")}>
          <select
            class="bg-bg border border-border rounded px-2 py-1 text-sm"
            value={d().match_method}
            onChange={(e) => patch({ match_method: e.currentTarget.value })}
          >
            <For each={["ANY", "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]}>
              {(m) => (
                <option value={m}>
                  {m === "ANY" ? t()("rules.method_any") : m}
                </option>
              )}
            </For>
          </select>
        </FieldRow>
        <FieldRow label={t()("rules.path_label")}>
          <input {...NO_AC}
            class="flex-1 bg-bg border border-border rounded px-2 py-1 text-sm font-mono"
            placeholder={t()("rules.path_placeholder")}
            value={d().match_path_glob}
            onInput={(e) => patch({ match_path_glob: e.currentTarget.value })}
          />
        </FieldRow>
        <FieldRow label={t()("rules.params_label")}>
          <div class="flex-1 space-y-1">
            <Index each={d().match_params}>
              {(q, i) => {
                const volatileValue = () => looksVolatile(q().value);
                // Params are EXACT-match — a leading comparison operator is a
                // common mix-up with Conditions ("=10000" never matches the
                // body's "10000"). Warn and point the user at Conditions.
                const operatorLike = () => /^\s*[=≠<>≥≤!]/.test(q().value);
                const warnValue = () => volatileValue() || operatorLike();
                const warnHint = () =>
                  operatorLike()
                    ? t()("rules.param_operator_hint")
                    : volatileValue()
                      ? t()("rules.param_volatile_hint")
                      : undefined;
                return (
                  <div class="flex items-center gap-2">
                    <input {...NO_AC}
                      class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                      placeholder={t()("rules.param_name_placeholder")}
                      value={q().name}
                      onInput={(e) => {
                        const arr = d().match_params.slice();
                        arr[i] = { ...arr[i], name: e.currentTarget.value };
                        patch({ match_params: arr });
                      }}
                    />
                    <input {...NO_AC}
                      class={`flex-1 bg-bg border rounded px-2 py-1 text-xs font-mono ${
                        warnValue()
                          ? "border-warn ring-1 ring-warn/40"
                          : "border-border"
                      }`}
                      placeholder={t()("rules.param_value_placeholder")}
                      value={q().value}
                      title={warnHint()}
                      onInput={(e) => {
                        const arr = d().match_params.slice();
                        arr[i] = { ...arr[i], value: e.currentTarget.value };
                        patch({ match_params: arr });
                      }}
                    />
                    <Show when={warnValue()}>
                      <span class="text-warn shrink-0" title={warnHint()}>
                        <AlertTriangle size={12} />
                      </span>
                    </Show>
                    <button
                      class="text-fg-muted hover:text-danger"
                      onClick={() =>
                        patch({ match_params: d().match_params.filter((_, j) => j !== i) })
                      }
                    >
                      <X size={12} />
                    </button>
                  </div>
                );
              }}
            </Index>
            <button
              class="text-xs text-accent hover:underline"
              onClick={() =>
                patch({ match_params: [...d().match_params, { name: "", value: "" }] })
              }
            >
              {t()("rules.add_param")}
            </button>
            <div class="text-xs text-fg-muted italic">{t()("rules.params_note")}</div>
          </div>
        </FieldRow>
        <FieldRow label={t()("rules.req_body_label")}>
          <div class="flex-1 space-y-1">
            <JsonFieldEditor
              value={d().match_req_body}
              onInput={(v) => patch({ match_req_body: v })}
              placeholder={t()("rules.req_body_placeholder")}
              expandKey="pane.rules.reqbody-expanded"
            />
            <Show when={bodyHasVolatile(d().match_req_body)}>
              <div class="text-warn text-xs flex items-start gap-1">
                <AlertTriangle size={12} class="shrink-0 mt-0.5" />
                <span>{t()("rules.req_body_volatile_hint")}</span>
              </div>
            </Show>
            <div class="text-xs text-fg-muted italic">{t()("rules.req_body_note")}</div>
          </div>
        </FieldRow>
        <FieldRow label={t()("rules.conditions_label")}>
          <div class="flex-1 space-y-1">
            <Index each={d().match_conditions}>
              {(c, i) => (
                <div class="flex items-center gap-2">
                  <input {...NO_AC}
                    class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder={t()("rules.cond_path_placeholder")}
                    value={c().path}
                    onInput={(e) => {
                      const arr = d().match_conditions.slice();
                      arr[i] = { ...arr[i], path: e.currentTarget.value };
                      patch({ match_conditions: arr });
                    }}
                  />
                  <select
                    class="bg-bg-muted text-fg rounded px-2 py-1 text-xs shrink-0 border border-transparent outline-none focus:ring-1 focus:ring-accent"
                    value={c().op}
                    onChange={(e) => {
                      const arr = d().match_conditions.slice();
                      arr[i] = { ...arr[i], op: e.currentTarget.value as RuleConditionOp };
                      patch({ match_conditions: arr });
                    }}
                  >
                    <For each={CONDITION_OPS}>
                      {(op) => (
                        <option value={op.value}>
                          {op.value === "contains" ? t()("rules.cond_op_contains") : op.symbol}
                        </option>
                      )}
                    </For>
                  </select>
                  <input {...NO_AC}
                    class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder={t()("rules.cond_value_placeholder")}
                    value={c().value}
                    onInput={(e) => {
                      const arr = d().match_conditions.slice();
                      arr[i] = { ...arr[i], value: e.currentTarget.value };
                      patch({ match_conditions: arr });
                    }}
                  />
                  <button
                    class="text-fg-muted hover:text-danger"
                    onClick={() =>
                      patch({ match_conditions: d().match_conditions.filter((_, j) => j !== i) })
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
                patch({
                  match_conditions: [...d().match_conditions, { path: "", op: "gte", value: "" }],
                })
              }
            >
              {t()("rules.add_condition")}
            </button>
            <div class="text-xs text-fg-muted italic">{t()("rules.conditions_note")}</div>
          </div>
        </FieldRow>
      </Section>

      <Section title={t()("rules.response_section")}>
        <FieldRow label={t()("rules.mode_label")} help={{ path: "/rules/", title: t()("rules.mode_help_title") }}>
          <select
            class="bg-bg border border-border rounded px-2 py-1 text-sm"
            value={d().mode}
            onChange={(e) => patch({ mode: e.currentTarget.value as RuleMode })}
          >
            <option value="stub">{t()("rules.mode_stub_long")}</option>
            <option value="patch">{t()("rules.mode_patch_long")}</option>
          </select>
          <label class="flex items-center gap-1 text-xs text-fg-muted ml-3">
            {t()("rules.delay_label")}
            <input {...NO_AC}
              type="number"
              placeholder="0"
              class="w-20 bg-bg border border-border rounded px-2 py-1 text-sm text-fg"
              value={d().res_delay_ms || ""}
              // Allow a transient empty value mid-edit: a fallback in onInput
              // (`|| 0`) snaps the field back the moment the user erases the
              // last digit, making it impossible to retype a fresh number.
              // On blur, an empty field means "no delay" → patch to 0; the
              // value binding (`|| ""`) keeps the box visually blank with
              // the placeholder "0" hint instead of literally showing "0",
              // matching the "no delay by default" reading users expect.
              onInput={(e) => {
                const v = e.currentTarget.value;
                if (v === "") return;
                const n = Number(v);
                if (Number.isFinite(n)) patch({ res_delay_ms: n });
              }}
              onBlur={(e) => {
                if (e.currentTarget.value === "") patch({ res_delay_ms: 0 });
              }}
            />
          </label>
        </FieldRow>
        <Show when={d().mode === "stub"}>
        <FieldRow label={t()("rules.status_label")}>
          <input {...NO_AC}
            type="number"
            class="w-20 bg-bg border border-border rounded px-2 py-1 text-sm"
            value={d().res_status}
            // Same trick as res_delay_ms above: don't snap back to 200 mid-edit
            // (e.g. user clears "200" to retype "300" — the fallback would
            // reset the field every keystroke). Blur restores the last valid
            // value imperatively because Solid won't re-set value= when the
            // signal didn't actually change.
            onInput={(e) => {
              const v = e.currentTarget.value;
              if (v === "") return;
              const n = Number(v);
              if (Number.isFinite(n)) patch({ res_status: n });
            }}
            onBlur={(e) => {
              if (e.currentTarget.value === "") {
                e.currentTarget.value = String(d().res_status);
              }
            }}
          />
        </FieldRow>
        <FieldRow
          label={t()("rules.headers_label")}
          collapsed={headersCollapsed()}
          onToggleCollapsed={() => setHeadersCollapsed(!headersCollapsed())}
        >
          <Show
            when={!headersCollapsed()}
            fallback={
              <button
                type="button"
                class="text-xs text-fg-muted hover:text-fg pt-1"
                onClick={() => setHeadersCollapsed(false)}
              >
                {t()("rules.headers_count", { count: d().res_headers.length })}
              </button>
            }
          >
          <div class="flex-1 space-y-1">
            <Index each={d().res_headers}>
              {(h, i) => (
                <div class="flex items-center gap-2">
                  <input {...NO_AC}
                    class="flex-1 bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                    placeholder={t()("rules.header_name_placeholder")}
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
                    placeholder={t()("rules.header_value_placeholder")}
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
                {t()("rules.add_header")}
              </button>
              <button
                class="text-xs text-accent hover:underline"
                title={t()("rules.paste_header_title")}
                onClick={async () => {
                  try {
                    const text = await readClipboard();
                    const split = splitHeaderPair(text);
                    if (split) {
                      patch({ res_headers: [...d().res_headers, split] });
                    }
                  } catch {
                    // Clipboard access may be denied; user can fall back to manual paste.
                  }
                }}
              >
                {t()("rules.paste_header")}
              </button>
              <Show when={d().res_headers.length > 0}>
                <button
                  class="text-xs text-danger hover:underline"
                  title={t()("rules.clear_headers_title")}
                  onClick={() => patch({ res_headers: [] })}
                >
                  {t()("rules.clear_headers")}
                </button>
              </Show>
            </div>
          </div>
          </Show>
        </FieldRow>
        <FieldRow label={t()("rules.body_mime_label")}>
          <input {...NO_AC}
            class="flex-1 bg-bg border border-border rounded px-2 py-1 text-sm font-mono"
            placeholder={t()("rules.body_mime_placeholder")}
            value={d().res_body_mime}
            onInput={(e) => patch({ res_body_mime: e.currentTarget.value })}
          />
        </FieldRow>
        <FieldRow label={t()("rules.body_label")}>
          <div class="flex-1">
            <JsonFieldEditor
              value={d().res_body_text}
              onInput={(v) => patch({ res_body_text: v })}
              placeholder={
                bodyLoading()
                  ? t()("rules.body_loading_placeholder")
                  : existingBodyId()
                  ? t()("rules.body_existing_placeholder")
                  : t()("rules.body_new_placeholder")
              }
              disabled={bodyLoading()}
              expandKey="pane.rules.body-expanded"
            />
            <Show when={!!existingBodyId() && !bodyLoading()}>
              <div class="text-xs text-fg-muted mt-1">
                {tr("rules.body_stored_note", { size: formatBytes(existingBodySize()) })}
              </div>
            </Show>
          </div>
        </FieldRow>
        </Show>
        <Show when={d().mode === "patch"}>
          <FieldRow label={t()("rules.patches_section")} help={{ path: "/rules/#синтаксис-path", title: t()("rules.patches_help_title") }}>
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
          onClick={dismiss}
          disabled={busy()}
        >
          {t()("rules.cancel")}
        </button>
        <button
          class={`text-sm px-3 py-1.5 rounded text-white hover:opacity-90 inline-flex items-center gap-1 disabled:opacity-50 ${ruleEditorDirty() ? "bg-danger" : "bg-accent"}`}
          onClick={save}
          disabled={busy() || bodyLoading()}
        >
          <Check size={14} />{" "}
          {busy()
            ? t()("rules.saving")
            : bodyLoading()
            ? t()("rules.loading")
            : t()("rules.save")}
        </button>
      </div>
    </div>
  );
};

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
              placeholder={t()("rules.patch_path_placeholder")}
              value={op().path}
              onInput={(e) => setRow(i, { path: e.currentTarget.value })}
            />
            <Show when={op().op !== "delete"}>
              <input
                {...NO_AC}
                class="flex-[2] bg-bg border border-border rounded px-2 py-1 text-xs font-mono"
                placeholder={t()("rules.patch_value_placeholder")}
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
        {t()("rules.add_patch")}
      </button>
      <div class="text-xs text-fg-muted italic" innerHTML={t()("rules.patches_note_html")} />
    </div>
  );
};

const Section: Component<{ title: string; children: any }> = (p) => (
  <div>
    <div class="text-xs uppercase tracking-wide text-fg-muted mb-2">{p.title}</div>
    <div class="space-y-2">{p.children}</div>
  </div>
);

/**
 * Tri-state checkbox used for per-rule and per-collection enable toggles.
 * - "on"    → filled blue square with a white check.
 * - "off"   → empty bordered square on the muted background.
 * - "mixed" → filled blue square with a horizontal bar (collection has
 *             both enabled and disabled rules).
 *
 * Stops click propagation so it doesn't trip drag handlers / collapse
 * toggles on the surrounding row or section header.
 */
const Checkbox: Component<{
  state: "on" | "off" | "mixed";
  onClick: () => void;
  title?: string;
}> = (p) => (
  <button
    type="button"
    class={`shrink-0 w-[18px] h-[18px] rounded-[3px] border inline-flex items-center justify-center transition ${
      p.state === "off"
        ? "bg-transparent border-fg-muted/50 hover:border-fg"
        : "bg-accent border-transparent ring-1 ring-inset ring-white/10 hover:brightness-110"
    }`}
    title={p.title}
    onClick={(e) => {
      e.stopPropagation();
      p.onClick();
    }}
  >
    <Show when={p.state === "on"}>
      <Check size={12} class="text-white" strokeWidth={3} />
    </Show>
    <Show when={p.state === "mixed"}>
      <Minus size={12} class="text-white" strokeWidth={3} />
    </Show>
  </button>
);

const FieldRow: Component<{
  label: string;
  help?: { path: string; title?: string };
  // When provided, the label gets a chevron toggle (collapsible section).
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
  children: any;
}> = (p) => (
  <div class="flex items-start gap-3">
    <div class="w-40 text-xs text-fg-muted pt-1.5 shrink-0 inline-flex items-center gap-1">
      <Show when={p.onToggleCollapsed}>
        <button
          type="button"
          class="text-fg-muted hover:text-fg shrink-0 -ml-0.5"
          aria-label={p.collapsed ? t()("rules.expand") : t()("rules.collapse")}
          onClick={() => p.onToggleCollapsed!()}
        >
          <Show when={p.collapsed} fallback={<ChevronDown size={14} />}>
            <ChevronRight size={14} />
          </Show>
        </button>
      </Show>
      {p.label}
      <Show when={p.help}>
        {(h) => <HelpButton path={h().path} title={h().title} size={12} />}
      </Show>
    </div>
    <div class="flex-1 flex items-center gap-2 flex-wrap">{p.children}</div>
  </div>
);

export default RulesView;
