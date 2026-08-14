/**
 * Naming for rules auto-generated from captures.
 *
 * "Add to rules" names a rule `METHOD host/path · YYYY-MM-DD HH:MM`. The
 * stamp only has minute precision, so several captures of the SAME endpoint
 * taken seconds apart produce identical names — and that is exactly the
 * traffic someone multi-selects (a polling endpoint, a retried request).
 * The rows themselves are always distinct (the backend mints a fresh uuid
 * per rule), but a Rules list showing N identical lines is unreadable and
 * looks like the batch misfired.
 */

/** Name budget mirrored from `buildRuleFromCapture`'s own 120-char cap. */
export const RULE_NAME_MAX = 120;

/**
 * Return `name`, or `name (2)`, `name (3)` … if it is already in `used`,
 * and record the result in `used`. The base is trimmed so the suffixed
 * name still fits {@link RULE_NAME_MAX}.
 */
export function uniqueRuleName(name: string, used: Set<string>): string {
  let candidate = name;
  let n = 2;
  while (used.has(candidate)) {
    const suffix = ` (${n})`;
    candidate = `${name.slice(0, RULE_NAME_MAX - suffix.length)}${suffix}`;
    n += 1;
  }
  used.add(candidate);
  return candidate;
}
