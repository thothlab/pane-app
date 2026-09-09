/**
 * Query string → rule params for "Add to rules".
 *
 * A captured request carries its query in `url_path`. The path glob can't
 * hold it — the engine splits path from query before globbing, so a glob
 * with a `?…` tail matches nothing and the rule looks ticked but is dead.
 * `match_params` is the slot that actually matches a query pair, so that is
 * where the captured query goes.
 *
 * The decoding mirrors `parse_query`/`percent_decode` in
 * `crates/pane-engine-mitm/src/rules.rs`: the engine decodes the incoming
 * request's pairs and compares them to the rule's stored strings, so the
 * rule has to store the DECODED name and value or an encoded param would
 * never match itself.
 */

import type { RuleParamDto } from "../ipc/types";

/** `+` is a space in a query string, and `%zz` is left as-is rather than
 * throwing — the engine's decoder is equally forgiving. */
function decodeComponent(raw: string): string {
  const plussed = raw.replace(/\+/g, " ");
  try {
    return decodeURIComponent(plussed);
  } catch {
    return plussed;
  }
}

/**
 * Parse a query string into rule params. Takes the part after `?` (a leading
 * `?` is tolerated). Empty segments and nameless pairs are dropped — a param
 * row with a blank name can never match and only clutters the editor.
 */
export function paramsFromQuery(query: string): RuleParamDto[] {
  const q = query.startsWith("?") ? query.slice(1) : query;
  if (q === "") return [];
  const out: RuleParamDto[] = [];
  for (const kv of q.split("&")) {
    if (kv === "") continue;
    const eq = kv.indexOf("=");
    // `flag` with no `=` is a param with an empty value, same as the engine.
    const name = decodeComponent(eq >= 0 ? kv.slice(0, eq) : kv);
    if (name === "") continue;
    out.push({ name, value: decodeComponent(eq >= 0 ? kv.slice(eq + 1) : "") });
  }
  return out;
}

/** Split a captured `url_path` into its path and its query string (without
 * the `?`). The path half is what the rule's path glob gets. */
export function splitPathQuery(urlPath: string): {
  path: string;
  query: string;
} {
  const i = urlPath.indexOf("?");
  if (i < 0) return { path: urlPath, query: "" };
  return { path: urlPath.slice(0, i), query: urlPath.slice(i + 1) };
}
