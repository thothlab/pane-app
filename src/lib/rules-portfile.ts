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
  // Optional — absent in files exported before tags existed, which is the
  // same thing as untagged, so no format version bump.
  tags?: string[];
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
  // Same, for tags.
  tags?: string[];
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
    tags: rule.tags,
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
    tags: c.tags,
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

// ── Readable dumps ──────────────────────────────────────────────────

/**
 * The other shape in circulation: rules nested inside their collection,
 * `match`/`response` as objects, and the response body as literal JSON
 * rather than base64.
 *
 *     { collections: [ { name, enabled, rules: [
 *         { name, enabled, mode, patches,
 *           match:    { method, path, host, req_body, params, conditions },
 *           response: { status, delay_ms, mime, body } } ] } ] }
 *
 * It comes out of forks that serialise the backend DTOs directly, and it
 * is what anyone hand-writing a mock set reaches for — a port file's
 * base64 bodies are effectively uneditable by hand. Accepting it costs
 * one normalisation pass and removes a class of "the file is fine but
 * Pane says it's broken" reports.
 *
 * Everything the port file carries and this shape doesn't is inferred:
 * `priority` from array order (which is the order the author wrote, and
 * the order rules are evaluated in), `res_headers` as empty.
 */
interface ReadableRule {
  name?: unknown;
  enabled?: unknown;
  mode?: unknown;
  patches?: unknown;
  tags?: unknown;
  match?: Record<string, unknown>;
  response?: Record<string, unknown>;
}

function isReadableRule(v: unknown): v is ReadableRule {
  if (!v || typeof v !== "object") return false;
  const r = v as Record<string, unknown>;
  // `match`/`response` as objects is the discriminator: a port-file rule
  // has neither, carrying flat match_* / res_* fields instead.
  return (
    (typeof r.match === "object" && r.match !== null) ||
    (typeof r.response === "object" && r.response !== null)
  );
}

/** Does this payload look like a readable dump rather than a port file? */
export function looksLikeReadableDump(raw: unknown): boolean {
  if (!raw || typeof raw !== "object") return false;
  const o = raw as Record<string, unknown>;
  if (o.format === FORMAT_ID) return false;
  const collections = Array.isArray(o.collections) ? o.collections : [];
  const nested = collections.some(
    (c) =>
      c &&
      typeof c === "object" &&
      Array.isArray((c as Record<string, unknown>).rules),
  );
  const looseRules = Array.isArray(o.rules) && o.rules.some(isReadableRule);
  return nested || looseRules;
}

/**
 * UTF-8 safe base64. `btoa` is Latin-1 only, so Cyrillic mock bodies —
 * which is most of them here — would throw or mangle without the
 * encode-then-widen step. Built one char at a time rather than via
 * `String.fromCharCode(...bytes)`, which blows the argument limit on
 * bodies of any size.
 */
function utf8ToBase64(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

/** JSON text for a body/matcher value, or the string itself if already text. */
function asJsonText(v: unknown): string {
  return typeof v === "string" ? v : JSON.stringify(v, null, 2);
}

function str(v: unknown): string | null {
  return typeof v === "string" && v.length > 0 ? v : null;
}

function arr<T>(v: unknown): T[] {
  return Array.isArray(v) ? (v as T[]) : [];
}

function readableRuleToExported(
  r: ReadableRule,
  collectionRef: string | null,
  priority: number,
): ExportedRule {
  const m = (r.match ?? {}) as Record<string, unknown>;
  const resp = (r.response ?? {}) as Record<string, unknown>;

  // Body: literal JSON is the common case, but accept an already-encoded
  // blob so a half-converted file still imports.
  let bodyBase64: string | undefined;
  if (typeof resp.body_base64 === "string" && resp.body_base64.length > 0) {
    bodyBase64 = resp.body_base64;
  } else if (resp.body !== undefined && resp.body !== null) {
    bodyBase64 = utf8ToBase64(asJsonText(resp.body));
  }

  const reqBody = m.req_body;
  return {
    collection_ref: collectionRef,
    name: typeof r.name === "string" ? r.name : "",
    enabled: r.enabled === true,
    priority,
    mode: r.mode === "patch" ? "patch" : "stub",
    patches: arr(r.patches),
    match_host_glob: str(m.host),
    match_method: str(m.method),
    match_path_glob: str(m.path),
    match_params: arr(m.params),
    match_req_body:
      reqBody === undefined || reqBody === null ? null : asJsonText(reqBody),
    match_conditions: arr(m.conditions),
    tags: arr<string>(r.tags).filter((tg) => typeof tg === "string"),
    res_status: typeof resp.status === "number" ? resp.status : 200,
    res_headers: arr(resp.headers),
    res_body_mime: str(resp.mime),
    res_body_base64: bodyBase64,
    res_delay_ms: typeof resp.delay_ms === "number" ? resp.delay_ms : 0,
  };
}

/**
 * Convert a readable dump into the port file the importer already knows
 * how to apply. Refs are positional (`c0`, `c1`, …) rather than UUIDs —
 * they only ever key the in-memory ref→id map during import.
 */
export function readableDumpToPortFile(raw: unknown): PortFile | string {
  if (!raw || typeof raw !== "object") return "not a JSON object";
  const o = raw as Record<string, unknown>;

  const collections: ExportedCollectionEntry[] = [];
  const rules: ExportedRule[] = [];

  const rawCollections = Array.isArray(o.collections) ? o.collections : [];
  rawCollections.forEach((rawC, ci) => {
    if (!rawC || typeof rawC !== "object") return;
    const c = rawC as Record<string, unknown>;
    const ref = `c${ci}`;
    collections.push({
      ref,
      name: typeof c.name === "string" ? c.name : `Collection ${ci + 1}`,
      enabled: c.enabled === true,
      tags: arr<string>(c.tags).filter((tg) => typeof tg === "string"),
      // Evaluation order is collection priority, then rule priority, both
      // ascending — so array position is exactly the intended precedence.
      priority: ci,
    });
    arr<ReadableRule>(c.rules).forEach((r, ri) => {
      rules.push(readableRuleToExported(r, ref, ri));
    });
  });

  // Rules sitting at the top level are ungrouped, the same way a port
  // file spells `collection_ref: null`.
  arr<ReadableRule>(o.rules).forEach((r, ri) => {
    if (isReadableRule(r)) rules.push(readableRuleToExported(r, null, ri));
  });

  if (rules.length === 0) return "no rules found in this file";

  return {
    format: FORMAT_ID,
    version: FORMAT_VERSION,
    exported_at:
      typeof o.exported_at === "string" ? o.exported_at : nowIso(),
    kind: "library",
    collections,
    rules,
  };
}

/**
 * The importer's entry point: take whatever JSON the user picked and
 * either produce a port file or explain what's wrong.
 *
 * Two accepted shapes — our own port file, and the readable dump above.
 * A file that is neither gets a message naming what it actually looked
 * like, because the old one ("unexpected format: undefined") reads as
 * "your file is corrupt" when the real answer is "this is a different
 * revision of the format".
 */
export function parseImportFile(raw: unknown): PortFile | string {
  if (!raw || typeof raw !== "object") return "not a JSON object";
  const o = raw as Record<string, unknown>;
  if (o.format === FORMAT_ID) return validatePortFile(raw);
  if (looksLikeReadableDump(raw)) return readableDumpToPortFile(raw);
  if (o.format === undefined) {
    return (
      "no `format` field — expected a Pane rules export " +
      `("format": "${FORMAT_ID}") or a readable dump with ` +
      "collections[].rules[]"
    );
  }
  return `unexpected format: ${String(o.format)}`;
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
      tags: c.tags ?? [],
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
      tags: r.tags ?? [],
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
