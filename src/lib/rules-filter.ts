/**
 * Free-text filter over the rules library.
 *
 * Deliberately not the captures Filter DSL. There the grammar earns
 * itself: the haystack is an unbounded stream of traffic and you narrow
 * it by structured facets (`host:`, `status:`, ranges, negation). Here
 * the library is a few dozen rows the user named themselves, and
 * "orders 500" should just work without anyone learning a syntax.
 *
 * Semantics: whitespace-separated terms, ANDed, case-insensitive
 * substring. One haystack per rule — its own name, the name of the
 * collection holding it, the request it matches (method, host glob,
 * path glob, query params), and its tags.
 *
 * Inside one term, `a,b` is OR — `tag:smoke,ios` finds the rules
 * labelled with either. The comma is borrowed from the captures filter
 * DSL, where it already means exactly this within one key; two filters
 * in one app that spell OR differently is a worse cost than the rule
 * itself. Space still means AND, so `tag:smoke,ios tag:v2` reads as
 * "(smoke or ios) and v2".
 *
 * The one keyed term is `tag:`. It narrows the same substring match to
 * the tags alone, which is what makes a label worth attaching: `smoke`
 * finds a rule whose *name* happens to contain the word too, `tag:smoke`
 * finds exactly the ones labelled with it.
 *
 * The collection name AND the collection's tags live in the PER-RULE
 * haystack — that is what makes "type a group name (or a group's tag),
 * get that group" work with no second code path: every rule inside
 * inherits the match.
 *
 * The response side (status, headers, body) is intentionally out. A
 * rule is looked up by what it intercepts, and folding a JSON body
 * template into the haystack makes short queries match huge blobs —
 * noise that would make the filter less trustworthy, not more capable.
 */

import type { RuleDto } from "@/ipc/types";

/** Prefix that switches a term from "search everything" to "search tags". */
const TAG_PREFIX = "tag:";

/** One parsed term. `key: "tag"` came in as `tag:<value>`.
 *
 *  `values` is an OR-set: `a,b` produces two, and the term matches when
 *  either does. A single value is the ordinary case. */
export interface FilterTerm {
  key: "any" | "tag";
  values: string[];
}

/**
 * The collection side of a rule's identity. Ungrouped rules pass the
 * localized "Ungrouped" label and no tags, so they behave like a group
 * that simply carries no labels.
 */
export interface CollectionContext {
  name: string;
  tags?: string[];
}

/** The OR-set inside one term. Blanks are dropped, so a stray comma cannot
 *  contribute an empty needle that matches every row. */
function splitAlternatives(raw: string): string[] {
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

/** Split a raw query into normalised terms. Empty query → no terms. */
export function parseFilterTerms(query: string): FilterTerm[] {
  const terms: FilterTerm[] = [];
  for (const word of query.toLowerCase().split(/\s+/).filter(Boolean)) {
    if (word.startsWith(TAG_PREFIX)) {
      const values = splitAlternatives(word.slice(TAG_PREFIX.length));
      if (values.length > 0) {
        terms.push({ key: "tag", values });
        continue;
      }
      // A bare `tag:` with nothing after it falls through to a plain term:
      // as a keyed one it would have an empty needle and quietly match every
      // tagged row, which is the opposite of what half-typing should do.
    }
    const values = splitAlternatives(word);
    // A word made of nothing but commas leaves no needle at all. Dropping it
    // is the safe reading — keeping it would match everything.
    if (values.length > 0) terms.push({ key: "any", values });
  }
  return terms;
}

/** Tags of a rule and of its collection, lowercased and joined. */
export function ruleTagHaystack(
  rule: RuleDto,
  collection: CollectionContext,
): string {
  return [...rule.tags, ...(collection.tags ?? [])].join(" ").toLowerCase();
}

/** Everything one rule can be found by, lowercased and joined. */
export function ruleHaystack(
  rule: RuleDto,
  collection: CollectionContext,
): string {
  return [
    rule.name,
    collection.name,
    rule.match_method ?? "",
    rule.match_host_glob ?? "",
    rule.match_path_glob ?? "",
    ...rule.match_params.map((q) => `${q.name}=${q.value}`),
    ...rule.tags,
    ...(collection.tags ?? []),
  ]
    .join(" ")
    .toLowerCase();
}

/** True when every term appears somewhere in the rule's haystack — where a
 *  term with several alternatives needs only one of them to appear. */
export function matchesRuleFilter(
  rule: RuleDto,
  collection: CollectionContext,
  terms: FilterTerm[],
): boolean {
  if (terms.length === 0) return true;
  const hay = ruleHaystack(rule, collection);
  const tagHay = ruleTagHaystack(rule, collection);
  return terms.every((term) => {
    const target = term.key === "tag" ? tagHay : hay;
    return term.values.some((v) => target.includes(v));
  });
}

/** True when every term appears in a collection's own name or tags. Used to
 *  keep an empty group on screen when the user types its name, and by the
 *  captures context menu, whose whole list is collections. */
export function matchesCollection(
  collection: CollectionContext,
  terms: FilterTerm[],
): boolean {
  if (terms.length === 0) return true;
  const tagHay = (collection.tags ?? []).join(" ").toLowerCase();
  const hay = `${collection.name} ${tagHay}`.toLowerCase();
  return terms.every((term) => {
    const target = term.key === "tag" ? tagHay : hay;
    return term.values.some((v) => target.includes(v));
  });
}
