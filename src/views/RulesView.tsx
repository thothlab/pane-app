import { type Component, createSignal, createMemo, createEffect, For, Index, Show, ErrorBoundary, onMount, onCleanup } from "solid-js";
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
  Search,
  ChevronsDownUp,
  ChevronsUpDown,
  Tag as TagIcon,
  FolderTree,
} from "lucide-solid";
import { open, save } from "@tauri-apps/plugin-dialog";
import { readClipboard } from "@/lib/clipboard";
import { api } from "@/ipc/client";
import HelpButton from "@/components/HelpButton";
import JsonEditor from "@/components/JsonEditor";
import { VerticalResizer } from "@/components/VerticalResizer";
import { t, tr } from "@/i18n";
import {
  matchesCollection,
  matchesRuleFilter,
  parseFilterTerms,
  type CollectionContext,
} from "@/lib/rules-filter";
import { TagChips, TagEditor } from "@/components/Tags";
import { askConfirm, showMessage, type ConfirmRequest } from "@/stores/confirm";
import { groupState, isLiveOn, isScoped, SCOPE_HOST } from "@/lib/rule-liveness";
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
  rulesFilter,
  setRulesFilter,
  rulesFilterCollapsed,
  setRulesFilterCollapsed,
  rulesDevice,
  setRulesDevice,
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
  rulesViewMode,
  setRulesViewMode,
  rulesTreeSelectedCollection,
  setRulesTreeSelectedCollection,
} from "@/stores/rules-ui";
import type {
  RuleBulkScope,
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
  DeviceDto,
} from "@/ipc/types";

const UNGROUPED_KEY = "__ungrouped__";

// Tree view's left/right split, same clamped-localStorage-width pattern as
// CapturesView's list/detail split.
const TREE_PANE_DEFAULT = 280;
const TREE_PANE_MIN = 180;
const TREE_PANE_MAX = 560;
const TREE_PANE_STORAGE_KEY = "pane:rules.treePaneWidth";

function loadTreePaneWidth(): number {
  try {
    const raw = localStorage.getItem(TREE_PANE_STORAGE_KEY);
    if (!raw) return TREE_PANE_DEFAULT;
    const n = JSON.parse(raw);
    if (typeof n === "number" && n >= TREE_PANE_MIN && n <= TREE_PANE_MAX) return n;
  } catch {
    /* fall through */
  }
  return TREE_PANE_DEFAULT;
}

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
  // Paired devices, for the plane selector. iOS is listed but not selectable:
  // its traffic shares the host proxy port and is never attributed to the
  // device row, so a rule scoped to it would never fire.
  const [devices, setDevices] = createSignal<DeviceDto[]>([]);
  // `null` = the global plane, where a checkbox means "on everywhere".
  const device = rulesDevice;
  const deviceName = (id: string) =>
    id === SCOPE_HOST
      ? t()("rules.device_this_mac")
      : (devices().find((d) => d.id === id)?.display_name ?? id.slice(0, 8));
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
  // Id of the collection whose tag editor is open, or null — one at a time,
  // like renaming — plus the tags being edited.
  //
  // The editor works on a DRAFT and saves once, on close. Saving per chip
  // would mean a refresh per chip, and a refresh replaces the collection
  // objects, which recreates the section and with it the tag input the user
  // is typing into: focus lost after every Enter. The draft also keeps the
  // header honest — it shows what is saved, the editor shows what is being
  // edited.
  const [taggingId, setTaggingId] = createSignal<string | null>(null);
  const [taggingDraft, setTaggingDraft] = createSignal<string[]>([]);
  let filterInput: HTMLInputElement | undefined;

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
      const [r, c, d] = await Promise.all([
        api.rules.list(),
        api.collections.list(),
        api.devices.list().catch(() => [] as DeviceDto[]),
      ]);
      setRules(r);
      setCollections(c);
      setDevices(d);
      // A device unpaired while the view was open would leave every checkbox
      // writing to a phone that is no longer there. Fall back to the global
      // plane rather than silently doing nothing.
      const sel = device();
      if (sel && sel !== SCOPE_HOST && !d.some((x) => x.id === sel)) {
        setRulesDevice(null);
      }
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

  // ── Library filter ────────────────────────────────────────────────
  // Matching itself lives in `lib/rules-filter` (pure, unit-tested);
  // what stays here is only the reactive plumbing around it.
  const filterTerms = createMemo(() => parseFilterTerms(rulesFilter()));
  const filterActive = createMemo(() => filterTerms().length > 0);

  // What the filter knows about a rule's group: its name and its tags. An
  // ungrouped rule gets the localized "Ungrouped" label and no tags, so it
  // behaves like a group that simply carries none.
  const collectionContextOf = (id: string | null): CollectionContext => {
    if (id === null) return { name: t()("rules.ungrouped") };
    const c = collections().find((x) => x.id === id);
    return { name: c?.name ?? "", tags: c?.tags };
  };

  const matchesFilter = (r: RuleDto): boolean =>
    matchesRuleFilter(r, collectionContextOf(r.collection_id), filterTerms());

  // Rendering-only view. `rulesByCollection` above stays UNFILTERED on
  // purpose: reorderRule renumbers a section to a dense 0..N-1 from it,
  // and doing that over a filtered subset would rewrite the precedence
  // of rules the user cannot see. Drag is disabled while filtering for
  // the same reason.
  const visibleRulesByCollection = createMemo(() => {
    if (!filterActive()) return rulesByCollection();
    const map = new Map<string, RuleDto[]>();
    for (const [k, list] of rulesByCollection()) {
      const kept = list.filter(matchesFilter);
      if (kept.length > 0) map.set(k, kept);
    }
    return map;
  });

  const visibleRules = createMemo(() =>
    filterActive() ? rules().filter(matchesFilter) : rules(),
  );

  // A collection survives the filter if it still holds matching rules,
  // or if its own name matches — an empty group whose name you just
  // typed should be on screen, not missing.
  const visibleCollections = createMemo(() => {
    if (!filterActive()) return collections();
    return collections().filter(
      (c) =>
        (visibleRulesByCollection().get(c.id) ?? []).length > 0 ||
        matchesCollection(c, filterTerms()),
    );
  });

  const ungroupedVisible = createMemo(
    () =>
      !filterActive() ||
      (visibleRulesByCollection().get(UNGROUPED_KEY) ?? []).length > 0,
  );

  const noFilterMatches = createMemo(
    () => filterActive() && visibleRules().length === 0 && visibleCollections().length === 0,
  );

  // Every label in use anywhere, first-seen spelling wins. Feeds the
  // suggestion chips in both tag editors: free text drifts into "smoke",
  // "Smoke" and "smoke-test" living side by side, and then no single query
  // finds them all. Offering what already exists is the cheap fix.
  const allTags = createMemo(() => {
    const seen = new Map<string, string>();
    for (const src of [...collections(), ...rules()]) {
      for (const tag of src.tags) {
        const key = tag.toLowerCase();
        if (!seen.has(key)) seen.set(key, tag);
      }
    }
    return [...seen.values()].sort((a, b) => a.localeCompare(b));
  });

  // Clicking a chip anywhere means "show me everything with this label".
  // Tags are single tokens by construction (see components/Tags), so this
  // always produces an addressable query.
  const filterByTag = (tag: string) => setRulesFilter(`tag:${tag}`);

  const sameTags = (a: string[], b: string[]) =>
    a.length === b.length && a.every((tag, i) => tag === b[i]);

  /** Close the open tag editor, writing the draft only if it changed. */
  const closeTagging = async () => {
    const id = taggingId();
    if (!id) return;
    const c = collections().find((x) => x.id === id);
    const next = taggingDraft();
    setTaggingId(null);
    if (!c || sameTags(c.tags, next)) return;
    await api.collections.upsert({
      id: c.id,
      name: c.name,
      enabled: c.enabled,
      priority: c.priority,
      tags: next,
    });
    await refresh();
  };

  const toggleTagging = (c: RuleCollectionDto) => {
    if (taggingId() === c.id) {
      void closeTagging();
      return;
    }
    // Opening another collection's editor commits the one already open;
    // nothing is silently thrown away.
    void closeTagging().then(() => {
      setTaggingDraft(c.tags.slice());
      setTaggingId(c.id);
    });
  };

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
        tags: c.tags,
      });
      await refresh();
    }
    setRenamingId(null);
  };

  // `askConfirm`, never `confirm()`: the native one returns false without
  // drawing anything in this webview, so every delete behind it was a
  // guaranteed no-op. See stores/confirm.
  const deleteCollection = async (c: RuleCollectionDto) => {
    const ok = await askConfirm({
      message: tr("rules.delete_collection_confirm", { name: c.name }),
      detail: tr("rules.delete_collection_detail"),
      confirmLabel: tr("common.delete"),
      danger: true,
    });
    if (!ok) return;
    // Surfaced in the load-error banner, not `alert()` — that is dead in this
    // webview for the same reason `confirm()` was (see stores/confirm), so a
    // failed delete would otherwise look exactly like a successful one.
    try {
      await api.collections.delete(c.id);
    } catch (e) {
      setLoadError((e as { message?: string })?.message ?? String(e));
      return;
    }
    await refresh();
  };

  const toggleRule = async (r: RuleDto) => {
    await api.rules.setEnabled(r.id, !isLiveOn(r, device()), device());
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

  // Bulk-toggle every rule in a collection. One call, one transaction — this
  // used to fan out an IPC call per rule because the backend had no batch
  // endpoint; it does now, and the same op backs `pane rules enable --all`.
  const toggleCollection = async (
    collectionId: string | null,
    list: RuleDto[],
    targetEnabled: boolean,
  ) => {
    if (list.every((r) => isLiveOn(r, device()) === targetEnabled)) return;
    await api.rules.setEnabledBulk(
      targetEnabled,
      collectionId === null
        ? { kind: "ungrouped" }
        : { kind: "collection", id: collectionId },
      device(),
    );
    await refresh();
  };

  // Tick or untick every rule in the library, in one call.
  const toggleAllRules = async (targetEnabled: boolean) => {
    if (rules().every((r) => isLiveOn(r, device()) === targetEnabled)) return;
    await api.rules.setEnabledBulk(targetEnabled, { kind: "all" }, device());
    await refresh();
  };

  // With a filter on, the bulk checkboxes act on WHAT IS ON SCREEN.
  //
  // They have to: the backend's bulk selector speaks all / ungrouped /
  // collection and has no "these ids" variant, so the unfiltered path
  // would tick rules the user cannot see while the count next to the
  // box reports the filtered number. That is the same trap the captures
  // context menu had — a control whose label describes one set and
  // whose action hits another.
  //
  // Fanning out per rule is fine here: a filtered subset is small by
  // construction, which is the entire reason someone filtered.
  const setEnabledFor = async (list: RuleDto[], targetEnabled: boolean) => {
    const changed = list.filter((r) => isLiveOn(r, device()) !== targetEnabled);
    if (changed.length === 0) return;
    await Promise.all(
      changed.map((r) => api.rules.setEnabled(r.id, targetEnabled, device())),
    );
    await refresh();
  };

  const toggleAllVisible = (targetEnabled: boolean) =>
    filterActive()
      ? setEnabledFor(visibleRules(), targetEnabled)
      : toggleAllRules(targetEnabled);

  // ── The ticked rules, as a set to act on ──────────────────────────
  //
  // "Selected" here means TICKED. The row checkbox is the one the filter
  // row's "Select all" ticks, and adding a second column of selection
  // checkboxes beside it would make the word mean two different things in
  // one list — the same "two places to look" trap the collection cascade
  // was rejected for.
  //
  // Read through `isLiveOn` on the current device plane, so with a phone
  // picked the set is what those checkboxes are showing as on for THAT
  // phone. Intersected with `visibleRules` for the reason every bulk action
  // here is: a button must never reach past what its count promised.
  const selectedRules = createMemo(() =>
    visibleRules().filter((r) => isLiveOn(r, device())),
  );

  // Same "ticked" reading as `selectedRules`, but over an arbitrary list —
  // lets the tree view's per-collection panel scope bulk actions to just
  // that collection's rules instead of the whole visible library.
  const selectedRulesIn = (list: RuleDto[]) =>
    list.filter((r) => isLiveOn(r, device()));

  const toggleSectionVisible = (
    collectionId: string | null,
    list: RuleDto[],
    targetEnabled: boolean,
  ) =>
    filterActive()
      ? setEnabledFor(list, targetEnabled)
      : toggleCollection(collectionId, list, targetEnabled);

  // Every field of an existing rule as upsert args, so a partial edit (move,
  // duplicate, rename) changes one thing and preserves the rest.
  //
  // `res_body_id` is carried over deliberately. The editor's own Save sends
  // `res_body_id: null` because it always holds the body text and re-uploads
  // it as base64; a partial edit has no such text, and storage reads
  // (res_body_id: null, res_body_base64: null) as "this rule has no body" —
  // which would silently drop the stub's response.
  const upsertArgsFrom = (
    r: RuleDto,
    over: Partial<RuleUpsertArgs>,
  ): RuleUpsertArgs => ({
    id: r.id,
    collection_id: r.collection_id,
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
    // Same trap as res_body_id below: the upsert rewrites the row, so a
    // partial edit that forgets the tags silently unlabels the rule.
    tags: r.tags,
    // Same again for the device scope: dropping these would make a copy of a
    // rule pinned to one phone go live on all of them — a silent widening
    // from a button that says "duplicate", and a silent one from rename and
    // move too.
    enabled_scope: r.enabled_scope,
    devices: r.devices,
    res_status: r.res_status,
    res_headers: r.res_headers,
    res_body_id: r.res_body_id,
    res_body_base64: null,
    res_body_mime: r.res_body_mime,
    res_delay_ms: r.res_delay_ms,
    ...over,
  });

  const moveRule = async (r: RuleDto, collectionId: string | null) => {
    if (r.collection_id === collectionId) return;
    await api.rules.upsert(upsertArgsFrom(r, { collection_id: collectionId }));
    await refresh();
  };

  // Inline rename from the row (double-click on the name). Patched in place
  // rather than refreshed for the same reason `onRuleSaved` is: a refresh
  // replaces every rule object and would remount an editor open elsewhere.
  const renameRule = async (r: RuleDto, name: string) => {
    const trimmed = name.trim();
    if (!trimmed || trimmed === r.name) return;
    const saved = await api.rules.upsert(upsertArgsFrom(r, { name: trimmed }));
    setRules((prev) =>
      prev.map((x) => (x.id === saved.id ? Object.assign(x, saved) : x)),
    );
  };

  // Clone a rule in place: same collection, same everything, name suffixed
  // with " - copy"/" - копия". The stored body is shared by id (storage
  // dedupes bodies by sha256, so referencing res_body_id is safe — no
  // re-upload). Keeping the source priority lands the copy right next to
  // the original in the list.
  const duplicateRule = async (r: RuleDto) => {
    await api.rules.upsert(
      upsertArgsFrom(r, {
        // No id → a fresh row rather than an update of the source.
        id: undefined,
        name: `${r.name || "Unnamed rule"}${t()("rules.copy_suffix")}`,
      }),
    );
    await refresh();
  };

  const removeRule = async (r: RuleDto) => {
    const ok = await askConfirm({
      message: tr("rules.delete_rule_confirm", {
        name: r.name || t()("rules.unnamed_rule"),
      }),
      confirmLabel: tr("common.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.rules.delete(r.id);
    } catch (e) {
      setLoadError((e as { message?: string })?.message ?? String(e));
      return;
    }
    const ed = editing();
    if (ed?.kind === "rule" && ed.id === r.id) setEditing(null);
    // Drop any persisted draft for this rule — its key would otherwise
    // orphan in localStorage forever.
    clearRuleDraft(ruleDraftKey(r.id, r.collection_id));
    await refresh();
  };

  // Confirm, delete, then clean up after the rules that are gone — the shared
  // body of both bulk deletes. `scope` is passed separately from `doomed` so
  // "the whole library" can still go through the cheap `all` selector instead
  // of shipping every id, while the dialog and the cleanup always speak about
  // the exact same list.
  //
  // Collections survive either way: they are grouping, not content (see
  // `delete_rules_bulk` in storage).
  const deleteRuleSet = async (
    doomed: RuleDto[],
    scope: RuleBulkScope,
    confirm: ConfirmRequest,
  ) => {
    if (doomed.length === 0) return;
    if (!(await askConfirm(confirm))) return;
    try {
      await api.rules.deleteBulk(scope);
    } catch (e) {
      setLoadError((e as { message?: string })?.message ?? String(e));
      return;
    }
    // The editor may be sitting on a rule that no longer exists, and each
    // deleted rule may have left a draft key behind.
    const ed = editing();
    if (ed?.kind === "rule" && doomed.some((r) => r.id === ed.id)) {
      setRuleEditorDirty(false);
      setEditing(null);
    }
    for (const r of doomed) clearRuleDraft(ruleDraftKey(r.id, r.collection_id));
    await refresh();
  };

  // Delete exactly the ticked rules.
  //
  // With a device plane picked, the tick means "live on this phone" but the
  // delete is still library-wide — a rule is one row, not one row per device.
  // That gap is spelled out in the dialog rather than left to be discovered.
  // `list` is the scope the caller's checkbox/count promised — the whole
  // visible library from the filter bar, or one collection's rules from
  // the tree view's detail panel.
  const deleteRulesIn = (list: RuleDto[]) => {
    const doomed = selectedRulesIn(list);
    const dev = device();
    return deleteRuleSet(
      doomed,
      { kind: "ids", ids: doomed.map((r) => r.id) },
      {
        message: tr("rules.delete_selected_confirm", { count: String(doomed.length) }),
        detail:
          dev === null
            ? tr("rules.delete_all_detail")
            : `${tr("rules.delete_all_detail")} ${tr(
                "rules.delete_selected_device_detail",
                { name: deviceName(dev) },
              )}`,
        confirmLabel: tr("common.delete"),
        danger: true,
      },
    );
  };
  const deleteSelectedRules = () => deleteRulesIn(visibleRules());

  // ── Tagging the ticked rules ──────────────────────────────────────
  //
  // A draft plus one save, like the collection tag editor above: writing a
  // chip at a time would refresh the list under the input and take the focus
  // with it. `tagSelection` holds the rules snapshotted when the dialog
  // opened, so a checkbox flipped behind it cannot change what gets labelled.
  const [tagSelection, setTagSelection] = createSignal<RuleDto[] | null>(null);
  const [tagSelectionDraft, setTagSelectionDraft] = createSignal<string[]>([]);
  const [tagSelectionRaw, setTagSelectionRaw] = createSignal("");
  const [tagSelectionBusy, setTagSelectionBusy] = createSignal(false);

  const openTagSelectionIn = (list: RuleDto[]) => {
    const ticked = selectedRulesIn(list);
    if (ticked.length === 0) return;
    setTagSelectionDraft([]);
    setTagSelectionRaw("");
    setTagSelection(ticked);
  };
  const openTagSelection = () => openTagSelectionIn(visibleRules());

  const closeTagSelection = () => {
    if (tagSelectionBusy()) return;
    setTagSelection(null);
  };

  const applyTagSelection = async () => {
    const list = tagSelection();
    const tags = tagSelectionDraft();
    // Empty means the user opened the dialog and changed their mind — the
    // backend would happily report `changed: 0`, but doing nothing is clearer.
    if (!list || tags.length === 0 || tagSelectionBusy()) return;
    setTagSelectionBusy(true);
    try {
      await api.rules.addTagsBulk(
        { kind: "ids", ids: list.map((r) => r.id) },
        tags,
      );
      setTagSelection(null);
      await refresh();
    } catch (e) {
      void showMessage({
        message: tr("rules.tag_selected_failed", {
          message: (e as { message?: string })?.message ?? String(e),
        }),
        danger: true,
      });
    } finally {
      setTagSelectionBusy(false);
    }
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
      void showMessage({
        message: tr("rules.export_failed", {
          message: (e as { message?: string })?.message ?? String(e),
        }),
        danger: true,
      });
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
        void showMessage({
          message: tr("rules.import_invalid", { message: validated }),
          danger: true,
        });
        return;
      }
      const summary = await applyImport(validated);
      await refresh();
      void showMessage({
        message: tr("rules.import_success", {
          collections: String(summary.collections),
          rules: String(summary.rules),
        }),
      });
    } catch (e) {
      void showMessage({
        message: tr("rules.import_failed", {
          message: (e as { message?: string })?.message ?? String(e),
        }),
        danger: true,
      });
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

  // ── Tree view ──────────────────────────────────────────────────────
  //
  // A rule being edited always wins the tree's highlight/toolbar target —
  // `rulesEditing` already is that state in both view modes (see
  // stores/rules-ui.ts) — otherwise fall back to whichever collection (or
  // Ungrouped) was last picked. Picking a collection closes any open rule
  // editor via the same dirty-check `guard` every other nav path here uses.
  const treeSelection = createMemo<TreeSelection>(() => {
    const ed = editing();
    if (ed) return { kind: "rule", id: ed.id, collectionId: ed.collectionId };
    const sel = rulesTreeSelectedCollection();
    return sel ? { kind: "collection", id: sel.id } : null;
  });

  const selectTreeCollection = (id: string | null) =>
    guard(() => {
      setEditing(null);
      setRulesTreeSelectedCollection({ id });
    });

  // Recreate the open `RuleEditor` only when the *target* rule changes —
  // switching directly between two tree rows — not when the same rule is
  // re-saved (`onRuleSaved` re-sets `editing` to the same id/collectionId,
  // so this string is unchanged). See `Show ... keyed` in the render below.
  const editingKey = createMemo(() => {
    const ed = editing();
    return ed ? `${ed.collectionId ?? ""}::${ed.id}` : null;
  });
  // Same object reference every recompute (rules are mutated in place on
  // save — see onRuleSaved above), so RuleEditor's `initial` prop never
  // itself forces a remount.
  const editingRule = createMemo(() => {
    const ed = editing();
    if (!ed || ed.id === "new") return null;
    return rules().find((r) => r.id === ed.id) ?? null;
  });

  // The collection (or Ungrouped) the tree has picked, resolved to its
  // rule list — null when nothing is picked or a rule is open instead.
  const selectedCollectionInfo = createMemo(() => {
    const sel = treeSelection();
    if (!sel || sel.kind !== "collection") return null;
    const key = sel.id ?? UNGROUPED_KEY;
    return {
      id: sel.id,
      name: sel.id === null ? t()("rules.ungrouped") : collections().find((c) => c.id === sel.id)?.name ?? "",
      rules: visibleRulesByCollection().get(key) ?? [],
    };
  });
  const panelRules = () => selectedCollectionInfo()?.rules ?? [];
  const panelSelectedCount = () => selectedRulesIn(panelRules()).length;

  const treeToolbarAddRule = () => {
    const sel = treeSelection();
    if (!sel) return;
    startNewRule(sel.kind === "collection" ? sel.id : sel.collectionId);
  };
  const treeToolbarTag = () => {
    const sel = treeSelection();
    if (!sel) return;
    if (sel.kind === "rule") {
      const r = editingRule();
      if (r) openTagSelectionIn([r]);
      return;
    }
    if (sel.id === null) return; // Ungrouped has no tags
    const c = collections().find((cc) => cc.id === sel.id);
    if (c) toggleTagging(c);
  };
  const treeToolbarExport = () => {
    const sel = treeSelection();
    if (!sel) return;
    if (sel.kind === "rule") {
      const r = editingRule();
      if (r) void exportRule(r);
      return;
    }
    if (sel.id === null) return;
    const c = collections().find((cc) => cc.id === sel.id);
    if (c) void exportCollection(c);
  };
  const treeToolbarRename = () => {
    const sel = treeSelection();
    if (!sel || sel.kind === "rule" || sel.id === null) return;
    const c = collections().find((cc) => cc.id === sel.id);
    if (c) startRename(c);
  };
  const treeToolbarDelete = () => {
    const sel = treeSelection();
    if (!sel) return;
    if (sel.kind === "rule") {
      const r = editingRule();
      if (r) void removeRule(r);
      return;
    }
    if (sel.id === null) return;
    const c = collections().find((cc) => cc.id === sel.id);
    if (c) void deleteCollection(c);
  };

  const [treePaneWidth, setTreePaneWidth] = createSignal(loadTreePaneWidth());
  const treeSplitTemplate = createMemo(() => `${treePaneWidth()}px 6px 1fr`);
  createEffect(() => {
    try {
      localStorage.setItem(TREE_PANE_STORAGE_KEY, JSON.stringify(treePaneWidth()));
    } catch {
      /* storage full / disabled */
    }
  });

  // Which map answers "is this section collapsed" depends on whether a
  // filter is on — see `rulesFilterCollapsed` for why the two are separate.
  // Both readers and writers go through this pair so the choice is made in
  // exactly one place.
  const collapseMap = () => (filterActive() ? rulesFilterCollapsed() : collapsed());
  // Resolved eagerly by the caller: the guarded path writes after the user
  // answers the unsaved-changes prompt, and must land in the map that was
  // authoritative when they clicked the chevron.
  const collapseWriter = () =>
    filterActive() ? setRulesFilterCollapsed : setCollapsed;

  const isCollapsed = (k: string) => collapseMap()[k] === true;

  // Collapsing a section that holds the open, dirty editor hides unsaved
  // work — route those through the unsaved-changes prompt.
  const collapseHidesDirtyEditor = (keys: string[]) => {
    const ed = editing();
    if (!ed || !ruleEditorDirty()) return false;
    return keys.includes(ed.collectionId ?? UNGROUPED_KEY);
  };

  const toggleSection = (k: string) => {
    const collapsing = !isCollapsed(k);
    const next = { ...collapseMap(), [k]: collapsing };
    const write = collapseWriter();
    if (collapsing && collapseHidesDirtyEditor([k])) {
      guard(() => write(next));
      return;
    }
    write(next);
  };

  // Every section currently on screen, in render order. Drives the
  // fold-everything button below.
  const sectionKeys = createMemo(() => {
    const keys = visibleCollections().map((c) => c.id);
    if (ungroupedVisible()) keys.push(UNGROUPED_KEY);
    return keys;
  });

  const allSectionsCollapsed = createMemo(
    () => sectionKeys().length > 0 && sectionKeys().every(isCollapsed),
  );

  // One switch for the whole list: everything folded → unfold, otherwise
  // fold. Same reading as the master checkbox on the filter row, and it
  // acts on what is on screen for the same reason — with a filter on, the
  // sections that are not rendered are not the user's current subject.
  const toggleAllSections = () => {
    const target = !allSectionsCollapsed();
    const next = { ...collapseMap() };
    for (const k of sectionKeys()) next[k] = target;
    const write = collapseWriter();
    if (target && collapseHidesDirtyEditor(sectionKeys())) {
      guard(() => write(next));
      return;
    }
    write(next);
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
    // The device banner is a row of its own when it is showing, so the row
    // template has to grow with it — otherwise the list drops out of the `1fr`
    // row and stops being the scrolling one. Both class strings are spelled out
    // literally so Tailwind's scanner still finds them.
    <div
      class={`h-full grid ${
        device() !== null
          ? "grid-rows-[auto_auto_auto_1fr]"
          : "grid-rows-[auto_auto_1fr]"
      }`}
    >
      <header class="border-b border-border bg-bg-subtle px-4 py-3 flex items-center gap-3">
        <Shuffle size={16} class="text-accent" />
        <div>
          <div class="text-sm font-semibold flex items-center gap-1.5">
            {t()("rules.title")}
            <HelpButton path="/rules/" title={t()("rules.help_title")} />
          </div>
        </div>

        {/* Device plane. A native <select> is fine here, unlike the captures
            device menu: that one had to stay in step with a free-text filter
            string and desynced from it, while this is the only writer of the
            signal it reads. Only shown when there is more than one plane to
            choose between, so a one-phone desk sees exactly what it saw
            before. */}
        <Show when={devices().length > 0}>
          <select
            class="text-sm rounded px-2 py-1 border border-border bg-bg hover:bg-bg-muted"
            title={t()("rules.device_plane_title")}
            value={device() ?? ""}
            onChange={(e) => setRulesDevice(e.currentTarget.value || null)}
          >
            <option value="">{t()("rules.device_all")}</option>
            <option value={SCOPE_HOST}>{t()("rules.device_this_mac")}</option>
            <For each={devices()}>
              {(d) => (
                <option
                  value={d.id}
                  disabled={d.platform === "ios"}
                  title={d.platform === "ios" ? t()("rules.device_ios_note") : undefined}
                >
                  {d.display_name}
                  {d.platform === "ios" ? ` — ${t()("rules.device_ios_note")}` : ""}
                </option>
              )}
            </For>
          </select>
        </Show>

        <div class="ml-auto flex items-center gap-2">
          <button
            class="inline-flex items-center gap-1 text-sm rounded px-3 py-1.5 border border-border hover:bg-bg-muted"
            onClick={importFile}
            title={t()("rules.import_title")}
          >
            <Download size={14} /> {t()("rules.import")}
          </button>
          <button
            class="inline-flex items-center gap-1 text-sm rounded px-3 py-1.5 border border-border hover:bg-bg-muted disabled:opacity-40"
            onClick={exportLibrary}
            disabled={rules().length === 0 && collections().length === 0}
            title={t()("rules.export_all_title")}
          >
            <Upload size={14} /> {t()("rules.export_all")}
          </button>
          <button
            class="inline-flex items-center gap-1 text-sm rounded px-3 py-1.5 border border-border hover:bg-bg-muted"
            onClick={() => openCreateForm()}
          >
            <FolderPlus size={14} /> {t()("rules.new_collection")}
          </button>
        </div>
      </header>

      {/* Filter bar. Same anatomy as the captures one (icon, borderless
          input, × that only appears when there is something to clear,
          Escape clears) minus the save-filter star: a rules filter is a
          way to find a row, not a view worth naming and keeping. */}
      {/* px-4, not px-3: the header above and the section list below both
          inset by 16px (`px-4` / `p-4`), so the magnifier's left edge and
          the "Select all" right edge line up with the collection boxes'
          borders instead of hanging 4px outside them. */}
      <Show when={device() !== null}>
        <div class="px-4 py-1.5 text-xs bg-accent/10 border-b border-border text-fg-muted">
          {tr("rules.device_banner", { name: deviceName(device()!) })}
        </div>
      </Show>

      <div class="flex items-center gap-2 px-4 py-2 border-b border-border bg-bg-subtle">
        {/* Tree-view toggle. Leftmost, ahead of the search icon: it picks
            which layout the rest of the row's filter acts on, so it reads
            as the first decision in the row rather than one more control
            at the end of it. */}
        <button
          type="button"
          class={`shrink-0 p-1.5 rounded border ${
            rulesViewMode() === "tree"
              ? "border-accent bg-accent/15 text-accent"
              : "border-border text-fg-muted hover:bg-bg-muted"
          }`}
          title={rulesViewMode() === "tree" ? t()("rules.flat_view_title") : t()("rules.tree_view_title")}
          aria-label={rulesViewMode() === "tree" ? t()("rules.flat_view_title") : t()("rules.tree_view_title")}
          onClick={() => setRulesViewMode(rulesViewMode() === "tree" ? "flat" : "tree")}
        >
          <FolderTree size={14} />
        </button>
        <Search size={14} class="text-fg-muted shrink-0" />
        <div class="flex-1 relative flex items-center">
          <input {...NO_AC}
            ref={(el) => (filterInput = el)}
            class="w-full bg-transparent outline-none text-sm placeholder:text-fg-muted pr-8"
            placeholder={t()("rules.filter_placeholder")}
            title={t()("rules.filter_help")}
            value={rulesFilter()}
            onInput={(e) => setRulesFilter(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && rulesFilter()) {
                e.preventDefault();
                setRulesFilter("");
              }
            }}
          />
          {/* Same construction as the captures × — an `inset-y-0` wrapper
              that centres the button against the input, painted over the
              text on the row's own background, rather than a bare
              `absolute right-0` whose vertical placement falls back to
              the static position. `.trim()` so a field holding only
              spaces still offers the clear. */}
          <Show when={rulesFilter().trim()}>
            <div class="absolute right-0 inset-y-0 flex items-center bg-bg-subtle">
              <button
                type="button"
                class="text-fg-muted hover:text-fg p-1 rounded hover:bg-bg-muted"
                title={t()("rules.clear_filter_title")}
                aria-label={t()("rules.clear_filter")}
                onClick={() => {
                  setRulesFilter("");
                  filterInput?.focus();
                }}
              >
                <X size={14} />
              </button>
            </div>
          </Show>
        </div>
        {/*
          Master checkbox over the library. Same tri-state reading as the
          per-collection one, one level up: all ticked → clears everything,
          otherwise → ticks everything.

          It sits at the end of the filter row, not up in the title bar,
          because the two belong to one thought: the filter picks a set,
          this ticks that set. Reads and writes `visibleRules` for the
          same reason — the box and the count next to it must never
          describe different sets (see setEnabledFor). With no filter on,
          "visible" and "the whole library" are the same thing.
        */}
        {/* Fold/unfold every section on screen. Lives on the filter row, not
            in the title bar, for the same reason the master checkbox does:
            the filter picks the set, these two act on that set. */}
        <Show when={sectionKeys().length > 0}>
          <button
            type="button"
            class="shrink-0 text-fg-muted hover:text-fg p-1 rounded hover:bg-bg-muted"
            title={
              allSectionsCollapsed()
                ? t()("rules.expand_all_title")
                : t()("rules.collapse_all_title")
            }
            aria-label={
              allSectionsCollapsed()
                ? t()("rules.expand_all")
                : t()("rules.collapse_all")
            }
            onClick={toggleAllSections}
          >
            <Show when={allSectionsCollapsed()} fallback={<ChevronsDownUp size={14} />}>
              <ChevronsUpDown size={14} />
            </Show>
          </button>
        </Show>
        <Show when={visibleRules().length > 0}>
          <div class="flex items-center gap-2 shrink-0 pl-1">
            <Checkbox
              state={groupState(visibleRules(), device())}
              title={
                groupState(visibleRules(), device()) === "on"
                  ? t()("rules.uncheck_all_title")
                  : t()("rules.check_all_title")
              }
              onClick={() =>
                void toggleAllVisible(groupState(visibleRules(), device()) !== "on")
              }
            />
            {/* Clickable, like a real <label>: a caption sitting next to a
                checkbox is a click target whether or not we wire it up,
                and one that ignores clicks just reads as broken. */}
            <span
              class="text-xs text-fg-muted cursor-pointer select-none"
              onClick={() =>
                void toggleAllVisible(groupState(visibleRules(), device()) !== "on")
              }
            >
              {t()("rules.select_all")}
            </span>
            {/* The two actions on the ticked set, next to the box that ticks
                it — same reasoning that put the box here rather than in the
                title bar. Always rendered, disabled at zero: a pair of
                buttons that came and went with the tick state would read as
                a glitch, and the title says why they are dead. */}
            <button
              type="button"
              class="shrink-0 inline-flex items-center gap-1 text-xs rounded px-2 py-1 border border-border hover:bg-bg-muted disabled:opacity-40"
              disabled={selectedRules().length === 0}
              title={
                selectedRules().length === 0
                  ? t()("rules.delete_selected_none_title")
                  : t()("rules.tag_selected_title")
              }
              onClick={openTagSelection}
            >
              <TagIcon size={12} />
              {tr("rules.tag_selected", { count: String(selectedRules().length) })}
            </button>
            {/* Tree view has its own scoped delete (the collection panel's
                own "Delete selected" plus the tree toolbar's trash icon
                for the selected node) — this one stays flat-mode only. */}
            <Show when={rulesViewMode() === "flat"}>
              <button
                type="button"
                class="shrink-0 inline-flex items-center gap-1 text-xs rounded px-2 py-1 border border-danger/40 text-danger hover:bg-danger/10 disabled:opacity-40"
                disabled={selectedRules().length === 0}
                title={
                  selectedRules().length === 0
                    ? t()("rules.delete_selected_none_title")
                    : t()("rules.delete_selected_title")
                }
                onClick={() => void deleteSelectedRules()}
              >
                <Trash2 size={12} />
                {tr("rules.delete_selected", {
                  count: String(selectedRules().length),
                })}
              </button>
            </Show>
          </div>
        </Show>
      </div>

      <Show
        when={rulesViewMode() === "flat"}
        fallback={
          <div
            class="grid overflow-hidden"
            style={{ "grid-template-columns": treeSplitTemplate(), "min-height": "0" }}
          >
            <div class="flex flex-col overflow-hidden">
              <RulesTreeToolbar
                selection={treeSelection()}
                isUngroupedCollection={
                  treeSelection()?.kind === "collection" && treeSelection()!.id === null
                }
                ruleIsNew={treeSelection()?.kind === "rule" && editing()?.id === "new"}
                onAddRule={treeToolbarAddRule}
                onTag={treeToolbarTag}
                onExport={treeToolbarExport}
                onRename={treeToolbarRename}
                onDelete={treeToolbarDelete}
              />
              <div class="flex-1 overflow-auto">
                <RulesTree
                  collections={visibleCollections()}
                  rulesByCollection={visibleRulesByCollection()}
                  ungroupedVisible={ungroupedVisible()}
                  selection={treeSelection()}
                  dragDisabled={filterActive()}
                  isCollapsed={isCollapsed}
                  onToggleCollapsed={toggleSection}
                  onSelectCollection={selectTreeCollection}
                  onSelectRule={startEditRule}
                  onToggleCollectionEnabled={(collectionId, list, en) =>
                    toggleSectionVisible(collectionId, list, en)
                  }
                  onToggleRule={toggleRule}
                  renamingId={renamingId()}
                  renamingName={renamingName()}
                  onRenamingNameChange={setRenamingName}
                  onConfirmRename={confirmRename}
                  onCancelRename={() => setRenamingId(null)}
                  onStartRename={startRename}
                  taggingId={taggingId()}
                  taggingDraft={taggingDraft()}
                  onTagDraftChange={setTaggingDraft}
                  onToggleTagging={toggleTagging}
                  allTags={allTags()}
                  draggingRuleId={draggingRuleId()}
                  draggingCollectionId={draggingCollectionId()}
                  dragOverKey={dragOverKey()}
                  onDragStartRule={onDragStartRule}
                  onDragEndRule={onDragEndRule}
                  onDragStartCollection={onDragStartCollection}
                  onDragEndCollection={onDragEndCollection}
                  onDragOverSection={onDragOverSection}
                  onDragLeaveSection={onDragLeaveSection}
                  onDropOnSection={onDropOnSection}
                  onReorderRule={reorderRule}
                  onReorderCollection={reorderCollection}
                />
              </div>
            </div>
            <VerticalResizer
              onResize={(dx) =>
                setTreePaneWidth((w) => Math.min(TREE_PANE_MAX, Math.max(TREE_PANE_MIN, w + dx)))
              }
              onReset={() => setTreePaneWidth(TREE_PANE_DEFAULT)}
            />
            <div class="overflow-y-auto overflow-x-hidden min-w-[240px] border-l border-border">
              <Show
                when={editingKey()}
                keyed
                fallback={
                  <Show
                    when={selectedCollectionInfo()}
                    fallback={
                      <div class="h-full flex items-center justify-center text-sm text-fg-muted">
                        {t()("rules.tree_empty_selection")}
                      </div>
                    }
                  >
                    {(info) => (
                      <CollectionRulesPanel
                        title={info().name}
                        rules={info().rules}
                        dragDisabled={filterActive()}
                        onToggleRule={toggleRule}
                        onRenameRule={(r, name) => void renameRule(r, name)}
                        onPickTag={filterByTag}
                        onCopyRule={duplicateRule}
                        onExportRule={exportRule}
                        onDeleteRule={removeRule}
                        onEditRule={startEditRule}
                        draggingRuleId={draggingRuleId()}
                        onDragStartRule={onDragStartRule}
                        onDragEndRule={onDragEndRule}
                        onReorderRule={reorderRule}
                        selectAllState={groupState(info().rules, device())}
                        selectedCount={panelSelectedCount()}
                        onSelectAll={() =>
                          toggleSectionVisible(
                            info().id,
                            info().rules,
                            groupState(info().rules, device()) !== "on",
                          )
                        }
                        onTagSelected={() => openTagSelectionIn(info().rules)}
                        onDeleteSelected={() => void deleteRulesIn(info().rules)}
                      />
                    )}
                  </Show>
                }
              >
                {(_key) => (
                  <EditorErrorBoundary onClose={cancelEdit}>
                    <RuleEditor
                      initial={editingRule()}
                      defaultCollectionId={editing()!.collectionId}
                      allTags={allTags()}
                      onCancel={cancelEdit}
                      onSaved={onRuleSaved}
                      onLiveSync={syncRuleEnabled}
                    />
                  </EditorErrorBoundary>
                )}
              </Show>
            </div>
          </div>
        }
      >
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
        <For each={visibleCollections()}>
          {(c) => (
            <CollectionSection
              collection={c}
              rules={visibleRulesByCollection().get(c.id) ?? []}
              // While a filter is on this reads the filter's own collapse
              // map, which starts empty (= everything open) and never
              // touches the persisted one — see `rulesFilterCollapsed`.
              collapsed={isCollapsed(c.id)}
              dragDisabled={filterActive()}
              onToggleCollapsed={() => toggleSection(c.id)}
              onRename={() => startRename(c)}
              onDelete={() => deleteCollection(c)}
              onExportCollection={() => exportCollection(c)}
              onExportRule={exportRule}
              onAddRule={() => startNewRule(c.id)}
              onEditRule={startEditRule}
              onRenameRule={(r, name) => void renameRule(r, name)}
              onCopyRule={duplicateRule}
              onReorderRule={reorderRule}
              onReorderCollection={reorderCollection}
              onToggleRule={toggleRule}
              onToggleCollection={(en) =>
                toggleSectionVisible(c.id, visibleRulesByCollection().get(c.id) ?? [], en)
              }
              onDeleteRule={removeRule}
              editing={editing()}
              onSaved={onRuleSaved}
              onCancel={cancelEdit}
              onLiveSync={syncRuleEnabled}
                  onStartRename={startRename}
              allTags={allTags()}
              tagging={taggingId() === c.id}
              tagDraft={taggingDraft()}
              onToggleTagging={() => toggleTagging(c)}
              onTagDraftChange={setTaggingDraft}
              onPickTag={filterByTag}
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

        <Show when={ungroupedVisible()}>
        <CollectionSection
          collection={null}
          rules={visibleRulesByCollection().get(UNGROUPED_KEY) ?? []}
          collapsed={isCollapsed(UNGROUPED_KEY)}
          dragDisabled={filterActive()}
          onToggleCollapsed={() => toggleSection(UNGROUPED_KEY)}
          onExportRule={exportRule}
          onAddRule={() => startNewRule(null)}
          onEditRule={startEditRule}
          onRenameRule={(r, name) => void renameRule(r, name)}
          onCopyRule={duplicateRule}
          onReorderRule={reorderRule}
          onReorderCollection={reorderCollection}
          onToggleRule={toggleRule}
          onToggleCollection={(en) =>
            toggleSectionVisible(
              null,
              visibleRulesByCollection().get(UNGROUPED_KEY) ?? [],
              en,
            )
          }
          onDeleteRule={removeRule}
          editing={editing()}
          onSaved={onRuleSaved}
          onCancel={cancelEdit}
          onLiveSync={syncRuleEnabled}
          onStartRename={startRename}
          allTags={allTags()}
          tagging={false}
          tagDraft={[]}
          onToggleTagging={() => {}}
          onTagDraftChange={() => {}}
          onPickTag={filterByTag}
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
        </Show>

        <Show when={!loading() && rules().length === 0 && collections().length === 0}>
          <div class="text-center text-fg-muted text-sm py-12">
            {t()("rules.empty_state")}
          </div>
        </Show>

        {/* Distinct from the empty library above: the library HAS rules,
            this query just matches none of them. Says so, and offers the
            way out rather than leaving a blank pane. */}
        <Show when={noFilterMatches()}>
          <div class="text-center text-fg-muted text-sm py-12 space-y-2">
            <div>{t()("rules.filter_no_matches", { query: rulesFilter().trim() })}</div>
            <button
              class="text-sm px-3 py-1.5 rounded border border-border hover:bg-bg-muted"
              onClick={() => {
                setRulesFilter("");
                filterInput?.focus();
              }}
            >
              {t()("rules.clear_filter")}
            </button>
          </div>
        </Show>
      </div>
      </Show>

      {/* Bulk tag dialog. Modal rather than a panel at the top of the list:
          the list scrolls, and a control that answers a button in the fixed
          filter row must not be able to open somewhere off screen. */}
      <Show when={tagSelection()}>
        {(list) => (
          <div
            class="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
            onMouseDown={closeTagSelection}
            onKeyDown={(e) => {
              if (e.key === "Escape") closeTagSelection();
            }}
          >
            <div
              class="bg-bg border border-border rounded-lg p-4 shadow-xl w-[30rem] max-w-[90vw] mx-4 space-y-3"
              onMouseDown={(e) => e.stopPropagation()}
            >
              <div class="flex items-center gap-2 text-sm font-medium">
                <TagIcon size={14} class="text-accent shrink-0" />
                {tr("rules.tag_selected_dialog_title", {
                  count: String(list().length),
                })}
              </div>
              <div class="flex">
                <TagEditor
                  tags={tagSelectionDraft()}
                  onChange={setTagSelectionDraft}
                  suggestions={allTags()}
                  onDraftChange={setTagSelectionRaw}
                  autoFocus
                />
              </div>
              <div class="text-fg-muted text-[11px]">
                {t()("rules.tag_selected_hint")}
              </div>
              <div class="flex justify-end gap-2">
                <button
                  type="button"
                  class="text-sm px-3 py-1.5 rounded hover:bg-bg-muted text-fg-muted disabled:opacity-50"
                  disabled={tagSelectionBusy()}
                  onClick={closeTagSelection}
                >
                  {t()("rules.cancel")}
                </button>
                {/* Enabled while there is uncommitted text too — see
                    `onDraftChange` in components/Tags: a disabled button
                    never takes the mousedown that commits it. */}
                <button
                  type="button"
                  class="text-sm px-3 py-1.5 rounded bg-accent text-white hover:opacity-90 disabled:opacity-50"
                  disabled={
                    tagSelectionBusy() ||
                    (tagSelectionDraft().length === 0 && !tagSelectionRaw().trim())
                  }
                  onClick={() => void applyTagSelection()}
                >
                  {t()("rules.tag_selected_apply")}
                </button>
              </div>
            </div>
          </div>
        )}
      </Show>

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
  /** True while a filter is narrowing the list. Reordering is priority
   *  renumbering over a whole section, so it is meaningless — and
   *  destructive to the hidden rules' precedence — on a partial view. */
  dragDisabled?: boolean;
  onToggleCollapsed: () => void;
  onRename?: () => void;
  onDelete?: () => void;
  onAddRule: () => void;
  onEditRule: (r: RuleDto) => void;
  onRenameRule: (r: RuleDto, name: string) => void;
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
  /** Tags in use across the library — suggestion chips in the editor. */
  allTags: string[];
  tagging: boolean;
  /** The draft the open editor is working on; ignored when `tagging` is false. */
  tagDraft: string[];
  onToggleTagging: () => void;
  onTagDraftChange: (tags: string[]) => void;
  onPickTag: (tag: string) => void;
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
        <Show when={!isUngrouped() && !p.dragDisabled}>
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
            state={groupState(p.rules, rulesDevice())}
            title={
              groupState(p.rules, rulesDevice()) === "on"
                ? t()("rules.disable")
                : t()("rules.enable")
            }
            onClick={() =>
              p.onToggleCollection(groupState(p.rules, rulesDevice()) !== "on")
            }
          />
        </Show>

        <Show
          when={isRenaming()}
          fallback={
            <>
              {/* Double-click renames, same as the pencil. A name shown in a
                  list is where people try to rename it first; the button
                  stays for discoverability. Ungrouped is a pseudo-section
                  with no row to rename. */}
              <div
                class={`font-medium text-sm ${isUngrouped() ? "" : "cursor-text"}`}
                title={isUngrouped() ? undefined : t()("rules.rename_dblclick_title")}
                onDblClick={() => {
                  if (!isUngrouped()) p.onRename?.();
                }}
              >
                {p.collection?.name ?? t()("rules.ungrouped")}
              </div>
              <div class="text-xs text-fg-muted">({p.rules.length})</div>
              <Show when={p.collection && !p.tagging ? p.collection : null}>
                {(c) => <TagChips tags={c().tags} onPick={p.onPickTag} />}
              </Show>
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
              class={`text-xs p-1 rounded hover:bg-bg-muted ${
                p.tagging ? "text-accent bg-bg-muted" : "text-fg-muted"
              }`}
              title={t()("rules.tag_collection_title")}
              onClick={() => p.onToggleTagging()}
            >
              <TagIcon size={12} />
            </button>
            <button
              class="text-xs p-1 rounded hover:bg-bg-muted text-fg-muted"
              title={t()("rules.export_collection_title")}
              onClick={p.onExportCollection}
            >
              <Upload size={12} />
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

      {/* Sits outside the collapsed check on purpose: the button that opens
          it lives in the header, and a control that answers by changing
          something hidden reads as broken. */}
      <Show when={p.tagging}>
        <div class="px-3 pb-2 flex items-start gap-2">
          <TagEditor
            tags={p.tagDraft}
            suggestions={p.allTags}
            onChange={p.onTagDraftChange}
          />
          <button
            class="text-xs px-2 py-0.5 rounded bg-accent text-white hover:opacity-90 shrink-0"
            onClick={() => p.onToggleTagging()}
          >
            {t()("rules.save")}
          </button>
        </div>
      </Show>

      <Show when={!p.collapsed}>
        <div class="px-3 pb-3 space-y-2">
          <Show when={editingNewHere()}>
            <EditorErrorBoundary onClose={p.onCancel}>
              <RuleEditor
                initial={null}
                defaultCollectionId={p.collection?.id ?? null}
                allTags={p.allTags}
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
                    dragDisabled={p.dragDisabled}
                    isDragging={p.draggingRuleId === rule.id}
                    onToggle={() => p.onToggleRule(rule)}
                    onEdit={() => p.onEditRule(rule)}
                    onRename={(name) => p.onRenameRule(rule, name)}
                    onPickTag={p.onPickTag}
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
                    allTags={p.allTags}
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
  /** See CollectionSection.dragDisabled — off while a filter is active. */
  dragDisabled?: boolean;
  isDragging: boolean;
  onToggle: () => void;
  onEdit: () => void;
  onRename: (name: string) => void;
  onPickTag: (tag: string) => void;
  onCopy: () => void;
  onExport: () => void;
  onDelete: () => void;
  onDragStart: () => void;
  onDragEnd: () => void;
  onReorder: (draggedId: string, position: "before" | "after") => void;
}> = (p) => {
  // Reads the plane the user is looking at: globally when no device is
  // selected, "live on THIS phone" when one is. Mirrors the engine's own
  // predicate (see lib/rule-liveness) so the list cannot claim a rule is
  // running where the proxy would not serve it.
  const effectivelyOn = () => isLiveOn(p.rule, rulesDevice());
  // Which edge the dragged row would drop against, or null when nothing is
  // hovering this row. Drives the insertion indicator line.
  const [dropEdge, setDropEdge] = createSignal<"before" | "after" | null>(null);
  // Inline rename, opened by double-clicking the name. Local to the row (only
  // one can be open, and it should not survive the row being unmounted) —
  // unlike the collection rename, which the parent owns because its pencil
  // button lives in the same header.
  const [renaming, setRenaming] = createSignal(false);
  const [renameDraft, setRenameDraft] = createSignal("");
  const startRename = () => {
    setRenameDraft(p.rule.name);
    setRenaming(true);
  };
  const confirmRename = () => {
    if (!renaming()) return;
    const next = renameDraft();
    setRenaming(false);
    p.onRename(next);
  };
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
      // Drag off while renaming: in WKWebView a draggable ancestor swallows
      // the text selection inside the input, so the field can be typed into
      // but not selected with the mouse.
      draggable={!p.dragDisabled && !renaming()}
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
          state={effectivelyOn() ? "on" : "off"}
          title={effectivelyOn() ? t()("rules.disable") : t()("rules.enable")}
          onClick={p.onToggle}
        />
      </div>
      <div class="flex-1 min-w-0">
        <div class="flex items-baseline gap-2">
          <Show
            when={renaming()}
            fallback={
              <div
                class="font-medium text-sm truncate cursor-text"
                title={t()("rules.rename_dblclick_title")}
                onDblClick={startRename}
              >
                {p.rule.name || t()("rules.unnamed_rule")}
              </div>
            }
          >
            <input {...NO_AC}
              ref={(el) =>
                setTimeout(() => {
                  el?.focus();
                  el?.select();
                }, 0)
              }
              class="flex-1 min-w-0 bg-bg border border-border rounded px-2 py-0.5 text-sm"
              value={renameDraft()}
              onInput={(e) => setRenameDraft(e.currentTarget.value)}
              // The row is a drag source and the buttons on its right are
              // click targets; keep both from reacting to typing/clicking
              // inside the field.
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => e.stopPropagation()}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  confirmRename();
                }
                if (e.key === "Escape") {
                  e.preventDefault();
                  setRenaming(false);
                }
              }}
              onBlur={confirmRename}
            />
          </Show>
          <Show when={isScoped(p.rule)}>
            <span
              class="shrink-0 text-[10px] uppercase tracking-wide px-1 py-px rounded border border-border text-fg-muted"
              title={tr("rules.badge_scoped_title", {
                count: p.rule.devices.length,
              })}
            >
              {tr("rules.badge_scoped", { count: p.rule.devices.length })}
            </span>
          </Show>
          <TagChips
            tags={p.rule.tags}
            onPick={p.onPickTag}
            dim={!effectivelyOn()}
          />
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
          <Upload size={12} />
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

// ── Tree view ────────────────────────────────────────────────────────────────
//
// Alternative to the accordion list above: a collection/rule tree on the
// left, navigation only. Actions (rename, tag, export, delete, new rule)
// live in the shared `RulesTreeToolbar` above it and act on whichever node
// is selected — a rule editor or a collection's rule list renders in the
// detail panel to the right (see the `rulesViewMode() === "tree"` branch
// in `RulesView`'s render).

export type TreeSelection =
  | { kind: "collection"; id: string | null }
  | { kind: "rule"; id: string; collectionId: string | null }
  | null;

const RulesTreeRuleRow: Component<{
  rule: RuleDto;
  /** Reactive accessor to the enclosing collection's current rule list —
   *  `<For>` reuses this row's DOM/component whenever `rule`'s own object
   *  reference is unchanged (e.g. after an in-place `enabled`/name patch
   *  from an open editor's live-sync, which deliberately mutates in place
   *  rather than replacing the object so the editor itself isn't
   *  remounted). That leaves the frozen `rule` prop's fields stale for a
   *  row that never itself re-renders. Re-deriving the current rule from
   *  this live accessor on every read keeps the checkbox/name in step. */
  liveRules: () => RuleDto[];
  selected: boolean;
  dragDisabled?: boolean;
  isDragging: boolean;
  onSelect: () => void;
  onToggle: () => void;
  onDragStart: () => void;
  onDragEnd: () => void;
  onReorder: (draggedId: string, position: "before" | "after") => void;
}> = (p) => {
  // Deliberately NOT a createMemo: `.find()` returns the same object
  // reference before and after an in-place `enabled`/name mutation, and a
  // memo suppresses notifying its readers when its result is
  // reference-equal to the last one — so it would recompute silently and
  // never actually update the checkbox. Calling this plain function fresh
  // from each JSX read leaves the final on/off comparison to Solid's
  // primitive (boolean/string) diffing instead of an object-identity one.
  const current = () => p.liveRules().find((r) => r.id === p.rule.id) ?? p.rule;
  const effectivelyOn = () => isLiveOn(current(), rulesDevice());
  const [dropEdge, setDropEdge] = createSignal<"before" | "after" | null>(null);
  return (
    <div
      draggable={!p.dragDisabled}
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
        if (!related || !(e.currentTarget as Node).contains(related)) setDropEdge(null);
      }}
      onDrop={(e) => {
        if (!e.dataTransfer?.types.includes("text/plain")) return;
        const edge = dropEdge();
        setDropEdge(null);
        if (p.isDragging || !edge) return;
        e.preventDefault();
        e.stopPropagation();
        const draggedId = e.dataTransfer?.getData("text/plain");
        if (draggedId) p.onReorder(draggedId, edge);
      }}
      class={`relative flex items-center gap-1.5 px-2 py-1 rounded cursor-pointer text-sm ${
        p.selected ? "bg-accent/15 text-accent" : "hover:bg-bg-muted"
      } ${p.isDragging ? "opacity-40" : ""} ${!effectivelyOn() ? "opacity-60" : ""}`}
      onClick={p.onSelect}
    >
      <Show when={dropEdge()}>
        {(edge) => (
          <div
            class={`pointer-events-none absolute inset-x-1 h-0.5 bg-accent rounded ${
              edge() === "before" ? "-top-px" : "-bottom-px"
            }`}
          />
        )}
      </Show>
      <Checkbox
        state={effectivelyOn() ? "on" : "off"}
        title={effectivelyOn() ? t()("rules.disable") : t()("rules.enable")}
        onClick={p.onToggle}
      />
      <div class="truncate flex-1 min-w-0">{current().name || t()("rules.unnamed_rule")}</div>
    </div>
  );
};

const RulesTreeNode: Component<{
  collection: RuleCollectionDto | null;
  rules: RuleDto[];
  collapsed: boolean;
  dragDisabled: boolean;
  selection: TreeSelection;
  onToggleCollapsed: () => void;
  onSelectCollection: () => void;
  onSelectRule: (rule: RuleDto) => void;
  onToggleCollectionEnabled: (enable: boolean) => void;
  onToggleRule: (rule: RuleDto) => void;
  renamingId: string | null;
  renamingName: string;
  onRenamingNameChange: (s: string) => void;
  onConfirmRename: () => void;
  onCancelRename: () => void;
  onStartRename: () => void;
  tagging: boolean;
  tagDraft: string[];
  onTagDraftChange: (tags: string[]) => void;
  onToggleTagging: () => void;
  allTags: string[];
  draggingRuleId: string | null;
  draggingCollectionId: string | null;
  dragOverKey: string | null;
  onDragStartRule: (id: string) => void;
  onDragEndRule: () => void;
  onDragStartCollection: () => void;
  onDragEndCollection: () => void;
  onDragOverSection: () => void;
  onDragLeaveSection: () => void;
  onDropOnSection: (ruleId: string | null) => void;
  onReorderRule: (draggedId: string, targetId: string, position: "before" | "after") => void;
  onReorderCollection: (draggedId: string, targetId: string, position: "before" | "after") => void;
}> = (p) => {
  const isUngrouped = () => p.collection === null;
  const sectionKey = () => p.collection?.id ?? UNGROUPED_KEY;
  const isRenaming = () => p.collection !== null && p.renamingId === p.collection.id;
  const isSelected = () =>
    p.selection?.kind === "collection" && p.selection.id === (p.collection?.id ?? null);
  const isDragOver = () => p.draggingRuleId !== null && p.dragOverKey === sectionKey();
  const [collectionDropEdge, setCollectionDropEdge] = createSignal<"before" | "after" | null>(null);
  const COLLECTION_DND = "application/x-pane-collection";
  const isCollectionDrag = () => p.draggingCollectionId !== null;
  const isBeingDragged = () => p.collection !== null && p.draggingCollectionId === p.collection.id;
  let headerEl: HTMLDivElement | undefined;

  return (
    <div
      class="relative"
      onDragEnter={(e) => {
        e.preventDefault();
        if (!isCollectionDrag()) p.onDragOverSection();
      }}
      onDragOver={(e) => {
        if (!isUngrouped() && isCollectionDrag()) {
          e.preventDefault();
          e.stopPropagation();
          if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
          const rect = e.currentTarget.getBoundingClientRect();
          setCollectionDropEdge(e.clientY < rect.top + rect.height / 2 ? "before" : "after");
          return;
        }
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
        p.onDragOverSection();
      }}
      onDragLeave={(e) => {
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
      <div
        ref={headerEl}
        class={`flex items-center gap-1.5 px-2 py-1 rounded cursor-pointer text-sm ${
          isSelected() ? "bg-accent/15 text-accent" : "hover:bg-bg-muted"
        } ${isDragOver() ? "ring-2 ring-accent/40" : ""} ${isBeingDragged() ? "opacity-40" : ""}`}
        onClick={p.onSelectCollection}
      >
        <Show when={!isUngrouped() && !p.dragDisabled}>
          <div
            draggable={true}
            class="cursor-grab active:cursor-grabbing text-fg-muted hover:text-fg shrink-0"
            title={t()("rules.drag_collection")}
            onDragStart={(e) => {
              if (e.dataTransfer) {
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData(COLLECTION_DND, p.collection!.id);
                if (headerEl) {
                  e.dataTransfer.setDragImage(headerEl, 16, headerEl.offsetHeight / 2);
                }
              }
              p.onDragStartCollection();
            }}
            onDragEnd={() => {
              p.onDragEndCollection();
              setCollectionDropEdge(null);
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <GripVertical size={12} />
          </div>
        </Show>
        <button
          class="text-fg-muted hover:text-fg shrink-0"
          onClick={(e) => {
            e.stopPropagation();
            p.onToggleCollapsed();
          }}
          aria-label={p.collapsed ? t()("rules.expand") : t()("rules.collapse")}
        >
          <Show when={p.collapsed} fallback={<ChevronDown size={12} />}>
            <ChevronRight size={12} />
          </Show>
        </button>
        <Show when={p.rules.length > 0} fallback={<div class="w-[18px] shrink-0" />}>
          <Checkbox
            state={groupState(p.rules, rulesDevice())}
            title={
              groupState(p.rules, rulesDevice()) === "on"
                ? t()("rules.disable")
                : t()("rules.enable")
            }
            onClick={() => p.onToggleCollectionEnabled(groupState(p.rules, rulesDevice()) !== "on")}
          />
        </Show>
        <Show
          when={isRenaming()}
          fallback={
            <>
              <div
                class={`font-medium truncate flex-1 min-w-0 ${isUngrouped() ? "" : "cursor-text"}`}
                title={isUngrouped() ? undefined : t()("rules.rename_dblclick_title")}
                onDblClick={(e) => {
                  e.stopPropagation();
                  if (!isUngrouped()) p.onStartRename();
                }}
              >
                {p.collection?.name ?? t()("rules.ungrouped")}
              </div>
              <div class="text-xs text-fg-muted shrink-0">({p.rules.length})</div>
            </>
          }
        >
          <input {...NO_AC}
            ref={(el) => setTimeout(() => el?.focus(), 0)}
            class="bg-bg border border-border rounded px-1.5 py-0.5 text-sm flex-1 min-w-0"
            value={p.renamingName}
            onClick={(e) => e.stopPropagation()}
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
      </div>

      <Show when={p.tagging}>
        <div class="pl-7 pr-2 pb-1 flex items-start gap-2" onClick={(e) => e.stopPropagation()}>
          <TagEditor tags={p.tagDraft} suggestions={p.allTags} onChange={p.onTagDraftChange} />
          <button
            class="text-xs px-2 py-0.5 rounded bg-accent text-white hover:opacity-90 shrink-0"
            onClick={p.onToggleTagging}
          >
            {t()("rules.save")}
          </button>
        </div>
      </Show>

      <Show when={!p.collapsed}>
        <div class="pl-6 space-y-0.5 pb-0.5">
          <Show when={p.rules.length === 0}>
            <div class="text-xs text-fg-muted italic px-2 py-1">{t()("rules.empty_collection")}</div>
          </Show>
          <For each={p.rules}>
            {(rule) => (
              <RulesTreeRuleRow
                rule={rule}
                liveRules={() => p.rules}
                selected={p.selection?.kind === "rule" && p.selection.id === rule.id}
                dragDisabled={p.dragDisabled}
                isDragging={p.draggingRuleId === rule.id}
                onSelect={() => p.onSelectRule(rule)}
                onToggle={() => p.onToggleRule(rule)}
                onDragStart={() => p.onDragStartRule(rule.id)}
                onDragEnd={p.onDragEndRule}
                onReorder={(draggedId, position) => p.onReorderRule(draggedId, rule.id, position)}
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

const RulesTree: Component<{
  collections: RuleCollectionDto[];
  rulesByCollection: Map<string, RuleDto[]>;
  ungroupedVisible: boolean;
  selection: TreeSelection;
  dragDisabled: boolean;
  isCollapsed: (key: string) => boolean;
  onToggleCollapsed: (key: string) => void;
  onSelectCollection: (id: string | null) => void;
  onSelectRule: (rule: RuleDto) => void;
  onToggleCollectionEnabled: (collectionId: string | null, rules: RuleDto[], enable: boolean) => void;
  onToggleRule: (rule: RuleDto) => void;
  renamingId: string | null;
  renamingName: string;
  onRenamingNameChange: (s: string) => void;
  onConfirmRename: () => void;
  onCancelRename: () => void;
  onStartRename: (c: RuleCollectionDto) => void;
  taggingId: string | null;
  taggingDraft: string[];
  onTagDraftChange: (tags: string[]) => void;
  onToggleTagging: (c: RuleCollectionDto) => void;
  allTags: string[];
  draggingRuleId: string | null;
  draggingCollectionId: string | null;
  dragOverKey: string | null;
  onDragStartRule: (id: string) => void;
  onDragEndRule: () => void;
  onDragStartCollection: (id: string) => void;
  onDragEndCollection: () => void;
  onDragOverSection: (key: string) => void;
  onDragLeaveSection: () => void;
  onDropOnSection: (key: string, ruleId: string | null) => void;
  onReorderRule: (draggedId: string, targetId: string, position: "before" | "after") => void;
  onReorderCollection: (draggedId: string, targetId: string, position: "before" | "after") => void;
}> = (p) => (
  <div class="p-2 space-y-0.5">
    <For each={p.collections}>
      {(c) => (
        <RulesTreeNode
          collection={c}
          rules={p.rulesByCollection.get(c.id) ?? []}
          collapsed={p.isCollapsed(c.id)}
          dragDisabled={p.dragDisabled}
          selection={p.selection}
          onToggleCollapsed={() => p.onToggleCollapsed(c.id)}
          onSelectCollection={() => p.onSelectCollection(c.id)}
          onSelectRule={p.onSelectRule}
          onToggleCollectionEnabled={(en) =>
            p.onToggleCollectionEnabled(c.id, p.rulesByCollection.get(c.id) ?? [], en)
          }
          onToggleRule={p.onToggleRule}
          renamingId={p.renamingId}
          renamingName={p.renamingName}
          onRenamingNameChange={p.onRenamingNameChange}
          onConfirmRename={p.onConfirmRename}
          onCancelRename={p.onCancelRename}
          onStartRename={() => p.onStartRename(c)}
          tagging={p.taggingId === c.id}
          tagDraft={p.taggingDraft}
          onTagDraftChange={p.onTagDraftChange}
          onToggleTagging={() => p.onToggleTagging(c)}
          allTags={p.allTags}
          draggingRuleId={p.draggingRuleId}
          draggingCollectionId={p.draggingCollectionId}
          dragOverKey={p.dragOverKey}
          onDragStartRule={p.onDragStartRule}
          onDragEndRule={p.onDragEndRule}
          onDragStartCollection={() => p.onDragStartCollection(c.id)}
          onDragEndCollection={p.onDragEndCollection}
          onDragOverSection={() => p.onDragOverSection(c.id)}
          onDragLeaveSection={p.onDragLeaveSection}
          onDropOnSection={(rid) => p.onDropOnSection(c.id, rid)}
          onReorderRule={p.onReorderRule}
          onReorderCollection={p.onReorderCollection}
        />
      )}
    </For>
    <Show when={p.ungroupedVisible}>
      <RulesTreeNode
        collection={null}
        rules={p.rulesByCollection.get(UNGROUPED_KEY) ?? []}
        collapsed={p.isCollapsed(UNGROUPED_KEY)}
        dragDisabled={p.dragDisabled}
        selection={p.selection}
        onToggleCollapsed={() => p.onToggleCollapsed(UNGROUPED_KEY)}
        onSelectCollection={() => p.onSelectCollection(null)}
        onSelectRule={p.onSelectRule}
        onToggleCollectionEnabled={(en) =>
          p.onToggleCollectionEnabled(null, p.rulesByCollection.get(UNGROUPED_KEY) ?? [], en)
        }
        onToggleRule={p.onToggleRule}
        renamingId={p.renamingId}
        renamingName={p.renamingName}
        onRenamingNameChange={p.onRenamingNameChange}
        onConfirmRename={p.onConfirmRename}
        onCancelRename={p.onCancelRename}
        onStartRename={() => {}}
        tagging={false}
        tagDraft={[]}
        onTagDraftChange={() => {}}
        onToggleTagging={() => {}}
        allTags={p.allTags}
        draggingRuleId={p.draggingRuleId}
        draggingCollectionId={p.draggingCollectionId}
        dragOverKey={p.dragOverKey}
        onDragStartRule={p.onDragStartRule}
        onDragEndRule={p.onDragEndRule}
        onDragStartCollection={() => {}}
        onDragEndCollection={p.onDragEndCollection}
        onDragOverSection={() => p.onDragOverSection(UNGROUPED_KEY)}
        onDragLeaveSection={p.onDragLeaveSection}
        onDropOnSection={(rid) => p.onDropOnSection(UNGROUPED_KEY, rid)}
        onReorderRule={p.onReorderRule}
        onReorderCollection={p.onReorderCollection}
      />
    </Show>
  </div>
);

// Shared toolbar above the tree — one control surface for both collections
// and rules, replacing the per-row buttons `CollectionSection`'s header
// used to carry. Acts on whatever is selected in the tree.
const RulesTreeToolbar: Component<{
  selection: TreeSelection;
  isUngroupedCollection: boolean;
  /** The selected rule is a not-yet-saved draft ("new") — no id to tag,
   *  export, or delete yet. */
  ruleIsNew: boolean;
  onAddRule: () => void;
  onTag: () => void;
  onExport: () => void;
  onRename: () => void;
  onDelete: () => void;
}> = (p) => {
  const kind = () => p.selection?.kind ?? null;
  // Rename is disabled for a selected rule: the full editor already shows
  // an editable name field, so a second rename control would just be a
  // second place to look for the same action.
  const renameDisabled = () => kind() === null || kind() === "rule" || p.isUngroupedCollection;
  // Tag / export / delete need a saved rule or a real collection: disabled
  // with nothing selected, with Ungrouped selected, or with an unsaved
  // "new" rule draft selected.
  const tagExportDeleteDisabled = () => {
    const k = kind();
    if (k === null) return true;
    if (k === "rule") return p.ruleIsNew;
    return p.isUngroupedCollection;
  };
  return (
    <div class="flex items-center gap-1 px-2 py-1.5 border-b border-border">
      <button
        class="inline-flex items-center gap-1 text-xs rounded px-2 py-1 hover:bg-bg-muted disabled:opacity-40"
        disabled={kind() === null}
        title={t()("rules.tree_add_rule_title")}
        onClick={p.onAddRule}
      >
        <Plus size={12} /> {t()("rules.new_rule")}
      </button>
      <div class="ml-auto flex items-center gap-1">
        <button
          class="text-xs p-1 rounded hover:bg-bg-muted text-fg-muted disabled:opacity-40"
          disabled={tagExportDeleteDisabled()}
          title={
            p.isUngroupedCollection ? t()("rules.tree_no_actions_title") : t()("rules.tree_tag_title")
          }
          onClick={p.onTag}
        >
          <TagIcon size={12} />
        </button>
        <button
          class="text-xs p-1 rounded hover:bg-bg-muted text-fg-muted disabled:opacity-40"
          disabled={tagExportDeleteDisabled()}
          title={
            p.isUngroupedCollection ? t()("rules.tree_no_actions_title") : t()("rules.tree_export_title")
          }
          onClick={p.onExport}
        >
          <Upload size={12} />
        </button>
        <button
          class="text-xs p-1 rounded hover:bg-bg-muted text-fg-muted disabled:opacity-40"
          disabled={renameDisabled()}
          title={
            kind() === "rule"
              ? t()("rules.tree_rename_disabled_title")
              : p.isUngroupedCollection
                ? t()("rules.tree_no_actions_title")
                : t()("rules.tree_rename_title")
          }
          onClick={p.onRename}
        >
          <Pencil size={12} />
        </button>
        <button
          class="text-xs p-1 rounded hover:bg-danger/10 text-danger disabled:opacity-40"
          disabled={tagExportDeleteDisabled()}
          title={
            p.isUngroupedCollection ? t()("rules.tree_no_actions_title") : t()("rules.tree_delete_title")
          }
          onClick={p.onDelete}
        >
          <Trash2 size={12} />
        </button>
      </div>
    </div>
  );
};

// Right-hand detail panel shown when a collection (or Ungrouped) is
// selected in the tree and no rule is open — the same bulk select-all /
// tag-selected / delete-selected row from the flat view's filter bar
// (`1197-1257`), scoped to just this collection's rules instead of the
// whole visible library, plus the same `RuleRow` used everywhere else.
const CollectionRulesPanel: Component<{
  title: string;
  rules: RuleDto[];
  dragDisabled: boolean;
  onToggleRule: (r: RuleDto) => void;
  onRenameRule: (r: RuleDto, name: string) => void;
  onPickTag: (tag: string) => void;
  onCopyRule: (r: RuleDto) => void;
  onExportRule: (r: RuleDto) => void;
  onDeleteRule: (r: RuleDto) => void;
  onEditRule: (r: RuleDto) => void;
  draggingRuleId: string | null;
  onDragStartRule: (id: string) => void;
  onDragEndRule: () => void;
  onReorderRule: (draggedId: string, targetId: string, position: "before" | "after") => void;
  selectAllState: "on" | "off" | "mixed";
  selectedCount: number;
  onSelectAll: () => void;
  onTagSelected: () => void;
  onDeleteSelected: () => void;
}> = (p) => (
  <div class="flex flex-col h-full overflow-hidden">
    <div class="flex items-center gap-2 px-3 py-2 border-b border-border shrink-0">
      <div class="text-sm font-medium truncate flex-1 min-w-0">{p.title}</div>
    </div>
    <Show when={p.rules.length > 0}>
      <div class="flex items-center gap-2 px-3 py-1.5 border-b border-border shrink-0">
        <Checkbox
          state={p.selectAllState}
          title={p.selectAllState === "on" ? t()("rules.uncheck_all_title") : t()("rules.check_all_title")}
          onClick={p.onSelectAll}
        />
        <span
          class="text-xs text-fg-muted cursor-pointer select-none"
          title={t()("rules.tree_select_all_title")}
          onClick={p.onSelectAll}
        >
          {t()("rules.select_all")}
        </span>
        <button
          type="button"
          class="ml-auto shrink-0 inline-flex items-center gap-1 text-xs rounded px-2 py-1 border border-border hover:bg-bg-muted disabled:opacity-40"
          disabled={p.selectedCount === 0}
          title={p.selectedCount === 0 ? t()("rules.delete_selected_none_title") : t()("rules.tag_selected_title")}
          onClick={p.onTagSelected}
        >
          <TagIcon size={12} />
          {tr("rules.tag_selected", { count: String(p.selectedCount) })}
        </button>
        <button
          type="button"
          class="shrink-0 inline-flex items-center gap-1 text-xs rounded px-2 py-1 border border-danger/40 text-danger hover:bg-danger/10 disabled:opacity-40"
          disabled={p.selectedCount === 0}
          title={p.selectedCount === 0 ? t()("rules.delete_selected_none_title") : t()("rules.delete_selected_title")}
          onClick={p.onDeleteSelected}
        >
          <Trash2 size={12} />
          {tr("rules.delete_selected", { count: String(p.selectedCount) })}
        </button>
      </div>
    </Show>
    <div class="flex-1 overflow-auto p-3 space-y-2">
      <Show when={p.rules.length === 0}>
        <div class="text-xs text-fg-muted italic px-2 py-2">{t()("rules.empty_collection")}</div>
      </Show>
      <For each={p.rules}>
        {(rule) => (
          <RuleRow
            rule={rule}
            dragDisabled={p.dragDisabled}
            isDragging={p.draggingRuleId === rule.id}
            onToggle={() => p.onToggleRule(rule)}
            onEdit={() => p.onEditRule(rule)}
            onRename={(name) => p.onRenameRule(rule, name)}
            onPickTag={p.onPickTag}
            onCopy={() => p.onCopyRule(rule)}
            onExport={() => p.onExportRule(rule)}
            onDelete={() => p.onDeleteRule(rule)}
            onDragStart={() => p.onDragStartRule(rule.id)}
            onDragEnd={p.onDragEndRule}
            onReorder={(draggedId, position) => p.onReorderRule(draggedId, rule.id, position)}
          />
        )}
      </For>
    </div>
  </div>
);

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
  tags: string[];
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
  tags: [],
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
  /** Tags in use across the library — suggestion chips under the field. */
  allTags: string[];
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
      // A draft written before tags existed has no `tags` key, and every
      // reader here does list work on it.
      return { ...saved, tags: saved.tags ?? [] };
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
      tags: r.tags.slice(),
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
  // Draft persistence is debounced. `saveRuleDraft` is a JSON.stringify of
  // the whole draft plus a synchronous localStorage write, so its cost is
  // O(response body) per keystroke — with a large JSON body loaded, that
  // alone made typing in the name field visibly lag behind the keyboard.
  // Coalescing to one write per quiet moment makes the cost independent of
  // typing speed.
  //
  // The last keystrokes must not be lost, so the pending write is flushed
  // on unmount; and it must not resurrect a draft the user just discarded,
  // so the flush is conditional on the editor still being dirty (both
  // `save` and the discard path clear that flag before the editor goes).
  const DRAFT_DEBOUNCE_MS = 350;
  let draftTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingDraft: DraftState | null = null;

  const cancelDraftWrite = () => {
    if (draftTimer !== undefined) clearTimeout(draftTimer);
    draftTimer = undefined;
    pendingDraft = null;
  };

  const flushDraftWrite = () => {
    if (draftTimer !== undefined) clearTimeout(draftTimer);
    draftTimer = undefined;
    if (pendingDraft) saveRuleDraft(draftKey, pendingDraft);
    pendingDraft = null;
  };

  const queueDraftWrite = (next: DraftState) => {
    pendingDraft = next;
    if (draftTimer !== undefined) clearTimeout(draftTimer);
    draftTimer = setTimeout(flushDraftWrite, DRAFT_DEBOUNCE_MS);
  };

  const patch = (q: Partial<DraftState>) => {
    const next = { ...d(), ...q };
    setD(next);
    setRuleEditorDirty(true);
    queueDraftWrite(next);
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
      cancelDraftWrite();
      clearRuleDraft(draftKey);
      p.onCancel();
    }
  };

  onCleanup(() => {
    registerEditorSaveFn(null);
    // Dirty ⇒ the user has unsaved keystrokes that the debounce may still
    // be holding; write them. Not dirty ⇒ the draft was just saved or
    // discarded, and writing would put it back.
    if (ruleEditorDirty()) flushDraftWrite();
    else cancelDraftWrite();
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
    queueDraftWrite(next);
    try {
      await api.rules.setEnabled(id, checked, rulesDevice());
      p.onLiveSync?.(id, checked);
    } catch (e: any) {
      setErr(e?.message ?? String(e));
    }
  };

  // The reverse direction: something OTHER than this editor's own toggle
  // flipped `enabled` — the tree/row checkbox, whose click goes through a
  // full `refresh()` rather than the in-place `onLiveSync` patch above, so
  // `p.initial` arrives as a genuinely new object once that refresh lands.
  // In the flat list this never came up: the row this editor replaces is
  // the only other view of the rule, and it's hidden while editing. The
  // tree keeps that row visible the whole time, so without this the two
  // checkboxes could disagree about the same rule. Deliberately narrow —
  // only `enabled`, not a general "adopt every external field" effect —
  // so it never clobbers what the user is mid-typing elsewhere in the draft.
  createEffect(() => {
    const ext = p.initial?.enabled;
    if (ext !== undefined && ext !== d().enabled) {
      setD((prev) => ({ ...prev, enabled: ext }));
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
        // Only ever set for a brand-new rule, and only while a device plane is
        // selected: the banner above says changes affect that phone alone, and
        // a rule created there going live on all four would contradict it. On
        // an existing rule both stay undefined, which means "leave the scope
        // alone" — otherwise editing a status code would unbind the rule from
        // its device.
        enabled_scope: draft.id ? undefined : rulesDevice() ? "set" : undefined,
        devices: draft.id ? undefined : rulesDevice() ? [rulesDevice()!] : undefined,
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
        tags: draft.tags,
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
      cancelDraftWrite();
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

      {/* Tags sit with the name, above Match, because they describe what the
          rule IS, not what it intercepts. */}
      <FieldRow label={t()("rules.tags_label")}>
        <div class="flex-1 space-y-1 min-w-0">
          <TagEditor
            tags={d().tags}
            suggestions={p.allTags}
            onChange={(tags) => patch({ tags })}
          />
          <div class="text-xs text-fg-muted italic">{t()("rules.tags_note")}</div>
        </div>
      </FieldRow>

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
