/**
 * Import / export format for the Rules view — one rule, one
 * collection-plus-its-rules, or the whole library.
 *
 * Goal: a single JSON the user can share with another Pane install.
 * Each rule carries the body inline as base64, so the file is
 * self-contained — no companion blobs to ship alongside.
 *
 * Conflict model on import: every entity gets a fresh UUID. Names are
 * preserved as-is; if a collection or rule with the same name already
 * exists, the import lands beside it as a duplicate (the user can
 * rename or delete). This matches the user's choice — no merge magic,
 * no overwrite prompts.
 */
import { api } from "@/ipc/client";
import type {
  RuleCollectionDto,
  RuleDto,
  RuleUpsertArgs,
} from "@/ipc/types";

/** File-format identifier. Bump `version` only on breaking changes. */
export const FORMAT_ID = "pane-rules";
export const FORMAT_VERSION = 1;

export type ExportKind = "rule" | "collection" | "library";

export interface ExportedCollection {
  name: string;
  enabled: boolean;
  priority: number;
}

export interface ExportedRule {
  // The original collection id from the source library — used as a key
  // to remap into the freshly-created collection id during import.
  // Null means "ungrouped" on import too.
  collection_ref: string | null;
  name: string;
  enabled: boolean;
  priority: number;
  mode: RuleDto["mode"];
  patches: RuleDto["patches"];
  match_host_glob: string | null;
  match_method: string | null;
  match_path_glob: string | null;
  match_params: RuleDto["match_params"];
  match_req_body: string | null;
  // Optional — absent in files exported before conditions existed.
  match_conditions?: RuleDto["match_conditions"];
  res_status: number;
  res_headers: RuleDto["res_headers"];
  res_body_mime: string | null;
  // Body bytes inline. Absent ⇒ no body (e.g. patch-mode rules, or a
  // stub with res_body_id=null).
  res_body_base64?: string;
  res_delay_ms: number;
}

export interface ExportedCollectionEntry extends ExportedCollection {
  // The original id, used by ExportedRule.collection_ref to point at
  // this collection. Not used after import for anything else.
  ref: string;
}

export interface PortFile {
  format: typeof FORMAT_ID;
  version: number;
  exported_at: string;
  kind: ExportKind;
  // Always present on `collection` and `library` exports; on `rule`
  // exports either omitted or empty.
  collections?: ExportedCollectionEntry[];
  rules: ExportedRule[];
}

// ── Fetching bodies ─────────────────────────────────────────────────

/**
 * Pull a rule's response body as base64. Returns null when the rule
 * has no body or the lookup fails (body row vanished, disk gone).
 * Body GC respects `rule.res_body_id` (see clear_captures in
 * pane-storage), so missing-body should be a rare edge.
 *
 * No max_bytes — we want the full payload in the export file. Pane
 * mock bodies are typically small JSON; if a user has stubbed a 50MB
 * binary, the export will be large, but that's their choice.
 */
async function fetchRuleBodyBase64(
  rule: RuleDto,
): Promise<string | undefined> {
  if (!rule.res_body_id) return undefined;
  try {
    const body = await api.captures.body(rule.res_body_id);
    return body.bytes_base64 || undefined;
  } catch {
    return undefined;
  }
}

// ── Build payloads ──────────────────────────────────────────────────

function ruleToExported(
  rule: RuleDto,
  bodyBase64: string | undefined,
): ExportedRule {
  return {
    collection_ref: rule.collection_id,
    name: rule.name,
    enabled: rule.enabled,
    priority: rule.priority,
    mode: rule.mode,
    patches: rule.patches,
    match_host_glob: rule.match_host_glob,
    match_method: rule.match_method,
    match_path_glob: rule.match_path_glob,
    match_params: rule.match_params,
    match_req_body: rule.match_req_body,
    match_conditions: rule.match_conditions,
    res_status: rule.res_status,
    res_headers: rule.res_headers,
    res_body_mime: rule.res_body_mime,
    res_body_base64: bodyBase64,
    res_delay_ms: rule.res_delay_ms,
  };
}

function collectionToExported(c: RuleCollectionDto): ExportedCollectionEntry {
  return {
    ref: c.id,
    name: c.name,
    enabled: c.enabled,
    priority: c.priority,
  };
}

function nowIso(): string {
  // `new Date()` is fine here — this is renderer code, no resume
  // mechanism cares about the timestamp's purity.
  return new Date().toISOString();
}

/** Build a one-rule port file (rule-row Export button). */
export async function buildRuleExport(rule: RuleDto): Promise<PortFile> {
  const body = await fetchRuleBodyBase64(rule);
  return {
    format: FORMAT_ID,
    version: FORMAT_VERSION,
    exported_at: nowIso(),
    kind: "rule",
    rules: [ruleToExported(rule, body)],
  };
}

/** Build a collection-plus-its-rules port file (collection Export). */
export async function buildCollectionExport(
  collection: RuleCollectionDto,
  rulesInCollection: RuleDto[],
): Promise<PortFile> {
  const rules: ExportedRule[] = [];
  for (const r of rulesInCollection) {
    const body = await fetchRuleBodyBase64(r);
    rules.push(ruleToExported(r, body));
  }
  return {
    format: FORMAT_ID,
    version: FORMAT_VERSION,
    exported_at: nowIso(),
    kind: "collection",
    collections: [collectionToExported(collection)],
    rules,
  };
}

/** Build a full-library port file (header "Export all"). */
export async function buildLibraryExport(
  collections: RuleCollectionDto[],
  rules: RuleDto[],
): Promise<PortFile> {
  const exported: ExportedRule[] = [];
  for (const r of rules) {
    const body = await fetchRuleBodyBase64(r);
    exported.push(ruleToExported(r, body));
  }
  return {
    format: FORMAT_ID,
    version: FORMAT_VERSION,
    exported_at: nowIso(),
    kind: "library",
    collections: collections.map(collectionToExported),
    rules: exported,
  };
}

// ── Import ──────────────────────────────────────────────────────────

export interface ImportSummary {
  collections: number;
  rules: number;
}

/**
 * Validate a parsed payload and report what's wrong in human terms.
 * Returns null when the shape passes the gate; otherwise an error
 * message the caller surfaces in an alert.
 *
 * We don't deep-check every field — the backend's `rule_upsert` will
 * reject invalid params at insert time. The check here just covers
 * "is this even our file format."
 */
export function validatePortFile(raw: unknown): PortFile | string {
  if (!raw || typeof raw !== "object") return "not a JSON object";
  const o = raw as Record<string, unknown>;
  if (o.format !== FORMAT_ID) return `unexpected format: ${String(o.format)}`;
  if (typeof o.version !== "number") return "missing version";
  if (o.version > FORMAT_VERSION) {
    return `file version ${o.version} is newer than supported (${FORMAT_VERSION})`;
  }
  if (!Array.isArray(o.rules)) return "missing rules array";
  const kind = o.kind;
  if (kind !== "rule" && kind !== "collection" && kind !== "library") {
    return `unknown kind: ${String(kind)}`;
  }
  if (o.collections !== undefined && !Array.isArray(o.collections)) {
    return "collections must be an array";
  }
  return o as unknown as PortFile;
}

/**
 * Apply a parsed port file to the live database.
 *
 * Always creates new entities (new UUIDs, fresh body rows). Collisions
 * with existing names are tolerated — the imported entry just lands
 * beside the existing one. This is the user-chosen conflict policy.
 *
 * Returns count of created collections and rules. Throws on the first
 * IPC error; partial state is left behind (no rollback) because rule
 * upserts each go through their own SQL transaction and unwinding
 * them in the renderer would require a separate backend command.
 */
export async function applyImport(file: PortFile): Promise<ImportSummary> {
  // Map original collection ref → freshly-created collection id, so
  // ExportedRule.collection_ref points at the right new id.
  const refToId = new Map<string, string>();
  let createdCollections = 0;
  for (const c of file.collections ?? []) {
    const saved = await api.collections.upsert({
      name: c.name,
      enabled: c.enabled,
      priority: c.priority,
    });
    refToId.set(c.ref, saved.id);
    createdCollections++;
  }

  let createdRules = 0;
  for (const r of file.rules) {
    const collectionId =
      r.collection_ref !== null ? (refToId.get(r.collection_ref) ?? null) : null;
    const args: RuleUpsertArgs = {
      name: r.name,
      enabled: r.enabled,
      priority: r.priority,
      collection_id: collectionId,
      mode: r.mode,
      patches: r.patches,
      match_host_glob: r.match_host_glob,
      match_method: r.match_method,
      match_path_glob: r.match_path_glob,
      match_params: r.match_params,
      match_req_body: r.match_req_body ?? null,
      match_conditions: r.match_conditions ?? [],
      res_status: r.res_status,
      res_headers: r.res_headers,
      res_body_base64: r.res_body_base64 ?? null,
      res_body_mime: r.res_body_mime,
      res_delay_ms: r.res_delay_ms,
    };
    await api.rules.upsert(args);
    createdRules++;
  }

  return { collections: createdCollections, rules: createdRules };
}

// ── File-system glue ───────────────────────────────────────────────

/** Filename-safe slug for the dialog `defaultPath`. */
export function slugifyForFilename(name: string): string {
  const trimmed = name.trim().toLowerCase().replace(/[^a-z0-9._-]+/g, "-");
  return trimmed.replace(/^-+|-+$/g, "") || "untitled";
}
