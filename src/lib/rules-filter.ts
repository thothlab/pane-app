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
 * collection holding it, and the request it matches (method, host
 * glob, path glob, query params).
 *
 * The collection name living in the PER-RULE haystack is what makes
 * "type a group name, get that group" work with no second code path:
 * every rule inside inherits the match.
 *
 * The response side (status, headers, body) is intentionally out. A
 * rule is looked up by what it intercepts, and folding a JSON body
 * template into the haystack makes short queries match huge blobs —
 * noise that would make the filter less trustworthy, not more capable.
 */

import type { RuleDto } from "@/ipc/types";

/** Split a raw query into normalised terms. Empty query → no terms. */
export function parseFilterTerms(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

/** Everything one rule can be found by, lowercased and joined. */
export function ruleHaystack(rule: RuleDto, collectionName: string): string {
  return [
    rule.name,
    collectionName,
    rule.match_method ?? "",
    rule.match_host_glob ?? "",
    rule.match_path_glob ?? "",
    ...rule.match_params.map((q) => `${q.name}=${q.value}`),
  ]
    .join(" ")
    .toLowerCase();
}

/** True when every term appears somewhere in the rule's haystack. */
export function matchesRuleFilter(
  rule: RuleDto,
  collectionName: string,
  terms: string[],
): boolean {
  if (terms.length === 0) return true;
  const hay = ruleHaystack(rule, collectionName);
  return terms.every((term) => hay.includes(term));
}

/** True when every term appears in a collection's own name. Used to keep
 *  an empty group on screen when the user types its name. */
export function matchesCollectionName(name: string, terms: string[]): boolean {
  if (terms.length === 0) return true;
  const lower = name.toLowerCase();
  return terms.every((term) => lower.includes(term));
}
