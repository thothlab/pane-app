/**
 * Group hostnames by the domain a person would call "the same site".
 *
 * A device under test produces one entry per subdomain — `acdn.`, `api.`,
 * `as.`, `cdn.`, `id.` of the same bank app — and listing them flat turns a
 * handful of real services into a screenful of near-identical rows.
 *
 * The grouping key is the registrable domain, approximated as the last two
 * labels. That is right for everything Pane sees in practice
 * (`cqohtp.inapps.appsflyersdk.com` → `appsflyersdk.com`,
 * `mymts.apm-proxy.obs.mts.ru` → `mts.ru`) and wrong only under multi-label
 * public suffixes, where `foo.co.uk` would group as `co.uk`. Rather than ship
 * the whole Public Suffix List for a settings panel, the handful of suffixes
 * that actually show up get one extra label. Mis-grouping costs a cosmetically
 * odd heading, never a wrong action: every row still carries its full host and
 * "forget" is always per-host.
 */

/** Two-label suffixes where the registrable domain needs a third label. */
const MULTI_LABEL_SUFFIXES = new Set([
  "co.uk",
  "org.uk",
  "gov.uk",
  "ac.uk",
  "co.jp",
  "com.br",
  "com.au",
  "com.cn",
  "com.tr",
  "com.mx",
  "co.kr",
  "co.in",
  "com.ua",
]);

export function baseDomain(host: string): string {
  const clean = host.trim().toLowerCase().replace(/\.$/, "");
  const labels = clean.split(".").filter(Boolean);
  if (labels.length <= 2) return clean;
  const lastTwo = labels.slice(-2).join(".");
  if (MULTI_LABEL_SUFFIXES.has(lastTwo) && labels.length >= 3) {
    return labels.slice(-3).join(".");
  }
  return lastTwo;
}

export interface HostGroup<T> {
  /** Registrable domain — the heading and the collapse key. */
  domain: string;
  items: T[];
}

/**
 * Bucket items by their host's base domain. Groups are ordered by size
 * (biggest first — that's where the noise is), then alphabetically; items keep
 * the order they arrived in, which is already sorted by host.
 */
export function groupByBaseDomain<T>(
  items: readonly T[],
  hostOf: (item: T) => string,
): HostGroup<T>[] {
  const buckets = new Map<string, T[]>();
  for (const item of items) {
    const key = baseDomain(hostOf(item));
    const bucket = buckets.get(key);
    if (bucket) bucket.push(item);
    else buckets.set(key, [item]);
  }
  return [...buckets.entries()]
    .map(([domain, groupItems]) => ({ domain, items: groupItems }))
    .sort((a, b) => b.items.length - a.items.length || a.domain.localeCompare(b.domain));
}
